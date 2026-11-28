// Runs the pending Drizzle migrations with drizzle-orm's better-sqlite3
// migrator on a copy of the database, or with `base` on a database migrated
// to the migration files at that git revision, records every statement the
// connection executes and scans that SQL with the sqlite-doctor binary.

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";

export const CAPTURE_HEADER = "-- sqlite-doctor capture:";
export const SUPPORTED = /^0\.45\./;
const RESERVED_LINE = /^[ \t\f\r\ufeff]*-- sqlite-doctor:/m;
const BREAKPOINT = "--> statement-breakpoint";
const MAX_BUFFER = 1024 ** 3;

export class CheckError extends Error {}

// drizzle-orm and better-sqlite3 as the project at `root` resolves them.
export function loadRunner(root) {
  const require = createRequire(path.join(root, "package.json"));
  const load = (name) => {
    try {
      return { module: require(name), file: require.resolve(name) };
    } catch (error) {
      if (error.code === "MODULE_NOT_FOUND") throw new CheckError(`${name} is not installed in ${root}`);
      throw error;
    }
  };
  const orm = load("drizzle-orm/better-sqlite3");
  const migrator = load("drizzle-orm/better-sqlite3/migrator");
  const Database = load("better-sqlite3");
  const version = (file, name) => {
    for (let dir = path.dirname(file); dir !== path.dirname(dir); dir = path.dirname(dir)) {
      const manifest = path.join(dir, "package.json");
      if (!fs.existsSync(manifest)) continue;
      const parsed = JSON.parse(fs.readFileSync(manifest, "utf8"));
      if (parsed.name === name) return parsed.version;
    }
    throw new CheckError(`cannot find the version of ${name}`);
  };
  const ormVersion = version(orm.file, "drizzle-orm");
  if (!SUPPORTED.test(ormVersion)) {
    throw new CheckError(`drizzle-orm ${ormVersion} is not supported; this adapter models the 0.45 migrator`);
  }
  // drizzle-kit migrate uses @libsql/client when it can import it, from the
  // project or from drizzle-kit's own dependencies.
  const resolvers = [require];
  const kit = (require.resolve.paths("drizzle-kit") ?? []).map((dir) => path.join(dir, "drizzle-kit", "package.json"))
    .find((manifest) => fs.existsSync(manifest));
  if (kit) resolvers.push(createRequire(fs.realpathSync(kit)));
  const libsql = resolvers.some((resolver) => {
    try {
      resolver.resolve("@libsql/client");
      return true;
    } catch {
      return false;
    }
  });
  return {
    Database: Database.module,
    drizzle: orm.module.drizzle,
    migrate: migrator.module.migrate,
    ormVersion,
    sqliteVersion: version(Database.file, "better-sqlite3"),
    libsql,
  };
}

// The migrations drizzle-orm 0.45 reads: the journal's entries, each file
// split at statement breakpoints.
// `source` is where the files came from, for labels, when `folder` is a
// snapshot of it.
export function readMigrations(folder, source = folder) {
  const journal = path.join(folder, "meta", "_journal.json");
  if (!fs.existsSync(journal)) throw new CheckError(`${path.join(source, "meta", "_journal.json")} does not exist`);
  return JSON.parse(fs.readFileSync(journal, "utf8")).entries.map((entry) => {
    const file = path.join(source, `${entry.tag}.sql`);
    if (!fs.existsSync(path.join(folder, `${entry.tag}.sql`))) {
      throw new CheckError(`${file} is listed in the journal but does not exist`);
    }
    const text = fs.readFileSync(path.join(folder, `${entry.tag}.sql`), "utf8");
    const chunks = [];
    let offset = 0;
    for (const chunk of text.split(BREAKPOINT)) {
      chunks.push({ text: chunk, offset });
      offset += chunk.length + BREAKPOINT.length;
    }
    return { tag: entry.tag, when: entry.when, file, text, chunks };
  });
}

export function quoteIdentifier(name) {
  return `"${name.replaceAll('"', '""')}"`;
}

// The journal and the files it names, copied to `target`, so the migrator
// runs exactly the files that were checked.
export function snapshot(folder, target) {
  const journal = fs.readFileSync(path.join(folder, "meta", "_journal.json"));
  fs.mkdirSync(path.join(target, "meta"), { recursive: true });
  fs.writeFileSync(path.join(target, "meta", "_journal.json"), journal);
  for (const { tag } of JSON.parse(journal.toString("utf8")).entries) {
    const name = path.posix.normalize(`${tag}.sql`);
    if (name.startsWith("../") || path.isAbsolute(name)) throw new CheckError(`the journal names ${tag}, outside the folder`);
    const file = path.join(target, ...name.split("/"));
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.copyFileSync(path.join(folder, name), file);
  }
  return target;
}

// The SQL with comments, strings and quoted names blanked out.
function code(sql) {
  return sql.replace(/--[^\n]*|\/\*[\s\S]*?(\*\/|$)|'(?:[^']|'')*('|$)|"(?:[^"]|"")*("|$)|`(?:[^`]|``)*(`|$)|\[[^\]]*(\]|$)/g, " ");
}

// ATTACH and VACUUM INTO write to files other than the copy, so a migration
// with such a statement is not run.
function refuseOutsideWrites(migrations, relative) {
  const found = migrations.find((migration) => code(migration.text).split(";")
    .some((statement) => /^\s*(ATTACH\b|VACUUM\b[\s\S]*\bINTO\b)/i.test(statement)));
  if (found) {
    throw new CheckError(`${relative(found.file)} has an ATTACH or VACUUM INTO statement, which can write outside the ` +
      "temporary database, so the check does not run it");
  }
}

// The migrations the 0.45 migrator runs: every journal entry newer than the
// newest one recorded in the ledger.
function pendingMigrations(db, migrations, table) {
  const exists = db.prepare("SELECT 1 FROM main.sqlite_schema WHERE type = 'table' AND name = ? COLLATE NOCASE").get(table);
  if (!exists) return migrations;
  const last = db.prepare(`SELECT created_at FROM main.${quoteIdentifier(table)} ORDER BY created_at DESC LIMIT 1`).raw().get();
  return last ? migrations.filter((migration) => Number(last[0]) < migration.when) : migrations;
}

function rowCounts(db) {
  const tables = db.prepare("SELECT name FROM main.sqlite_schema WHERE type = 'table' AND name NOT GLOB 'sqlite_*'").pluck().all();
  return new Map(tables.map((table) => [table, db.prepare(`SELECT count(*) FROM main.${quoteIdentifier(table)}`).pluck().get()]));
}

export function terminate(sql) {
  const body = sql.replace(/;?\s*$/, "");
  return body.includes("--") || body.includes("/*") ? `${body}\n/**/;` : `${body};`;
}

export function singleLine(text) {
  return String(text).replace(/[\u0000-\u001f\u007f\u0085\u2028\u2029]/g, " ");
}

function lineAt(text, offset) {
  let line = 1;
  for (let index = text.indexOf("\n"); index !== -1 && index < offset; index = text.indexOf("\n", index + 1)) line += 1;
  return line;
}

// Migrates `database` with the runner while recording what the connection
// executes. The caller owns the file; it is changed.
export function capture({ runner, database, folder, source = folder, table = "__drizzle_migrations", foreignKeys, root }) {
  const migrations = readMigrations(folder, source);
  const trace = [];
  let recording = false;
  const db = new runner.Database(database, { fileMustExist: true, verbose: (sql) => recording && trace.push({ sql }) });
  // better-sqlite3 logs a statement when it starts running. One that fails
  // to prepare or to bind is never logged, and one that fails while running
  // is the one logged during that call.
  const prepare = db.prepare.bind(db);
  db.prepare = (source) => {
    let statement;
    try {
      statement = prepare(source);
    } catch (error) {
      if (recording) trace.push({ sql: source, failed: true });
      throw error;
    }
    for (const method of ["run", "all", "get"]) {
      const original = statement[method].bind(statement);
      statement[method] = (...args) => {
        const logged = trace.length;
        try {
          return original(...args);
        } catch (error) {
          if (recording) {
            if (trace.length > logged) trace[trace.length - 1].failed = true;
            else trace.push({ sql: source, failed: true });
          }
          throw error;
        }
      };
    }
    return statement;
  };
  try {
    if (foreignKeys !== undefined) db.pragma(`foreign_keys = ${foreignKeys ? "ON" : "OFF"}`);
    const schema = db
      .prepare("SELECT sql FROM main.sqlite_schema WHERE sql IS NOT NULL AND name NOT GLOB 'sqlite_*' ORDER BY rowid")
      .pluck()
      .all();
    const fkState = db.pragma("foreign_keys", { simple: true });
    const pending = pendingMigrations(db, migrations, table);
    const relative = (file) => path.relative(root, file) || file;
    refuseOutsideWrites(pending, relative);
    const before = rowCounts(db);
    let error = null;
    recording = true;
    try {
      runner.migrate(runner.drizzle(db), { migrationsFolder: folder, migrationsTable: table });
    } catch (caught) {
      error = caught;
    } finally {
      recording = false;
    }
    const after = rowCounts(db);
    const attached = db.pragma("database_list").filter((entry) => !["main", "temp"].includes(entry.name));
    const shrunk = [...before].filter(([name, count]) => after.has(name) && after.get(name) < count)
      .map(([name, count]) => `${name}: ${count} -> ${after.get(name)}`);
    const { statements, incomplete } = attribute(trace, pending, table, error, relative);
    if (error) incomplete.push(`the migration raised ${singleLine(error.name || error.constructor.name)}`);
    if (attached.length) incomplete.push("a migration attached another database");
    if ([...schema, ...statements.map((statement) => statement.sql)].some((sql) => RESERVED_LINE.test(sql))) {
      incomplete.push("copied SQL has a line that starts with `-- sqlite-doctor:`");
    }
    return {
      header: `${CAPTURE_HEADER} drizzle-orm ${runner.ormVersion}, better-sqlite3 ${runner.sqliteVersion}`,
      schema,
      foreignKeys: fkState ? "ON" : "OFF",
      statements,
      pending: error ? null : pending.map((migration) => relative(migration.file)),
      shrunk,
      error,
      incomplete,
    };
  } finally {
    db.close();
  }
}

// Labels each traced statement with the file and line it came from. The
// migrator runs every chunk of each pending file in order and then records
// the file in the ledger, all between its BEGIN and COMMIT. Anything else
// makes the check incomplete.
function attribute(trace, pending, table, error, relative) {
  const ledgerInsert = `INSERT INTO ${quoteIdentifier(table)} ("hash", "created_at") VALUES('`;
  const statements = [];
  const incomplete = new Set();
  let index = 0;
  let position = 0;
  let inTransaction = false;
  let prologue = 0;
  let committed = false;
  // SQLite keeps a statement's text up to its end, so only whitespace and
  // comments may follow it in the chunk.
  const runs = (chunk, sql) => sql.trim().length && chunk.text.startsWith(sql) &&
    /^(?:\s|--[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\/)*$/.test(chunk.text.slice(sql.length));
  for (const { sql, failed } of trace) {
    const migration = pending[index];
    const chunk = migration?.chunks[position];
    if (!inTransaction && !committed) {
      statements.push({ sql, label: { synthetic: "Drizzle migrator" }, failed });
      if (/^\s*BEGIN\s*$/i.test(sql)) inTransaction = true;
      else prologue += 1;
    } else if (inTransaction && chunk && runs(chunk, sql)) {
      const lead = sql.match(/^\s*/)[0].length;
      const line = lineAt(migration.text, chunk.offset + lead);
      statements.push({ sql: sql.slice(lead), label: { file: relative(migration.file), line }, failed });
      position += 1;
    } else if (inTransaction && migration && sql.startsWith(ledgerInsert)) {
      const [, hash, when] = sql.slice(ledgerInsert.length - 1).match(/^'([0-9a-f]*)'(?:\/\*[^*]*\*\/)?, ([^,()]+)\)$/) ?? [];
      const digest = crypto.createHash("sha256").update(migration.text).digest("hex");
      if (position !== migration.chunks.length || Number(when) !== migration.when || !hash || !digest.startsWith(hash)) {
        incomplete.add(`the statements recorded for ${relative(migration.file)} do not match the file`);
      }
      statements.push({ sql, label: { synthetic: "Drizzle migrator" }, failed });
      index += 1;
      position = 0;
    } else if (inTransaction && !migration && /^\s*COMMIT\s*$/i.test(sql)) {
      statements.push({ sql, label: { synthetic: "Drizzle migrator" }, failed });
      inTransaction = false;
      committed = true;
    } else if (error && /^\s*ROLLBACK\s*$/i.test(sql)) {
      statements.push({ sql, label: { synthetic: "Drizzle migrator" }, failed });
      inTransaction = false;
    } else {
      statements.push({ sql, label: migration ? { migration: relative(migration.file) } : { synthetic: "unknown" }, failed });
      incomplete.add("the migrator ran a statement that is not in the migration files");
    }
  }
  if (!error && (index !== pending.length || prologue !== 2 || !committed)) {
    incomplete.add("the recorded statements do not match the migrator's steps");
  }
  return { statements, incomplete: [...incomplete] };
}

export function render(result) {
  const lines = [result.header, "-- Migration target: temporary database."];
  if (result.error) lines.push(`-- Migration failed: ${singleLine(`${result.error.name}: ${result.error.message}`)}`);
  for (const reason of result.incomplete) lines.push(`-- sqlite-doctor:incomplete ${singleLine(reason)}`);
  for (const line of result.shrunk) lines.push(`-- Observed on the copy: rows in ${singleLine(line)}`);
  lines.push("", "-- Baseline schema", "-- sqlite-doctor:migration schema before migrating");
  for (const sql of result.schema) lines.push(terminate(sql));
  lines.push("", "-- Connection state before migrating", "-- sqlite-doctor:synthetic connection state before migrating");
  lines.push(`PRAGMA foreign_keys = ${result.foreignKeys};`, "", "-- Executed statements");
  let current = null;
  for (const { sql, label, failed } of result.statements) {
    let directive;
    if (label.file) directive = `-- sqlite-doctor:file ${label.line} ${singleLine(label.file)}`;
    else if (label.migration) directive = `-- sqlite-doctor:migration ${singleLine(label.migration)}`;
    else directive = `-- sqlite-doctor:synthetic ${label.synthetic}`;
    if (directive !== current || label.file) lines.push(directive);
    current = directive;
    lines.push(failed ? `-- failed: ${sql.replace(/\s+/g, " ")}` : terminate(sql));
  }
  return `${lines.join("\n")}\n`;
}

// Writes the capture through a new file in the same directory, and only
// replaces a regular file that holds an earlier capture.
export function writeCapture(output, content) {
  const stat = fs.lstatSync(output, { throwIfNoEntry: false });
  if (stat) {
    if (stat.isSymbolicLink()) throw new CheckError(`refusing to write the capture over ${output}: it is a symbolic link`);
    const head = stat.isFile() ? fs.readFileSync(output, "utf8").slice(0, CAPTURE_HEADER.length) : "";
    if (head !== CAPTURE_HEADER) throw new CheckError(`refusing to write the capture over ${output}: it is not an earlier capture`);
  }
  const staging = path.join(path.dirname(output), `.${path.basename(output)}.${crypto.randomUUID()}.tmp`);
  fs.writeFileSync(staging, content, { flag: "wx", mode: 0o600 });
  try {
    if (stat) {
      fs.renameSync(staging, output);
    } else {
      // A link fails if anything appeared at the name in the meantime.
      try {
        fs.linkSync(staging, output);
      } catch (error) {
        if (error.code === "EEXIST") throw new CheckError(`refusing to write the capture over ${output}: it appeared while writing`);
        throw error;
      }
    }
  } finally {
    fs.rmSync(staging, { force: true });
  }
}

// Paths are passed as literal pathspecs whatever the environment says.
const GIT_ENV = Object.fromEntries(Object.entries(process.env)
  .filter(([name]) => !/^GIT_(LITERAL|GLOB|NOGLOB|ICASE)_PATHSPECS$/.test(name)));
GIT_ENV.GIT_LITERAL_PATHSPECS = "1";

function git(directory, ...args) {
  const result = spawnSync("git", ["-C", directory, ...args], { encoding: "buffer", maxBuffer: MAX_BUFFER, env: GIT_ENV });
  if (result.error) throw new CheckError(`git ${args[0]} failed: ${result.error.message}`);
  if (result.status !== 0) throw new CheckError(`git ${args[0]} failed: ${result.stderr.toString().trim()}`);
  return result.stdout;
}

// The tree entry at `name` in `commit`, or null.
function treeEntry(top, commit, name) {
  const listing = git(top, "ls-tree", "-z", "--full-tree", commit, "--", name).toString("utf8");
  const entry = listing.split("\0").find(Boolean);
  if (!entry) return null;
  const tab = entry.indexOf("\t");
  const [mode, type, object] = entry.slice(0, tab).split(" ");
  return entry.slice(tab + 1) === name ? { mode, type, object } : null;
}

// The migrations folder as it is at `base`, the journal and the files it
// names written under `dir`, or null if the folder has no journal there.
export function folderAt(base, folder, dir) {
  if (!base || base.startsWith("-") || /[\u0000-\u001f\u007f]/.test(base)) throw new CheckError("BASE must name a git revision");
  const absolute = path.resolve(folder);
  const top = git(absolute, "rev-parse", "--show-toplevel").toString().trim();
  // The folder's own name as given, so a folder that is a symbolic link now
  // is looked up under that name; the directory holding it as stored on disk.
  const parent = fs.realpathSync.native(path.dirname(absolute));
  const entries = fs.readdirSync(parent);
  const own = entries.find((name) => name === path.basename(absolute)) ??
    entries.find((name) => name.toLowerCase() === path.basename(absolute).toLowerCase()) ?? path.basename(absolute);
  const relative = path.relative(fs.realpathSync.native(top), path.join(parent, own)).split(path.sep).join("/");
  if (relative === ".." || relative.startsWith("../")) throw new CheckError(`${folder} is outside the git repository`);
  const commit = git(top, "rev-parse", "--verify", "--end-of-options", `${base}^{commit}`).toString().trim();
  // A missing entry under a symbolic link in the commit is not an absence.
  const linked = (name) => {
    const parts = name.split("/");
    for (let k = 1; k < parts.length; k += 1) {
      const entry = treeEntry(top, commit, parts.slice(0, k).join("/"));
      if (entry?.mode === "120000") {
        throw new CheckError(`${parts.slice(0, k).join("/")} is a symbolic link at ${base}, which the check does not follow`);
      }
      if (!entry) return;
    }
  };
  const blob = (name) => {
    const entry = treeEntry(top, commit, name);
    if (!entry) {
      linked(name);
      return null;
    }
    if (entry.mode === "120000") throw new CheckError(`${name} is a symbolic link at ${base}, which the check does not follow`);
    if (entry.type !== "blob") throw new CheckError(`${name} is not a file at ${base}`);
    return git(top, "cat-file", "blob", entry.object);
  };
  if (relative) {
    const entry = treeEntry(top, commit, relative);
    if (!entry) {
      linked(relative);
      return null;
    }
    if (entry.mode === "120000") throw new CheckError(`${relative} is a symbolic link at ${base}, which the check does not follow`);
    if (entry.type !== "tree") throw new CheckError(`${relative} is not a directory at ${base}`);
  }
  const prefix = relative ? `${relative}/` : "";
  const journal = blob(`${prefix}meta/_journal.json`);
  if (!journal) return null;
  const target = path.join(dir, "base-migrations");
  fs.mkdirSync(path.join(target, "meta"), { recursive: true });
  fs.writeFileSync(path.join(target, "meta", "_journal.json"), journal);
  for (const { tag } of JSON.parse(journal.toString("utf8")).entries) {
    const name = path.posix.normalize(`${tag}.sql`);
    if (name.startsWith("../") || path.posix.isAbsolute(name)) throw new CheckError(`the journal at ${base} names ${tag}, outside the folder`);
    const content = blob(`${prefix}${name}`);
    if (!content) throw new CheckError(`${prefix}${name} is listed in the journal at ${base} but does not exist`);
    const file = path.join(target, ...name.split("/"));
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, content);
  }
  return target;
}

export function findBinary(binary = process.env.SQLITE_DOCTOR) {
  const executable = (file) => {
    try {
      return fs.statSync(file).isFile() && (fs.accessSync(file, fs.constants.X_OK), true);
    } catch {
      return false;
    }
  };
  if (binary) {
    if (executable(binary)) return path.resolve(binary);
    throw new CheckError(`SQLITE_DOCTOR=${binary} is not an executable file`);
  }
  const found = (process.env.PATH ?? "").split(path.delimiter).map((dir) => path.join(dir, "sqlite-doctor")).find(executable);
  if (!found) {
    throw new CheckError("the sqlite-doctor binary is not on PATH; install it with `cargo install sqlite-doctor`, the " +
      "kholdrex/sqlite-doctor GitHub Action or a release archive, or set SQLITE_DOCTOR");
  }
  return found;
}

export function report(binary, result, output, print, shown = output) {
  const run = spawnSync(binary, ["--format", "json", output], { encoding: "utf8", maxBuffer: MAX_BUFFER });
  if (run.error) throw new CheckError(`${binary} could not be run: ${run.error.message}`);
  let parsed = null;
  try {
    parsed = JSON.parse(run.stdout);
  } catch {}
  if (!parsed || typeof parsed !== "object" || !("complete" in parsed)) {
    const detail = run.stderr?.trim() ? `: ${run.stderr.trim()}` : "";
    throw new CheckError(`${binary} did not produce sqlite-doctor 0.3.0 JSON output; 0.3.0 or newer is needed${detail}`);
  }
  if (result.pending === null) print("The pending migrations could not be determined.");
  else if (!result.pending.length) print("No pending migrations.");
  else print(`${result.pending.length} pending migration(s):\n${result.pending.map((name) => `  ${name}`).join("\n")}`);
  for (const finding of parsed.findings) {
    const where = finding.location.line ? `${finding.location.source}:${finding.location.line}` : finding.location.source;
    print(`\n${finding.code}: ${finding.title}\n  --> ${where} (${shown}:${finding.line})\n  ${finding.detail}\n  help: ${finding.remediation}`);
  }
  for (const line of result.shrunk) print(`Rows lost on the copy: ${line}`);
  if (!parsed.complete) print(`\nCheck incomplete:\n${parsed.incomplete.map((item) => `- ${item.reason}`).join("\n")}`);
  if (result.error) {
    print(`Migration failed on the copy: ${result.error.name}. Its message can quote SQL with data, so it is only in ` +
      "the capture: rerun with --output <file> to keep it.");
  }
  if (run.status === null || run.status > 1 || !parsed.complete || result.error || result.incomplete.length) return 2;
  return run.status;
}

// The whole check. Returns the exit status: 0 nothing found, 1 findings, 2 an
// incomplete check or an error.
export async function check(options, print = console.log) {
  const root = path.resolve(options.root ?? process.cwd());
  const folder = path.resolve(root, options.migrations ?? "drizzle");
  const binary = findBinary(options.binary);
  const runner = options.runner ?? loadRunner(root);
  if (runner.libsql && options.driver !== "better-sqlite3") {
    throw new CheckError("@libsql/client is installed, so `drizzle-kit migrate` runs the migrations with it, and this " +
      "check models better-sqlite3; pass --driver better-sqlite3 if better-sqlite3 runs your migrations");
  }
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sqlite-doctor-drizzle-"));
  try {
    const database = path.join(dir, "database.sqlite3");
    if (!fs.existsSync(path.join(folder, "meta", "_journal.json"))) {
      throw new CheckError(`${path.join(folder, "meta", "_journal.json")} does not exist`);
    }
    const migrations = snapshot(folder, path.join(dir, "migrations"));
    if (options.base) {
      const baseFolder = folderAt(options.base, folder, dir);
      const db = new runner.Database(database);
      try {
        if (options.foreignKeys !== undefined) db.pragma(`foreign_keys = ${options.foreignKeys ? "ON" : "OFF"}`);
        if (baseFolder) {
          refuseOutsideWrites(readMigrations(baseFolder), (file) => `${path.relative(baseFolder, file)} at ${options.base}`);
          runner.migrate(runner.drizzle(db), { migrationsFolder: baseFolder, migrationsTable: options.table });
        }
      } finally {
        db.close();
      }
      print(`Checking against the migrations at ${options.base} (no rows). A migration whose SQL depends on rows is not covered.`);
    } else {
      if (!options.database) throw new CheckError("pass --database <file>, or --base <git revision>");
      const source = path.resolve(root, options.database);
      if (options.database.startsWith("file:") || options.database === ":memory:" || !fs.existsSync(source)) {
        throw new CheckError(`${options.database} is not an existing database file`);
      }
      const db = new runner.Database(source, { readonly: true, fileMustExist: true });
      try {
        await db.backup(database);
      } finally {
        db.close();
      }
    }
    const result = capture({
      runner,
      database,
      folder: migrations,
      source: folder,
      table: options.table,
      foreignKeys: options.foreignKeys,
      root,
    });
    // The binary scans this run's own file; --output gets a copy.
    const content = render(result);
    const scanned = path.join(dir, "capture.sql");
    fs.writeFileSync(scanned, content, { flag: "wx", mode: 0o600 });
    const output = options.output && path.resolve(root, options.output);
    if (output) writeCapture(output, content);
    return report(binary, result, scanned, print, output ? path.relative(root, output) : scanned);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

// Runs the pending Drizzle migrations with drizzle-orm's better-sqlite3
// migrator on a copy of the database, or with `base` on a database migrated
// to the migration files at that git revision, records every statement the
// connection executes and scans that SQL with the sqlite-doctor binary.

import { execFileSync, spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";

export const CAPTURE_HEADER = "-- sqlite-doctor capture:";
export const SUPPORTED = /^0\.45\./;
const RESERVED_LINE = /^[ \t\f\r\ufeff]*-- sqlite-doctor:/m;
const BREAKPOINT = "--> statement-breakpoint";

export class CheckError extends Error {}

// drizzle-orm and better-sqlite3 from the project, or else from this package.
export function loadRunner(root) {
  const requires = [createRequire(path.join(root, "package.json")), createRequire(import.meta.url)];
  const load = (name) => {
    for (const require of requires) {
      try {
        return { module: require(name), file: require.resolve(name) };
      } catch (error) {
        if (error.code !== "MODULE_NOT_FOUND") throw error;
      }
    }
    throw new CheckError(`${name} is not installed in ${root}`);
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
  let libsql = true;
  try {
    requires[0].resolve("@libsql/client");
  } catch {
    libsql = false;
  }
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
export function readMigrations(folder) {
  const journal = path.join(folder, "meta", "_journal.json");
  if (!fs.existsSync(journal)) throw new CheckError(`${journal} does not exist`);
  return JSON.parse(fs.readFileSync(journal, "utf8")).entries.map((entry) => {
    const file = path.join(folder, `${entry.tag}.sql`);
    if (!fs.existsSync(file)) throw new CheckError(`${file} is listed in the journal but does not exist`);
    const text = fs.readFileSync(file, "utf8");
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

// The migrations the 0.45 migrator runs: every journal entry newer than the
// newest one recorded in the ledger.
function pendingMigrations(db, migrations, table) {
  const exists = db.prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?").get(table);
  if (!exists) return migrations;
  const last = db.prepare(`SELECT created_at FROM ${quoteIdentifier(table)} ORDER BY created_at DESC LIMIT 1`).raw().get();
  return last ? migrations.filter((migration) => Number(last[0]) < migration.when) : migrations;
}

function rowCounts(db) {
  const tables = db.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT GLOB 'sqlite_*'").pluck().all();
  return new Map(tables.map((table) => [table, db.prepare(`SELECT count(*) FROM ${quoteIdentifier(table)}`).pluck().get()]));
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
export function capture({ runner, database, folder, table = "__drizzle_migrations", foreignKeys, root }) {
  const migrations = readMigrations(folder);
  const trace = [];
  let recording = false;
  const db = new runner.Database(database, { fileMustExist: true, verbose: (sql) => recording && trace.push({ sql }) });
  // better-sqlite3 logs a statement when it runs, so one that fails to
  // prepare is recorded here.
  const prepare = db.prepare.bind(db);
  db.prepare = (source) => {
    try {
      return prepare(source);
    } catch (error) {
      if (recording) trace.push({ sql: source, failed: true });
      throw error;
    }
  };
  try {
    if (foreignKeys !== undefined) db.pragma(`foreign_keys = ${foreignKeys ? "ON" : "OFF"}`);
    const schema = db
      .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT GLOB 'sqlite_*' ORDER BY rowid")
      .pluck()
      .all();
    const fkState = db.pragma("foreign_keys", { simple: true });
    const pending = pendingMigrations(db, migrations, table);
    const before = rowCounts(db);
    let error = null;
    recording = true;
    try {
      runner.migrate(runner.drizzle(db), { migrationsFolder: folder, migrationsTable: table });
    } catch (caught) {
      error = caught;
      // A statement that prepared and then failed is the last one logged
      // before the migrator's ROLLBACK.
      if (!trace.some((entry) => entry.failed)) {
        const last = trace.length - (/^\s*ROLLBACK\s*$/i.test(trace.at(-1)?.sql ?? "") ? 2 : 1);
        if (last >= 0) trace[last].failed = true;
      }
    } finally {
      recording = false;
    }
    const after = rowCounts(db);
    const attached = db.pragma("database_list").filter((entry) => !["main", "temp"].includes(entry.name));
    const shrunk = [...before].filter(([name, count]) => after.has(name) && after.get(name) < count)
      .map(([name, count]) => `${name}: ${count} -> ${after.get(name)}`);
    const relative = (file) => path.relative(root, file) || file;
    const { statements, incomplete } = attribute(trace, pending, table, error, relative);
    if (error) incomplete.push(`the migration raised ${singleLine(error.name || error.constructor.name)}`);
    if (attached.length) incomplete.push("a migration attached another database");
    if (statements.some((statement) => RESERVED_LINE.test(statement.sql))) {
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
// migrator runs the chunks of each pending file in order and then records
// the file in the ledger, all between one BEGIN and one COMMIT.
function attribute(trace, pending, table, error, relative) {
  const ledgerInsert = `INSERT INTO ${quoteIdentifier(table)} ("hash", "created_at") VALUES(`;
  const statements = [];
  const incomplete = [];
  let group = [];
  let index = 0;
  let inTransaction = false;
  const synthetic = ({ sql, failed }) => statements.push({ sql, label: { synthetic: "Drizzle migrator" }, failed });
  const flush = () => {
    if (!group.length) return;
    const migration = pending[index];
    const chunks = migration?.chunks ?? [];
    const exact = group.length <= chunks.length && group.every(({ sql }, k) => sql.length && chunks[k].text.startsWith(sql));
    group.forEach(({ sql, failed }, k) => {
      let label;
      if (exact) {
        const lead = sql.match(/^\s*/)[0].length;
        label = { file: relative(migration.file), line: lineAt(migration.text, chunks[k].offset + lead) };
        sql = sql.slice(lead);
      } else {
        label = migration ? { migration: relative(migration.file) } : { synthetic: "unknown migration" };
      }
      statements.push({ sql, label, failed });
    });
    group = [];
  };
  for (const entry of trace) {
    const { sql } = entry;
    if (!inTransaction) {
      synthetic(entry);
      inTransaction = /^\s*BEGIN\s*$/i.test(sql);
      continue;
    }
    if (sql.startsWith(ledgerInsert)) {
      const when = Number(sql.match(/, ([^,()]+)\)$/)?.[1]);
      if (pending[index]?.when !== when) incomplete.push("the migrator ran migrations in an unexpected order");
      flush();
      synthetic(entry);
      index += 1;
    } else if (/^\s*(COMMIT|ROLLBACK)\s*$/i.test(sql)) {
      flush();
      synthetic(entry);
      inTransaction = false;
    } else {
      group.push(entry);
    }
  }
  flush();
  if (!error && index !== pending.length) incomplete.push("the migrator ran a different number of migrations than expected");
  return { statements, incomplete };
}

export function render(result) {
  const lines = [result.header, "-- Migration target: temporary database."];
  if (result.error) lines.push(`-- Migration failed: ${singleLine(`${result.error.name}: ${result.error.message}`)}`);
  for (const reason of result.incomplete) lines.push(`-- sqlite-doctor:incomplete ${singleLine(reason)}`);
  for (const line of result.shrunk) lines.push(`-- Observed on the copy: rows in ${line}`);
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
  fs.renameSync(staging, output);
}

function git(directory, ...args) {
  const result = spawnSync("git", ["-C", directory, ...args], { encoding: "buffer" });
  if (result.status !== 0) throw new CheckError(`git ${args[0]} failed: ${result.stderr.toString().trim()}`);
  return result.stdout;
}

// The migrations folder as it is at `base`, written under `dir`, or null if
// the folder does not exist there.
export function folderAt(base, folder, dir) {
  if (!base || base.startsWith("-") || /[\u0000-\u001f\u007f]/.test(base)) throw new CheckError("BASE must name a git revision");
  const absolute = path.resolve(folder);
  const top = git(absolute, "rev-parse", "--show-toplevel").toString().trim();
  const relative = path.relative(fs.realpathSync(top), fs.realpathSync(absolute)).split(path.sep).join("/");
  if (relative === ".." || relative.startsWith("../")) throw new CheckError(`${folder} is outside the git repository`);
  const commit = git(top, "rev-parse", "--verify", "--end-of-options", `${base}^{commit}`).toString().trim();
  const prefix = relative ? `${relative}/` : "";
  const listing = git(top, "ls-tree", "-r", "-z", "--full-tree", commit, "--", `:(literal)${prefix}`).toString("utf8");
  const target = path.join(dir, "base-migrations");
  let found = false;
  for (const entry of listing.split("\0").filter(Boolean)) {
    const [meta, name] = entry.split("\t");
    const [mode, type, object] = meta.split(" ");
    const inside = name.slice(prefix.length);
    if (type !== "blob" || mode === "120000" || !(inside === "meta/_journal.json" || /^[^/]+\.sql$/.test(inside))) continue;
    found = true;
    const file = path.join(target, ...inside.split("/"));
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, git(top, "cat-file", "blob", object));
  }
  return found ? target : null;
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
    if (executable(binary)) return binary;
    throw new CheckError(`SQLITE_DOCTOR=${binary} is not an executable file`);
  }
  const found = (process.env.PATH ?? "").split(path.delimiter).map((dir) => path.join(dir, "sqlite-doctor")).find(executable);
  if (!found) {
    throw new CheckError("the sqlite-doctor binary is not on PATH; install it with `cargo install sqlite-doctor`, the " +
      "kholdrex/sqlite-doctor GitHub Action or a release archive, or set SQLITE_DOCTOR");
  }
  return found;
}

export function report(binary, result, output, print) {
  const run = spawnSync(binary, ["--format", "json", output], { encoding: "utf8" });
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
    print(`\n${finding.code}: ${finding.title}\n  --> ${where} (${parsed.source}:${finding.line})\n  ${finding.detail}\n  help: ${finding.remediation}`);
  }
  for (const line of result.shrunk) print(`Rows lost on the copy: ${line}`);
  if (!parsed.complete) print(`\nCheck incomplete:\n${parsed.incomplete.map((item) => `- ${item.reason}`).join("\n")}`);
  if (result.error) {
    print(`Migration failed on the copy: ${result.error.name}. Its message can quote SQL with data, so it is only in ` +
      "the capture: rerun with --output <file> to keep it.");
  }
  if (run.status === null || run.status > 1 || !parsed.complete) return 2;
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
    if (options.base) {
      const baseFolder = folderAt(options.base, folder, dir);
      const db = new runner.Database(database);
      try {
        if (baseFolder) runner.migrate(runner.drizzle(db), { migrationsFolder: baseFolder, migrationsTable: options.table });
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
      folder,
      table: options.table,
      foreignKeys: options.foreignKeys,
      root,
    });
    const output = options.output ? path.resolve(root, options.output) : path.join(dir, "capture.sql");
    writeCapture(output, render(result));
    return report(binary, result, output, print);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

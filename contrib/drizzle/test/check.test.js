import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import { migrate } from "drizzle-orm/better-sqlite3/migrator";
import { check, CheckError, loadRunner } from "../lib/index.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const fixtures = path.join(here, "fixtures", "drizzle");
const binary = process.env.SQLITE_DOCTOR ?? path.resolve(here, "../../../target/debug/sqlite-doctor");
const journal = JSON.parse(fs.readFileSync(path.join(fixtures, "meta", "_journal.json"), "utf8"));
const directories = [];

before(() => assert.ok(fs.existsSync(binary), `build sqlite-doctor or set SQLITE_DOCTOR (${binary})`));
after(() => directories.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));

function project(count, root = fs.mkdtempSync(path.join(os.tmpdir(), "drizzle-test-"))) {
  directories.push(root);
  const folder = path.join(root, "drizzle");
  fs.mkdirSync(path.join(folder, "meta"), { recursive: true });
  const entries = journal.entries.slice(0, count);
  for (const entry of entries) fs.copyFileSync(path.join(fixtures, `${entry.tag}.sql`), path.join(folder, `${entry.tag}.sql`));
  fs.writeFileSync(path.join(folder, "meta", "_journal.json"), JSON.stringify({ ...journal, entries }));
  return root;
}

function addMigration(root, tag, sql, when) {
  const file = path.join(root, "drizzle", "meta", "_journal.json");
  const current = JSON.parse(fs.readFileSync(file, "utf8"));
  current.entries.push({ idx: current.entries.length, version: "6", when, tag, breakpoints: true });
  fs.writeFileSync(file, JSON.stringify(current));
  fs.writeFileSync(path.join(root, "drizzle", `${tag}.sql`), sql);
}

// A database migrated to the first file, with a parent and its child.
function seededDatabase(root) {
  const db = new Database(path.join(root, "app.db"));
  migrate(drizzle(db), { migrationsFolder: path.join(root, "drizzle") });
  db.exec("INSERT INTO parents VALUES (1, 'a'); INSERT INTO children VALUES (1, 1);");
  db.close();
}

async function run(options) {
  const lines = [];
  const status = await check({ binary, ...options }, (line) => lines.push(line));
  return { status, output: lines.join("\n") };
}

test("a pending rebuild is reported at its file lines and the database is left alone", async () => {
  const root = project(1);
  seededDatabase(root);
  addMigration(root, journal.entries[1].tag, fs.readFileSync(path.join(fixtures, `${journal.entries[1].tag}.sql`), "utf8"),
    journal.entries[1].when);

  const { status, output } = await run({ root, database: "app.db" });

  assert.equal(status, 1, output);
  assert.match(output, /1 pending migration\(s\):\n {2}drizzle\/0001_name_required\.sql/);
  assert.match(output, /SQLD001: .*\n {2}--> drizzle\/0001_name_required\.sql:8 /);
  assert.match(output, /SQLD002: .*\n {2}--> drizzle\/0001_name_required\.sql:1 /);
  assert.match(output, /SQLD002: .*\n {2}--> drizzle\/0001_name_required\.sql:10 /);
  assert.match(output, /Rows lost on the copy: children: 1 -> 0/);
  const db = new Database(path.join(root, "app.db"), { readonly: true });
  assert.equal(db.prepare("SELECT count(*) FROM children").pluck().get(), 1);
  assert.equal(db.prepare("SELECT count(*) FROM __drizzle_migrations").pluck().get(), 1);
  db.close();
});

test("no pending migrations is a clean check", async () => {
  const root = project(2);
  const db = new Database(path.join(root, "app.db"));
  migrate(drizzle(db), { migrationsFolder: path.join(root, "drizzle") });
  db.close();

  const { status, output } = await run({ root, database: "app.db" });

  assert.equal(status, 0, output);
  assert.match(output, /No pending migrations\./);
});

test("base builds the database from the migrations at a revision of a repository subdirectory", async () => {
  const repository = fs.mkdtempSync(path.join(os.tmpdir(), "drizzle-repo-"));
  directories.push(repository);
  const root = project(1, path.join(repository, "apps", "web"));
  const git = (...args) => execFileSync("git", ["-C", repository, "-c", "user.name=t", "-c", "user.email=t@example.invalid", ...args]);
  git("init", "-q");
  git("add", ".");
  git("commit", "-q", "-m", "base");
  addMigration(root, journal.entries[1].tag, fs.readFileSync(path.join(fixtures, `${journal.entries[1].tag}.sql`), "utf8"),
    journal.entries[1].when);

  const { status, output } = await run({ root, base: "HEAD" });

  assert.equal(status, 1, output);
  assert.match(output, /Checking against the migrations at HEAD/);
  assert.match(output, /1 pending migration\(s\):\n {2}drizzle\/0001_name_required\.sql/);
  assert.match(output, /SQLD001: .*\n {2}--> drizzle\/0001_name_required\.sql:8 /);
  assert.equal(fs.existsSync(path.join(root, "app.db")), false);
});

test("base without the migrations folder starts from an empty database", async () => {
  const repository = fs.mkdtempSync(path.join(os.tmpdir(), "drizzle-repo-"));
  directories.push(repository);
  fs.writeFileSync(path.join(repository, "README"), "");
  const git = (...args) => execFileSync("git", ["-C", repository, "-c", "user.name=t", "-c", "user.email=t@example.invalid", ...args]);
  git("init", "-q");
  git("add", ".");
  git("commit", "-q", "-m", "base");
  project(2, repository);

  const { status, output } = await run({ root: repository, base: "HEAD" });

  assert.equal(status, 1, output);
  assert.match(output, /2 pending migration\(s\):\n {2}drizzle\/0000_init\.sql\n {2}drizzle\/0001_name_required\.sql/);
});

test("foreign keys off before migrating means the rebuild deletes nothing", async () => {
  const root = project(1);
  seededDatabase(root);
  addMigration(root, journal.entries[1].tag, fs.readFileSync(path.join(fixtures, `${journal.entries[1].tag}.sql`), "utf8"),
    journal.entries[1].when);

  const { status, output } = await run({ root, database: "app.db", foreignKeys: false });

  assert.equal(status, 1, output);
  assert.doesNotMatch(output, /SQLD001/);
  assert.doesNotMatch(output, /Rows lost/);
});

test("a failing migration makes the check incomplete", async () => {
  const root = project(2);
  seededDatabase(root);
  addMigration(root, "0002_broken", "ALTER TABLE `parents` ADD `email` text;--> statement-breakpoint\nINSERT INTO `missing` VALUES (1);", 1795893540000);

  const { status, output } = await run({ root, database: "app.db", output: "capture.sql" });

  assert.equal(status, 2, output);
  assert.match(output, /The pending migrations could not be determined\./);
  assert.match(output, /- the migration raised \w+/);
  assert.doesNotMatch(output, /missing/);
  const capture = fs.readFileSync(path.join(root, "capture.sql"), "utf8");
  assert.match(capture, /-- failed: INSERT INTO `missing` VALUES \(1\);/);
  assert.match(capture, /-- sqlite-doctor:file 1 drizzle\/0002_broken\.sql\nALTER TABLE/);
});

test("a migration line that looks like a directive makes the check incomplete", async () => {
  const root = project(2);
  seededDatabase(root);
  addMigration(root, "0002_comment", "-- sqlite-doctor:synthetic not from the runner\nCREATE TABLE `notes` (`id` integer);", 1795893540000);

  const { status, output } = await run({ root, database: "app.db" });

  assert.equal(status, 2, output);
  assert.match(output, /copied SQL has a line that starts with `-- sqlite-doctor:`/);
});

test("the capture is not written over a file that is not a capture", async () => {
  const root = project(2);
  seededDatabase(root);
  fs.writeFileSync(path.join(root, "notes.sql"), "keep me");

  await assert.rejects(run({ root, database: "app.db", output: "notes.sql" }), CheckError);
  assert.equal(fs.readFileSync(path.join(root, "notes.sql"), "utf8"), "keep me");
});

test("an installed @libsql/client needs the driver confirmed", async () => {
  const root = project(2);
  seededDatabase(root);
  const runner = { ...loadRunner(root), libsql: true };

  await assert.rejects(run({ root, database: "app.db", runner }), /--driver better-sqlite3/);
  const { status } = await run({ root, database: "app.db", runner, driver: "better-sqlite3" });
  assert.equal(status, 0);
});

test("a statement that fails while running is the one marked failed", async () => {
  const root = project(2);
  seededDatabase(root);
  addMigration(root, "0002_null_name", "CREATE TABLE `notes` (`id` integer);--> statement-breakpoint\nINSERT INTO `parents` VALUES (2, NULL);", 1795893540000);

  const { status } = await run({ root, database: "app.db", output: "capture.sql" });

  assert.equal(status, 2);
  const capture = fs.readFileSync(path.join(root, "capture.sql"), "utf8");
  assert.match(capture, /-- sqlite-doctor:file 1 drizzle\/0002_null_name\.sql\nCREATE TABLE `notes` \(`id` integer\);\n/);
  assert.match(capture, /-- sqlite-doctor:file 2 drizzle\/0002_null_name\.sql\n-- failed: INSERT INTO `parents` VALUES \(2, NULL\);/);
});

test("the command exits with the check's status", () => {
  const root = project(1);
  seededDatabase(root);
  addMigration(root, journal.entries[1].tag, fs.readFileSync(path.join(fixtures, `${journal.entries[1].tag}.sql`), "utf8"),
    journal.entries[1].when);
  const command = path.resolve(here, "../bin/sqlite-doctor-drizzle.js");
  const env = { ...process.env, SQLITE_DOCTOR: binary };

  const found = spawnSync(process.execPath, [command, "--database", "app.db"], { cwd: root, env, encoding: "utf8" });
  const usage = spawnSync(process.execPath, [command, "--foreign-keys", "maybe"], { cwd: root, env, encoding: "utf8" });

  assert.equal(found.status, 1, found.stdout + found.stderr);
  assert.match(found.stdout, /SQLD001/);
  assert.equal(usage.status, 2);
  assert.match(usage.stderr, /--foreign-keys must be on or off/);
});

#!/usr/bin/env node
import { parseArgs } from "node:util";
import { check, CheckError } from "../lib/index.js";

const usage = `Usage: sqlite-doctor-drizzle [--database <file> | --base <git revision>] [options]

Runs the pending Drizzle migrations with drizzle-orm's better-sqlite3 migrator on a
copy of the database, or with --base on a database migrated to the migration files
at that revision, and checks the executed SQL with the sqlite-doctor binary.

  --database <file>       the SQLite database to copy (it is not changed)
  --base <revision>       build the database from the migrations at a git revision
  --migrations <dir>      the migrations folder (default: drizzle)
  --table <name>          the migrations table (default: __drizzle_migrations)
  --foreign-keys on|off   set foreign keys before migrating (default: better-sqlite3's, on)
  --driver better-sqlite3 confirm that better-sqlite3 runs the migrations when
                          @libsql/client is also installed
  --output <file>         keep the captured SQL in <file>

Exit status: 0 nothing found, 1 findings, 2 the check is incomplete or failed.`;

let values;
try {
  ({ values } = parseArgs({
    options: {
      database: { type: "string" },
      base: { type: "string" },
      migrations: { type: "string" },
      table: { type: "string" },
      "foreign-keys": { type: "string" },
      driver: { type: "string" },
      output: { type: "string" },
      help: { type: "boolean", short: "h" },
    },
  }));
} catch (error) {
  console.error(`sqlite-doctor-drizzle: ${error.message}\n\n${usage}`);
  process.exit(2);
}
if (values.help) {
  console.log(usage);
  process.exit(0);
}

const fail = (message) => {
  console.error(`sqlite-doctor-drizzle: ${message}`);
  process.exit(2);
};
const foreignKeys = values["foreign-keys"];
if (foreignKeys !== undefined && !["on", "off"].includes(foreignKeys)) fail("--foreign-keys must be on or off");
if (values.driver !== undefined && values.driver !== "better-sqlite3") fail("--driver supports only better-sqlite3");
if (values.database && values.base) fail("pass either --database or --base");

try {
  process.exitCode = await check({
    database: values.database,
    base: values.base,
    migrations: values.migrations,
    table: values.table,
    foreignKeys: foreignKeys === undefined ? undefined : foreignKeys === "on",
    driver: values.driver,
    output: values.output,
  });
} catch (error) {
  fail(error instanceof CheckError ? error.message : `${error.name}: ${error.message}`);
}

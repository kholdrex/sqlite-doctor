# sqlite-doctor

Checks SQLite migration SQL for table rebuilds that can delete or change dependent rows, or drop indexes and
triggers.

```bash
cargo install --path .        # from a clone; not on crates.io yet
sqlite-doctor migration.sql
```

Building needs Rust 1.88 or newer. Exit code `0` means nothing found, `1` means at least one finding, `2` means bad
arguments or an unreadable file. Add `--format json` for CI, and pass `-` to read from stdin.

## Why this exists

SQLite can't change most column definitions in place, so ORMs rebuild the table: copy the rows out, drop the
table, create it again, copy the rows back. Dropping the table also fires every `ON DELETE` action that points at
it. If foreign keys are on, children with `ON DELETE CASCADE` are deleted and children with `ON DELETE SET NULL`
lose their reference. Turning foreign keys off doesn't help inside a transaction, because SQLite ignores changes
to `PRAGMA foreign_keys` there.

In Rails 8.1.4, with foreign keys on and the default migration transaction, `change_column_null` or
`remove_column` on a parent table deletes its cascading children. Drizzle ORM 0.45.4 with Kit 0.31.11 does the
same when a column is made `NOT NULL`. In both, the migration reports success.

## What it reports

| Code | Meaning |
|---|---|
| `SQLD001` | A table is rebuilt while a table created earlier in the input references it with `ON DELETE CASCADE`, `SET NULL` or `SET DEFAULT`. Reported unless the input turns foreign keys off before the drop, or drops that child table first. |
| `SQLD002` | `PRAGMA foreign_keys` is changed inside a transaction, where SQLite ignores the change. |
| `SQLD003` | An index or trigger created on a table before the rebuild is not created again afterwards. |

To rebuild a table safely, follow SQLite's order:

1. `PRAGMA foreign_keys = OFF;` before `BEGIN`
2. `BEGIN;`, then the rebuild
3. `PRAGMA foreign_key_check;` and `ROLLBACK;` if it reports anything
4. `COMMIT;`
5. `PRAGMA foreign_keys = ON;` after `COMMIT`

For `SQLD003`, recreate the index or trigger after the rebuild, or drop it first if you meant to remove it. An
index on a plain column that the rebuilt table no longer has is not reported, because `remove_column` and
`remove_reference` drop that index on purpose. Expression indexes, and tables whose column list can't be read,
are still reported.

Example that reports `SQLD003`, because SQLite drops `idx_users_email` together with `users`:

```sql
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
```

## Rails

A migration file doesn't show the SQL that matters: ActiveRecord opens the transaction, sets `foreign_keys = OFF`
inside it (which does nothing), and rebuilds the table through a holding copy.

1. Copy `contrib/rails/sqlite_doctor_capture.rb` and `contrib/rails/sqlite_doctor.rake` into `lib/tasks/`.
2. Run `bin/rails sqlite_doctor:capture OUTPUT=tmp/migration.sql`.
3. Run `sqlite-doctor tmp/migration.sql`.

The task points ActiveRecord at a `VACUUM INTO` copy of the primary database and runs your pending migrations
there. The output holds the current schema, the `foreign_keys` setting and the SQL that ActiveRecord reported for
that copy while the migrations ran. SQL that a migration sends to another database, or straight to the
SQLite driver, is not in it. Failed statements are kept as comments. Tables that still exist but have fewer rows
at the end are listed in the header.

Before migrating, it refuses to start when:

- another connection pool in the app uses the same database file, by path or by `file:` URI;
- `OUTPUT` points at the database or one of its `-wal`/`-shm`/`-journal` files.

A migration that opens its own connection to the original database is not redirected, and it can change that
database. The task notices afterwards (`PRAGMA data_version`) and fails without writing a capture. A concurrent
`wal_checkpoint(TRUNCATE)` from another process trips the same check; run the task again.

Without Rails, run `ruby contrib/rails/sqlite_doctor_capture.rb DATABASE MIGRATIONS_DIR OUTPUT` from a clone, with
the `activerecord` and `sqlite3` gems installed. The capture code is tested on ActiveRecord 7.1, 7.2 and 8.1.

## Drizzle

With better-sqlite3, Drizzle ORM 0.45.4 runs all pending migration files in one transaction, so a generated file
alone hides the problem: its `PRAGMA foreign_keys=OFF` looks effective. Scan one input that matches what runs:

1. The starting schema: `CREATE TABLE`, `CREATE INDEX` and `CREATE TRIGGER` statements.
2. `PRAGMA foreign_keys = ON;` (better-sqlite3 turns foreign keys on by default).
3. One `BEGIN;`, every pending migration file in order, one `COMMIT;`

## Results in the paper

The tool is evaluated in "Successful Migrations, Lost Data: Silent Preservation Failures in SQLite Table Rebuilds
Across Five ORMs". The paper used two builds, kept here as tags:

| Tag | Commit | Use in the paper |
|---|---|---|
| [`study-original`](https://github.com/kholdrex/sqlite-doctor/tree/study-original) | `a65e428` | The linter in the experiments. It knows the create, copy, drop, rename rebuild and `ON DELETE CASCADE`. |
| [`study-relint`](https://github.com/kholdrex/sqlite-doctor/tree/study-relint) | `2bf86dc` | A later build run on the same inputs. It adds the Rails rebuild and `SET NULL` and `SET DEFAULT` children. |

The [replication package](https://github.com/kholdrex/sqlite-migration-preservation#linter-builds) has the build
commands, the hashes of both binaries, the inputs and the results. The READMEs at the two tags are historical: they
describe an early prototype, and the one at `study-relint` compares the tool with Atlas on inputs the paper does
not use. For usage read this README on `main`; for results read the package.

In the paper's matrix of schema operations, 178 cases lost data without an error and have a trace of the executed
SQL. Given the starting schema and that trace:

| Build | Named the affected dependent table, index or trigger |
|---|---|
| `study-original` | 29 of 178 |
| `study-relint` | 164 of 178 |

The rules of the later build were written with these results in view, so its row is not an independent test. It
named the affected dependent in all 48 Rails 8.1.4 cases. Leaving aside `SQLD002`, which fires on every Rails and
Drizzle rebuild, it warned on 25 of the 204 rebuilds that lost nothing: 19 that failed and were rolled back, and 6
Django migrations that remove a unique index on purpose. None of the 25 was a Rails rebuild. The package lists
each figure with its source file in `paper/numbers.md`; the later build's output is in
`results/relint-2026-11-01/relint.json`. `main` has changed since `study-relint`; the figures are for the tagged
builds.

## Limits

- It is a lexical scanner for one SQLite input. It does not run SQL, assumes every statement succeeds, and does
  not parse other SQL dialects.
- Include the starting schema. Tables, indexes and triggers from earlier migrations or the live database are
  invisible otherwise; the Rails task includes them for you.
- A reference counts from its `CREATE TABLE` until the input drops that child table, and follows a parent through
  `ALTER TABLE ... RENAME TO`. Rebuilding the child without the foreign key clears it the same way. A child created
  in a transaction that rolls back does not count; a drop that a `ROLLBACK` undoes before the parent rebuild does
  not clear it. When it can't tell whether a drop removed the child (a temp table used that name earlier, the
  child was renamed, a rollback names no open savepoint), it keeps warning.
- It recognises two rebuild shapes: create, copy, drop, rename (Drizzle, SQLite's documented procedure), and
  copy out, drop, create again, copy back (Rails). A rebuild is evidence for a finding, not proof of harm: one
  inside a transaction that later rolls back is still reported.
- `SQLD003` matches objects by schema and name, and an index or trigger stays with its table through a rename.
  Recreating an index under a different name still reports the old one as missing. Recreating it only after the table has been rebuilt again, or in a transaction that rolls back,
  does not count. A `DROP INDEX` or `DROP TRIGGER` that a `ROLLBACK` undoes does not count as removing it. An
  indexed column is followed through `ALTER TABLE ... RENAME COLUMN`; when the rebuild itself renames the column,
  a plain-column index on it counts as removed, so its loss is not reported. Views are not checked
  because `DROP TABLE` does not drop them.

## Background

- [rails/rails#55866](https://github.com/rails/rails/issues/55866) reported the data loss;
  [#55907](https://github.com/rails/rails/pull/55907) fixed part of it. Rebuilds inside a transaction still lose
  rows on 8.1.4. On 26 November 2026 [#57128](https://github.com/rails/rails/pull/57128) and
  [#58709](https://github.com/rails/rails/pull/58709), which keep the cascades from firing, and
  [#56811](https://github.com/rails/rails/pull/56811), which recreates triggers, were open.
- [drizzle-team/drizzle-orm#4938](https://github.com/drizzle-team/drizzle-orm/issues/4938) and
  [#5782](https://github.com/drizzle-team/drizzle-orm/issues/5782) report the same cascade loss in Drizzle.
- [SQLite foreign keys](https://www.sqlite.org/foreignkeys.html): SQLite ignores changes to `PRAGMA foreign_keys`
  inside a transaction.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
sh tests/sqlite_outcomes.sh
AR_VERSION=8.1.4 ruby contrib/rails/test/capture_test.rb
```

Each file under `tests/fixtures/sqlite` is a script for the `sqlite3` shell, and its first line records what
SQLite prints for it. `tests/sqlite_outcomes.sh` checks those lines against the `sqlite3` on your `PATH`.

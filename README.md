# sqlite-doctor

Finds SQLite migrations that quietly delete or change data when an ORM rebuilds a table.

```bash
cargo install --path .        # from a clone; not on crates.io yet
sqlite-doctor migration.sql
```

Exit code `0` means nothing found, `1` means at least one finding, `2` means bad arguments or an unreadable file.
Add `--format json` for CI, and pass `-` to read from stdin.

## Why this exists

SQLite can't change most column definitions in place, so ORMs rebuild the table: copy the rows out, drop the
table, create it again, copy the rows back. Dropping the table also fires every `ON DELETE` action that points at
it. If foreign keys are on, children with `ON DELETE CASCADE` are deleted and children with `ON DELETE SET NULL`
lose their reference. Turning foreign keys off doesn't help inside a transaction, because SQLite ignores
`PRAGMA foreign_keys` there.

Rails 8.1 and Drizzle both hit this. A one-line `change_column_null` or `remove_column` on a parent table can empty
its child tables, and the migration still reports success.

## What it reports

| Code | Meaning |
|---|---|
| `SQLD001` | A table is rebuilt while a table created earlier in the input references it with `ON DELETE CASCADE`, `SET NULL` or `SET DEFAULT`. Reported unless the input turns foreign keys off before the drop. |
| `SQLD002` | `PRAGMA foreign_keys` is set inside a transaction, where SQLite ignores it. |
| `SQLD003` | An index or trigger created on a table before the rebuild is not created again afterwards. |

To rebuild a table safely, follow SQLite's order:

1. `PRAGMA foreign_keys = OFF;` before `BEGIN`
2. `BEGIN;`, then the rebuild
3. `PRAGMA foreign_key_check;` and fix what it reports
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
inside it (which does nothing), and rebuilds the table through a holding copy. So scan what actually runs.

1. Copy `contrib/rails/sqlite_doctor_capture.rb` and `contrib/rails/sqlite_doctor.rake` into `lib/tasks/`.
2. Run `bin/rails sqlite_doctor:capture OUTPUT=tmp/migration.sql`.
3. Run `sqlite-doctor tmp/migration.sql`.

The task points ActiveRecord at a `VACUUM INTO` copy of the primary database and runs your pending migrations
there. The output holds the current schema, the `foreign_keys` setting and every statement the migrations
executed. Failed statements are kept as comments. Tables that still exist but have fewer rows at the end are
listed in the header.

Before migrating, it refuses to start when:

- another connection pool in the app uses the same database file;
- `OUTPUT` points at the database or one of its `-wal`/`-shm`/`-journal` files.

A migration that opens its own connection to the original database is not redirected, and it can change that
database. The task notices afterwards (`PRAGMA data_version`) and fails without writing a capture. A
`wal_checkpoint(TRUNCATE)` from another process, which Litestream runs, trips the same check; run the task again.

Without Rails, run `ruby sqlite_doctor_capture.rb DATABASE MIGRATIONS_DIR OUTPUT` with the `activerecord` and
`sqlite3` gems installed. The capture code is tested on ActiveRecord 7.1, 7.2 and 8.1.

## Drizzle

Drizzle's migrator wraps each generated file in a transaction, so the file alone hides the problem: its
`PRAGMA foreign_keys=OFF` looks effective. Scan an input that matches what runs:

1. The existing schema: `CREATE TABLE`, `CREATE INDEX` and `CREATE TRIGGER` statements.
2. `PRAGMA foreign_keys = ON;` (better-sqlite3 turns foreign keys on by default).
3. `BEGIN;`, the generated migration SQL, `COMMIT;`

## How well it works

On 144 Rails 8.1.4 test cases, `SQLD001` and `SQLD003` reported all 48 that lost rows, foreign-key values or
triggers, and none of the 96 that kept everything. `SQLD002` fires on every Rails rebuild, so it is not counted
as a loss warning. The cases are 16 migration operations, each run with 9 setups:

- no dependents
- a child table with `CASCADE`, `SET NULL` or `RESTRICT`
- a two-level cascade
- a trigger, an index or a view on the parent
- a parent that is itself a cascading child

On the same SQL, `atlas migrate lint` (Community Edition 1.3.0) reported its generic `DS102` "Dropping table" on
all 117 rebuilds, including the 69 that lost nothing, and never named the child table.

On migrations from 60 public Rails apps, replayed on seeded data (2,840 steps), `SQLD001` and `SQLD003` caught 24
of the 25 steps that lost data and fired on one step that lost nothing. The miss was a `drop_table` inside
`disable_referential_integrity`, which is not a rebuild.

## Limits

- It is a lexical scanner for one SQLite input. It does not run SQL and does not parse other SQL dialects.
- It only knows what is in the input. Tables, indexes and triggers from earlier migrations or the live database
  are invisible unless you include them; the Rails task does that for you.
- A reference stays known once its `CREATE TABLE` appears. Dropping the child table, or removing its foreign key,
  before a later parent rebuild does not clear it, so `SQLD001` can still fire.
- It recognises two rebuild shapes: create, copy, drop, rename (Drizzle, SQLite's documented procedure), and
  copy out, drop, create again, copy back (Rails). A rebuild is evidence for a finding, not proof of harm.
- `SQLD003` matches objects by name. Recreating an index under a different name still reports the old one as
  missing. A plain-column index on a renamed column counts as removed, so its loss is not reported. Views are not checked
  because `DROP TABLE` does not drop them.

## Background

- [rails/rails#55866](https://github.com/rails/rails/issues/55866) reported the data loss;
  [#55907](https://github.com/rails/rails/pull/55907) fixed part of it. Rebuilds inside a transaction still lose
  rows on 8.1.4; [#57128](https://github.com/rails/rails/pull/57128), [#58709](https://github.com/rails/rails/pull/58709)
  and [#56811](https://github.com/rails/rails/pull/56811) (triggers) are open.
- [drizzle-team/drizzle-orm#4938](https://github.com/drizzle-team/drizzle-orm/issues/4938) and
  [#5782](https://github.com/drizzle-team/drizzle-orm/issues/5782) report the same cascade loss in Drizzle.
- [SQLite foreign keys](https://www.sqlite.org/foreignkeys.html): `PRAGMA foreign_keys` is a no-op inside a
  transaction.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
AR_VERSION=8.1.4 ruby contrib/rails/test/capture_test.rb
```

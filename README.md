# sqlite-doctor

SQLite migration safety linter for ORM-generated rebuild hazards.

## Thesis

SQLite supports only a limited `ALTER TABLE` surface. ORMs often emulate richer changes by rebuilding tables, copying rows, dropping the original table, and renaming the replacement. That pattern can silently destroy data or drop schema objects when foreign keys, cascades, triggers, indexes, views, or transaction-scoped pragmas interact badly.

`sqlite-doctor` is a local-first CLI and future GitHub Action that will lint migration files and generated SQL for these rebuild hazards before they reach production.

## Initial target users

- Rails apps using SQLite in production or durable single-tenant deployments
- Drizzle / better-sqlite3 / libSQL / Turso projects shipping generated SQLite migrations
- Teams that want CI warnings before a schema change can trigger cascade deletes or lose schema objects during a rebuild

## Planned MVP

The first MVP focuses on static detection of high-signal hazards:

1. Table rebuilds that can delete child rows through `ON DELETE CASCADE` or rewrite their foreign keys through `ON DELETE SET NULL` / `SET DEFAULT`
2. `PRAGMA foreign_keys=OFF` inside a transaction, where SQLite treats it as a no-op
3. Rebuilds that recreate a table but omit explicit follow-up restoration of indexes or triggers
4. ORM-oriented guidance with concrete safer rewrite suggestions

## Evidence summary

- Rails issue `#55866` and follow-up PR `#55907` documented SQLite migration data loss during table alterations.
- Rails follow-up PR `#57128` remains open and documents transaction-wrapped rebuild corruption/data-loss cases.
- Drizzle issues `#4938` and `#5782` document the same class of silent cascade data loss in a second ecosystem.
- SQLite's official docs explain that `PRAGMA foreign_keys` is a no-op within a transaction and that richer schema changes require careful rebuild workflows.

## Development status

Private scout-created repository. Not published.

## CLI usage

Scan a SQL file with human-readable diagnostics:

```bash
cargo run -- migration.sql
```

Use `-` to read SQL from standard input, or select stable JSON output for CI:

```bash
cat migration.sql | cargo run -- --format json -
```

The CLI exits with `0` when the input is clean, `1` when it finds a supported
hazard, and `2` for invalid arguments or unreadable input.

- `SQLD001` reports a complete table rebuild when the same input also defines
  an existing dependent table with an explicit `ON DELETE CASCADE`, `SET NULL`
  or `SET DEFAULT` reference.
  A safe `PRAGMA foreign_keys=OFF` before the transaction suppresses that
  finding.
- `SQLD002` reports `PRAGMA foreign_keys` assignments inside a transaction,
  where SQLite ignores the attempted state change.
- `SQLD003` reports an index or trigger that the same input explicitly creates
  on a table before rebuilding it, then destroys at `DROP TABLE` without
  recreating the same object name on the replacement or renamed table. Recreate
  the object after dropping the original, or explicitly drop it before the
  rebuild when removal is intentional. An index on a column that the
  replacement table no longer has is not reported: removing the column
  (`remove_column`, `remove_reference`) removes its index on purpose.

For example, this input reports `SQLD003` because SQLite drops
`idx_users_email` with the original `users` table:

```sql
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
```

Recreating `idx_users_email` on either `__new_users` after the drop or `users`
after the rename makes the restoration explicit and suppresses the finding.

### Rails workflow

A Rails migration file does not contain the SQL that decides whether a rebuild
deletes rows: ActiveRecord opens the transaction, turns `foreign_keys` off inside
it (a no-op), and rebuilds the table by copying it to a holding table, dropping
it, recreating it and copying the rows back. Capture what actually runs instead.
Copy `contrib/rails/sqlite_doctor_capture.rb` and `contrib/rails/sqlite_doctor.rake`
into `lib/tasks/`, then:

```bash
bin/rails sqlite_doctor:capture OUTPUT=tmp/migration.sql
sqlite-doctor tmp/migration.sql
```

The task points ActiveRecord at a `VACUUM INTO` copy of the primary database,
runs the pending migrations there and writes one SQL input: the baseline
schema, the connection's `PRAGMA foreign_keys` state and every executed
statement, including transaction boundaries. Statements that failed are kept
as comments so they never count as restoring an object, and tables that lost
rows on the copy are listed in the header. It refuses to start when another
connection pool uses the same database file or when `OUTPUT` would overwrite
it, and it fails without writing a capture if another connection committed to
the original during the run (`PRAGMA data_version`), for example a migration
that opens its own connection to it. A `wal_checkpoint(TRUNCATE)` by another
process, as Litestream runs, also trips this check; rerun the task. The
application's connection is restored afterwards. Outside Rails, run
`ruby sqlite_doctor_capture.rb DATABASE MIGRATIONS_DIR OUTPUT` with
`activerecord` and `sqlite3` available.

On the 144 Rails 8.1.4 cells of an operation × dependency matrix (16 migration
operations, 9 dependency contexts) captured this way, `SQLD001` and `SQLD003`
reported all 48 cells that silently lost rows, foreign-key values or triggers,
and none of the 96 cells that preserved them. On the same SQL, `atlas migrate
lint` (Community Edition v1.3.0) reports its generic destructive-change check,
`DS102` "Dropping table", for every one of the 117 rebuilds, including the 69
that lost nothing, and none of its diagnostics name the dependent table.

### Drizzle workflow

Scan the generated SQLite migration SQL and include the earlier object creation
statements in the same input when checking a rebuild sequence. A finding is
intentionally based only on object lifecycle evidence visible in that combined
input.

## Scanner scope and limitations

The scanner is a dependency-free lexical scanner for one SQLite SQL input, not
a generic SQL parser or a migration executor. It sees only its input: a
transaction, pragma or existing schema object that an ORM or framework adds at
runtime is invisible unless it is captured into that input, as the Rails
capture task does. Comments and
literals are ignored and SQLite trigger bodies are kept together, but malformed
or non-SQLite dialect input is outside its scope.

A reported rebuild is a structural pattern anchor: create/copy/drop/rename, or
the create/copy/drop/recreate/copy-back sequence ActiveRecord emits. It
is useful evidence for later diagnostics, not proof by itself that the sequence
is unsafe. `ON DELETE` reference evidence is derived from `CREATE TABLE` statements
in the same input; the scanner does not currently model a dependent table being
dropped or having its foreign key removed before a later parent rebuild.

`SQLD003` is likewise evidence-bound: it only tracks indexes and triggers that
are explicitly created in the same input before the matched rebuild. Objects
from an existing database or earlier migration files are intentionally
invisible, so their omission cannot be diagnosed. Restoration matches object
kind and normalized name; recreating an equivalent object under a different
name still reports the original as missing. The removed-column exception needs
plain key columns and a replacement CREATE TABLE with a readable column list;
expression keys and other cases keep reporting. A column renamed by a rebuild
counts as removed, so a lost index on the old name is not reported. SQLite does not automatically drop
views with `DROP TABLE`, so views are not reported as missing restoration by
this rule.

## Local checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -- --help
```

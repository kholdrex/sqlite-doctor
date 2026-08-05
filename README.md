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

1. Table rebuilds that can trigger child-row deletion through `ON DELETE CASCADE`
2. `PRAGMA foreign_keys=OFF` inside a transaction, where SQLite treats it as a no-op
3. Rebuilds that recreate a table but omit likely follow-up restoration of indexes, triggers, or views
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
hazard, and `2` for invalid arguments or unreadable input. `SQLD001` reports a
complete table rebuild when the same input also defines an existing dependent
table with an explicit `ON DELETE CASCADE` reference. A safe
`PRAGMA foreign_keys=OFF` before the transaction suppresses that finding;
placing the pragma inside the transaction emits `SQLD002` because SQLite
ignores the attempted state change.

## Scanner scope and limitations

The scanner is a dependency-free lexical scanner for one SQLite SQL input, not
a generic SQL parser or a migration executor. It cannot observe transactions
injected by an ORM or framework, dynamically generated SQL, pragmas selected at
runtime, or transaction/schema state carried across files. Comments and
literals are ignored and SQLite trigger bodies are kept together, but malformed
or non-SQLite dialect input is outside its scope.

A reported rebuild is a structural create/copy/drop/rename pattern anchor. It
is useful evidence for later diagnostics, not proof by itself that the sequence
is unsafe. Cascade-reference evidence is derived from `CREATE TABLE` statements
in the same input; the scanner does not currently model a dependent table being
dropped or having its foreign key removed before a later parent rebuild.

## Local checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -- --help
```

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

## Local checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run
```

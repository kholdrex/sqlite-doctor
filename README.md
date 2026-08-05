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
  an existing dependent table with an explicit `ON DELETE CASCADE` reference.
  A safe `PRAGMA foreign_keys=OFF` before the transaction suppresses that
  finding.
- `SQLD002` reports `PRAGMA foreign_keys` assignments inside a transaction,
  where SQLite ignores the attempted state change.
- `SQLD003` reports an index or trigger that the same input explicitly creates
  on a table before rebuilding it, then destroys at `DROP TABLE` without
  recreating the same object name on the replacement or renamed table. Recreate
  the object after dropping the original, or explicitly drop it before the
  rebuild when removal is intentional.

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

### Rails and Drizzle workflows

For Rails, scan the SQL emitted for a migration together with relevant index and
trigger declarations from the schema snapshot when you want `SQLD003` coverage;
a migration fragment alone cannot prove which objects already exist. For
Drizzle, scan the generated SQLite migration SQL and include the earlier object
creation statements in the same input when checking a rebuild sequence. In both
workflows, a finding is intentionally based only on object lifecycle evidence
visible in that combined input.

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

`SQLD003` is likewise evidence-bound: it only tracks indexes and triggers that
are explicitly created in the same input before the matched rebuild. Objects
from an existing database or earlier migration files are intentionally
invisible, so their omission cannot be diagnosed. Restoration matches object
kind and normalized name; recreating an equivalent object under a different
name still reports the original as missing. SQLite does not automatically drop
views with `DROP TABLE`, so views are not reported as missing restoration by
this rule.

## Local checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -- --help
```

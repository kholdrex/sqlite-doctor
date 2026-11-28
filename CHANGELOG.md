# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-28

First public release. The tags `study-original` and `study-relint` mark earlier
builds used in the research paper; they are not releases.

### Added

- SQLD001: a table rebuild runs with foreign keys enforced, so dropping the old
  table fires `ON DELETE CASCADE`, `SET NULL` or `SET DEFAULT` on rows in other
  tables. Rebuilds are recognised when the new table is renamed into place, when
  the old table is renamed away first, and in the Rails recreate pattern, with
  quoted names, `WITH` clauses, `CREATE TABLE ... AS SELECT`, `VALUES`
  subqueries and rolled-back statements taken into account. Child references
  added with `ALTER TABLE ... ADD COLUMN ... REFERENCES` count too.
- SQLD002: `PRAGMA foreign_keys` changed inside a transaction, where SQLite
  ignores it.
- SQLD003: an index or trigger dropped with the old table is not created again.
- A suggested fix with each finding.
- Text and JSON output (`--format json`, version 1), reading from files or
  stdin. The README describes which JSON fields are stable.
- `contrib/rails`: runs pending ActiveRecord migrations against a copy of the
  database and writes the SQL they ran to one file for `sqlite-doctor`. Tested
  with ActiveRecord 7.1, 7.2 and 8.1.

### Known gaps

The README lists the inputs this version does not read correctly under
"Known gaps in 0.1.0".

[Unreleased]: https://github.com/kholdrex/sqlite-doctor/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kholdrex/sqlite-doctor/releases/tag/v0.1.0

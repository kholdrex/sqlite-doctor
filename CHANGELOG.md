# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The Rails helper is a gem, `sqlite_doctor`, on RubyGems from 0.4.0
  (gemspec at the repository root, code in `contrib/rails/lib`). `bin/rails sqlite_doctor:check` captures
  the pending migrations, scans them with the `sqlite-doctor` binary (0.3.0
  or newer), prints each finding with the migration it came from and exits
  `1` with findings or `2` when the check is incomplete or fails.
  `BASE=<git revision>` builds the database from the schema
  file and migration files at that revision, so CI needs no migrated
  database.
- Rails captures mark which migration ran each statement with
  `-- sqlite-doctor:migration` directives, and are incomplete when migration
  SQL runs on another database.
- `DATABASE=<name>` picks the database in an app that migrates several; without
  it such an app gets exit code `2`.

### Changed

- Migration announcements are not printed during a capture, because they
  show SQL that can hold data.
- The Rails capture code moved from `contrib/rails/sqlite_doctor_capture.rb`
  to `contrib/rails/lib/sqlite_doctor/capture.rb`, and its module from
  `SqliteDoctorCapture` to `SqliteDoctor::Capture`.

## [0.3.0] - 2026-11-28

Adds a contract for migration adapters and installable release archives.
The documented library API is unchanged from 0.2.0.

### Added

- Comment directives for migration adapters: `-- sqlite-doctor:file`,
  `migration` and `synthetic` say where the following lines came from, and
  `-- sqlite-doctor:incomplete` marks a check that did not see everything.
- JSON output: `location` and `related_location` on each finding, and
  top-level `complete` and `incomplete`. Existing fields are unchanged.
- Release archives for Linux x86_64 and macOS arm64 with SHA-256 checksums,
  built by `dist`, and a GitHub Action (`kholdrex/sqlite-doctor@v<version>`)
  that installs one after checking its checksum. Each release installs the
  archives through the Action and through `cargo binstall` on both platforms
  and runs smoke checks.
- The Rails capture marks itself incomplete (`-- sqlite-doctor:incomplete`)
  when a migration raised, a statement failed, SQL ran on a second
  connection to the copy, the migrating connection reconnected, or copied SQL
  has a line starting with `-- sqlite-doctor:`, so scanning a saved capture
  exits `2`.

### Changed

- Exit code `2` also means an incomplete check, including an unknown or
  malformed directive and input that ends inside a string, quoted name or
  block comment. The findings are still printed.

### Fixed

- A named parameter whose `(...)` suffix holds `/*`, a quote or `[` (legal in
  SQLite, such as `$p(/*)`) no longer hides the statements after it.
- The Rails capture puts each header note on one line, so a table name with
  a line break cannot turn the note into SQL.

## [0.3.0-rc.1] - 2026-11-28

Release candidate for 0.3.0, with the same changes. Its release run built
both archives and passed the smoke checks through the Action and
`cargo binstall` on Linux and macOS.

## [0.2.0] - 2026-11-28

First release on crates.io: `cargo install sqlite-doctor`. The command-line
tool and its JSON output are unchanged; the library API changed.

### Added

- `check_sql`, which scans SQL and returns its findings in one call.
- Crate documentation with an example.

### Changed

- `HazardId`, `RebuildKind`, `SchemaObjectKind` and `DeleteAction` are
  `#[non_exhaustive]`, so a later version can add a hazard or a variant
  without fields without a breaking change. Matches on them need a wildcard
  arm; use `HazardId::code` rather than an integer cast.
- `Finding` and the scan records (`RebuildSequence`, `SchemaObject`,
  `DroppedSchemaObject`, `DeleteActionReference`,
  `ForeignKeysPragmaInTransaction`) are `#[non_exhaustive]`: their fields can
  still be read, but code outside the crate cannot build them with a struct
  literal.
- `ScanReport` fields are private; the five that were public are read through
  methods of the same names, which return slices. `ScanReport` no longer
  implements `Default`: a report comes only from `scan_sql`.
- `HazardId::ALL` is a slice instead of a three-element array.
- `Finding::new` and `Finding::with_related_line` are no longer public.

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

The "Known gaps" item under Limits in the README lists the inputs this
version does not read correctly.

[Unreleased]: https://github.com/kholdrex/sqlite-doctor/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/kholdrex/sqlite-doctor/compare/v0.2.0...v0.3.0
[0.3.0-rc.1]: https://github.com/kholdrex/sqlite-doctor/compare/v0.2.0...v0.3.0-rc.1
[0.2.0]: https://github.com/kholdrex/sqlite-doctor/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kholdrex/sqlite-doctor/releases/tag/v0.1.0

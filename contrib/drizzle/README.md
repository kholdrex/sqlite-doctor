# sqlite-doctor-drizzle

Checks pending Drizzle migrations for SQLite table rebuilds that delete or orphan rows. It runs them with the
project's own drizzle-orm 0.45 better-sqlite3 migrator on a copy of the database, records every statement the
connection executes, and scans that SQL with [sqlite-doctor](https://github.com/kholdrex/sqlite-doctor).

```bash
npm install --save-dev sqlite-doctor-drizzle
npx sqlite-doctor-drizzle --database sqlite.db
npx sqlite-doctor-drizzle --base origin/main   # in CI: no database needed
```

The `sqlite-doctor` binary (0.3.0 or newer) must be on `PATH` or named by `SQLITE_DOCTOR`. Exit codes: `0` nothing
found, `1` findings, `2` the check is incomplete or failed. Run `npx sqlite-doctor-drizzle --help` for the options
and see the [Drizzle section](https://github.com/kholdrex/sqlite-doctor#drizzle) of the main README.

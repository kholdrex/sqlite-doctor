#!/bin/sh
# Runs every fixture under tests/fixtures/sqlite in the sqlite3 shell and
# compares what it prints with the fixture's first line. A statement that
# SQLite rejects fails the fixture.
set -eu

status=0
for fixture in "$(dirname "$0")"/fixtures/sqlite/*/*.sql; do
  expected=$(head -n 1 "$fixture")
  if ! output=$(sqlite3 -batch -bail :memory: < "$fixture"); then
    echo "$fixture: sqlite3 rejected a statement"
    status=1
    continue
  fi
  printed=$(printf '%s\n' "$output" | tr '\n' ' ' | sed 's/ $//')
  if [ "$expected" != "-- sqlite3 prints: $printed" ]; then
    echo "$fixture: sqlite3 prints: $printed"
    status=1
  fi
done
exit $status

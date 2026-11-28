#!/bin/sh
# Checks an installed sqlite-doctor binary end to end: stdin, files, JSON and
# exit codes. Usage: tests/release_smoke.sh [BINARY] (default: sqlite-doctor
# on PATH).
set -u

binary=${1:-sqlite-doctor}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

expect_exit() {
  expected=$1
  shift
  "$@" >"$work/stdout" 2>"$work/stderr"
  actual=$?
  if [ "$actual" -ne "$expected" ]; then
    echo "FAIL: expected exit $expected, got $actual: $*"
    cat "$work/stdout" "$work/stderr"
    failures=$((failures + 1))
  fi
}

expect_output() {
  if ! grep -q -- "$1" "$work/stdout"; then
    echo "FAIL: output lacks '$1'"
    cat "$work/stdout"
    failures=$((failures + 1))
  fi
}

cat >"$work/cascade.sql" <<'SQL'
CREATE TABLE accounts(id INTEGER PRIMARY KEY);
CREATE TABLE orders(account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE);
PRAGMA foreign_keys=ON;
BEGIN;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
COMMIT;
SQL

expect_exit 0 "$binary" --help
expect_output "SQLD001"

printf 'CREATE TABLE widgets(id INTEGER PRIMARY KEY);\n' >"$work/safe.sql"
expect_exit 0 sh -c "\"$binary\" - < \"$work/safe.sql\""
expect_output "No findings."

expect_exit 1 "$binary" "$work/cascade.sql"
expect_output "SQLD001"

expect_exit 1 "$binary" --format json "$work/cascade.sql"
python3 - "$work/stdout" <<'PY' || failures=$((failures + 1))
import json, sys
report = json.load(open(sys.argv[1]))
assert report["version"] == 1 and report["complete"] is True, report
codes = [finding["code"] for finding in report["findings"]]
assert codes == ["SQLD001"], codes
assert report["findings"][0]["location"]["line"] == 7, report["findings"][0]
PY

{ printf -- '-- sqlite-doctor:incomplete smoke test\n'; cat "$work/cascade.sql"; } >"$work/incomplete.sql"
expect_exit 2 "$binary" "$work/incomplete.sql"
expect_output "Check incomplete:"

expect_exit 2 "$binary" "$work/missing.sql"

if [ "$failures" -ne 0 ]; then
  echo "$failures smoke check(s) failed"
  exit 1
fi
echo "release smoke checks passed: $("$binary" --help | head -n 1)"

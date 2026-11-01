use sqlite_doctor::scan_sql;

#[test]
fn trigger_body_is_one_statement_and_cannot_change_outer_state() {
    let sql = r#"
BEGIN;
CREATE TEMP TRIGGER audit AFTER UPDATE ON widgets BEGIN
  INSERT INTO log VALUES (CASE WHEN new.id > 0 THEN 'END; COMMIT;' ELSE 'x' END);
  CREATE TABLE __new_fake(id);
  INSERT INTO __new_fake SELECT id FROM fake;
  DROP TABLE fake;
  ALTER TABLE __new_fake RENAME TO fake;
END;
PRAGMA foreign_keys = OFF;
END;
PRAGMA foreign_keys = OFF;
"#;
    let report = scan_sql(sql);
    assert_eq!(report.foreign_keys_pragmas_in_transactions.len(), 1);
    assert_eq!(report.foreign_keys_pragmas_in_transactions[0].line, 10);
    assert!(report.rebuilds.is_empty());
}

#[test]
fn transaction_and_savepoint_state_matches_sqlite_semantics() {
    let sql = r#"
SAVEPOINT outer;
SAVEPOINT inner;
ROLLBACK TO SAVEPOINT outer;
PRAGMA foreign_keys=NO;
RELEASE outer;
PRAGMA foreign_keys=OFF;
BEGIN IMMEDIATE;
SAVEPOINT one;
RELEASE SAVEPOINT one;
PRAGMA foreign_keys=0;
ROLLBACK TO one;
PRAGMA foreign_keys=FALSE;
ROLLBACK;
PRAGMA foreign_keys=OFF;
"#;
    let report = scan_sql(sql);
    let lines: Vec<_> = report
        .foreign_keys_pragmas_in_transactions
        .iter()
        .map(|finding| finding.line)
        .collect();
    assert_eq!(lines, vec![5, 11, 13]);
}

#[test]
fn pragma_assignment_accepts_schema_and_boolean_spellings_but_not_queries() {
    let sql = r#"
BEGIN;
PRAGMA main.foreign_keys = off;
pragma "aux" . FOREIGN_KEYS=0;
PRAGMA foreign_keys = FaLsE;
PRAGMA foreign_keys=NO;
PRAGMA foreign_keys;
PRAGMA foreign_keys();
PRAGMA foreign_keys(ON);
PRAGMA foreign_keys(OFF);
PRAGMA main.foreign_keys(0);
pragma "aux" . FOREIGN_KEYS(FaLsE);
PRAGMA foreign_keys(NO);
COMMIT;
"#;
    let report = scan_sql(sql);
    let lines: Vec<_> = report
        .foreign_keys_pragmas_in_transactions
        .iter()
        .map(|finding| finding.line)
        .collect();
    assert_eq!(lines, vec![3, 4, 5, 6, 9, 10, 11, 12, 13]);
}

#[test]
fn lexer_ignores_decoys_and_handles_quoting_crlf_and_eof() {
    let sql = concat!(
        "BEGIN;\r\n",
        "-- PRAGMA foreign_keys=OFF;\r\n",
        "SELECT 'it''s; END', x'434f4d4d49543b';\r\n",
        "SELECT \"a\"\";b\", `c``;d`, [e]];f];\r\n",
        "/* CREATE TABLE __new_bad(x); */ PRAGMA foreign_keys = 0"
    );
    let report = scan_sql(sql);
    assert_eq!(report.foreign_keys_pragmas_in_transactions.len(), 1);
    assert_eq!(report.foreign_keys_pragmas_in_transactions[0].line, 5);
    assert!(report.rebuilds.is_empty());
}

#[test]
fn rebuild_variants_normalize_names_and_allow_unrelated_statements() {
    let sql = r#"
CREATE TABLE IF NOT EXISTS "Main"."__NeW_Users" (id INTEGER, name TEXT);
SELECT 'unrelated';
INSERT OR IGNORE INTO [__new_users] (id, name)
  SELECT id, name FROM `main`.`USERS` WHERE id > 0;
PRAGMA user_version=2;
DROP TABLE IF EXISTS "Users";
SELECT 1;
ALTER TABLE main.__NEW_USERS RENAME TO [users];
"#;
    let report = scan_sql(sql);
    assert_eq!(report.rebuilds.len(), 1);
    let rebuild = &report.rebuilds[0];
    assert_eq!(rebuild.temporary_table, "main.__new_users");
    assert_eq!(rebuild.original_table, "main.users");
    assert_eq!(
        (
            rebuild.create_line,
            rebuild.copy_line,
            rebuild.drop_line,
            rebuild.replacement_line
        ),
        (2, 4, 7, 9)
    );
}

#[test]
fn supports_two_independent_rebuilds_without_cross_candidate_reuse() {
    let sql = r#"
CREATE TABLE __new_a(id);
CREATE TABLE __new_b(id);
INSERT OR REPLACE INTO __new_b SELECT id FROM b;
DROP TABLE b;
ALTER TABLE __new_b RENAME TO b;
INSERT INTO __new_a(id) SELECT id FROM a WHERE id IS NOT NULL;
DROP TABLE IF EXISTS a;
ALTER TABLE __new_a RENAME TO a;
"#;
    let report = scan_sql(sql);
    assert_eq!(report.rebuilds.len(), 2);
    assert_eq!(report.rebuilds[0].original_table, "b");
    assert_eq!(report.rebuilds[1].original_table, "a");
}

#[test]
fn rejects_rename_column_out_of_order_and_cross_candidate_matches() {
    let rename_column = r#"
CREATE TABLE __new_t(id);
INSERT INTO __new_t SELECT id FROM t;
DROP TABLE t;
ALTER TABLE __new_t RENAME COLUMN id TO other;
"#;
    assert!(scan_sql(rename_column).rebuilds.is_empty());

    let out_of_order = r#"
CREATE TABLE __new_t(id);
INSERT INTO __new_t SELECT id FROM t;
ALTER TABLE __new_t RENAME TO t;
DROP TABLE t;
"#;
    assert!(scan_sql(out_of_order).rebuilds.is_empty());

    let reused_steps = r#"
CREATE TABLE __new_t(id);
INSERT INTO __new_t SELECT id FROM t;
CREATE TABLE __new_t(id, value);
DROP TABLE t;
ALTER TABLE __new_t RENAME TO t;
"#;
    assert!(scan_sql(reused_steps).rebuilds.is_empty());
}

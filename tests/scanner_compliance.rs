use sqlite_doctor::{RebuildKind, scan_sql};

#[test]
fn trigger_body_is_one_statement_and_cannot_change_outer_state() {
    let sql = r#"
CREATE TABLE widgets(id INTEGER);
CREATE TABLE log(message TEXT);
CREATE TABLE fake(id INTEGER);
CREATE TABLE __new_fake(id INTEGER);
BEGIN;
CREATE TEMP TRIGGER audit AFTER UPDATE ON widgets BEGIN
  INSERT INTO log VALUES (CASE WHEN new.id > 0 THEN 'END; COMMIT;' ELSE 'x' END);
  INSERT INTO __new_fake SELECT id FROM fake;
END;
PRAGMA foreign_keys = OFF;
DROP TRIGGER audit;
DROP TABLE fake;
ALTER TABLE __new_fake RENAME TO fake;
END;
PRAGMA foreign_keys = OFF;
"#;
    let report = scan_sql(sql);
    assert_eq!(report.schema_objects.len(), 1);
    assert_eq!(report.foreign_keys_pragmas_in_transactions.len(), 1);
    assert_eq!(report.foreign_keys_pragmas_in_transactions[0].line, 11);
    assert!(report.rebuilds.is_empty());
}

#[test]
fn every_insert_conflict_clause_copies_rows_in_both_rebuild_shapes() {
    for insert in [
        "INSERT INTO",
        "INSERT OR ABORT INTO",
        "INSERT OR FAIL INTO",
        "INSERT OR IGNORE INTO",
        "INSERT OR REPLACE INTO",
        "INSERT OR ROLLBACK INTO",
        "REPLACE INTO",
    ] {
        let rename = format!(
            "CREATE TABLE __new_t(id);\n{insert} __new_t SELECT id FROM t;\nDROP TABLE t;\nALTER TABLE __new_t RENAME TO t;\n"
        );
        let recreate = format!(
            "CREATE TEMP TABLE held(id);\n{insert} held SELECT id FROM t;\nDROP TABLE t;\nCREATE TABLE t(id INTEGER PRIMARY KEY);\n{insert} t SELECT id FROM held;\n"
        );
        assert_eq!(scan_sql(&rename).rebuilds.len(), 1, "{insert}");
        assert_eq!(scan_sql(&recreate).rebuilds.len(), 1, "{insert}");
    }
}

#[test]
fn a_copy_reads_from_the_table_of_its_own_select() {
    let sql = r#"
CREATE TABLE __new_t(id, label);
INSERT INTO __new_t (id, label)
  SELECT id, (SELECT label FROM labels WHERE labels.id = t.id) FROM t;
DROP TABLE t;
ALTER TABLE __new_t RENAME TO t;
"#;
    let report = scan_sql(sql);
    assert_eq!(report.rebuilds.len(), 1);
    assert_eq!(report.rebuilds[0].original_table, "t");
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
fn every_rollback_to_savepoint_spelling_stays_inside_the_transaction() {
    for rollback in [
        "ROLLBACK TO s",
        "ROLLBACK TO SAVEPOINT s",
        "ROLLBACK TRANSACTION TO s",
        "ROLLBACK TRANSACTION TO SAVEPOINT s",
        "ROLLBACK TRANSACTION migration TO s",
    ] {
        let sql = format!(
            "BEGIN;\nSAVEPOINT s;\n{rollback};\nPRAGMA foreign_keys=OFF;\nCREATE TABLE __new_t(id);\nINSERT INTO __new_t SELECT id FROM t;\nDROP TABLE t;\nALTER TABLE __new_t RENAME TO t;\nCOMMIT;\n"
        );
        let report = scan_sql(&sql);
        let lines: Vec<_> = report
            .foreign_keys_pragmas_in_transactions
            .iter()
            .map(|pragma| pragma.line)
            .collect();
        assert_eq!(lines, vec![4], "{rollback}");
        assert_eq!(
            report.rebuilds[0].foreign_keys_enabled_at_drop, None,
            "{rollback}"
        );
    }
}

#[test]
fn named_transactions_end_like_unnamed_ones() {
    for end in [
        "COMMIT TRANSACTION migration",
        "END TRANSACTION 'migration'",
        "ROLLBACK TRANSACTION migration",
    ] {
        let sql = format!("BEGIN TRANSACTION migration;\n{end};\nPRAGMA foreign_keys=OFF;\n");
        let report = scan_sql(&sql);
        assert!(
            report.foreign_keys_pragmas_in_transactions.is_empty(),
            "{end}"
        );
    }
}

#[test]
fn a_rejected_commit_and_a_quoted_savepoint_leave_the_transaction_open() {
    for opening in ["BEGIN;\nCOMMIT unexpected", "SAVEPOINT 's'"] {
        let sql = format!("{opening};\nPRAGMA foreign_keys=OFF;\n");
        let report = scan_sql(&sql);
        assert_eq!(
            report.foreign_keys_pragmas_in_transactions.len(),
            1,
            "{opening}"
        );
    }
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

#[test]
fn rename_away_rebuild_ends_at_the_drop_of_the_renamed_table() {
    let sql = r#"
ALTER TABLE users RENAME TO users_old;
CREATE TABLE users(id INTEGER, name TEXT);
INSERT INTO users SELECT id, name FROM users_old;
DROP TABLE users_old;
"#;
    let report = scan_sql(sql);
    assert_eq!(report.rebuilds.len(), 1);
    let rebuild = &report.rebuilds[0];
    assert_eq!(rebuild.kind, RebuildKind::RenameAway);
    assert_eq!(rebuild.original_table, "users");
    assert_eq!(rebuild.temporary_table, "users_old");
    assert_eq!(rebuild.dropped_table(), "users_old");
    assert_eq!(
        (
            rebuild.create_line,
            rebuild.copy_line,
            rebuild.drop_line,
            rebuild.replacement_line
        ),
        (3, 4, 5, 3)
    );

    let copied_before_the_rename = r#"
CREATE TABLE users(id INTEGER, name TEXT);
INSERT INTO users SELECT id, name FROM users_old;
ALTER TABLE users RENAME TO users_old;
DROP TABLE users_old;
"#;
    assert!(scan_sql(copied_before_the_rename).rebuilds.is_empty());

    let nothing_copied = r#"
ALTER TABLE users RENAME TO users_old;
CREATE TABLE users(id INTEGER, name TEXT);
DROP TABLE users_old;
"#;
    assert!(scan_sql(nothing_copied).rebuilds.is_empty());
}

#[test]
fn a_rename_after_the_drop_makes_it_a_rename_rebuild_reported_once() {
    let sql = r#"
CREATE TABLE __new_t(id);
INSERT INTO __new_t SELECT id FROM t;
DROP TABLE t;
ALTER TABLE __new_t RENAME TO t;
CREATE TABLE __new_t(id, value);
INSERT INTO __new_t SELECT id, NULL FROM t;
DROP TABLE t;
ALTER TABLE __new_t RENAME TO t;
"#;
    let report = scan_sql(sql);
    assert_eq!(
        report
            .rebuilds
            .iter()
            .map(|rebuild| (rebuild.kind, rebuild.drop_line))
            .collect::<Vec<_>>(),
        [(RebuildKind::Rename, 4), (RebuildKind::Rename, 8)]
    );
}

#[test]
fn a_with_clause_is_part_of_the_copy_only_in_an_insert() {
    for copy in [
        "WITH src AS (SELECT id FROM t) INSERT INTO __new_t SELECT id FROM src",
        "WITH RECURSIVE src(id) AS NOT MATERIALIZED (SELECT id FROM t) REPLACE INTO __new_t SELECT id FROM src",
        "INSERT INTO __new_t(id) WITH src AS (SELECT id FROM t) SELECT id FROM src",
        "WITH \"insert\" AS (SELECT id FROM t) INSERT OR IGNORE INTO __new_t SELECT id FROM \"insert\"",
    ] {
        let sql = format!(
            "CREATE TABLE __new_t(id);\n{copy};\nDROP TABLE t;\nALTER TABLE __new_t RENAME TO t;"
        );
        let report = scan_sql(&sql);
        assert_eq!(report.rebuilds.len(), 1, "{copy}");
        assert_eq!(report.rebuilds[0].original_table, "t", "{copy}");
        assert_eq!(report.rebuilds[0].copy_line, 2, "{copy}");
    }

    for not_a_copy in [
        "WITH src AS (SELECT id FROM t) SELECT id FROM src",
        "WITH src AS (SELECT id FROM t) DELETE FROM __new_t WHERE id IN (SELECT id FROM src)",
        "WITH",
    ] {
        let sql = format!(
            "CREATE TABLE __new_t(id);\n{not_a_copy};\nDROP TABLE t;\nALTER TABLE __new_t RENAME TO t;"
        );
        assert!(scan_sql(&sql).rebuilds.is_empty(), "{not_a_copy}");
    }
}

use sqlite_doctor::{HazardId, RebuildKind, diagnose, scan_sql};

fn codes(sql: &str) -> Vec<(&'static str, usize)> {
    diagnose(&scan_sql(sql))
        .into_iter()
        .map(|finding| (finding.hazard.code(), finding.line))
        .collect()
}

#[test]
fn rails_add_foreign_key_capture_reports_cascade_loss_at_the_drop() {
    let sql = include_str!("fixtures/rails_8_1_add_foreign_key.sql");
    let report = scan_sql(sql);

    assert_eq!(report.rebuilds.len(), 1);
    let rebuild = &report.rebuilds[0];
    assert_eq!(rebuild.kind, RebuildKind::Recreate);
    assert_eq!(rebuild.temporary_table, "aparents");
    assert_eq!(rebuild.original_table, "parents");
    assert_eq!((rebuild.create_line, rebuild.drop_line), (52, 78));
    assert_eq!(rebuild.replacement_line, 91);
    assert_eq!(rebuild.foreign_keys_enabled_at_drop, Some(true));

    assert_eq!(
        codes(sql),
        vec![("SQLD002", 39), ("SQLD001", 78), ("SQLD002", 122)]
    );
}

#[test]
fn rails_rebuild_without_cascading_dependents_is_not_reported_as_data_loss() {
    let sql = include_str!("fixtures/rails_8_1_change_default_control.sql");

    assert_eq!(scan_sql(sql).rebuilds.len(), 1);
    assert!(
        diagnose(&scan_sql(sql))
            .iter()
            .all(|finding| finding.hazard != HazardId::DeleteActionOnRebuild)
    );
}

#[test]
fn rails_rebuild_reports_the_dropped_trigger_but_not_restored_indexes() {
    let sql = include_str!("fixtures/rails_8_1_trigger_on_rebuilt_table.sql");
    let findings: Vec<_> = diagnose(&scan_sql(sql))
        .into_iter()
        .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
        .collect();

    assert_eq!(findings.len(), 1);
    assert!(findings[0].detail.contains("trigger `trg_parents_update`"));
    assert_eq!(findings[0].related_line, Some(9));
}

#[test]
fn recreate_shape_requires_copying_rows_back() {
    let sql = r#"
CREATE TABLE posts (id integer);
CREATE TABLE comments (post_id integer REFERENCES posts(id) ON DELETE CASCADE);
CREATE TEMPORARY TABLE aposts (id integer);
INSERT INTO aposts SELECT id FROM posts;
DROP TABLE posts;
CREATE TABLE posts (id integer, title text);
"#;

    assert!(scan_sql(sql).rebuilds.is_empty());
    assert!(codes(sql).is_empty());
}

#[test]
fn objects_on_the_holding_table_do_not_restore_the_original() {
    let sql = r#"
CREATE TABLE users (id integer, email text);
CREATE INDEX idx_users_email ON users(email);
CREATE TEMPORARY TABLE ausers (id integer, email text);
INSERT INTO ausers SELECT id, email FROM users;
DROP TABLE users;
CREATE INDEX idx_users_email ON ausers(email);
CREATE TABLE users (id integer, email text NOT NULL);
INSERT INTO users SELECT id, email FROM ausers;
DROP TABLE ausers;
"#;

    let report = scan_sql(sql);
    assert_eq!(report.rebuilds[0].kind, RebuildKind::Recreate);
    assert_eq!(codes(sql), vec![("SQLD003", 6)]);
}

#[test]
fn recreate_shape_counts_objects_created_after_the_recreated_table() {
    let sql = r#"
CREATE TABLE users (id integer, email text);
CREATE INDEX idx_users_email ON users(email);
CREATE TEMPORARY TABLE ausers (id integer, email text);
INSERT INTO ausers SELECT id, email FROM users;
DROP TABLE users;
CREATE TABLE users (id integer, email text NOT NULL);
CREATE INDEX idx_users_email ON users(email);
INSERT INTO users SELECT id, email FROM ausers;
DROP TABLE ausers;
"#;

    assert!(codes(sql).is_empty());
}

#[test]
fn set_null_and_set_default_dependents_are_reported_but_restrict_is_not() {
    let rebuild = r#"
CREATE TEMPORARY TABLE aparents (id integer);
INSERT INTO aparents SELECT id FROM parents;
DROP TABLE parents;
CREATE TABLE parents (id integer, name text);
INSERT INTO parents SELECT id FROM aparents;
"#;
    for (action, reported) in [
        ("ON DELETE SET NULL", true),
        ("ON DELETE SET DEFAULT", true),
        ("ON DELETE RESTRICT", false),
        ("ON DELETE NO ACTION", false),
        ("ON UPDATE SET NULL", false),
    ] {
        let sql = format!(
            "CREATE TABLE parents (id integer);\nCREATE TABLE children (parent_id integer REFERENCES parents(id) {action});\n{rebuild}"
        );
        let findings: Vec<_> = diagnose(&scan_sql(&sql))
            .into_iter()
            .filter(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
            .collect();
        assert_eq!(findings.len(), usize::from(reported), "{action}");
        if reported {
            assert!(findings[0].detail.contains(action), "{action}");
            assert!(findings[0].detail.contains("foreign keys"), "{action}");
        }
    }
}

#[test]
fn a_backup_copy_does_not_hide_the_rebuild_that_drops_the_same_table() {
    let sql = include_str!("fixtures/backup_copy_beside_rebuild.sql");
    let report = scan_sql(sql);

    assert_eq!(report.rebuilds.len(), 1);
    assert_eq!(report.rebuilds[0].temporary_table, "__new_users");
    assert_eq!(codes(sql), vec![("SQLD001", 11), ("SQLD003", 11)]);
}

#[test]
fn a_backup_holding_table_does_not_hide_the_recreate_rebuild() {
    let sql = r#"
CREATE TABLE parents (id integer);
CREATE TABLE children (parent_id integer REFERENCES parents(id) ON DELETE CASCADE);
CREATE TABLE aparents (id integer);
INSERT INTO aparents SELECT id FROM parents;
CREATE TABLE parents_backup (id integer);
INSERT INTO parents_backup SELECT id FROM parents;
DROP TABLE parents;
CREATE TABLE parents (id integer, name text);
INSERT INTO parents SELECT id FROM aparents;
"#;
    let report = scan_sql(sql);

    assert_eq!(report.rebuilds.len(), 1);
    assert_eq!(report.rebuilds[0].temporary_table, "aparents");
    assert_eq!(codes(sql), vec![("SQLD001", 8)]);
}

#[test]
fn copying_a_table_into_itself_is_not_a_rebuild() {
    let sql = include_str!("fixtures/self_copy_is_not_a_rebuild.sql");

    assert!(scan_sql(sql).rebuilds.is_empty());
    assert!(codes(sql).is_empty());
}

#[test]
fn a_temp_table_shadowing_the_original_can_hold_its_rows() {
    let sql = include_str!("fixtures/temp_table_shadows_original.sql");

    assert_eq!(scan_sql(sql).rebuilds.len(), 1);
    assert_eq!(codes(sql), vec![("SQLD001", 9), ("SQLD003", 9)]);
}

#[test]
fn two_holding_copies_of_one_dropped_table_report_one_rebuild() {
    let sql = include_str!("fixtures/two_holding_copies_one_rebuild.sql");

    assert_eq!(scan_sql(sql).rebuilds.len(), 1);
    assert_eq!(codes(sql), vec![("SQLD001", 11), ("SQLD003", 11)]);
}

#[test]
fn a_qualified_self_copy_does_not_consume_the_rebuild_candidate() {
    let sql = include_str!("fixtures/qualified_self_copy_before_rebuild.sql");

    assert_eq!(codes(sql), vec![("SQLD001", 10), ("SQLD003", 10)]);
}

#[test]
fn a_qualified_self_copy_is_not_a_rebuild() {
    let sql = include_str!("fixtures/qualified_self_copy_is_not_a_rebuild.sql");

    assert!(scan_sql(sql).rebuilds.is_empty());
    assert!(codes(sql).is_empty());
}

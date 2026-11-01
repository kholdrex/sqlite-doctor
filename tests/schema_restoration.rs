use sqlite_doctor::{HazardId, SchemaObjectKind, diagnose, scan_sql};

fn rebuild(body_before: &str, body_after_drop: &str) -> String {
    format!(
        "{body_before}\nCREATE TABLE __new_users(id INTEGER, email TEXT);\nINSERT INTO __new_users SELECT id, email FROM users;\nDROP TABLE users;\n{body_after_drop}\nALTER TABLE __new_users RENAME TO users;\n"
    )
}

#[test]
fn reports_each_explicit_index_and_trigger_not_restored_after_rebuild() {
    let sql = rebuild(
        "CREATE UNIQUE INDEX idx_users_email ON users(email);\nCREATE TRIGGER users_audit AFTER UPDATE ON users BEGIN INSERT INTO audit_log DEFAULT VALUES; END;",
        "",
    );

    let report = scan_sql(&sql);
    assert_eq!(report.schema_objects.len(), 2);
    assert_eq!(report.schema_objects[0].kind, SchemaObjectKind::Index);
    assert_eq!(report.schema_objects[1].kind, SchemaObjectKind::Trigger);

    let findings: Vec<_> = diagnose(&report)
        .into_iter()
        .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
        .collect();
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|finding| finding.line == 5));
    assert_eq!(findings[0].related_line, Some(1));
    assert_eq!(findings[1].related_line, Some(2));
    assert!(findings[0].detail.contains("`idx_users_email`"));
    assert!(findings[1].detail.contains("`users_audit`"));
}

#[test]
fn recreation_on_replacement_table_between_drop_and_rename_is_restored() {
    let sql = rebuild(
        "CREATE INDEX idx_users_email ON users(email);\nCREATE TRIGGER users_audit AFTER UPDATE ON users BEGIN SELECT 1; END;",
        "CREATE INDEX IF NOT EXISTS idx_users_email ON __new_users(email);\nCREATE TRIGGER users_audit AFTER UPDATE ON __new_users BEGIN SELECT 1; END;",
    );

    assert!(
        diagnose(&scan_sql(&sql))
            .iter()
            .all(|finding| finding.hazard != HazardId::MissingSchemaObjectRestoration)
    );
}

#[test]
fn explicit_object_removal_before_table_drop_is_intentional() {
    let sql = rebuild(
        "CREATE INDEX idx_users_email ON users(email);\nDROP INDEX IF EXISTS idx_users_email;\nCREATE TRIGGER users_audit AFTER UPDATE ON users BEGIN SELECT 1; END;\nDROP TRIGGER users_audit;",
        "",
    );

    assert!(
        diagnose(&scan_sql(&sql))
            .iter()
            .all(|finding| finding.hazard != HazardId::MissingSchemaObjectRestoration)
    );
}

#[test]
fn quoted_names_match_recreation_but_different_names_do_not() {
    let restored = rebuild(
        "CREATE UNIQUE INDEX \"IDX_Email\" ON [users](email);",
        "CREATE INDEX `idx_email` ON `__new_users`(email);",
    );
    assert!(diagnose(&scan_sql(&restored)).is_empty());

    let renamed = rebuild(
        "CREATE INDEX idx_users_email ON users(email);",
        "CREATE INDEX idx_users_email_v2 ON users(email);",
    );
    let findings = diagnose(&scan_sql(&renamed));
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
            .count(),
        1
    );
}

#[test]
fn unrelated_and_post_rebuild_objects_do_not_create_expectations() {
    let sql = rebuild(
        "CREATE INDEX idx_orders_state ON orders(state);\nCREATE TRIGGER orders_touch AFTER UPDATE ON orders BEGIN UPDATE users SET email = email; END;\nCREATE VIEW active_users AS SELECT * FROM users;",
        "CREATE INDEX idx_users_email ON users(email);",
    );

    assert!(diagnose(&scan_sql(&sql)).is_empty());
}

#[test]
fn trigger_body_is_not_split_into_top_level_schema_or_rebuild_statements() {
    let sql = r#"
CREATE TRIGGER users_audit AFTER UPDATE ON users BEGIN
  INSERT INTO audit_log(message) VALUES ('CREATE INDEX fake ON users(id);');
  INSERT INTO __new_users SELECT * FROM users;
  DROP TABLE users;
END;
"#;

    let report = scan_sql(sql);
    assert_eq!(report.schema_objects.len(), 1);
    assert_eq!(report.schema_objects[0].kind, SchemaObjectKind::Trigger);
    assert!(report.rebuilds.is_empty());
}

#[test]
fn trigger_attachment_ignores_on_named_update_columns() {
    let sql = rebuild(
        "CREATE TRIGGER users_audit AFTER UPDATE OF \"on\", email ON users BEGIN SELECT 1; END;",
        "",
    );

    let findings: Vec<_> = diagnose(&scan_sql(&sql))
        .into_iter()
        .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
        .collect();
    assert_eq!(findings.len(), 1);
    assert!(findings[0].detail.contains("`users_audit`"));
}

#[test]
fn restoration_on_temporary_table_remains_visible_to_later_rebuilds() {
    let sql = r#"
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
CREATE INDEX idx_users_email ON __new_users(email);
ALTER TABLE __new_users RENAME TO users;
CREATE TABLE __new_users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
"#;

    let findings: Vec<_> = diagnose(&scan_sql(sql))
        .into_iter()
        .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
        .collect();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 10);
    assert_eq!(findings[0].related_line, Some(6));
}

#[test]
fn repeated_if_not_exists_creation_yields_one_missing_object() {
    let sql = rebuild(
        "CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);\nCREATE INDEX IF NOT EXISTS idx_users_email ON users(email);",
        "",
    );

    assert_eq!(
        diagnose(&scan_sql(&sql))
            .iter()
            .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
            .count(),
        1
    );
}

#[test]
fn invalid_restoration_table_windows_do_not_suppress() {
    let original_before_rename = rebuild(
        "CREATE INDEX idx_users_email ON users(email);",
        "CREATE INDEX idx_users_email ON users(email);",
    );
    assert_eq!(
        diagnose(&scan_sql(&original_before_rename))
            .iter()
            .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
            .count(),
        1
    );

    let temporary_after_rename = format!(
        "{}CREATE INDEX idx_users_email ON __new_users(email);\n",
        rebuild("CREATE INDEX idx_users_email ON users(email);", "")
    );
    assert_eq!(
        diagnose(&scan_sql(&temporary_after_rename))
            .iter()
            .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
            .count(),
        1
    );
}

#[test]
fn temporary_trigger_is_tracked() {
    let sql = rebuild(
        "CREATE TEMP TRIGGER users_audit AFTER UPDATE ON users BEGIN SELECT 1; END;",
        "",
    );
    let findings = diagnose(&scan_sql(&sql));
    assert!(findings.iter().any(|finding| {
        finding.hazard == HazardId::MissingSchemaObjectRestoration
            && finding.detail.contains("`users_audit`")
    }));
}

#[test]
fn repeated_rebuild_only_tracks_the_current_object_instance() {
    let sql = r#"
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
"#;

    let findings: Vec<_> = diagnose(&scan_sql(sql))
        .into_iter()
        .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
        .collect();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 10);
    assert_eq!(findings[0].related_line, Some(7));
}

#[test]
fn qualified_object_names_match_only_compatible_qualification() {
    let restored = rebuild(
        "CREATE INDEX main.idx_users_email ON users(email);",
        "CREATE INDEX idx_users_email ON __new_users(email);",
    );
    assert!(diagnose(&scan_sql(&restored)).is_empty());

    let wrong_schema = rebuild(
        "CREATE INDEX main.idx_users_email ON users(email);",
        "CREATE INDEX temp.idx_users_email ON __new_users(email);",
    );
    assert_eq!(
        diagnose(&scan_sql(&wrong_schema))
            .iter()
            .filter(|finding| finding.hazard == HazardId::MissingSchemaObjectRestoration)
            .count(),
        1
    );
}

#[test]
fn index_on_a_column_the_rebuild_removes_is_not_reported() {
    let recreate = r#"
CREATE TABLE likes (id integer, post_id integer, user_id integer);
CREATE INDEX index_likes_on_post_id ON likes (post_id);
CREATE INDEX index_likes_on_user_id ON likes (user_id);
CREATE TEMPORARY TABLE alikes (id integer, post_id integer, user_id integer);
INSERT INTO alikes SELECT id, post_id, user_id FROM likes;
DROP TABLE likes;
CREATE TABLE likes (id integer, user_id integer);
INSERT INTO likes SELECT id, user_id FROM alikes;
DROP TABLE alikes;
"#;
    let findings = diagnose(&scan_sql(recreate));
    assert_eq!(findings.len(), 1);
    assert!(findings[0].detail.contains("index_likes_on_user_id"));

    let rename = r#"
CREATE TABLE likes (id integer, post_id integer);
CREATE INDEX index_likes_on_post_id ON likes ("post_id" DESC);
CREATE TABLE __new_likes (id integer);
INSERT INTO __new_likes SELECT id FROM likes;
DROP TABLE likes;
ALTER TABLE __new_likes RENAME TO likes;
"#;
    assert!(diagnose(&scan_sql(rename)).is_empty());
}

#[test]
fn expression_and_multi_column_indexes_keep_the_conservative_check() {
    let sql = r#"
CREATE TABLE users (id integer, email text, name text);
CREATE INDEX index_users_on_lower_email ON users (lower(email));
CREATE INDEX index_users_on_name_and_email ON users (name, email COLLATE NOCASE);
CREATE TEMPORARY TABLE ausers (id integer, email text, name text);
INSERT INTO ausers SELECT id, email, name FROM users;
DROP TABLE users;
CREATE TABLE users (id integer, name text);
INSERT INTO users SELECT id, name FROM ausers;
"#;
    let names: Vec<_> = diagnose(&scan_sql(sql))
        .into_iter()
        .map(|finding| finding.detail)
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].contains("index_users_on_lower_email"));
}

#[test]
fn literal_index_keys_and_unreadable_column_lists_keep_the_conservative_check() {
    for key in ["1", "NULL", "TRUE", "x'00'", "-1"] {
        let sql = format!(
            "CREATE TABLE t(id);\nCREATE INDEX idx ON t({key});\nCREATE TABLE n(id);\nINSERT INTO n SELECT id FROM t;\nDROP TABLE t;\nALTER TABLE n RENAME TO t;\n"
        );
        assert_eq!(diagnose(&scan_sql(&sql)).len(), 1, "{key}");
    }

    let quoted = "CREATE TABLE t(id, 'email' TEXT);\nCREATE INDEX idx ON t(email);\nCREATE TABLE n(id, 'email' TEXT);\nINSERT INTO n SELECT * FROM t;\nDROP TABLE t;\nALTER TABLE n RENAME TO t;\n";
    assert_eq!(diagnose(&scan_sql(quoted)).len(), 1);
}

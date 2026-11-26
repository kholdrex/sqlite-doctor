//! Each fixture under `tests/fixtures/sqlite` is a script for the `sqlite3`
//! shell. Its first line records what SQLite prints for it: the child rows
//! before and after the parent table is dropped, or how many indexes or
//! triggers of that name are left. `tests/sqlite_outcomes.sh` checks those lines with `sqlite3`.

use sqlite_doctor::{Finding, HazardId, diagnose, scan_sql};

fn fixtures(directory: &str) -> Vec<(String, String)> {
    let path = format!(
        "{}/tests/fixtures/sqlite/{directory}",
        env!("CARGO_MANIFEST_DIR")
    );
    let fixtures: Vec<_> = std::fs::read_dir(&path)
        .expect("fixture dir")
        .map(|entry| {
            let path = entry.expect("fixture").path();
            let sql = std::fs::read_to_string(&path).expect("fixture text");
            (path.display().to_string(), sql)
        })
        .collect();
    assert!(!fixtures.is_empty(), "{path}");
    fixtures
}

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/sqlite/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).expect("fixture text")
}

fn findings(sql: &str, hazard: HazardId) -> Vec<Finding> {
    diagnose(&scan_sql(sql))
        .into_iter()
        .filter(|finding| finding.hazard == hazard)
        .collect()
}

fn line_of(sql: &str, statement: &str) -> usize {
    sql.lines()
        .position(|line| line == statement)
        .expect("statement in fixture")
        + 1
}

#[test]
fn a_child_row_sqlite_deletes_is_reported() {
    for (path, sql) in fixtures("cascade") {
        assert!(
            sql.starts_with("-- sqlite3 prints: before=1 after=0\n"),
            "{path}"
        );
        assert_eq!(
            findings(&sql, HazardId::DeleteActionOnRebuild).len(),
            1,
            "{path}"
        );
    }
}

#[test]
fn a_rebuild_that_deletes_no_child_row_is_not_reported() {
    for (path, sql) in fixtures("no_cascade") {
        assert!(!sql.contains("after=0"), "{path}");
        assert_eq!(
            findings(&sql, HazardId::DeleteActionOnRebuild).len(),
            0,
            "{path}"
        );
    }
}

#[test]
fn an_index_or_trigger_sqlite_no_longer_has_is_reported() {
    for (path, sql) in fixtures("lost_index") {
        let outcome = sql.lines().next().expect("first line");
        assert!(
            ["index=0", "index_after_first_rebuild=0", "trigger=0"]
                .iter()
                .any(|lost| outcome.contains(lost)),
            "{path}"
        );
        assert_eq!(
            findings(&sql, HazardId::MissingSchemaObjectRestoration).len(),
            1,
            "{path}"
        );
    }
}

#[test]
fn an_index_that_is_kept_or_dropped_on_purpose_is_not_reported() {
    for (path, sql) in fixtures("no_lost_index") {
        assert_eq!(
            findings(&sql, HazardId::MissingSchemaObjectRestoration).len(),
            0,
            "{path}"
        );
    }
}

#[test]
fn a_renamed_parent_is_matched_by_its_new_name() {
    let sql = fixture("cascade/parent_renamed_before_rebuild.sql");
    let cascade = &findings(&sql, HazardId::DeleteActionOnRebuild)[0];

    assert_eq!(cascade.line, line_of(&sql, "DROP TABLE customers;"));
    assert!(
        cascade
            .detail
            .contains("`orders` references rebuilt table `customers`")
    );
}

#[test]
fn a_rename_away_rebuild_is_reported_at_the_drop_of_the_renamed_table() {
    let sql = fixture("cascade/renamed_away_and_replaced_under_the_old_name.sql");
    let cascade = &findings(&sql, HazardId::DeleteActionOnRebuild)[0];

    assert_eq!(cascade.line, line_of(&sql, "DROP TABLE accounts_old;"));
    assert!(
        cascade
            .detail
            .contains("`orders` references rebuilt table `accounts`")
    );
}

#[test]
fn a_table_recreated_as_select_is_reported_at_its_drop() {
    let sql = fixture("cascade/held_and_recreated_as_select.sql");
    let cascade = &findings(&sql, HazardId::DeleteActionOnRebuild)[0];

    assert_eq!(cascade.line, line_of(&sql, "DROP TABLE accounts;"));
    assert!(
        cascade
            .detail
            .contains("`orders` references rebuilt table `accounts`")
    );
}

#[test]
fn an_index_is_reported_when_create_table_as_select_leaves_its_column_out() {
    let sql = fixture("lost_index/indexed_column_not_selected_in_create_as_select.sql");
    let missing = &findings(&sql, HazardId::MissingSchemaObjectRestoration)[0];

    assert_eq!(missing.line, line_of(&sql, "DROP TABLE users;"));
    assert!(
        missing
            .detail
            .contains("Dropping `users` removes index `idx_users_email`")
    );
}

#[test]
fn an_index_lost_in_a_rename_away_rebuild_names_the_table_that_is_dropped() {
    let sql = fixture("lost_index/index_on_a_table_renamed_away.sql");
    let missing = &findings(&sql, HazardId::MissingSchemaObjectRestoration)[0];

    assert_eq!(missing.line, line_of(&sql, "DROP TABLE users_old;"));
    assert!(
        missing
            .detail
            .contains("Dropping `users_old` removes index `idx_users_email`")
    );
}

#[test]
fn a_renamed_child_is_reported_by_its_new_name() {
    let sql = fixture("cascade/child_renamed_before_rebuild.sql");
    let cascade = &findings(&sql, HazardId::DeleteActionOnRebuild)[0];

    assert!(cascade.detail.contains("Dependent table `purchases`"));
}

#[test]
fn a_rolled_back_duplicate_leaves_the_first_creation_as_the_lost_index() {
    let sql = fixture("lost_index/duplicate_creation_rolled_back.sql");
    let missing = &findings(&sql, HazardId::MissingSchemaObjectRestoration)[0];

    assert_eq!(
        missing.related_line,
        Some(line_of(
            &sql,
            "CREATE INDEX idx_users_email ON users(email);"
        ))
    );
}

#[test]
fn a_restoration_after_the_next_rebuild_does_not_cover_the_first() {
    let sql = fixture("lost_index/restored_only_after_a_second_rebuild.sql");
    let missing = &findings(&sql, HazardId::MissingSchemaObjectRestoration)[0];

    assert_eq!(missing.line, line_of(&sql, "DROP TABLE users;"));
}

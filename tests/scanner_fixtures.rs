use sqlite_doctor::scan_sql;

const RAILS_REBUILD: &str = include_str!("fixtures/rails_rebuild.sql");
const DRIZZLE_REBUILD: &str = include_str!("fixtures/drizzle_rebuild.sql");
const SAFE_FRAGMENTS: &str = include_str!("fixtures/safe_fragments.sql");

#[test]
fn rails_fixture_detects_transaction_pragma_and_rebuild() {
    let report = scan_sql(RAILS_REBUILD);

    assert_eq!(report.foreign_keys_off_in_transactions.len(), 1);
    assert_eq!(report.foreign_keys_off_in_transactions[0].line, 3);
    assert_eq!(report.rebuilds.len(), 1);

    let rebuild = &report.rebuilds[0];
    assert_eq!(rebuild.temporary_table, "__new_accounts");
    assert_eq!(rebuild.original_table, "accounts");
    assert_eq!(rebuild.create_line, 5);
    assert_eq!(rebuild.copy_line, 9);
    assert_eq!(rebuild.drop_line, 11);
    assert_eq!(rebuild.rename_line, 12);
}

#[test]
fn drizzle_fixture_detects_rebuild_but_not_out_of_scope_pragma() {
    let report = scan_sql(DRIZZLE_REBUILD);

    assert!(report.foreign_keys_off_in_transactions.is_empty());
    assert_eq!(report.rebuilds.len(), 1);
    assert_eq!(report.rebuilds[0].temporary_table, "__new_users");
    assert_eq!(report.rebuilds[0].original_table, "users");
}

#[test]
fn incomplete_sequences_and_sql_inside_literals_are_ignored() {
    let report = scan_sql(SAFE_FRAGMENTS);

    assert!(report.foreign_keys_off_in_transactions.is_empty());
    assert!(report.rebuilds.is_empty());
}

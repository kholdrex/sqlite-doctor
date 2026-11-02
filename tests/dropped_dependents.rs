use sqlite_doctor::{HazardId, diagnose, scan_sql};

const RENAME_REBUILD: &str = "CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
";

const RAILS_REBUILD: &str = "BEGIN;
CREATE TEMPORARY TABLE aaccounts (id INTEGER);
INSERT INTO aaccounts SELECT id FROM accounts;
DROP TABLE accounts;
CREATE TABLE accounts (id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts SELECT id FROM aaccounts;
DROP TABLE aaccounts;
COMMIT;
";

fn cascade_findings(sql: &str) -> usize {
    diagnose(&scan_sql(sql))
        .iter()
        .filter(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
        .count()
}

#[test]
fn a_dropped_dependent_is_not_cascaded_into() {
    let sql = format!(
        "CREATE TABLE \"main\".\"Orders\" (account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE);\nDROP TABLE IF EXISTS orders;\n{RENAME_REBUILD}"
    );
    assert_eq!(cascade_findings(&sql), 0);
}

#[test]
fn a_drop_committed_in_an_earlier_migration_retires_the_reference() {
    let sql = format!(
        "CREATE TABLE orders (account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE);\nBEGIN;\nDROP TABLE orders;\nCOMMIT;\n{RAILS_REBUILD}"
    );
    assert_eq!(cascade_findings(&sql), 0);
}

#[test]
fn a_drop_earlier_in_the_same_transaction_retires_the_reference() {
    let sql = format!(
        "CREATE TABLE orders (account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE);\n{}",
        RAILS_REBUILD.replacen("BEGIN;\n", "BEGIN;\nDROP TABLE orders;\n", 1)
    );
    assert_eq!(cascade_findings(&sql), 0);
}

#[test]
fn a_rolled_back_drop_keeps_the_reference() {
    let sql = format!(
        "CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);\nBEGIN;\nDROP TABLE orders;\nROLLBACK;\n{RENAME_REBUILD}"
    );
    assert_eq!(cascade_findings(&sql), 1);
}

#[test]
fn rolling_back_to_a_savepoint_before_the_drop_keeps_the_reference() {
    let sql = format!(
        "CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);\nBEGIN;\nSAVEPOINT a;\nDROP TABLE orders;\nROLLBACK TO a;\n{}COMMIT;\n",
        RAILS_REBUILD
            .replace("BEGIN;\n", "")
            .replace("COMMIT;\n", "")
    );
    assert_eq!(cascade_findings(&sql), 1);
}

#[test]
fn rolling_back_to_a_savepoint_set_after_the_drop_keeps_the_drop() {
    let sql = format!(
        "CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);\nBEGIN;\nSAVEPOINT a;\nDROP TABLE orders;\nRELEASE a;\nSAVEPOINT b;\nROLLBACK TO b;\n{}COMMIT;\n",
        RAILS_REBUILD
            .replace("BEGIN;\n", "")
            .replace("COMMIT;\n", "")
    );
    assert_eq!(cascade_findings(&sql), 0);
}

#[test]
fn a_dependent_created_again_declares_its_reference_anew() {
    let sql = format!(
        "CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);\nDROP TABLE orders;\nCREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);\n{RENAME_REBUILD}"
    );
    assert_eq!(cascade_findings(&sql), 1);
}

#[test]
fn a_dependent_rebuilt_with_a_new_cascade_before_the_parent_is_still_reported() {
    // Dropping the old child must not retire the cascade its replacement declares.
    let sql = format!(
        "CREATE TABLE checks (keyword_id INTEGER REFERENCES accounts(id));\n{}{RAILS_REBUILD}",
        "BEGIN;\nCREATE TEMPORARY TABLE achecks (keyword_id INTEGER);\nINSERT INTO achecks SELECT keyword_id FROM checks;\nDROP TABLE checks;\nCREATE TABLE checks (keyword_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE);\nINSERT INTO checks SELECT keyword_id FROM achecks;\nDROP TABLE achecks;\nCOMMIT;\n"
    );
    assert_eq!(cascade_findings(&sql), 1);
}

#[test]
fn a_dependent_rebuilt_without_its_foreign_key_is_no_longer_reported() {
    let sql = format!(
        "CREATE TABLE orders (account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE);\n{}{RAILS_REBUILD}",
        "BEGIN;\nCREATE TEMPORARY TABLE aorders (account_id INTEGER);\nINSERT INTO aorders SELECT account_id FROM orders;\nDROP TABLE orders;\nCREATE TABLE orders (account_id INTEGER);\nINSERT INTO orders SELECT account_id FROM aorders;\nDROP TABLE aorders;\nCOMMIT;\n"
    );
    assert_eq!(cascade_findings(&sql), 0);
}

#[test]
fn a_drop_with_trailing_tokens_does_not_retire_the_reference() {
    let sql = format!(
        "CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);\nDROP TABLE orders unexpected;\n{RENAME_REBUILD}"
    );
    assert_eq!(cascade_findings(&sql), 1);
}

/// Inputs where SQLite still deletes the child row at the parent rebuild, so
/// the reference must stay.
#[test]
fn drops_that_do_not_remove_the_child_keep_the_reference() {
    for (name, sql) in [
        (
            "temp shadow dropped",
            include_str!("fixtures/dropped/unqualified_shadow_drop.sql"),
        ),
        (
            "renamed away and back",
            include_str!("fixtures/dropped/rename_away_if_exists.sql"),
        ),
        (
            "ROLLBACK TRANSACTION TO",
            include_str!("fixtures/dropped/rollback_transaction_then_drop.sql"),
        ),
        (
            "single-quoted savepoint",
            include_str!("fixtures/dropped/single_quoted_rollback_target.sql"),
        ),
        (
            "malformed COMMIT",
            include_str!("fixtures/dropped/invalid_commit_suffix.sql"),
        ),
        (
            "three-part drop name",
            include_str!("fixtures/dropped/invalid_qualified_drop.sql"),
        ),
    ] {
        assert_eq!(cascade_findings(sql), 1, "{name}");
    }
}

/// More inputs where SQLite still cascades into the child: named and quoted
/// transactions, temp shadows brought back by a rollback, savepoints whose
/// names the scanner cannot read.
#[test]
fn ambiguous_transactions_and_temp_shadows_keep_the_reference() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dropped/round2");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).expect("fixture dir") {
        let path = entry.expect("fixture").path();
        let sql = std::fs::read_to_string(&path).expect("fixture text");
        assert_eq!(cascade_findings(&sql), 1, "{}", path.display());
        checked += 1;
    }
    assert_eq!(checked, 14);
}

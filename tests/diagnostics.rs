use sqlite_doctor::{HazardId, diagnose, scan_sql};

#[test]
fn emits_cascade_risk_for_dependent_table_referencing_rebuilt_parent() {
    let sql = r#"
CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE
);
BEGIN;
PRAGMA foreign_keys = OFF;
CREATE TABLE __new_accounts (id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_accounts SELECT id, name FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
COMMIT;
"#;

    let report = scan_sql(sql);
    assert_eq!(report.delete_action_references.len(), 1);
    assert_eq!(report.delete_action_references[0].dependent_table, "orders");
    assert_eq!(
        report.delete_action_references[0].referenced_table,
        "accounts"
    );

    let findings = diagnose(&report);
    let cascade = findings
        .iter()
        .find(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
        .expect("cascade risk should be diagnosed");
    assert_eq!(cascade.line, 10);
    assert!(cascade.detail.contains("`orders`"));
    assert!(cascade.detail.contains("`accounts`"));
    assert!(cascade.detail.contains("ON DELETE CASCADE"));
    let remediation = &cascade.remediation;
    let check = remediation
        .find("PRAGMA foreign_key_check before COMMIT")
        .expect("the integrity check belongs inside the transaction");
    let restore = remediation
        .find("Restore enforcement after the transaction ends")
        .expect("enforcement is restored after the transaction");
    assert!(
        remediation.starts_with("Disable foreign-key enforcement before starting the transaction")
    );
    assert!(check < restore);
}

#[test]
fn supports_table_constraints_qualified_names_and_multiple_rebuilds() {
    let sql = r#"
CREATE TABLE main.memberships (
  account_id INTEGER,
  team_id INTEGER,
  FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
  FOREIGN KEY (team_id) REFERENCES teams(id) ON DELETE RESTRICT
);
CREATE TABLE audit_events (
  team_id INTEGER REFERENCES teams(id) ON DELETE CASCADE
);
CREATE TABLE main.__new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
CREATE TABLE __new_teams(id INTEGER PRIMARY KEY);
INSERT INTO __new_teams SELECT id FROM teams;
DROP TABLE teams;
ALTER TABLE __new_teams RENAME TO teams;
"#;

    let findings = diagnose(&scan_sql(sql));
    let cascade_details: Vec<_> = findings
        .iter()
        .filter(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
        .map(|finding| finding.detail.as_str())
        .collect();

    assert_eq!(cascade_details.len(), 2);
    assert!(
        cascade_details
            .iter()
            .any(|detail| detail.contains("`main.memberships`"))
    );
    assert!(
        cascade_details
            .iter()
            .any(|detail| detail.contains("`audit_events`"))
    );
}

#[test]
fn does_not_emit_cascade_risk_without_a_matching_complete_rebuild() {
    let non_cascade = r#"
CREATE TABLE orders (
  account_id INTEGER REFERENCES accounts(id) ON DELETE RESTRICT,
  owner_id INTEGER REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
"#;
    assert!(
        diagnose(&scan_sql(non_cascade))
            .iter()
            .all(|finding| finding.hazard != HazardId::DeleteActionOnRebuild)
    );

    let incomplete = r#"
CREATE TABLE orders (
  account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
"#;
    assert!(
        diagnose(&scan_sql(incomplete))
            .iter()
            .all(|finding| finding.hazard != HazardId::DeleteActionOnRebuild)
    );
}

#[test]
fn safe_pre_transaction_disable_suppresses_cascade_risk() {
    let sql = r#"
CREATE TABLE orders (
  account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE
);
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
COMMIT;
PRAGMA foreign_keys=ON;
"#;

    let report = scan_sql(sql);
    assert_eq!(report.rebuilds[0].foreign_keys_enabled_at_drop, Some(false));
    assert!(diagnose(&report).is_empty());
}

#[test]
fn cascade_reference_created_after_rebuild_does_not_trigger() {
    let sql = r#"
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
CREATE TABLE orders (
  account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE
);
"#;

    assert!(diagnose(&scan_sql(sql)).is_empty());
}

#[test]
fn ignored_restore_inside_transaction_emits_pragma_finding() {
    let sql = r#"
CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
PRAGMA foreign_keys=ON;
COMMIT;
"#;

    let findings = diagnose(&scan_sql(sql));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].hazard,
        HazardId::ForeignKeysPragmaInsideTransaction
    );
    assert!(findings[0].detail.contains("ON"));
    assert!(findings[0].remediation.contains("foreign_keys=ON"));
    assert!(!findings[0].remediation.contains("OFF"));
}

#[test]
fn same_line_statements_preserve_reference_before_drop_order() {
    let sql = "CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE); CREATE TABLE __new_accounts(id); INSERT INTO __new_accounts SELECT id FROM accounts; DROP TABLE accounts; ALTER TABLE __new_accounts RENAME TO accounts;";
    assert!(
        diagnose(&scan_sql(sql))
            .iter()
            .any(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
    );
}

#[test]
fn nonzero_numeric_pragma_enables_foreign_keys() {
    let sql = r#"
CREATE TABLE orders (account_id REFERENCES accounts(id) ON DELETE CASCADE);
PRAGMA foreign_keys=OFF;
PRAGMA foreign_keys=2;
BEGIN;
CREATE TABLE __new_accounts(id);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
COMMIT;
"#;
    let report = scan_sql(sql);
    assert_eq!(report.rebuilds[0].foreign_keys_enabled_at_drop, Some(true));
    assert!(
        diagnose(&report)
            .iter()
            .any(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
    );
}

#[test]
fn in_transaction_pragma_reports_sqld002() {
    let findings = diagnose(&scan_sql("BEGIN;\nPRAGMA foreign_keys=OFF;\nCOMMIT;"));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].hazard,
        HazardId::ForeignKeysPragmaInsideTransaction
    );
    assert_eq!(findings[0].line, 2);
    assert!(
        findings[0]
            .remediation
            .starts_with("Set PRAGMA foreign_keys=OFF before BEGIN")
    );
    assert!(
        findings[0]
            .remediation
            .contains("PRAGMA foreign_key_check before COMMIT")
    );
}

#[test]
fn rename_away_remediation_keeps_the_original_name_until_the_drop() {
    let sql = r#"
PRAGMA foreign_keys = ON;
CREATE TABLE p (id INTEGER PRIMARY KEY);
CREATE TABLE c (pid INTEGER REFERENCES p(id) ON DELETE CASCADE);
BEGIN;
ALTER TABLE p RENAME TO old_p;
CREATE TABLE p (id INTEGER PRIMARY KEY);
INSERT INTO p SELECT id FROM old_p;
DROP TABLE old_p;
COMMIT;
"#;
    let findings = diagnose(&scan_sql(sql));
    let cascade = findings
        .iter()
        .find(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
        .expect("rename-away rebuild should be diagnosed");
    let remediation = &cascade.remediation;
    assert!(remediation.starts_with("Do not rename the original table away"));
    let drop = remediation
        .find("drop the original")
        .expect("the original is dropped");
    let rename = remediation
        .find("rename the replacement to the original name")
        .expect("the replacement takes the name after the drop");
    assert!(drop < rename);
}

#[test]
fn the_last_on_delete_clause_decides_the_action() {
    let rebuild = r#"
CREATE TABLE __new_p (id INTEGER PRIMARY KEY);
INSERT INTO __new_p SELECT id FROM p;
DROP TABLE p;
ALTER TABLE __new_p RENAME TO p;
"#;
    let delete_actions = |child: &str| {
        diagnose(&scan_sql(&format!(
            "PRAGMA foreign_keys = ON;\n{child}\n{rebuild}"
        )))
        .into_iter()
        .filter(|finding| finding.hazard == HazardId::DeleteActionOnRebuild)
        .count()
    };

    assert_eq!(
        delete_actions(
            "CREATE TABLE c (id INTEGER);\nALTER TABLE c ADD COLUMN pid REFERENCES p(id) ON DELETE CASCADE ON DELETE NO ACTION;"
        ),
        0
    );
    assert_eq!(
        delete_actions(
            "CREATE TABLE c (pid REFERENCES p(id) ON DELETE CASCADE ON DELETE RESTRICT);"
        ),
        0
    );
    assert_eq!(
        delete_actions(
            "CREATE TABLE c (pid REFERENCES p(id) ON DELETE RESTRICT ON DELETE SET NULL);"
        ),
        1
    );
}

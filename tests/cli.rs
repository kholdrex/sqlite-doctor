use std::fs;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMPORARY_FILE: AtomicUsize = AtomicUsize::new(0);

const HAZARDOUS_SQL: &str = r#"
CREATE TABLE orders (
  account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE
);
BEGIN;
PRAGMA foreign_keys=OFF;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);
INSERT INTO __new_accounts SELECT id FROM accounts;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
COMMIT;
"#;

#[test]
fn human_output_is_actionable_and_findings_fail_the_check() {
    let output = run_cli(&[], HAZARDOUS_SQL);
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("SQLD001"));
    assert!(stdout.contains("ON DELETE CASCADE"));
    assert!(stdout.contains("SQLD002"));
    assert!(stdout.contains("help:"));
}

#[test]
fn json_output_has_a_stable_machine_readable_shape() {
    let output = run_cli(&["--format", "json"], HAZARDOUS_SQL);
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"version\": 1"));
    assert!(stdout.contains("\"code\": \"SQLD001\""));
    assert!(stdout.contains("\"code\": \"SQLD002\""));
    assert!(stdout.contains("\"line\": 9"));
    assert!(stdout.contains("\"related_line\": 3"));
    assert!(stdout.contains("\"findings\""));
}

#[test]
fn schema_object_omission_is_reported_in_human_and_json_output() {
    let sql = r#"
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
"#;

    let human = run_cli(&[], sql);
    assert_eq!(human.status.code(), Some(1));
    let stdout = String::from_utf8(human.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("SQLD003"));
    assert!(stdout.contains("idx_users_email"));

    let json = run_cli(&["--format", "json"], sql);
    assert_eq!(json.status.code(), Some(1));
    let stdout = String::from_utf8(json.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"code\": \"SQLD003\""));
    assert!(stdout.contains("\"related_line\": 2"));
}

#[test]
fn safe_input_succeeds_and_reports_no_findings() {
    let output = run_cli(&[], "CREATE TABLE widgets(id INTEGER PRIMARY KEY);\n");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "No findings.\n"
    );
}

#[test]
fn invalid_arguments_are_usage_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_sqlite-doctor"))
        .arg("--format")
        .arg("yaml")
        .output()
        .expect("CLI should run");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("human or json"));
}

#[test]
fn missing_input_is_a_usage_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_sqlite-doctor"))
        .output()
        .expect("CLI should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("provide a SQL file"));
}

fn run_cli(extra_arguments: &[&str], sql: &str) -> Output {
    let path = temporary_sql_path();
    fs::write(&path, sql).expect("temporary SQL fixture should be written");
    let output = Command::new(env!("CARGO_BIN_EXE_sqlite-doctor"))
        .args(extra_arguments)
        .arg(&path)
        .output()
        .expect("CLI should run");
    fs::remove_file(path).expect("temporary SQL fixture should be removed");
    output
}

fn temporary_sql_path() -> std::path::PathBuf {
    let nonce = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("sqlite-doctor-{}-{nonce}.sql", std::process::id()))
}

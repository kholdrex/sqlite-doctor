use std::fs;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

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
fn human_output_lists_codes_and_help_and_exits_1() {
    let output = run_cli(&[], HAZARDOUS_SQL);
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("SQLD001"));
    assert!(stdout.contains("ON DELETE CASCADE"));
    assert!(stdout.contains("SQLD002"));
    assert!(stdout.contains("help:"));
}

#[test]
fn json_output_contains_version_codes_and_lines() {
    let output = run_cli(&["--format", "json"], HAZARDOUS_SQL);
    assert_eq!(output.status.code(), Some(1));

    let report = json(&output);
    assert_eq!(report["version"], 1);
    let findings = report["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 2);

    assert_eq!(findings[0]["code"], "SQLD002");
    assert_eq!(findings[0]["line"], 6);
    assert_eq!(findings[0].get("related_line"), Some(&Value::Null));

    assert_eq!(findings[1]["code"], "SQLD001");
    assert_eq!(findings[1]["line"], 9);
    assert_eq!(findings[1]["related_line"], 3);

    assert_eq!(report["complete"], true);
    assert_eq!(report["incomplete"], serde_json::json!([]));
    assert_eq!(
        findings[1]["location"],
        serde_json::json!({ "source": report["source"], "line": 9, "synthetic": false })
    );
    assert_eq!(findings[1]["related_location"]["line"], 3);
    assert_eq!(findings[0]["related_location"], Value::Null);
    for finding in findings {
        for field in ["title", "detail", "remediation"] {
            assert!(finding[field].is_string(), "{field}");
        }
    }
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

    let output = run_cli(&["--format", "json"], sql);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    let findings = report["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["code"], "SQLD003");
    assert_eq!(findings[0]["line"], 5);
    assert_eq!(findings[0]["related_line"], 2);
}

const RUNNER_STREAM: &str = concat!(
    "-- sqlite-doctor:file 1 baseline.sql\n",
    "CREATE TABLE accounts(id INTEGER PRIMARY KEY);\n",
    "CREATE TABLE orders (account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE);\n",
    "-- sqlite-doctor:synthetic drizzle migrator\n",
    "PRAGMA foreign_keys=ON;\n",
    "BEGIN;\n",
    "-- sqlite-doctor:file 1 drizzle/0001_rebuild.sql\n",
    "PRAGMA foreign_keys=OFF;\n",
    "CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY);\n",
    "INSERT INTO __new_accounts SELECT id FROM accounts;\n",
    "DROP TABLE accounts;\n",
    "ALTER TABLE __new_accounts RENAME TO accounts;\n",
    "PRAGMA foreign_keys=ON;\n",
    "-- sqlite-doctor:synthetic drizzle migrator\n",
    "COMMIT;\n",
);

#[test]
fn findings_in_a_runner_stream_point_at_the_migration_files() {
    let output = run_cli(&["--format", "json"], RUNNER_STREAM);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["complete"], true);

    let located: Vec<_> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|finding| {
            (
                finding["code"].as_str().expect("code").to_string(),
                finding["line"].as_u64().expect("line"),
                finding["location"]["source"]
                    .as_str()
                    .expect("source")
                    .to_string(),
                finding["location"]["line"].as_u64(),
            )
        })
        .collect();
    let rebuild = "drizzle/0001_rebuild.sql".to_string();
    assert_eq!(
        located,
        [
            ("SQLD002".to_string(), 8, rebuild.clone(), Some(1)),
            ("SQLD001".to_string(), 11, rebuild.clone(), Some(4)),
            ("SQLD002".to_string(), 13, rebuild, Some(6)),
        ]
    );
    let cascade = &report["findings"][1];
    assert_eq!(
        cascade["related_location"],
        serde_json::json!({ "source": "baseline.sql", "line": 2, "synthetic": false })
    );

    let human = run_cli(&[], RUNNER_STREAM);
    let stdout = String::from_utf8(human.stdout).expect("stdout should be UTF-8");
    assert!(
        stdout.contains("--> drizzle/0001_rebuild.sql:4 ("),
        "{stdout}"
    );
}

#[test]
fn a_statement_the_runner_added_is_marked_synthetic() {
    let sql = concat!(
        "BEGIN;\n",
        "-- sqlite-doctor:synthetic django schema editor\n",
        "PRAGMA foreign_keys=OFF;\n",
        "-- sqlite-doctor:migration blog.0002_rename\n",
        "COMMIT;\n",
    );
    let report = json(&run_cli(&["--format", "json"], sql));
    assert_eq!(
        report["findings"][0]["location"],
        serde_json::json!({ "source": "django schema editor", "line": null, "synthetic": true })
    );

    let human = run_cli(&[], sql);
    let stdout = String::from_utf8(human.stdout).expect("stdout should be UTF-8");
    assert!(
        stdout.contains("--> django schema editor, added by the migration runner ("),
        "{stdout}"
    );
}

#[test]
fn an_incomplete_check_exits_2_and_still_lists_its_findings() {
    let sql =
        format!("-- sqlite-doctor:incomplete migration 0003 raised an error\n{RUNNER_STREAM}");
    let output = run_cli(&["--format", "json"], &sql);
    assert_eq!(output.status.code(), Some(2));
    let report = json(&output);
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["incomplete"],
        serde_json::json!([{ "line": 1, "reason": "migration 0003 raised an error" }])
    );
    assert_eq!(
        report["findings"].as_array().expect("findings array").len(),
        3
    );

    let clean = run_cli(
        &[],
        "-- sqlite-doctor:incomplete second connection\nSELECT 1;\n",
    );
    assert_eq!(clean.status.code(), Some(2));
    let stdout = String::from_utf8(clean.stdout).expect("stdout should be UTF-8");
    assert!(
        stdout.starts_with("No findings.\n\nCheck incomplete:\n"),
        "{stdout}"
    );
    assert!(stdout.contains(":1: second connection"), "{stdout}");
}

#[test]
fn an_unknown_directive_makes_the_check_incomplete() {
    let output = run_cli(&[], "-- sqlite-doctor:fiel 1 a.sql\nSELECT 1;\n");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("unknown directive `fiel`"), "{stdout}");
}

#[test]
fn a_parameter_name_with_comment_or_quote_characters_hides_nothing() {
    for parameter in ["$p(/*)", "$p(')", "$p([)", "$a::b(\")", "@p", ":p"] {
        let sql = format!(
            "SELECT {parameter};\n-- sqlite-doctor:incomplete actual\nBEGIN;\nPRAGMA foreign_keys=OFF;\nCOMMIT;\n"
        );
        let output = run_cli(&["--format", "json"], &sql);
        assert_eq!(output.status.code(), Some(2), "{parameter}");
        let report = json(&output);
        assert_eq!(report["incomplete"][0]["reason"], "actual", "{parameter}");
        assert_eq!(report["findings"][0]["code"], "SQLD002", "{parameter}");
    }
}

#[test]
fn a_comment_after_a_non_ascii_name_is_not_a_directive() {
    for name in ["\u{a0}", "\u{2003}"] {
        let sql = format!("SELECT\n{name}-- sqlite-doctor:incomplete column comment\nFROM t;\n");
        let output = run_cli(&["--format", "json"], &sql);
        assert_eq!(output.status.code(), Some(0), "{name:?}");
        assert_eq!(json(&output)["complete"], true);
    }
}

#[test]
fn input_that_ends_inside_a_comment_or_string_is_incomplete() {
    for (sql, line) in [
        (
            "BEGIN; /* trailing note\n-- sqlite-doctor:incomplete lost\nPRAGMA foreign_keys=OFF;\n",
            1,
        ),
        ("SELECT 1;\nSELECT 'open\n", 2),
        ("SELECT [open\n", 1),
    ] {
        let output = run_cli(&["--format", "json"], sql);
        assert_eq!(output.status.code(), Some(2), "{sql:?}");
        let report = json(&output);
        assert_eq!(report["complete"], false);
        assert_eq!(report["incomplete"][0]["line"], line, "{sql:?}");
    }
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

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be JSON")
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

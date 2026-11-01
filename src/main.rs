use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;

use serde_json::json;
use sqlite_doctor::{Finding, diagnose, initial_hazards, scan_sql};

#[derive(Clone, Copy)]
enum OutputFormat {
    Human,
    Json,
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("sqlite-doctor: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<String>) -> Result<u8, String> {
    if arguments.is_empty() {
        return Err("provide a SQL file or `-` for stdin".to_string());
    }

    let mut format = OutputFormat::Human;
    let mut source = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--help" | "-h" => {
                print_help();
                return Ok(0);
            }
            "--format" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| "--format requires human or json".to_string())?;
                format = parse_format(value)?;
                index += 2;
            }
            argument if argument.starts_with('-') && argument != "-" => {
                return Err(format!("unknown option `{argument}`"));
            }
            argument => {
                if source.replace(argument.to_string()).is_some() {
                    return Err("only one SQL input may be scanned at a time".to_string());
                }
                index += 1;
            }
        }
    }

    let source = source.ok_or_else(|| "provide a SQL file or `-` for stdin".to_string())?;
    let sql = read_source(&source)?;
    let findings = diagnose(&scan_sql(&sql));

    match format {
        OutputFormat::Human => print_human(&source, &findings),
        OutputFormat::Json => print_json(&source, &findings)?,
    }

    Ok(u8::from(!findings.is_empty()))
}

fn parse_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "human" => Ok(OutputFormat::Human),
        "json" => Ok(OutputFormat::Json),
        _ => Err("--format must be human or json".to_string()),
    }
}

fn read_source(source: &str) -> Result<String, String> {
    if source == "-" {
        let mut sql = String::new();
        io::stdin()
            .read_to_string(&mut sql)
            .map_err(|error| format!("could not read stdin: {error}"))?;
        Ok(sql)
    } else {
        fs::read_to_string(source).map_err(|error| format!("could not read `{source}`: {error}"))
    }
}

fn print_human(source: &str, findings: &[Finding]) {
    if findings.is_empty() {
        println!("No findings.");
        return;
    }

    for (index, finding) in findings.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("{}", finding.hazard);
        println!("  --> {source}:{}", finding.line);
        println!("  {}", finding.detail);
        println!("  help: {}", finding.remediation);
    }
}

fn print_json(source: &str, findings: &[Finding]) -> Result<(), String> {
    let findings: Vec<_> = findings
        .iter()
        .map(|finding| {
            json!({
                "code": finding.hazard.code(),
                "title": finding.hazard.title(),
                "line": finding.line,
                "related_line": finding.related_line,
                "detail": finding.detail,
                "remediation": finding.remediation,
            })
        })
        .collect();
    let output = json!({
        "version": 1,
        "source": source,
        "findings": findings,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output)
            .map_err(|error| format!("could not encode JSON output: {error}"))?
    );
    Ok(())
}

fn print_help() {
    println!("sqlite-doctor [--format human|json] <SQL_FILE|->");
    println!("SQLite migration safety linter for ORM-generated rebuild hazards");
    println!();
    println!("Diagnostics:");
    for hazard in initial_hazards() {
        println!("- {hazard}");
    }
}

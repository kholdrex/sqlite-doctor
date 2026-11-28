use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;

use serde_json::json;
use sqlite_doctor::directives::{Directives, Location};
use sqlite_doctor::{Finding, HazardId, check_sql};

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
    let findings = check_sql(&sql);
    let directives = Directives::parse(&sql);

    match format {
        OutputFormat::Human => print_human(&source, &findings, &directives),
        OutputFormat::Json => print_json(&source, &findings, &directives),
    }

    if !directives.incomplete.is_empty() {
        return Ok(2);
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

fn print_human(source: &str, findings: &[Finding], directives: &Directives) {
    if findings.is_empty() {
        println!("No findings.");
    }

    for (index, finding) in findings.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("{}", finding.hazard);
        if directives.is_mapped() {
            let location = directives.locate(source, finding.line);
            let origin = match (location.line, location.synthetic) {
                (_, true) => format!("{}, added by the migration runner", location.source),
                (Some(line), false) => format!("{}:{line}", location.source),
                (None, false) => location.source,
            };
            println!("  --> {origin} ({source}:{})", finding.line);
        } else {
            println!("  --> {source}:{}", finding.line);
        }
        println!("  {}", finding.detail);
        println!("  help: {}", finding.remediation);
    }

    if !directives.incomplete.is_empty() {
        println!();
        println!("Check incomplete:");
        for (line, reason) in &directives.incomplete {
            println!("- {source}:{line}: {reason}");
        }
    }
}

fn print_json(source: &str, findings: &[Finding], directives: &Directives) {
    let location = |line: usize| {
        let Location {
            source,
            line,
            synthetic,
        } = directives.locate(source, line);
        json!({ "source": source, "line": line, "synthetic": synthetic })
    };
    let findings: Vec<_> = findings
        .iter()
        .map(|finding| {
            json!({
                "code": finding.hazard.code(),
                "title": finding.hazard.title(),
                "line": finding.line,
                "related_line": finding.related_line,
                "location": location(finding.line),
                "related_location": finding.related_line.map(location),
                "detail": finding.detail,
                "remediation": finding.remediation,
            })
        })
        .collect();
    let incomplete: Vec<_> = directives
        .incomplete
        .iter()
        .map(|(line, reason)| json!({ "line": line, "reason": reason }))
        .collect();
    let output = json!({
        "version": 1,
        "source": source,
        "complete": incomplete.is_empty(),
        "incomplete": incomplete,
        "findings": findings,
    });
    println!("{output:#}");
}

fn print_help() {
    println!("sqlite-doctor [--format human|json] <SQL_FILE|->");
    println!(
        "Checks SQLite migration SQL for table rebuilds that can delete or change dependent rows,"
    );
    println!("or drop indexes and triggers.");
    println!();
    println!("Exit codes: 0 no findings, 1 findings, 2 usage error, unreadable input or an");
    println!("incomplete check (see the README for its causes).");
    println!();
    println!("Diagnostics:");
    for hazard in HazardId::ALL {
        println!("- {hazard}");
    }
}

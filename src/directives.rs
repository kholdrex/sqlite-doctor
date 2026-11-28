//! Comment directives that a migration adapter writes into the SQL it hands
//! to the CLI. They relabel where lines came from or mark the check as
//! incomplete; they never hide a finding.
//!
//! Used by the `sqlite-doctor` binary. Not part of the stable API: it may
//! change in any release.

use crate::scanner::{DIRECTIVE_PREFIX, lex};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    File { name: String, first_line: usize },
    Migration(String),
    Synthetic(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub source: String,
    pub line: Option<usize>,
    pub synthetic: bool,
}

#[derive(Debug, Default)]
pub struct Directives {
    /// Origins in input order, each with the line of its directive.
    origins: Vec<(usize, Origin)>,
    /// Every directive line, in order. They are not source lines.
    lines: Vec<usize>,
    /// Lines and reasons that make the check incomplete.
    pub incomplete: Vec<(usize, String)>,
}

impl Directives {
    pub fn parse(sql: &str) -> Self {
        let total_lines = sql.split('\n').count();
        let lexed = lex(sql);
        let mut directives = Self::default();
        for (line, comment) in &lexed.standalone_comments {
            let Some(rest) = comment.strip_prefix(DIRECTIVE_PREFIX) else {
                continue;
            };
            let line = *line;
            directives.lines.push(line);
            let (keyword, argument) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let argument = argument.trim();
            let origin = match keyword {
                "file" => file_origin(argument, total_lines),
                "migration" if !argument.is_empty() => {
                    Some(Origin::Migration(argument.to_string()))
                }
                "synthetic" if !argument.is_empty() => {
                    Some(Origin::Synthetic(argument.to_string()))
                }
                "incomplete" => {
                    let reason = if argument.is_empty() {
                        "no reason given"
                    } else {
                        argument
                    };
                    directives.incomplete.push((line, reason.to_string()));
                    continue;
                }
                "migration" | "synthetic" => None,
                _ => {
                    directives
                        .incomplete
                        .push((line, format!("unknown directive `{keyword}`")));
                    continue;
                }
            };
            match origin {
                Some(origin) => directives.origins.push((line, origin)),
                None => directives
                    .incomplete
                    .push((line, format!("malformed `{keyword}` directive"))),
            }
        }
        if let Some(line) = lexed.unterminated {
            directives.incomplete.push((
                line,
                "the input ends inside a string, quoted name or block comment that starts here"
                    .to_string(),
            ));
        }
        directives
    }

    /// Where line `line` of `input` came from.
    pub fn locate(&self, input: &str, line: usize) -> Location {
        let index = self
            .origins
            .partition_point(|(origin_line, _)| *origin_line < line);
        let Some((origin_line, origin)) = index.checked_sub(1).map(|index| &self.origins[index])
        else {
            return Location {
                source: input.to_string(),
                line: Some(line),
                synthetic: false,
            };
        };
        match origin {
            Origin::File { name, first_line } => {
                let directive_lines = self.lines.partition_point(|directive| *directive < line)
                    - self
                        .lines
                        .partition_point(|directive| directive <= origin_line);
                Location {
                    source: name.clone(),
                    line: Some(first_line + (line - origin_line - 1 - directive_lines)),
                    synthetic: false,
                }
            }
            Origin::Migration(name) => Location {
                source: name.clone(),
                line: None,
                synthetic: false,
            },
            Origin::Synthetic(description) => Location {
                source: description.clone(),
                line: None,
                synthetic: true,
            },
        }
    }

    pub fn is_mapped(&self) -> bool {
        !self.origins.is_empty()
    }
}

fn file_origin(argument: &str, total_lines: usize) -> Option<Origin> {
    let (first_line, name) = argument.split_once(char::is_whitespace)?;
    let first_line = first_line
        .parse::<usize>()
        .ok()
        .filter(|line| *line > 0 && line.checked_add(total_lines).is_some())?;
    let name = name.trim();
    (!name.is_empty()).then(|| Origin::File {
        name: name.to_string(),
        first_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_before_any_directive_belong_to_the_input() {
        let directives = Directives::parse("SELECT 1;\n-- sqlite-doctor:migration m\nSELECT 2;\n");
        assert_eq!(
            directives.locate("in.sql", 1),
            Location {
                source: "in.sql".to_string(),
                line: Some(1),
                synthetic: false
            }
        );
        assert_eq!(directives.locate("in.sql", 3).source, "m");
        assert_eq!(directives.locate("in.sql", 3).line, None);
    }

    #[test]
    fn a_file_directive_maps_the_following_lines() {
        let sql = "-- sqlite-doctor:file 10 drizzle/0003 add notes.sql\nA;\nB;\n";
        let directives = Directives::parse(sql);
        let location = directives.locate("-", 3);
        assert_eq!(location.source, "drizzle/0003 add notes.sql");
        assert_eq!(location.line, Some(11));
        assert!(directives.incomplete.is_empty());
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_first_directive() {
        let directives = Directives::parse("\u{feff}-- sqlite-doctor:incomplete copy failed\n");
        assert_eq!(directives.incomplete, [(1, "copy failed".to_string())]);
    }

    #[test]
    fn a_byte_order_mark_inside_the_stream_does_not_hide_a_directive() {
        let directives = Directives::parse(
            "SELECT 0;\n\u{feff}-- sqlite-doctor:incomplete inner stream partial\r\n",
        );
        assert_eq!(
            directives.incomplete,
            [(2, "inner stream partial".to_string())]
        );
    }

    #[test]
    fn directive_text_in_strings_identifiers_and_block_comments_is_data() {
        let sql = concat!(
            "SELECT 'first\n-- sqlite-doctor:incomplete in a string\nit''s';\n",
            "SELECT \"a\n-- sqlite-doctor:incomplete in an identifier\n\";\n",
            "SELECT [a\n-- sqlite-doctor:incomplete in brackets\n];\n",
            "SELECT `a\n-- sqlite-doctor:incomplete in backticks\n`;\n",
            "/*\n-- sqlite-doctor:synthetic in a comment\n*/\n",
            "SELECT 1; -- sqlite-doctor:incomplete after code\n",
            "-- sqlite-doctor:incomplete real\n",
        );
        let directives = Directives::parse(sql);
        assert_eq!(directives.incomplete, [(17, "real".to_string())]);
        assert!(!directives.is_mapped());
    }

    #[test]
    fn directive_lines_are_not_counted_as_source_lines() {
        let sql = "-- sqlite-doctor:file 40 a.sql\nBEGIN;\n-- sqlite-doctor:incomplete gap\nPRAGMA foreign_keys=OFF;\n";
        let directives = Directives::parse(sql);
        assert_eq!(directives.locate("-", 2).line, Some(40));
        assert_eq!(directives.locate("-", 4).line, Some(41));
    }

    #[test]
    fn a_first_line_that_would_overflow_is_malformed() {
        let sql = format!("-- sqlite-doctor:file {} real.sql\nBEGIN;\n", usize::MAX);
        let directives = Directives::parse(&sql);
        assert_eq!(
            directives.incomplete,
            [(1, "malformed `file` directive".to_string())]
        );
        assert_eq!(directives.locate("-", 2).source, "-");
    }

    #[test]
    fn unknown_and_malformed_directives_make_the_check_incomplete() {
        let sql = "-- sqlite-doctor:fiel 1 a.sql\n-- sqlite-doctor:file x a.sql\n-- sqlite-doctor:file 0 a.sql\n-- sqlite-doctor:file 3\n-- sqlite-doctor:migration\n-- sqlite-doctor: file 1 a.sql\n";
        let reasons: Vec<_> = Directives::parse(sql)
            .incomplete
            .into_iter()
            .map(|(_, reason)| reason)
            .collect();
        assert_eq!(
            reasons,
            [
                "unknown directive `fiel`",
                "malformed `file` directive",
                "malformed `file` directive",
                "malformed `file` directive",
                "malformed `migration` directive",
                "unknown directive ``",
            ]
        );
    }

    #[test]
    fn similar_comments_are_not_directives() {
        let directives = Directives::parse(
            "-- sqlite-doctor capture: ActiveRecord 8.1\n--sqlite-doctor:file 1 a\n",
        );
        assert!(directives.incomplete.is_empty());
        assert!(!directives.is_mapped());
    }
}

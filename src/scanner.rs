#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PragmaInTransaction {
    /// One-based source line containing the PRAGMA statement.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildSequence {
    pub temporary_table: String,
    pub original_table: String,
    /// One-based source lines for each statement in the rebuild sequence.
    pub create_line: usize,
    pub copy_line: usize,
    pub drop_line: usize,
    pub rename_line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub foreign_keys_off_in_transactions: Vec<PragmaInTransaction>,
    pub rebuilds: Vec<RebuildSequence>,
}

/// Scans raw SQLite SQL for the two structural signals needed by the first
/// milestone. This intentionally recognizes a small statement vocabulary
/// rather than attempting to parse arbitrary SQL.
pub fn scan_sql(sql: &str) -> ScanReport {
    let statements = statements(sql);
    let mut report = ScanReport::default();
    let mut transaction_depth = 0usize;

    for statement in &statements {
        if is_transaction_end(statement) {
            transaction_depth = transaction_depth.saturating_sub(1);
        } else if is_transaction_start(statement) {
            transaction_depth += 1;
        } else if transaction_depth > 0 && is_foreign_keys_off(statement) {
            report
                .foreign_keys_off_in_transactions
                .push(PragmaInTransaction {
                    line: statement.line,
                });
        }
    }

    for (create_index, statement) in statements.iter().enumerate() {
        let Some(temporary_table) = created_rebuild_table(statement) else {
            continue;
        };

        let Some((copy_index, original_table)) = statements
            .iter()
            .enumerate()
            .skip(create_index + 1)
            .find_map(|(index, candidate)| {
                copied_table(candidate, &temporary_table)
                    .map(|original| (index, original.to_owned()))
            })
        else {
            continue;
        };

        let Some((drop_index, drop_statement)) = statements
            .iter()
            .enumerate()
            .skip(copy_index + 1)
            .find(|(_, candidate)| {
                dropped_table(candidate)
                    .is_some_and(|name| name.eq_ignore_ascii_case(&original_table))
            })
        else {
            continue;
        };

        let Some(rename_statement) = statements.iter().skip(drop_index + 1).find(|candidate| {
            renamed_table(candidate).is_some_and(|(from, to)| {
                from.eq_ignore_ascii_case(&temporary_table)
                    && to.eq_ignore_ascii_case(&original_table)
            })
        }) else {
            continue;
        };

        report.rebuilds.push(RebuildSequence {
            temporary_table,
            original_table,
            create_line: statement.line,
            copy_line: statements[copy_index].line,
            drop_line: drop_statement.line,
            rename_line: rename_statement.line,
        });
    }

    report
}

#[derive(Debug)]
struct Statement {
    tokens: Vec<String>,
    line: usize,
}

fn statements(sql: &str) -> Vec<Statement> {
    let mut result = Vec::new();
    let mut tokens = Vec::new();
    let mut statement_line = None;
    let mut line = 1usize;
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\n' => line += 1,
            ch if ch.is_whitespace() => {}
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                for comment_ch in chars.by_ref() {
                    if comment_ch == '\n' {
                        line += 1;
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for comment_ch in chars.by_ref() {
                    if comment_ch == '\n' {
                        line += 1;
                    }
                    if previous == '*' && comment_ch == '/' {
                        break;
                    }
                    previous = comment_ch;
                }
            }
            ';' => finish_statement(&mut result, &mut tokens, &mut statement_line),
            '\'' => {
                statement_line.get_or_insert(line);
                skip_quoted(&mut chars, '\'', &mut line);
                tokens.push("<string>".to_owned());
            }
            '"' | '`' => {
                statement_line.get_or_insert(line);
                tokens.push(read_quoted_identifier(&mut chars, ch, &mut line));
            }
            '[' => {
                statement_line.get_or_insert(line);
                tokens.push(read_quoted_identifier(&mut chars, ']', &mut line));
            }
            '=' | '(' | ')' | ',' => {
                statement_line.get_or_insert(line);
                tokens.push(ch.to_string());
            }
            _ => {
                statement_line.get_or_insert(line);
                let mut token = String::from(ch);
                while chars
                    .peek()
                    .is_some_and(|next| is_identifier_character(*next))
                {
                    token.push(chars.next().expect("peeked character is present"));
                }
                tokens.push(token);
            }
        }
    }

    finish_statement(&mut result, &mut tokens, &mut statement_line);
    result
}

fn finish_statement(
    statements: &mut Vec<Statement>,
    tokens: &mut Vec<String>,
    line: &mut Option<usize>,
) {
    if !tokens.is_empty() {
        statements.push(Statement {
            tokens: std::mem::take(tokens),
            line: line.take().expect("non-empty statement has a line"),
        });
    } else {
        *line = None;
    }
}

fn skip_quoted<I>(chars: &mut std::iter::Peekable<I>, delimiter: char, line: &mut usize)
where
    I: Iterator<Item = char>,
{
    while let Some(ch) = chars.next() {
        if ch == '\n' {
            *line += 1;
        }
        if ch == delimiter {
            if chars.peek() == Some(&delimiter) {
                chars.next();
            } else {
                break;
            }
        }
    }
}

fn read_quoted_identifier<I>(
    chars: &mut std::iter::Peekable<I>,
    delimiter: char,
    line: &mut usize,
) -> String
where
    I: Iterator<Item = char>,
{
    let mut identifier = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\n' {
            *line += 1;
        }
        if ch == delimiter {
            if chars.peek() == Some(&delimiter) {
                chars.next();
                identifier.push(delimiter);
            } else {
                break;
            }
        } else {
            identifier.push(ch);
        }
    }
    identifier
}

fn is_identifier_character(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '$' | '.')
}

fn token_is(statement: &Statement, index: usize, expected: &str) -> bool {
    statement
        .tokens
        .get(index)
        .is_some_and(|token| token.eq_ignore_ascii_case(expected))
}

fn is_transaction_start(statement: &Statement) -> bool {
    token_is(statement, 0, "BEGIN") || token_is(statement, 0, "SAVEPOINT")
}

fn is_transaction_end(statement: &Statement) -> bool {
    token_is(statement, 0, "COMMIT")
        || token_is(statement, 0, "END")
        || token_is(statement, 0, "ROLLBACK")
        || token_is(statement, 0, "RELEASE")
}

fn is_foreign_keys_off(statement: &Statement) -> bool {
    token_is(statement, 0, "PRAGMA")
        && token_is(statement, 1, "foreign_keys")
        && token_is(statement, 2, "=")
        && token_is(statement, 3, "OFF")
}

fn created_rebuild_table(statement: &Statement) -> Option<String> {
    if !token_is(statement, 0, "CREATE") {
        return None;
    }
    let mut index = 1;
    if token_is(statement, index, "TEMP") || token_is(statement, index, "TEMPORARY") {
        index += 1;
    }
    if !token_is(statement, index, "TABLE") {
        return None;
    }
    index += 1;
    if token_is(statement, index, "IF")
        && token_is(statement, index + 1, "NOT")
        && token_is(statement, index + 2, "EXISTS")
    {
        index += 3;
    }
    let table = statement.tokens.get(index)?;
    table
        .to_ascii_lowercase()
        .starts_with("__new_")
        .then(|| table.to_owned())
}

fn copied_table<'a>(statement: &'a Statement, temporary_table: &str) -> Option<&'a str> {
    if !token_is(statement, 0, "INSERT") {
        return None;
    }
    let into_index = statement
        .tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case("INTO"))?;
    let target = statement.tokens.get(into_index + 1)?;
    if !target.eq_ignore_ascii_case(temporary_table) {
        return None;
    }
    let select_index = statement
        .tokens
        .iter()
        .enumerate()
        .skip(into_index + 2)
        .find(|(_, token)| token.eq_ignore_ascii_case("SELECT"))?
        .0;
    let from_index = statement
        .tokens
        .iter()
        .enumerate()
        .skip(select_index + 1)
        .find(|(_, token)| token.eq_ignore_ascii_case("FROM"))?
        .0;
    statement.tokens.get(from_index + 1).map(String::as_str)
}

fn dropped_table(statement: &Statement) -> Option<&str> {
    if !token_is(statement, 0, "DROP") || !token_is(statement, 1, "TABLE") {
        return None;
    }
    let index = if token_is(statement, 2, "IF") && token_is(statement, 3, "EXISTS") {
        4
    } else {
        2
    };
    statement.tokens.get(index).map(String::as_str)
}

fn renamed_table(statement: &Statement) -> Option<(&str, &str)> {
    if !token_is(statement, 0, "ALTER")
        || !token_is(statement, 1, "TABLE")
        || !token_is(statement, 3, "RENAME")
        || !token_is(statement, 4, "TO")
    {
        return None;
    }
    Some((
        statement.tokens.get(2)?.as_str(),
        statement.tokens.get(5)?.as_str(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_case_and_comments_without_parsing_literal_contents() {
        let sql = r#"
            begin immediate;
            /* ignored: PRAGMA foreign_keys=ON; */
            pragma FOREIGN_KEYS = off;
            select 'COMMIT; PRAGMA foreign_keys=OFF;';
            commit;
        "#;

        let report = scan_sql(sql);
        assert_eq!(report.foreign_keys_off_in_transactions.len(), 1);
        assert_eq!(report.foreign_keys_off_in_transactions[0].line, 4);
    }

    #[test]
    fn requires_a_complete_ordered_rebuild_sequence() {
        let sql = r#"
            CREATE TABLE __new_posts (id integer);
            INSERT INTO __new_posts SELECT id FROM posts;
            ALTER TABLE __new_posts RENAME TO posts;
            DROP TABLE posts;
        "#;

        assert!(scan_sql(sql).rebuilds.is_empty());
    }
}

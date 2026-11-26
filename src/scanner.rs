//! Scans one SQLite SQL input for table rebuilds and foreign-key pragmas.
//!
//! This is a lexical scanner, not a SQL parser. It cannot see a transaction a
//! framework opens around the input, SQL built at run time, or state left by
//! other inputs.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeysPragmaInTransaction {
    /// One-based source line containing the PRAGMA statement.
    pub line: usize,
    /// The ignored state requested by the statement.
    pub requested_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildKind {
    /// Create a replacement, copy rows into it, drop the original, then rename
    /// the replacement (Drizzle, SQLite's documented procedure).
    Rename,
    /// Copy rows out to a holding table, drop the original, recreate it under
    /// its own name, then copy the rows back (Rails `alter_table`).
    Recreate,
    /// Rename the original away, create the replacement under the original
    /// name, copy the rows into it, then drop the renamed original. SQLite
    /// points the references of other tables at the new name when it renames
    /// the original, so that drop fires their `ON DELETE` actions.
    RenameAway,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildSequence {
    pub kind: RebuildKind,
    /// The replacement or holding table. In a rename-away rebuild, the name
    /// the original has when it is dropped.
    pub temporary_table: String,
    pub original_table: String,
    /// One-based source lines for each statement in the rebuild sequence.
    pub create_line: usize,
    pub copy_line: usize,
    pub drop_line: usize,
    /// The `ALTER TABLE ... RENAME TO` or the `CREATE TABLE` that gives the
    /// replacement the original name.
    pub replacement_line: usize,
    /// Effective lexical `PRAGMA foreign_keys` state at the drop. `None`
    /// means the input did not establish a state the scanner can observe.
    pub foreign_keys_enabled_at_drop: Option<bool>,
    pub(crate) drop_statement_index: usize,
    /// The statement from which an index or trigger can be created on the
    /// replacement under the original name. In a rename-away rebuild that is
    /// the drop: until then the renamed original still holds the name.
    pub(crate) replacement_statement_index: usize,
    pub(crate) temporary_table_parts: Vec<String>,
    pub(crate) original_table_parts: Vec<String>,
    /// Schema of the dropped table, when the drop names or resolves to one
    /// with certainty.
    pub(crate) original_schema: Option<String>,
    /// Columns of the table that takes the original name, when its CREATE
    /// TABLE lists them.
    pub(crate) replacement_columns: Option<Vec<String>>,
}

impl RebuildSequence {
    /// The name of the original table when the DROP removes it.
    pub fn dropped_table(&self) -> &str {
        match self.kind {
            RebuildKind::RenameAway => &self.temporary_table,
            RebuildKind::Rename | RebuildKind::Recreate => &self.original_table,
        }
    }

    pub(crate) fn dropped_table_parts(&self) -> &[String] {
        match self.kind {
            RebuildKind::RenameAway => &self.temporary_table_parts,
            RebuildKind::Rename | RebuildKind::Recreate => &self.original_table_parts,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaObjectKind {
    Index,
    Trigger,
}

impl SchemaObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Trigger => "trigger",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaObject {
    pub kind: SchemaObjectKind,
    pub name: String,
    pub table: String,
    /// One-based source line containing the CREATE statement.
    pub line: usize,
    pub(crate) statement_index: usize,
    /// Schema and name the object was created under.
    pub(crate) identity: (String, String),
    pub(crate) table_parts: Vec<String>,
    /// Schema of the table or view the object is on. A temp trigger can be
    /// on a main table.
    pub(crate) table_schema: String,
    /// Indexed columns when every key is a plain column; `None` for triggers
    /// and expression indexes.
    pub(crate) columns: Option<Vec<String>>,
    /// Neither the object nor its table names a schema, and a rollback undid
    /// the drop of a temp table of that name: the object may be on it.
    pub(crate) table_shadowed: bool,
    /// Declared with IF NOT EXISTS, which does nothing if the object exists.
    pub(crate) if_not_exists: bool,
    /// The `ROLLBACK` that undid the creation, if one did.
    pub(crate) undone_at: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedSchemaObject {
    pub kind: SchemaObjectKind,
    pub name: String,
    /// One-based source line containing the DROP statement.
    pub line: usize,
    pub(crate) statement_index: usize,
    /// Schema and name the drop removes. An unqualified name already used by
    /// a temp index or trigger counts as temp, which keeps a main object.
    pub(crate) identity: (String, String),
    /// The `ROLLBACK` that undid the drop, if one did.
    pub(crate) undone_at: Option<usize>,
}

/// A foreign-key `ON DELETE` action that rewrites dependent rows when the
/// referenced row is deleted, including by `DROP TABLE` on the parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteAction {
    Cascade,
    SetNull,
    SetDefault,
}

impl DeleteAction {
    pub fn sql(self) -> &'static str {
        match self {
            Self::Cascade => "ON DELETE CASCADE",
            Self::SetNull => "ON DELETE SET NULL",
            Self::SetDefault => "ON DELETE SET DEFAULT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteActionReference {
    pub dependent_table: String,
    pub referenced_table: String,
    pub action: DeleteAction,
    /// One-based source line containing the reference.
    pub line: usize,
    pub(crate) statement_index: usize,
    /// Schema and table from the CREATE TABLE that declared the reference.
    pub(crate) dependent_identity: (String, String),
    pub(crate) referenced_table_parts: Vec<String>,
    /// The column that declares the reference inline. `DROP COLUMN` takes
    /// such a reference with it; a table constraint has none.
    pub(crate) column: Option<String>,
    /// The `ROLLBACK` that undid the CREATE TABLE, if one did.
    pub(crate) undone_at: Option<usize>,
}

/// A `DROP TABLE` or an `ALTER TABLE ... RENAME TO`, and the statement that
/// undid it if a `ROLLBACK` or `ROLLBACK TO` an earlier savepoint did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableChange {
    pub(crate) statement_index: usize,
    /// Schema and table the statement names. In a drop, an unqualified name
    /// already used by a temp table counts as temp, which keeps a main
    /// child's reference.
    pub(crate) table: (String, String),
    pub(crate) kind: TableChangeKind,
    pub(crate) undone_at: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TableChangeKind {
    Dropped,
    RenamedTo {
        name: String,
        /// Whether SQLite rewrote references to the table in other tables:
        /// not with `legacy_alter_table` on and foreign keys off. `None`
        /// when those settings are not known.
        rewrites_references: Option<bool>,
        /// The old name has no schema, so the rename may have hit a table of
        /// that name in another schema than the one resolved here.
        unqualified: bool,
        /// A temp table has used the old name, so the rename may have hit
        /// that temp table.
        shadowed: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub foreign_keys_pragmas_in_transactions: Vec<ForeignKeysPragmaInTransaction>,
    pub rebuilds: Vec<RebuildSequence>,
    pub delete_action_references: Vec<DeleteActionReference>,
    pub schema_objects: Vec<SchemaObject>,
    pub dropped_schema_objects: Vec<DroppedSchemaObject>,
    pub(crate) table_changes: Vec<TableChange>,
    pub(crate) column_changes: Vec<ColumnChange>,
}

/// An `ALTER TABLE ... RENAME COLUMN`, or with `to` empty a `DROP COLUMN`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColumnChange {
    pub(crate) statement_index: usize,
    pub(crate) table_parts: Vec<String>,
    pub(crate) from: String,
    pub(crate) to: Option<String>,
}

/// Recognizes copy/drop/rename, copy-out/drop/recreate/copy-back and
/// rename-away/create/copy/drop rebuilds. Records the foreign-key pragmas,
/// `ON DELETE` references, indexes and triggers in the input.
pub fn scan_sql(sql: &str) -> ScanReport {
    let statements = statements(sql);
    let mut report = ScanReport::default();
    let mut foreign_keys_enabled = None;
    let mut legacy_alter_table = Some(false);
    let mut attached = false;
    let mut foreign_key_states = Vec::with_capacity(statements.len());
    let mut journal = TransactionJournal::default();
    let mut temps = TempTables::default();
    let mut temp_objects = Vec::new();
    let mut unreadable_temp = false;

    for (statement_index, statement) in statements.iter().enumerate() {
        attached |= token_is(&statement.tokens, 0, "ATTACH");
        if let Some(name) = exact_dropped_table(statement) {
            let table = if name.parts.len() == 1 && (unreadable_temp || temps.shadowed(&name)) {
                ("temp".to_string(), name.leaf().to_string())
            } else {
                owned(temps.resolve(&name))
            };
            report.table_changes.push(TableChange {
                statement_index,
                table,
                kind: TableChangeKind::Dropped,
                undone_at: None,
            });
            journal.record(Undoable::TableChange(report.table_changes.len() - 1));
        } else if let Some((from, to)) = renamed_table(statement) {
            report.table_changes.push(TableChange {
                statement_index,
                table: owned(temps.resolve(&from)),
                kind: TableChangeKind::RenamedTo {
                    name: to.leaf().to_string(),
                    rewrites_references: match (legacy_alter_table, foreign_keys_enabled) {
                        (Some(false), _) | (_, Some(true)) => Some(true),
                        (Some(true), Some(false)) => Some(false),
                        _ => None,
                    },
                    unqualified: from.parts.len() == 1,
                    // After ATTACH an unqualified name may also mean a table
                    // in the attached database.
                    shadowed: unreadable_temp
                        || temps.shadowed(&from)
                        || (attached && from.parts.len() == 1),
                },
                undone_at: None,
            });
            journal.record(Undoable::TableChange(report.table_changes.len() - 1));
        } else if let Some((table, from, to)) = changed_column(statement) {
            report.column_changes.push(ColumnChange {
                statement_index,
                table_parts: table.parts,
                from,
                to,
            });
        } else if let Some(command) = TransactionCommand::parse(statement) {
            journal.apply(command, statement_index, &mut report);
        }
        temps.apply(statement);
        let tokens = &statement.tokens;
        if token_is(tokens, 0, "CREATE")
            && (token_is(tokens, 1, "TEMP") || token_is(tokens, 1, "TEMPORARY"))
            && created_table(statement).is_none()
            && created_view(statement).is_none()
            && !create_trigger_prefix(tokens)
        {
            unreadable_temp = true;
        }
        if let Some(mut schema_object) = created_schema_object(statement, statement_index, &temps) {
            // The temp table of that name is back only if its last drop was
            // rolled back.
            schema_object.table_shadowed &= report
                .table_changes
                .iter()
                .rev()
                .find(|change| {
                    change.kind == TableChangeKind::Dropped
                        && change.table.0 == "temp"
                        && schema_object.table_parts.last() == Some(&change.table.1)
                })
                .is_some_and(|last_drop| last_drop.undone_at.is_some());
            if schema_object.identity.0 == "temp" {
                temp_objects.push((schema_object.kind, schema_object.identity.1.clone()));
            }
            report.schema_objects.push(schema_object);
            journal.record(Undoable::ObjectCreation(report.schema_objects.len() - 1));
        }
        if let Some((kind, name)) = dropped_schema_object(statement) {
            // As with tables, a temp index or trigger that a rollback brought
            // back would be the one an unqualified DROP removes.
            let schema = match name.parts.as_slice() {
                [schema, _] => schema.clone(),
                _ if temp_objects.contains(&(kind, name.leaf().to_string())) => "temp".to_string(),
                _ => "main".to_string(),
            };
            report.dropped_schema_objects.push(DroppedSchemaObject {
                kind,
                name: name.canonical(),
                line: statement.line,
                statement_index,
                identity: (schema, name.leaf().to_string()),
                undone_at: None,
            });
            journal.record(Undoable::ObjectDrop(
                report.dropped_schema_objects.len() - 1,
            ));
        }
        for reference in delete_action_references(statement, statement_index) {
            report.delete_action_references.push(reference);
            journal.record(Undoable::Reference(
                report.delete_action_references.len() - 1,
            ));
        }
        if let Some(value) = pragma_assignment(statement, "legacy_alter_table") {
            legacy_alter_table = value;
        }
        match pragma_assignment(statement, "foreign_keys") {
            Some(Some(enabled)) if journal.in_transaction => report
                .foreign_keys_pragmas_in_transactions
                .push(ForeignKeysPragmaInTransaction {
                    line: statement.line,
                    requested_enabled: enabled,
                }),
            // A value the scanner cannot read leaves the state unknown.
            Some(value) if !journal.in_transaction => foreign_keys_enabled = value,
            _ => {}
        }
        foreign_key_states.push(foreign_keys_enabled);
    }

    let mut candidates = Vec::<RebuildCandidate>::new();
    let mut temp_tables = TempTables::default();
    let mut attached = false;
    for (statement_index, (statement, foreign_keys_enabled)) in
        statements.iter().zip(foreign_key_states).enumerate()
    {
        attached |= token_is(&statement.tokens, 0, "ATTACH");
        let dropped = dropped_table(statement).map(|name| {
            // An unqualified name may mean a temp table that has used it, or
            // a table in an attached database.
            let uncertain = temp_tables.shadowed(&name) || (attached && name.parts.len() == 1);
            let table = (!uncertain).then(|| owned(temp_tables.resolve(&name)));
            (name, table)
        });
        temp_tables.apply(statement);
        let completed = (statement.line, statement_index);

        if let Some((created, _)) = created_table(statement) {
            // Creating a dropped source again under its own name is the
            // replacement step of the copy-out/copy-back shape Rails emits.
            // Every holding table copied from it stays a candidate; the
            // copy-back decides which one was the rebuild.
            let columns = table_columns(statement);
            for source in candidates
                .iter_mut()
                .flat_map(|candidate| candidate.sources.iter_mut())
                .filter(|source| !source.drops.is_empty() && source.table.matches(&created))
            {
                source.creations.push(Creation {
                    line: statement.line,
                    statement_index,
                    columns: columns.clone(),
                });
            }

            // IF NOT EXISTS does nothing to a replacement that is there. Whether
            // it is there is not tracked, so the copies stay and the columns
            // are unknown when the two declarations disagree.
            let if_not_exists =
                token_is(&statement.tokens, 2, "IF") || token_is(&statement.tokens, 3, "IF");
            if if_not_exists
                && let Some(candidate) = candidates
                    .iter_mut()
                    .rev()
                    .find(|candidate| candidate.temporary_table.matches(&created))
            {
                if candidate.columns != columns {
                    candidate.columns = None;
                }
                continue;
            }
            // A new table with the name of an older, incomplete replacement
            // supersedes it, unless that one holds the rows of this table.
            candidates.retain(|candidate| {
                !candidate.temporary_table.matches(&created)
                    || candidate.sources.iter().any(|source| {
                        source
                            .creations
                            .last()
                            .is_some_and(|creation| creation.statement_index == statement_index)
                    })
            });
            candidates.push(RebuildCandidate {
                schema: created_identity(statement, &created).0,
                temporary_table: created,
                create_line: statement.line,
                create_statement_index: statement_index,
                columns,
                sources: Vec::new(),
            });
            continue;
        }

        if let Some((target, sources)) = copied_table(statement) {
            let target_table = temp_tables.resolve(&target);
            let copied_back = candidates.iter().rev().find_map(|candidate| {
                if !sources
                    .iter()
                    .any(|source| candidate.temporary_table.matches(source))
                {
                    return None;
                }
                let rebuilds = candidate.rebuilds(
                    &report,
                    RebuildKind::Recreate,
                    &target,
                    target_table,
                    completed,
                );
                // A temp table that held the rows is scratch space: when it
                // is dropped, created and filled again for the next rebuild,
                // that is not a rebuild of the temp table.
                let scratch_table =
                    (candidate.schema == "temp").then(|| candidate.temporary_table.clone());
                (!rebuilds.is_empty()).then_some((scratch_table, rebuilds))
            });
            let (scratch_table, rebuilds) = copied_back.unwrap_or_default();
            for (rebuild, schema_matches) in rebuilds {
                complete(&mut report, &mut candidates, rebuild, schema_matches);
            }

            // `t` and `main.t` are one table unless a temp `t` shadows it.
            let copies: Vec<_> = sources
                .into_iter()
                .filter(|source| temp_tables.resolve(source) != target_table)
                .map(|table| CopySource {
                    replaces_only: scratch_table
                        .as_ref()
                        .is_some_and(|scratch| scratch.matches(&table)),
                    table,
                    copy_line: statement.line,
                    copy_statement_index: statement_index,
                    drops: Vec::new(),
                    creations: Vec::new(),
                })
                .collect();
            // Any table the rows come from may be the one being rebuilt; the
            // drop and the rename or copy-back decide.
            if let Some(candidate) = candidates
                .iter_mut()
                .rev()
                .find(|candidate| candidate.temporary_table.matches(&target))
            {
                candidate.sources.extend(copies);
            }
            continue;
        }

        if let Some((dropped, table)) = dropped {
            let drop = DropPoint {
                line: statement.line,
                statement_index,
                foreign_keys_enabled,
                table,
            };
            // The drop is the last step when the original was renamed away
            // and its replacement already has the original name.
            let renamed_away = candidates
                .iter()
                .rev()
                .find_map(|candidate| candidate.renamed_away(&report, &dropped, &drop));
            report.rebuilds.extend(renamed_away);
            // A backup copy of the same table must not consume the drop that
            // belongs to the real rebuild, so every copy records it; the
            // rename or copy-back completes only the one that replaces the
            // original.
            for source in candidates
                .iter_mut()
                .flat_map(|candidate| candidate.sources.iter_mut())
                .filter(|source| source.table.matches(&dropped))
            {
                source.drops.push(drop.clone());
            }
            continue;
        }

        if let Some((temporary_table, original_table)) = renamed_table(statement) {
            // The candidate stays: if a rollback undoes the rename, the same
            // replacement can go through the steps again.
            let renamed = candidates.iter().rev().find_map(|candidate| {
                if !candidate.temporary_table.matches(&temporary_table) {
                    return None;
                }
                let rebuilds = candidate.rebuilds(
                    &report,
                    RebuildKind::Rename,
                    &original_table,
                    (candidate.schema.as_str(), original_table.leaf()),
                    completed,
                );
                (!rebuilds.is_empty()).then_some(rebuilds)
            });
            for (rebuild, schema_matches) in renamed.into_iter().flatten() {
                complete(&mut report, &mut candidates, rebuild, schema_matches);
            }
            // A rename brings a dropped name back like a CREATE does; the
            // columns of that table are not known here.
            for source in candidates
                .iter_mut()
                .flat_map(|candidate| candidate.sources.iter_mut())
                .filter(|source| !source.drops.is_empty() && source.table.matches(&original_table))
            {
                source.creations.push(Creation {
                    line: statement.line,
                    statement_index,
                    columns: None,
                });
            }
        }
    }

    report
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Word(String),
    QuotedIdentifier(String),
    StringLiteral(String),
    Symbol(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: TokenKind,
    line: usize,
}

#[derive(Debug)]
struct Statement {
    tokens: Vec<Token>,
    line: usize,
}

fn statements(sql: &str) -> Vec<Statement> {
    let tokens = lex(sql);
    let mut result = Vec::new();
    let mut current = Vec::new();
    let mut is_trigger = false;
    let mut trigger_body = false;
    let mut trigger_end = false;
    let mut case_depth = 0usize;

    for token in tokens {
        if matches!(token.kind, TokenKind::Symbol(';')) {
            if is_trigger && !(trigger_body && trigger_end) {
                current.push(token);
            } else {
                finish_statement(&mut result, &mut current);
                is_trigger = false;
                trigger_body = false;
                trigger_end = false;
                case_depth = 0;
            }
            continue;
        }

        current.push(token);
        if !is_trigger {
            is_trigger = create_trigger_prefix(&current);
        }
        if !is_trigger {
            continue;
        }

        let last = current.last().expect("token was just pushed");
        if !trigger_body {
            if keyword(last, "BEGIN") {
                trigger_body = true;
            }
        } else if keyword(last, "CASE") {
            case_depth += 1;
        } else if keyword(last, "END") {
            if case_depth == 0 {
                trigger_end = true;
            } else {
                case_depth -= 1;
            }
        }
    }

    finish_statement(&mut result, &mut current);
    result
}

fn create_trigger_prefix(tokens: &[Token]) -> bool {
    if !token_is(tokens, 0, "CREATE") {
        return false;
    }
    let mut index = 1;
    if token_is(tokens, index, "TEMP") || token_is(tokens, index, "TEMPORARY") {
        index += 1;
    }
    token_is(tokens, index, "TRIGGER")
}

fn finish_statement(result: &mut Vec<Statement>, tokens: &mut Vec<Token>) {
    if let Some(first) = tokens.first() {
        result.push(Statement {
            line: first.line,
            tokens: std::mem::take(tokens),
        });
    }
}

fn lex(sql: &str) -> Vec<Token> {
    let mut result = Vec::new();
    let mut line = 1usize;
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\n' => line += 1,
            // Only ASCII whitespace and a byte-order mark separate tokens in
            // SQLite.
            ch if ch.is_ascii_whitespace() || ch == '\u{feff}' => {}
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
            '\'' => {
                let start_line = line;
                let text = read_quoted(&mut chars, '\'', &mut line);
                result.push(Token {
                    kind: TokenKind::StringLiteral(text),
                    line: start_line,
                });
            }
            '"' | '`' => {
                let start_line = line;
                let identifier = read_quoted(&mut chars, ch, &mut line);
                result.push(Token {
                    kind: TokenKind::QuotedIdentifier(identifier),
                    line: start_line,
                });
            }
            '[' => {
                let start_line = line;
                let identifier = read_quoted(&mut chars, ']', &mut line);
                result.push(Token {
                    kind: TokenKind::QuotedIdentifier(identifier),
                    line: start_line,
                });
            }
            ch if is_word_character(ch) => {
                let start_line = line;
                let mut word = String::from(ch);
                while chars.peek().is_some_and(|next| is_word_character(*next)) {
                    word.push(chars.next().expect("peeked character is present"));
                }
                result.push(Token {
                    kind: TokenKind::Word(word),
                    line: start_line,
                });
            }
            _ => result.push(Token {
                kind: TokenKind::Symbol(ch),
                line,
            }),
        }
    }

    result
}

fn read_quoted<I>(chars: &mut std::iter::Peekable<I>, delimiter: char, line: &mut usize) -> String
where
    I: Iterator<Item = char>,
{
    let mut text = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\n' {
            *line += 1;
        }
        if ch == delimiter {
            if chars.peek() == Some(&delimiter) {
                chars.next();
                text.push(delimiter);
            } else {
                break;
            }
        } else {
            text.push(ch);
        }
    }
    text
}

fn is_word_character(ch: char) -> bool {
    // SQLite takes every non-ASCII character as part of a name.
    ch.is_alphanumeric() || matches!(ch, '_' | '$') || !ch.is_ascii()
}

fn keyword(token: &Token, expected: &str) -> bool {
    matches!(&token.kind, TokenKind::Word(word) if word.eq_ignore_ascii_case(expected))
}

fn token_is(tokens: &[Token], index: usize, expected: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| keyword(token, expected))
}

fn symbol_is(tokens: &[Token], index: usize, expected: char) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Symbol(expected))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Name {
    parts: Vec<String>,
}

impl Name {
    fn leaf(&self) -> &str {
        self.parts.last().expect("a name has at least one part")
    }

    fn matches(&self, other: &Self) -> bool {
        self == other
            || ((self.parts.len() == 1 || other.parts.len() == 1) && self.leaf() == other.leaf())
    }

    fn canonical(&self) -> String {
        self.parts.join(".")
    }
}

fn parse_name(tokens: &[Token], start: usize) -> Option<(Name, usize)> {
    let mut index = start;
    let mut parts = vec![identifier(tokens.get(index)?)?];
    index += 1;
    while symbol_is(tokens, index, '.') {
        parts.push(identifier(tokens.get(index + 1)?)?);
        index += 2;
    }
    Some((Name { parts }, index))
}

/// SQLite reads a string literal as a name where the grammar expects one.
fn identifier(token: &Token) -> Option<String> {
    match &token.kind {
        TokenKind::Word(value)
        | TokenKind::QuotedIdentifier(value)
        | TokenKind::StringLiteral(value) => Some(value.to_ascii_lowercase()),
        TokenKind::Symbol(_) => None,
    }
}

/// A parsed transaction-control statement.
#[derive(Debug, PartialEq, Eq)]
enum TransactionCommand {
    Begin,
    /// `None` when the name cannot be read.
    Savepoint(Option<Name>),
    Commit,
    Rollback,
    /// `None` when the target cannot be read or tokens follow it.
    RollbackTo(Option<Name>),
    Release(Option<Name>),
}

impl TransactionCommand {
    fn parse(statement: &Statement) -> Option<Self> {
        let tokens = &statement.tokens;
        let ends_at = |at: usize| at == tokens.len();
        let skip = |at: usize, word: &str| at + usize::from(token_is(tokens, at, word));
        let last_name = |at: usize| {
            parse_name(tokens, at)
                .filter(|(_, end)| ends_at(*end))
                .map(|(name, _)| name)
        };

        if token_is(tokens, 0, "BEGIN") {
            // Any BEGIN counts, even one SQLite would reject: assuming an
            // open transaction only keeps warnings.
            Some(Self::Begin)
        } else if token_is(tokens, 0, "SAVEPOINT") {
            let name = last_name(1);
            (name.is_some() || tokens.len() == 2).then_some(Self::Savepoint(name))
        } else if token_is(tokens, 0, "COMMIT") || token_is(tokens, 0, "END") {
            ends_at(after_transaction_name(tokens, 1)).then_some(Self::Commit)
        } else if token_is(tokens, 0, "ROLLBACK") {
            let at = after_transaction_name(tokens, 1);
            if ends_at(at) {
                Some(Self::Rollback)
            } else if token_is(tokens, at, "TO") {
                Some(Self::RollbackTo(last_name(skip(at + 1, "SAVEPOINT"))))
            } else {
                Some(Self::RollbackTo(None))
            }
        } else if token_is(tokens, 0, "RELEASE") {
            Some(Self::Release(last_name(skip(1, "SAVEPOINT"))))
        } else {
            None
        }
    }
}

/// Skips the `TRANSACTION [name]` that COMMIT, END and ROLLBACK accept.
fn after_transaction_name(tokens: &[Token], at: usize) -> usize {
    if !token_is(tokens, at, "TRANSACTION") {
        return at;
    }
    match tokens.get(at + 1) {
        Some(token) if !keyword(token, "TO") && !matches!(token.kind, TokenKind::Symbol(_)) => {
            at + 2
        }
        _ => at + 1,
    }
}

/// A change made in the open transaction, by its position in the report.
#[derive(Debug, Clone, Copy)]
enum Undoable {
    TableChange(usize),
    ObjectCreation(usize),
    ObjectDrop(usize),
    Reference(usize),
}

/// Follows the open transaction and which changes a later `ROLLBACK` undoes:
/// all of them, or with `ROLLBACK TO` those made after the savepoint was set.
/// SQLite rejects a rollback whose target is not an open savepoint; the
/// journal still counts it as undoing every open drop, so a name it misreads
/// cannot hide a dependent table or a lost index. A savepoint whose name it
/// cannot read stops RELEASE and ROLLBACK TO from resolving past it.
#[derive(Debug, Default)]
struct TransactionJournal {
    open: Vec<Undoable>,
    /// Open savepoints (name, when readable) and the statement that set each.
    savepoints: Vec<(Option<Name>, usize)>,
    in_transaction: bool,
    explicit: bool,
}

impl TransactionJournal {
    fn record(&mut self, change: Undoable) {
        if self.in_transaction {
            self.open.push(change);
        }
    }

    fn apply(&mut self, command: TransactionCommand, index: usize, report: &mut ScanReport) {
        match command {
            TransactionCommand::Begin => {
                if !self.in_transaction {
                    self.in_transaction = true;
                    self.explicit = true;
                }
            }
            TransactionCommand::Savepoint(name) => {
                if !self.in_transaction {
                    self.in_transaction = true;
                    self.explicit = false;
                }
                self.savepoints.push((name, index));
            }
            TransactionCommand::Commit => self.close(),
            TransactionCommand::Rollback => {
                self.undo_open(report, index, Rollback::All);
                self.close();
            }
            TransactionCommand::RollbackTo(name) => {
                let target = name
                    .and_then(|name| self.savepoint(&name))
                    .filter(|&position| self.unambiguous_from(position));
                match target {
                    Some(position) => {
                        let since = self.savepoints[position].1;
                        self.savepoints.truncate(position + 1);
                        self.undo_open(report, index, Rollback::Since(since));
                    }
                    None => self.undo_open(report, index, Rollback::Unresolved),
                }
            }
            TransactionCommand::Release(name) => {
                let position = name.and_then(|name| self.savepoint(&name));
                if position.is_some_and(|position| !self.unambiguous_from(position)) {
                    self.undo_open(report, index, Rollback::Unresolved);
                } else if let Some(position) = position {
                    self.savepoints.truncate(position);
                    if self.savepoints.is_empty() && !self.explicit {
                        self.close();
                    }
                }
            }
        }
    }

    fn savepoint(&self, name: &Name) -> Option<usize> {
        self.savepoints
            .iter()
            .rposition(|(saved, _)| saved.as_ref().is_some_and(|saved| saved.matches(name)))
    }

    /// No savepoint with an unreadable name sits above `position`, so a
    /// RELEASE or ROLLBACK TO naming `position` cannot mean a nearer one.
    fn unambiguous_from(&self, position: usize) -> bool {
        self.savepoints[position + 1..]
            .iter()
            .all(|(name, _)| name.is_some())
    }

    fn undo_open(&mut self, report: &mut ScanReport, index: usize, rollback: Rollback) {
        self.open.retain(|change| {
            let (changed_at, undone_at, is_drop) = match *change {
                Undoable::TableChange(position) => {
                    let table = &mut report.table_changes[position];
                    (
                        table.statement_index,
                        &mut table.undone_at,
                        table.kind == TableChangeKind::Dropped,
                    )
                }
                Undoable::ObjectCreation(position) => {
                    let object = &mut report.schema_objects[position];
                    (object.statement_index, &mut object.undone_at, false)
                }
                Undoable::ObjectDrop(position) => {
                    let object = &mut report.dropped_schema_objects[position];
                    (object.statement_index, &mut object.undone_at, true)
                }
                Undoable::Reference(position) => {
                    let reference = &mut report.delete_action_references[position];
                    (reference.statement_index, &mut reference.undone_at, false)
                }
            };
            let undone = match rollback {
                Rollback::All => true,
                Rollback::Since(since) => changed_at > since,
                Rollback::Unresolved => is_drop,
            };
            if undone {
                *undone_at = Some(index);
            }
            !undone
        });
    }

    fn close(&mut self) {
        self.open.clear();
        self.savepoints.clear();
        self.in_transaction = false;
        self.explicit = false;
    }
}

#[derive(Debug, Clone, Copy)]
enum Rollback {
    All,
    /// To a savepoint set by the statement at this index.
    Since(usize),
    /// `ROLLBACK TO` or `RELEASE` of a target the journal cannot place.
    Unresolved,
}

/// The value an assignment to the boolean pragma `pragma` asks for: `None` for
/// other statements and for the query form, `Some(None)` when the value
/// cannot be read.
fn pragma_assignment(statement: &Statement, pragma: &str) -> Option<Option<bool>> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "PRAGMA") {
        return None;
    }
    let (name, index) = parse_name(tokens, 1)?;
    if name.leaf() != pragma || name.parts.len() > 2 {
        return None;
    }
    let parenthesized = symbol_is(tokens, index, '(');
    if !parenthesized && !symbol_is(tokens, index, '=') {
        return None;
    }
    let mut value_at = index + 1;
    if tokens.len() <= value_at || symbol_is(tokens, value_at, ')') {
        return None;
    }
    let negative = symbol_is(tokens, value_at, '-');
    let signed = negative || symbol_is(tokens, value_at, '+');
    value_at += usize::from(signed);
    let end = value_at + 1 + usize::from(parenthesized);
    if tokens.len() != end || (parenthesized && !symbol_is(tokens, value_at + 1, ')')) {
        return Some(None);
    }
    // The value may be bare or quoted: `ON`, `'ON'` and `"ON"` are the same.
    let Some(value) = tokens.get(value_at).and_then(identifier) else {
        return Some(None);
    };
    Some(match value.as_str() {
        "off" | "false" | "no" if !signed => Some(false),
        "on" | "true" | "yes" if !signed => Some(true),
        // SQLite reads a negative number as off.
        number => number
            .parse::<i64>()
            .ok()
            .map(|value| value != 0 && !negative),
    })
}

/// Temporary tables and views that exist at a point in the input, so an
/// unqualified name can be resolved the way SQLite does: `temp` first, then
/// `main`.
#[derive(Default)]
struct TempTables {
    names: Vec<String>,
    /// Every name a temp table has had, including tables since dropped.
    ever: std::collections::HashSet<String>,
}

impl TempTables {
    /// Called before the statement's own copy is resolved; only CREATE, DROP
    /// and RENAME change the set.
    fn apply(&mut self, statement: &Statement) {
        let tokens = &statement.tokens;
        if let Some((name, _)) = created_table(statement) {
            let temporary = token_is(tokens, 1, "TEMP")
                || token_is(tokens, 1, "TEMPORARY")
                || matches!(name.parts.as_slice(), [schema, _] if schema == "temp");
            if temporary {
                self.ever.insert(name.leaf().to_string());
                if !self.names.iter().any(|leaf| leaf == name.leaf()) {
                    self.names.push(name.leaf().to_string());
                }
            }
        } else if let Some((name, true)) = created_view(statement) {
            self.ever.insert(name.leaf().to_string());
            if !self.names.iter().any(|leaf| leaf == name.leaf()) {
                self.names.push(name.leaf().to_string());
            }
        } else if let Some(name) =
            exact_dropped_table(statement).or_else(|| dropped_table(statement))
            && self.resolve(&name).0 == "temp"
        {
            self.names.retain(|leaf| leaf != name.leaf());
        } else if let Some((from, to)) = renamed_table(statement)
            && self.resolve(&from).0 == "temp"
        {
            self.names.retain(|leaf| leaf != from.leaf());
            self.names.push(to.leaf().to_string());
            self.ever.insert(to.leaf().to_string());
        }
    }

    /// A rollback can bring a temp table back without the scanner seeing it,
    /// so an unqualified name a temp table once had is not known to mean the
    /// main table.
    fn shadowed(&self, name: &Name) -> bool {
        name.parts.len() == 1 && self.ever.contains(name.leaf())
    }

    fn resolve<'a>(&self, name: &'a Name) -> (&'a str, &'a str) {
        let leaf = name.leaf();
        match name.parts.as_slice() {
            [schema, _] => (schema.as_str(), leaf),
            _ if self.names.iter().any(|temp| temp == leaf) => ("temp", leaf),
            _ => ("main", leaf),
        }
    }
}

fn created_table(statement: &Statement) -> Option<(Name, usize)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "CREATE") {
        return None;
    }
    let mut index = 1;
    if token_is(tokens, index, "TEMP") || token_is(tokens, index, "TEMPORARY") {
        index += 1;
    }
    if !token_is(tokens, index, "TABLE") {
        return None;
    }
    index += 1;
    if token_is(tokens, index, "IF")
        && token_is(tokens, index + 1, "NOT")
        && token_is(tokens, index + 2, "EXISTS")
    {
        index += 3;
    }
    parse_name(tokens, index)
}

/// Plain column keys of an index, optionally with COLLATE and ASC/DESC.
fn index_columns(tokens: &[Token], opening_parenthesis: usize) -> Option<Vec<String>> {
    table_definition_ranges(tokens, opening_parenthesis)
        .into_iter()
        .map(|(start, end)| {
            let column = identifier(&tokens[start])?;
            // A bare word can still be a literal: 1, NULL, TRUE, CURRENT_TIME.
            let literal = matches!(&tokens[start].kind, TokenKind::Word(_))
                && (column.starts_with(|ch: char| ch.is_ascii_digit())
                    || [
                        "null",
                        "true",
                        "false",
                        "current_time",
                        "current_date",
                        "current_timestamp",
                    ]
                    .contains(&column.as_str()));
            if literal {
                return None;
            }
            let mut index = start + 1;
            if token_is(tokens, index, "COLLATE") {
                identifier(tokens.get(index + 1)?)?;
                index += 2;
            }
            if token_is(tokens, index, "ASC") || token_is(tokens, index, "DESC") {
                index += 1;
            }
            (index == end).then_some(column)
        })
        .collect()
}

/// Column names of a CREATE TABLE with a column list; `None` for AS SELECT.
fn table_columns(statement: &Statement) -> Option<Vec<String>> {
    let (_, opening_parenthesis) = created_table(statement)?;
    let tokens = &statement.tokens;
    if !symbol_is(tokens, opening_parenthesis, '(') {
        return None;
    }
    // A declaration whose name cannot be read makes the whole list unknown
    // rather than silently shorter.
    table_definition_ranges(tokens, opening_parenthesis)
        .into_iter()
        .filter(|(start, _)| {
            !["CONSTRAINT", "PRIMARY", "FOREIGN", "UNIQUE", "CHECK"]
                .iter()
                .any(|keyword| token_is(tokens, *start, keyword))
        })
        .map(|(start, _)| identifier(&tokens[start]))
        .collect()
}

fn created_schema_object(
    statement: &Statement,
    statement_index: usize,
    temps: &TempTables,
) -> Option<SchemaObject> {
    created_index(statement, statement_index, temps)
        .or_else(|| created_trigger(statement, statement_index, temps))
}

/// An index or a trigger without a schema of its own lives in its table's.
fn object_identity(
    name: &Name,
    temporary: bool,
    table: &Name,
    temps: &TempTables,
) -> (String, String) {
    let schema = match name.parts.as_slice() {
        [schema, _] => schema.as_str(),
        _ if temporary => "temp",
        _ => temps.resolve(table).0,
    };
    (schema.to_string(), name.leaf().to_string())
}

fn created_index(
    statement: &Statement,
    statement_index: usize,
    temps: &TempTables,
) -> Option<SchemaObject> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "CREATE") {
        return None;
    }
    let mut index = 1;
    if token_is(tokens, index, "UNIQUE") {
        index += 1;
    }
    if !token_is(tokens, index, "INDEX") {
        return None;
    }
    index += 1;
    let if_not_exists = token_is(tokens, index, "IF")
        && token_is(tokens, index + 1, "NOT")
        && token_is(tokens, index + 2, "EXISTS");
    if if_not_exists {
        index += 3;
    }
    let (name, index) = parse_name(tokens, index)?;
    if !token_is(tokens, index, "ON") {
        return None;
    }
    let (table, index) = parse_name(tokens, index + 1)?;
    let columns = symbol_is(tokens, index, '(')
        .then(|| index_columns(tokens, index))
        .flatten();
    Some(SchemaObject {
        kind: SchemaObjectKind::Index,
        name: name.canonical(),
        table: table.canonical(),
        line: statement.line,
        statement_index,
        identity: object_identity(&name, false, &table, temps),
        table_shadowed: name.parts.len() == 1
            && temps.shadowed(&table)
            && temps.resolve(&table).0 != "temp",
        if_not_exists,
        // An index is in the schema of its table.
        table_schema: object_identity(&name, false, &table, temps).0,
        table_parts: table.parts,
        columns,
        undone_at: None,
    })
}

fn created_trigger(
    statement: &Statement,
    statement_index: usize,
    temps: &TempTables,
) -> Option<SchemaObject> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "CREATE") {
        return None;
    }
    let mut index = 1;
    let mut temporary = token_is(tokens, index, "TEMP") || token_is(tokens, index, "TEMPORARY");
    if temporary {
        index += 1;
    }
    if !token_is(tokens, index, "TRIGGER") {
        return None;
    }
    index += 1;
    let if_not_exists = token_is(tokens, index, "IF")
        && token_is(tokens, index + 1, "NOT")
        && token_is(tokens, index + 2, "EXISTS");
    if if_not_exists {
        index += 3;
    }
    let (name, index) = parse_name(tokens, index)?;
    // `CREATE TRIGGER temp.t` is a temp trigger too, and like one it may be
    // on a main table.
    temporary |= matches!(name.parts.as_slice(), [schema, _] if schema == "temp");
    let (table, _) = (index..tokens.len()).find_map(|position| {
        if !token_is(tokens, position, "ON") {
            return None;
        }
        let (table, end) = parse_name(tokens, position + 1)?;
        let follows_attachment = end == tokens.len()
            || token_is(tokens, end, "FOR")
            || token_is(tokens, end, "WHEN")
            || token_is(tokens, end, "BEGIN");
        follows_attachment.then_some((table, end))
    })?;
    Some(SchemaObject {
        kind: SchemaObjectKind::Trigger,
        name: name.canonical(),
        table: table.canonical(),
        line: statement.line,
        statement_index,
        identity: object_identity(&name, temporary, &table, temps),
        table_shadowed: name.parts.len() == 1
            && !temporary
            && temps.shadowed(&table)
            && temps.resolve(&table).0 != "temp",
        if_not_exists,
        table_schema: match (table.parts.as_slice(), name.parts.as_slice()) {
            ([schema, _], _) => schema.clone(),
            (_, [schema, _]) if !temporary => schema.clone(),
            _ => temps.resolve(&table).0.to_string(),
        },
        table_parts: table.parts,
        columns: None,
        undone_at: None,
    })
}

fn dropped_schema_object(statement: &Statement) -> Option<(SchemaObjectKind, Name)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "DROP") {
        return None;
    }
    let kind = if token_is(tokens, 1, "INDEX") {
        SchemaObjectKind::Index
    } else if token_is(tokens, 1, "TRIGGER") {
        SchemaObjectKind::Trigger
    } else {
        return None;
    };
    let mut index = 2;
    if token_is(tokens, index, "IF") && token_is(tokens, index + 1, "EXISTS") {
        index += 2;
    }
    let (name, end) = parse_name(tokens, index)?;
    (end == tokens.len()).then_some((kind, name))
}

fn delete_action_references(
    statement: &Statement,
    statement_index: usize,
) -> Vec<DeleteActionReference> {
    let tokens = &statement.tokens;
    let Some((dependent_table, opening_parenthesis)) = created_table(statement) else {
        return Vec::new();
    };
    if !symbol_is(tokens, opening_parenthesis, '(') {
        return Vec::new();
    }

    let mut references = Vec::new();
    for (start, end) in table_definition_ranges(tokens, opening_parenthesis) {
        // A column can carry several REFERENCES clauses; the actions of each
        // run up to the next one.
        let clauses: Vec<_> = (start..end)
            .filter(|index| token_is(tokens, *index, "REFERENCES"))
            .collect();
        for (position, &reference_index) in clauses.iter().enumerate() {
            let end = clauses.get(position + 1).copied().unwrap_or(end);
            let Some((referenced_table, mut index)) = parse_name(tokens, reference_index + 1)
            else {
                continue;
            };
            if symbol_is(tokens, index, '(') {
                let Some(after_columns) = after_balanced_parentheses(tokens, index) else {
                    continue;
                };
                index = after_columns;
            }
            let Some(action) = delete_action(tokens, index, end) else {
                continue;
            };
            if dependent_table.matches(&referenced_table) {
                continue;
            }
            references.push(DeleteActionReference {
                dependent_table: dependent_table.canonical(),
                referenced_table: referenced_table.canonical(),
                action,
                line: tokens[reference_index].line,
                statement_index,
                dependent_identity: created_identity(statement, &dependent_table),
                referenced_table_parts: referenced_table.parts,
                column: (!["CONSTRAINT", "FOREIGN"]
                    .iter()
                    .any(|keyword| token_is(tokens, start, keyword)))
                .then(|| identifier(&tokens[start]))
                .flatten(),
                undone_at: None,
            });
        }
    }
    references
}

fn delete_action(tokens: &[Token], start: usize, end: usize) -> Option<DeleteAction> {
    (start..end).find_map(|index| {
        if !token_is(tokens, index, "ON") || !token_is(tokens, index + 1, "DELETE") {
            return None;
        }
        if token_is(tokens, index + 2, "CASCADE") {
            Some(DeleteAction::Cascade)
        } else if token_is(tokens, index + 2, "SET") && token_is(tokens, index + 3, "NULL") {
            Some(DeleteAction::SetNull)
        } else if token_is(tokens, index + 2, "SET") && token_is(tokens, index + 3, "DEFAULT") {
            Some(DeleteAction::SetDefault)
        } else {
            None
        }
    })
}

fn table_definition_ranges(tokens: &[Token], opening_parenthesis: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut depth = 1usize;
    let mut start = opening_parenthesis + 1;

    for (index, token) in tokens.iter().enumerate().skip(start) {
        match token.kind {
            TokenKind::Symbol('(') => depth += 1,
            TokenKind::Symbol(')') => {
                depth -= 1;
                if depth == 0 {
                    if start < index {
                        ranges.push((start, index));
                    }
                    break;
                }
            }
            TokenKind::Symbol(',') if depth == 1 => {
                if start < index {
                    ranges.push((start, index));
                }
                start = index + 1;
            }
            _ => {}
        }
    }

    ranges
}

/// The table an `INSERT ... SELECT` fills and every table its rows may come
/// from: the names after FROM and JOIN, also in subqueries and in the queries
/// of a WITH clause before the INSERT or before its SELECT. The name of such
/// a query counts as a table too.
fn copied_table(statement: &Statement) -> Option<(Name, Vec<Name>)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "WITH") {
        return inserted_table(tokens, 0);
    }
    // The queries of the WITH clause are in parentheses, so the INSERT is the
    // one outside them.
    let mut depth = 0usize;
    (0..tokens.len()).find_map(|position| {
        match tokens[position].kind {
            TokenKind::Symbol('(') => depth += 1,
            TokenKind::Symbol(')') => depth = depth.saturating_sub(1),
            _ => {}
        }
        (depth == 0)
            .then(|| inserted_table(tokens, position))
            .flatten()
    })
}

/// The copy whose INSERT or REPLACE keyword is the token at `start`.
fn inserted_table(tokens: &[Token], start: usize) -> Option<(Name, Vec<Name>)> {
    // `INSERT [OR resolution] INTO` or its alias `REPLACE INTO`.
    let mut index = start + 1;
    if token_is(tokens, start, "INSERT") {
        if token_is(tokens, index, "OR")
            && ["ABORT", "FAIL", "IGNORE", "REPLACE", "ROLLBACK"]
                .iter()
                .any(|resolution| token_is(tokens, index + 1, resolution))
        {
            index += 2;
        }
    } else if !token_is(tokens, start, "REPLACE") {
        return None;
    }
    if !token_is(tokens, index, "INTO") {
        return None;
    }
    let (target, mut index) = parse_name(tokens, index + 1)?;
    if symbol_is(tokens, index, '(') {
        index = after_balanced_parentheses(tokens, index)?;
    }
    if !token_is(tokens, index, "SELECT") && !token_is(tokens, index, "WITH") {
        return None;
    }
    let sources: Vec<_> = (0..start)
        .chain(index + 1..tokens.len())
        .filter(|position| {
            token_is(tokens, *position, "JOIN")
                // Not the FROM of `IS [NOT] DISTINCT FROM`.
                || (token_is(tokens, *position, "FROM")
                    && !token_is(tokens, *position - 1, "DISTINCT"))
        })
        .filter_map(|position| parse_name(tokens, position + 1))
        .map(|(source, _)| source)
        .collect();
    (!sources.is_empty()).then_some((target, sources))
}

fn after_balanced_parentheses(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        if token.kind == TokenKind::Symbol('(') {
            depth += 1;
        } else if token.kind == TokenKind::Symbol(')') {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index + 1);
            }
        }
    }
    None
}

/// The table a `DROP TABLE [IF EXISTS]` names and the position after it.
fn drop_table_name(statement: &Statement) -> Option<(Name, usize)> {
    drop_name(statement, "TABLE")
}

fn drop_name(statement: &Statement, kind: &str) -> Option<(Name, usize)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "DROP") || !token_is(tokens, 1, kind) {
        return None;
    }
    let index = if token_is(tokens, 2, "IF") && token_is(tokens, 3, "EXISTS") {
        4
    } else {
        2
    };
    parse_name(tokens, index)
}

fn dropped_table(statement: &Statement) -> Option<Name> {
    drop_table_name(statement).map(|(name, _)| name)
}

/// A malformed drop fails in SQLite, so it must not hide a surviving child.
/// A view shares the namespace of tables, and its triggers go with it.
fn exact_dropped_table(statement: &Statement) -> Option<Name> {
    let (name, end) = drop_table_name(statement).or_else(|| drop_name(statement, "VIEW"))?;
    (end == statement.tokens.len() && name.parts.len() <= 2).then_some(name)
}

/// The name in `CREATE [TEMP] VIEW [IF NOT EXISTS]` and whether the view is
/// temporary.
fn created_view(statement: &Statement) -> Option<(Name, bool)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "CREATE") {
        return None;
    }
    let temporary = token_is(tokens, 1, "TEMP") || token_is(tokens, 1, "TEMPORARY");
    let mut index = 1 + usize::from(temporary);
    if !token_is(tokens, index, "VIEW") {
        return None;
    }
    index += 1;
    if token_is(tokens, index, "IF")
        && token_is(tokens, index + 1, "NOT")
        && token_is(tokens, index + 2, "EXISTS")
    {
        index += 3;
    }
    let (name, _) = parse_name(tokens, index)?;
    let temporary = temporary || matches!(name.parts.as_slice(), [schema, _] if schema == "temp");
    Some((name, temporary))
}

fn owned((schema, table): (&str, &str)) -> (String, String) {
    (schema.to_string(), table.to_string())
}

/// An unqualified CREATE TABLE targets main even when a temp table shadows
/// the name.
fn created_identity(statement: &Statement, table: &Name) -> (String, String) {
    let tokens = &statement.tokens;
    let schema = match table.parts.as_slice() {
        [schema, _] => schema.clone(),
        _ if token_is(tokens, 1, "TEMP") || token_is(tokens, 1, "TEMPORARY") => "temp".to_string(),
        _ => "main".to_string(),
    };
    (schema, table.leaf().to_string())
}

fn renamed_table(statement: &Statement) -> Option<(Name, Name)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "ALTER") || !token_is(tokens, 1, "TABLE") {
        return None;
    }
    let (from, index) = parse_name(tokens, 2)?;
    if !token_is(tokens, index, "RENAME") || !token_is(tokens, index + 1, "TO") {
        return None;
    }
    let (to, end) = parse_name(tokens, index + 2)?;
    (end == tokens.len()).then_some((from, to))
}

/// The table and column of `ALTER TABLE ... RENAME [COLUMN] a TO b` (with the
/// new name) or `ALTER TABLE ... DROP [COLUMN] a` (without one).
fn changed_column(statement: &Statement) -> Option<(Name, String, Option<String>)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "ALTER") || !token_is(tokens, 1, "TABLE") {
        return None;
    }
    let (table, mut index) = parse_name(tokens, 2)?;
    let dropped = token_is(tokens, index, "DROP");
    if !dropped && (!token_is(tokens, index, "RENAME") || token_is(tokens, index + 1, "TO")) {
        return None;
    }
    index += 1 + usize::from(token_is(tokens, index + 1, "COLUMN"));
    let from = identifier(tokens.get(index)?)?;
    if dropped {
        return Some((table, from, None));
    }
    if !token_is(tokens, index + 1, "TO") {
        return None;
    }
    Some((table, from, Some(identifier(tokens.get(index + 2)?)?)))
}

/// Records a rebuild that a rename or a copy-back finished. If its drop had
/// been taken for the last step of a rename-away rebuild, this one replaces
/// that reading. A drop in another schema than the replacement stays
/// available to the candidate it belongs to.
fn complete(
    report: &mut ScanReport,
    candidates: &mut [RebuildCandidate],
    rebuild: RebuildSequence,
    schema_matches: bool,
) {
    if schema_matches {
        forget_siblings(candidates, rebuild.drop_statement_index);
    }
    report.rebuilds.retain(|earlier| {
        earlier.kind != RebuildKind::RenameAway
            || earlier.drop_statement_index != rebuild.drop_statement_index
    });
    report.rebuilds.push(rebuild);
}

/// Other copies taken before the same DROP describe the same destroyed table;
/// once one of them completes the rebuild they must not report it again. The
/// copies stay, in case the table is dropped again.
fn forget_siblings(candidates: &mut [RebuildCandidate], drop_statement_index: usize) {
    for source in candidates
        .iter_mut()
        .flat_map(|candidate| candidate.sources.iter_mut())
    {
        source
            .drops
            .retain(|drop| drop.statement_index != drop_statement_index);
    }
}

/// A table whose rows a candidate received, and each place where the input
/// dropped a table of that name or created one after such a drop.
#[derive(Debug)]
struct CopySource {
    table: Name,
    copy_line: usize,
    copy_statement_index: usize,
    drops: Vec<DropPoint>,
    creations: Vec<Creation>,
    /// A scratch table recorded at a copy-back: the candidate may replace it
    /// by a rename, but is not rebuilt through it.
    replaces_only: bool,
}

#[derive(Debug, Clone)]
struct DropPoint {
    line: usize,
    statement_index: usize,
    foreign_keys_enabled: Option<bool>,
    /// Schema and table the DROP resolved to, when that is certain.
    table: Option<(String, String)>,
}

#[derive(Debug)]
struct Creation {
    line: usize,
    statement_index: usize,
    columns: Option<Vec<String>>,
}

#[derive(Debug)]
struct RebuildCandidate {
    temporary_table: Name,
    schema: String,
    create_line: usize,
    create_statement_index: usize,
    columns: Option<Vec<String>>,
    sources: Vec<CopySource>,
}

impl RebuildCandidate {
    /// The rename-away rebuild that the drop of `dropped` finishes: one or
    /// more `ALTER TABLE ... RENAME TO` took the table now called `dropped`
    /// from this name to that one, and a table of this name then received
    /// rows from it.
    fn renamed_away(
        &self,
        report: &ScanReport,
        dropped: &Name,
        drop: &DropPoint,
    ) -> Option<RebuildSequence> {
        let original = self.temporary_table.leaf();
        let renamed_before = |at: usize| {
            let mut names = vec![dropped.leaf()];
            for change in report.table_changes.iter().rev() {
                if change.statement_index < at
                    && let TableChangeKind::RenamedTo { name, .. } = &change.kind
                    && names.contains(&name.as_str())
                {
                    names.push(&change.table.1);
                }
            }
            names[1..].contains(&original)
        };
        let source = self.sources.iter().find(|source| {
            source.table.matches(dropped)
                && !source.replaces_only
                && renamed_before(source.copy_statement_index)
        })?;
        Some(RebuildSequence {
            kind: RebuildKind::RenameAway,
            temporary_table: source.table.canonical(),
            original_table: self.temporary_table.canonical(),
            create_line: self.create_line,
            copy_line: source.copy_line,
            drop_line: drop.line,
            replacement_line: self.create_line,
            foreign_keys_enabled_at_drop: drop.foreign_keys_enabled,
            drop_statement_index: drop.statement_index,
            replacement_statement_index: drop.statement_index,
            temporary_table_parts: source.table.parts.clone(),
            original_table_parts: self.temporary_table.parts.clone(),
            original_schema: drop.table.as_ref().map(|(schema, _)| schema.clone()),
            // A CREATE TABLE from before the rename is the original's; the
            // replacement then came from a statement that left no record,
            // such as CREATE TABLE IF NOT EXISTS, and its columns are not
            // known.
            replacement_columns: renamed_before(self.create_statement_index)
                .then(|| self.columns.clone())
                .flatten(),
        })
    }

    /// The rebuilds of `original` that the statement at `completed` (line,
    /// index) finishes: one for every drop that may have destroyed that
    /// table.
    ///
    /// A drop that a rollback had undone does not count, nor one that
    /// resolved to another schema than `wanted` when a drop in that schema
    /// is known, nor one that found no table: a second drop in the same
    /// schema with no CREATE of the name in between. In the copy-back shape
    /// the table must have been created again after the drop.
    fn rebuilds(
        &self,
        report: &ScanReport,
        kind: RebuildKind,
        original: &Name,
        wanted: (&str, &str),
        completed: (usize, usize),
    ) -> Vec<(RebuildSequence, bool)> {
        let undone = |drop: &DropPoint| {
            report.table_changes.iter().any(|change| {
                change.statement_index == drop.statement_index
                    && change.undone_at.is_some_and(|undone| undone < completed.1)
            })
        };
        let sources = || {
            self.sources.iter().filter(|source| {
                source.table.matches(original)
                    && (kind == RebuildKind::Rename || !source.replaces_only)
            })
        };
        let created_between = |from: usize, to: usize| {
            sources().any(|source| {
                source.creations.iter().any(|creation| {
                    creation.statement_index > from && creation.statement_index < to
                })
            })
        };
        let mut drops: Vec<_> = sources()
            .flat_map(|source| source.drops.iter().map(move |drop| (source, drop)))
            .filter(|(_, drop)| {
                drop.statement_index < completed.1
                    && (kind == RebuildKind::Rename
                        || created_between(drop.statement_index, completed.1))
            })
            .collect();
        drops.sort_by_key(|(_, drop)| drop.statement_index);
        drops.dedup_by_key(|(_, drop)| drop.statement_index);
        if drops.iter().any(|(_, drop)| !undone(drop)) {
            drops.retain(|(_, drop)| !undone(drop));
        }
        let in_schema = |drop: &DropPoint| {
            drop.table
                .as_ref()
                .is_none_or(|(schema, table)| (schema.as_str(), table.as_str()) == wanted)
        };
        // With no drop in that schema, a drop of the name elsewhere is still
        // reported, but left for the candidate it may belong to.
        let schema_matches = drops.iter().any(|(_, drop)| in_schema(drop));
        if schema_matches {
            drops.retain(|(_, drop)| in_schema(drop));
        }
        (0..drops.len())
            .filter(|position| {
                let drop = drops[*position].1;
                *position == 0
                    || drop.table.is_none()
                    || drop.table != drops[*position - 1].1.table
                    || created_between(drops[*position - 1].1.statement_index, drop.statement_index)
            })
            .map(|position| {
                let (source, drop) = drops[position];
                (self.rebuild(kind, source, drop, completed), schema_matches)
            })
            .collect()
    }

    fn rebuild(
        &self,
        kind: RebuildKind,
        source: &CopySource,
        drop: &DropPoint,
        completed: (usize, usize),
    ) -> RebuildSequence {
        // The table is back from the first CREATE after the drop. If later
        // ones list other columns, the columns it has are not known.
        let mut creations = source.creations.iter().filter(|creation| {
            creation.statement_index > drop.statement_index
                && creation.statement_index < completed.1
        });
        let (replacement_line, replacement_statement_index, replacement_columns) =
            match creations.next() {
                Some(first) if kind == RebuildKind::Recreate => (
                    first.line,
                    first.statement_index,
                    creations
                        .all(|creation| creation.columns == first.columns)
                        .then(|| first.columns.clone())
                        .flatten(),
                ),
                _ => (completed.0, completed.1, self.columns.clone()),
            };
        RebuildSequence {
            kind,
            temporary_table: self.temporary_table.canonical(),
            original_table: source.table.canonical(),
            create_line: self.create_line,
            copy_line: source.copy_line,
            drop_line: drop.line,
            replacement_line,
            foreign_keys_enabled_at_drop: drop.foreign_keys_enabled,
            drop_statement_index: drop.statement_index,
            replacement_statement_index,
            temporary_table_parts: self.temporary_table.parts.clone(),
            original_table_parts: source.table.parts.clone(),
            original_schema: drop.table.as_ref().map(|(schema, _)| schema.clone()),
            replacement_columns,
        }
    }
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
        assert_eq!(report.foreign_keys_pragmas_in_transactions.len(), 1);
        assert_eq!(report.foreign_keys_pragmas_in_transactions[0].line, 4);
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

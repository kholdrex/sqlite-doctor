//! A small, dependency-free lexical scanner for SQLite migration text.
//!
//! This is deliberately not a general SQL parser. It splits a single SQLite
//! input, classifies a narrow statement vocabulary, tracks lexical transaction
//! state, and anchors structural table-rebuild patterns. It cannot observe
//! framework-injected transactions, dynamic SQL, runtime-selected pragmas, or
//! state carried across inputs. A rebuild match is an anchor, not proof that a
//! sequence is unsafe.

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildSequence {
    pub kind: RebuildKind,
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
    pub(crate) replacement_statement_index: usize,
    pub(crate) temporary_table_parts: Vec<String>,
    pub(crate) original_table_parts: Vec<String>,
    /// Columns of the table that takes the original name, when its CREATE
    /// TABLE lists them.
    pub(crate) replacement_columns: Option<Vec<String>>,
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
    pub(crate) name_parts: Vec<String>,
    pub(crate) table_parts: Vec<String>,
    /// Indexed columns when every key is a plain column; `None` for triggers
    /// and expression indexes.
    pub(crate) columns: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedSchemaObject {
    pub kind: SchemaObjectKind,
    pub name: String,
    /// One-based source line containing the DROP statement.
    pub line: usize,
    pub(crate) statement_index: usize,
    pub(crate) name_parts: Vec<String>,
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
}

/// A `DROP TABLE`, and the statement that undid it if a `ROLLBACK` or
/// `ROLLBACK TO` an earlier savepoint did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DroppedTable {
    pub(crate) statement_index: usize,
    /// Schema and table the drop removes. An unqualified name already used by
    /// a temp table counts as temp, which keeps a main child's reference.
    pub(crate) table: (String, String),
    pub(crate) undone_at: Option<usize>,
}

/// An `ALTER TABLE ... RENAME TO`, resolved like a drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenamedTable {
    pub(crate) statement_index: usize,
    pub(crate) from: (String, String),
    pub(crate) to: (String, String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub foreign_keys_pragmas_in_transactions: Vec<ForeignKeysPragmaInTransaction>,
    pub rebuilds: Vec<RebuildSequence>,
    pub delete_action_references: Vec<DeleteActionReference>,
    pub schema_objects: Vec<SchemaObject>,
    pub dropped_schema_objects: Vec<DroppedSchemaObject>,
    pub(crate) dropped_tables: Vec<DroppedTable>,
    pub(crate) renamed_tables: Vec<RenamedTable>,
}

/// Scans one SQLite SQL input for transaction-scoped foreign-key pragmas,
/// structural create/copy/drop/rename table-rebuild patterns, and explicit
/// index/trigger lifecycle evidence.
///
/// The result reflects only lexical SQL present in `sql`; a rebuild match is a
/// pattern anchor, not proof that the sequence is unsafe at runtime.
pub fn scan_sql(sql: &str) -> ScanReport {
    let statements = statements(sql);
    let mut report = ScanReport::default();
    let mut transaction = TransactionState::default();
    let mut foreign_keys_enabled = None;
    let mut foreign_key_states = Vec::with_capacity(statements.len());
    let mut drops = DropJournal::default();
    let mut temps = TempTables::default();
    let mut temp_leaves = std::collections::HashSet::new();
    let mut unreadable_temp = false;

    for (statement_index, statement) in statements.iter().enumerate() {
        if let Some(name) = exact_dropped_table(statement) {
            // Rollbacks can bring a temp shadow back without the scanner
            // seeing it, so an unqualified name once used by a temp table
            // never resolves to main here.
            let table = if name.parts.len() == 1
                && (unreadable_temp || temp_leaves.contains(name.leaf()))
            {
                ("temp".to_string(), name.leaf().to_string())
            } else {
                owned(temps.resolve(&name))
            };
            let drop = DroppedTable {
                statement_index,
                table,
                undone_at: None,
            };
            drops.record_drop(&mut report.dropped_tables, drop);
        } else if let Some((from, to)) = renamed_table(statement) {
            let from = owned(temps.resolve(&from));
            let to = (from.0.clone(), to.leaf().to_string());
            report.renamed_tables.push(RenamedTable {
                statement_index,
                from,
                to,
            });
        } else {
            drops.observe(statement, statement_index, &mut report.dropped_tables);
        }
        temps.apply(statement);
        temp_leaves.extend(temps.names.iter().cloned());
        let tokens = &statement.tokens;
        if token_is(tokens, 0, "CREATE")
            && (token_is(tokens, 1, "TEMP") || token_is(tokens, 1, "TEMPORARY"))
            && created_table(statement).is_none()
        {
            unreadable_temp = true;
        }
        if let Some(schema_object) = created_schema_object(statement, statement_index) {
            report.schema_objects.push(schema_object);
        }
        if let Some(schema_object) = dropped_schema_object(statement, statement_index) {
            report.dropped_schema_objects.push(schema_object);
        }
        report
            .delete_action_references
            .extend(delete_action_references(statement, statement_index));
        if let Some(enabled) = foreign_keys_setting(statement) {
            if transaction.in_transaction {
                report
                    .foreign_keys_pragmas_in_transactions
                    .push(ForeignKeysPragmaInTransaction {
                        line: statement.line,
                        requested_enabled: enabled,
                    });
            } else {
                foreign_keys_enabled = Some(enabled);
            }
        }
        foreign_key_states.push(foreign_keys_enabled);
        transaction.apply(statement);
    }

    let mut candidates = Vec::<RebuildCandidate>::new();
    let mut temp_tables = TempTables::default();
    for (statement_index, (statement, foreign_keys_enabled)) in
        statements.iter().zip(foreign_key_states).enumerate()
    {
        temp_tables.apply(statement);
        if let Some((created, _)) = created_table(statement) {
            // Recreating the dropped original under its own name is the
            // replacement step of the copy-out/copy-back shape Rails emits.
            // Every holding table copied from it stays a candidate; the
            // copy-back decides which one was the rebuild.
            let mut recreated = false;
            for candidate in candidates.iter_mut().filter(|candidate| {
                candidate.stage == CandidateStage::Dropped
                    && candidate
                        .original_table
                        .as_ref()
                        .is_some_and(|name| name.matches(&created))
            }) {
                candidate.replacement_line = statement.line;
                candidate.replacement_statement_index = statement_index;
                candidate.replacement_columns = table_columns(statement);
                candidate.stage = CandidateStage::Recreated;
                recreated = true;
            }
            if recreated {
                continue;
            }

            // A replacement table with the same identity supersedes an older,
            // incomplete candidate. Later steps must not finish the old create.
            candidates.retain(|candidate| !candidate.temporary_table.matches(&created));
            candidates.push(RebuildCandidate::new(
                created,
                statement.line,
                table_columns(statement),
            ));
            continue;
        }

        if let Some((target, source)) = copied_table(statement)
            && let Some(index) = candidates.iter().rposition(|candidate| {
                candidate.stage == CandidateStage::Recreated
                    && candidate.temporary_table.matches(&source)
                    && candidate
                        .original_table
                        .as_ref()
                        .is_some_and(|name| name.matches(&target))
            })
        {
            let candidate = candidates.remove(index);
            forget_siblings(&mut candidates, candidate.drop_statement_index);
            report
                .rebuilds
                .push(candidate.finish(RebuildKind::Recreate));
            continue;
        }

        if let Some((temporary_table, original_table)) = copied_table(statement) {
            // `t` and `main.t` are one table unless a temp `t` shadows it.
            if temp_tables.resolve(&temporary_table) == temp_tables.resolve(&original_table) {
                continue;
            }
            if let Some(candidate) = candidates.iter_mut().rev().find(|candidate| {
                candidate.stage == CandidateStage::Created
                    && candidate.temporary_table.matches(&temporary_table)
            }) {
                candidate.original_table = Some(original_table);
                candidate.copy_line = statement.line;
                candidate.stage = CandidateStage::Copied;
            }
            continue;
        }

        if let Some(original_table) = dropped_table(statement) {
            // A backup copy of the same table must not consume the drop that
            // belongs to the real rebuild, so every copy advances; the rename
            // or copy-back completes only the one that replaces the original.
            for candidate in candidates.iter_mut().filter(|candidate| {
                candidate.stage == CandidateStage::Copied
                    && candidate
                        .original_table
                        .as_ref()
                        .is_some_and(|name| name.matches(&original_table))
            }) {
                candidate.drop_line = statement.line;
                candidate.drop_statement_index = statement_index;
                candidate.foreign_keys_enabled_at_drop = foreign_keys_enabled;
                candidate.stage = CandidateStage::Dropped;
            }
            continue;
        }

        if let Some((temporary_table, original_table)) = renamed_table(statement)
            && let Some(index) = candidates.iter().rposition(|candidate| {
                candidate.stage == CandidateStage::Dropped
                    && candidate.temporary_table.matches(&temporary_table)
                    && candidate
                        .original_table
                        .as_ref()
                        .is_some_and(|name| name.matches(&original_table))
            })
        {
            let mut candidate = candidates.remove(index);
            forget_siblings(&mut candidates, candidate.drop_statement_index);
            candidate.replacement_line = statement.line;
            candidate.replacement_statement_index = statement_index;
            report.rebuilds.push(candidate.finish(RebuildKind::Rename));
        }
    }

    report
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Word(String),
    QuotedIdentifier(String),
    StringLiteral,
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
            '\'' => {
                let start_line = line;
                skip_quoted(&mut chars, '\'', &mut line);
                result.push(Token {
                    kind: TokenKind::StringLiteral,
                    line: start_line,
                });
            }
            '"' | '`' => {
                let start_line = line;
                let identifier = read_quoted_identifier(&mut chars, ch, &mut line);
                result.push(Token {
                    kind: TokenKind::QuotedIdentifier(identifier),
                    line: start_line,
                });
            }
            '[' => {
                let start_line = line;
                let identifier = read_quoted_identifier(&mut chars, ']', &mut line);
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

fn is_word_character(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '$')
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

fn identifier(token: &Token) -> Option<String> {
    match &token.kind {
        TokenKind::Word(value) | TokenKind::QuotedIdentifier(value) => {
            Some(value.to_ascii_lowercase())
        }
        TokenKind::StringLiteral | TokenKind::Symbol(_) => None,
    }
}

/// Follows which `DROP TABLE`s a later `ROLLBACK` undoes. A drop stays in
/// effect unless the whole transaction rolls back, or the input rolls back
/// to a savepoint that was set before the drop. A rollback it cannot read
/// undoes every open drop, and an unreadable savepoint name stops RELEASE or
/// ROLLBACK TO from resolving past it, so unclear input keeps the reference.
#[derive(Debug, Default)]
struct DropJournal {
    /// Indexes into `dropped_tables` of drops made in the open transaction.
    open: Vec<usize>,
    /// Open savepoints (name, when readable) and the statement that set each.
    savepoints: Vec<(Option<Name>, usize)>,
    in_transaction: bool,
    explicit: bool,
}

impl DropJournal {
    fn record_drop(&mut self, dropped: &mut Vec<DroppedTable>, drop: DroppedTable) {
        dropped.push(drop);
        if self.in_transaction {
            self.open.push(dropped.len() - 1);
        }
    }

    fn observe(&mut self, statement: &Statement, index: usize, dropped: &mut [DroppedTable]) {
        let tokens = &statement.tokens;
        let ends_at = |at: usize| at == tokens.len();
        let skip = |at: usize, word: &str| at + usize::from(token_is(tokens, at, word));
        if token_is(tokens, 0, "BEGIN") {
            // Any BEGIN opens the journal, even one it cannot fully read: if
            // SQLite rejected it, a later ROLLBACK only keeps a reference.
            if !self.in_transaction {
                self.in_transaction = true;
                self.explicit = true;
            }
        } else if token_is(tokens, 0, "SAVEPOINT") {
            let name = parse_name(tokens, 1).filter(|(_, end)| ends_at(*end));
            if name.is_some() || tokens.len() == 2 {
                if !self.in_transaction {
                    self.in_transaction = true;
                    self.explicit = false;
                }
                self.savepoints.push((name.map(|(name, _)| name), index));
            }
        } else if token_is(tokens, 0, "COMMIT") || token_is(tokens, 0, "END") {
            if ends_at(skip(1, "TRANSACTION")) {
                self.close();
            }
        } else if token_is(tokens, 0, "ROLLBACK") {
            let at = skip(1, "TRANSACTION");
            if ends_at(at) {
                self.undo_open(dropped, index, None);
                self.close();
            } else if token_is(tokens, at, "TO") {
                let target = parse_name(tokens, skip(at + 1, "SAVEPOINT"))
                    .filter(|(_, end)| ends_at(*end))
                    .and_then(|(name, _)| {
                        self.savepoints.iter().rposition(|(saved, _)| {
                            saved.as_ref().is_some_and(|saved| saved.matches(&name))
                        })
                    });
                match target.filter(|&position| self.unambiguous_from(position)) {
                    Some(position) => {
                        let since = self.savepoints[position].1;
                        self.savepoints.truncate(position + 1);
                        self.undo_open(dropped, index, Some(since));
                    }
                    None => self.undo_open(dropped, index, None),
                }
            } else {
                self.undo_open(dropped, index, None);
            }
        } else if token_is(tokens, 0, "RELEASE") {
            let name = parse_name(tokens, skip(1, "SAVEPOINT")).filter(|(_, end)| ends_at(*end));
            let position = name.and_then(|(name, _)| {
                self.savepoints.iter().rposition(|(saved, _)| {
                    saved.as_ref().is_some_and(|saved| saved.matches(&name))
                })
            });
            if position.is_some_and(|position| !self.unambiguous_from(position)) {
                self.undo_open(dropped, index, None);
            } else if let Some(position) = position {
                self.savepoints.truncate(position);
                if self.savepoints.is_empty() && !self.explicit {
                    self.close();
                }
            }
        }
    }

    /// No savepoint with an unreadable name sits above `position`, so a
    /// RELEASE or ROLLBACK TO naming `position` cannot mean a nearer one.
    fn unambiguous_from(&self, position: usize) -> bool {
        self.savepoints[position + 1..]
            .iter()
            .all(|(name, _)| name.is_some())
    }

    fn undo_open(&mut self, dropped: &mut [DroppedTable], index: usize, since: Option<usize>) {
        self.open.retain(|&drop| {
            let undone = since.is_none_or(|since| dropped[drop].statement_index > since);
            if undone {
                dropped[drop].undone_at = Some(index);
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

#[derive(Debug, Default)]
struct TransactionState {
    in_transaction: bool,
    explicit: bool,
    savepoints: Vec<Name>,
}

impl TransactionState {
    fn apply(&mut self, statement: &Statement) {
        let tokens = &statement.tokens;
        if token_is(tokens, 0, "BEGIN") {
            if !self.in_transaction {
                self.in_transaction = true;
                self.explicit = true;
            }
        } else if token_is(tokens, 0, "SAVEPOINT") {
            if let Some((name, _)) = parse_name(tokens, 1) {
                if !self.in_transaction {
                    self.in_transaction = true;
                    self.explicit = false;
                }
                self.savepoints.push(name);
            }
        } else if token_is(tokens, 0, "COMMIT") || token_is(tokens, 0, "END") {
            self.clear();
        } else if token_is(tokens, 0, "ROLLBACK") {
            if token_is(tokens, 1, "TO") {
                let name_index = if token_is(tokens, 2, "SAVEPOINT") {
                    3
                } else {
                    2
                };
                if let Some((name, _)) = parse_name(tokens, name_index)
                    && let Some(index) = self
                        .savepoints
                        .iter()
                        .rposition(|candidate| candidate.matches(&name))
                {
                    self.savepoints.truncate(index + 1);
                }
            } else {
                self.clear();
            }
        } else if token_is(tokens, 0, "RELEASE") {
            let name_index = if token_is(tokens, 1, "SAVEPOINT") {
                2
            } else {
                1
            };
            if let Some((name, _)) = parse_name(tokens, name_index)
                && let Some(index) = self
                    .savepoints
                    .iter()
                    .rposition(|candidate| candidate.matches(&name))
            {
                self.savepoints.truncate(index);
                if self.savepoints.is_empty() && !self.explicit {
                    self.clear();
                }
            }
        }
    }

    fn clear(&mut self) {
        self.in_transaction = false;
        self.explicit = false;
        self.savepoints.clear();
    }
}

fn foreign_keys_setting(statement: &Statement) -> Option<bool> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "PRAGMA") {
        return None;
    }
    let (name, index) = parse_name(tokens, 1)?;
    if name.leaf() != "foreign_keys" || name.parts.len() > 2 {
        return None;
    }
    let has_equals_assignment = symbol_is(tokens, index, '=') && tokens.len() == index + 2;
    let has_parenthesized_assignment = symbol_is(tokens, index, '(')
        && symbol_is(tokens, index + 2, ')')
        && tokens.len() == index + 3;
    if !has_equals_assignment && !has_parenthesized_assignment {
        return None;
    }
    let value = tokens.get(index + 1)?;
    if keyword(value, "OFF")
        || keyword(value, "FALSE")
        || keyword(value, "NO")
        || numeric_pragma_value(value) == Some(false)
    {
        Some(false)
    } else if keyword(value, "ON")
        || keyword(value, "TRUE")
        || keyword(value, "YES")
        || numeric_pragma_value(value) == Some(true)
    {
        Some(true)
    } else {
        None
    }
}

fn numeric_pragma_value(token: &Token) -> Option<bool> {
    match &token.kind {
        TokenKind::Word(word) => word.parse::<i64>().ok().map(|value| value != 0),
        _ => None,
    }
}

/// Temporary tables that exist at a point in the input, so an unqualified name
/// can be resolved the way SQLite does: `temp` first, then `main`.
#[derive(Default)]
struct TempTables {
    names: Vec<String>,
}

impl TempTables {
    /// Updates the set for the statement. Called before the statement's own
    /// copy is resolved; only CREATE, DROP and RENAME change it.
    fn apply(&mut self, statement: &Statement) {
        let tokens = &statement.tokens;
        if let Some((name, _)) = created_table(statement) {
            let temporary = token_is(tokens, 1, "TEMP")
                || token_is(tokens, 1, "TEMPORARY")
                || name.parts.first().is_some_and(|schema| schema == "temp");
            if temporary && !self.names.iter().any(|leaf| leaf == name.leaf()) {
                self.names.push(name.leaf().to_string());
            }
        } else if let Some(name) = dropped_table(statement)
            && self.resolve(&name).0 == "temp"
        {
            self.names.retain(|leaf| leaf != name.leaf());
        } else if let Some((from, to)) = renamed_table(statement)
            && self.resolve(&from).0 == "temp"
        {
            self.names.retain(|leaf| leaf != from.leaf());
            self.names.push(to.leaf().to_string());
        }
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
    // A declaration whose name cannot be read (a single-quoted name) makes the
    // whole list unknown rather than silently shorter.
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

fn created_schema_object(statement: &Statement, statement_index: usize) -> Option<SchemaObject> {
    created_index(statement, statement_index)
        .or_else(|| created_trigger(statement, statement_index))
}

fn created_index(statement: &Statement, statement_index: usize) -> Option<SchemaObject> {
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
    if token_is(tokens, index, "IF")
        && token_is(tokens, index + 1, "NOT")
        && token_is(tokens, index + 2, "EXISTS")
    {
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
        name_parts: name.parts,
        table_parts: table.parts,
        columns,
    })
}

fn created_trigger(statement: &Statement, statement_index: usize) -> Option<SchemaObject> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "CREATE") {
        return None;
    }
    let mut index = 1;
    if token_is(tokens, index, "TEMP") || token_is(tokens, index, "TEMPORARY") {
        index += 1;
    }
    if !token_is(tokens, index, "TRIGGER") {
        return None;
    }
    index += 1;
    if token_is(tokens, index, "IF")
        && token_is(tokens, index + 1, "NOT")
        && token_is(tokens, index + 2, "EXISTS")
    {
        index += 3;
    }
    let (name, index) = parse_name(tokens, index)?;
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
        name_parts: name.parts,
        table_parts: table.parts,
        columns: None,
    })
}

fn dropped_schema_object(
    statement: &Statement,
    statement_index: usize,
) -> Option<DroppedSchemaObject> {
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
    if end != tokens.len() {
        return None;
    }
    Some(DroppedSchemaObject {
        kind,
        name: name.canonical(),
        line: statement.line,
        statement_index,
        name_parts: name.parts,
    })
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

    table_definition_ranges(tokens, opening_parenthesis)
        .into_iter()
        .filter_map(|(start, end)| {
            let reference_index =
                (start..end).find(|index| token_is(tokens, *index, "REFERENCES"))?;
            let (referenced_table, mut index) = parse_name(tokens, reference_index + 1)?;
            if symbol_is(tokens, index, '(') {
                index = after_balanced_parentheses(tokens, index)?;
            }
            let action = (index..end).find_map(|index| {
                if !token_is(tokens, index, "ON") || !token_is(tokens, index + 1, "DELETE") {
                    return None;
                }
                if token_is(tokens, index + 2, "CASCADE") {
                    Some(DeleteAction::Cascade)
                } else if token_is(tokens, index + 2, "SET") && token_is(tokens, index + 3, "NULL")
                {
                    Some(DeleteAction::SetNull)
                } else if token_is(tokens, index + 2, "SET")
                    && token_is(tokens, index + 3, "DEFAULT")
                {
                    Some(DeleteAction::SetDefault)
                } else {
                    None
                }
            })?;
            (!dependent_table.matches(&referenced_table)).then(|| DeleteActionReference {
                dependent_table: dependent_table.canonical(),
                referenced_table: referenced_table.canonical(),
                action,
                line: tokens[reference_index].line,
                statement_index,
                dependent_identity: created_identity(statement, &dependent_table),
                referenced_table_parts: referenced_table.parts,
            })
        })
        .collect()
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

fn copied_table(statement: &Statement) -> Option<(Name, Name)> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "INSERT") {
        return None;
    }
    let mut index = 1;
    if token_is(tokens, index, "OR")
        && (token_is(tokens, index + 1, "IGNORE") || token_is(tokens, index + 1, "REPLACE"))
    {
        index += 2;
    }
    if !token_is(tokens, index, "INTO") {
        return None;
    }
    let (target, mut index) = parse_name(tokens, index + 1)?;
    if symbol_is(tokens, index, '(') {
        index = after_balanced_parentheses(tokens, index)?;
    }
    if !token_is(tokens, index, "SELECT") {
        return None;
    }
    let from_index = tokens
        .iter()
        .enumerate()
        .skip(index + 1)
        .find(|(_, token)| keyword(token, "FROM"))?
        .0;
    let (source, _) = parse_name(tokens, from_index + 1)?;
    Some((target, source))
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

fn dropped_table(statement: &Statement) -> Option<Name> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "DROP") || !token_is(tokens, 1, "TABLE") {
        return None;
    }
    let index = if token_is(tokens, 2, "IF") && token_is(tokens, 3, "EXISTS") {
        4
    } else {
        2
    };
    parse_name(tokens, index).map(|(name, _)| name)
}

/// A malformed drop fails in SQLite, so it must not hide a surviving child.
fn exact_dropped_table(statement: &Statement) -> Option<Name> {
    let tokens = &statement.tokens;
    if !token_is(tokens, 0, "DROP") || !token_is(tokens, 1, "TABLE") {
        return None;
    }
    let index = if token_is(tokens, 2, "IF") && token_is(tokens, 3, "EXISTS") {
        4
    } else {
        2
    };
    let (name, end) = parse_name(tokens, index)?;
    (end == tokens.len() && name.parts.len() <= 2).then_some(name)
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

/// Other copies taken before the same DROP describe the same destroyed table;
/// once one of them completes the rebuild they must not report it again.
fn forget_siblings(candidates: &mut Vec<RebuildCandidate>, drop_statement_index: usize) {
    candidates.retain(|candidate| {
        !matches!(
            candidate.stage,
            CandidateStage::Dropped | CandidateStage::Recreated
        ) || candidate.drop_statement_index != drop_statement_index
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateStage {
    Created,
    Copied,
    Dropped,
    Recreated,
}

#[derive(Debug)]
struct RebuildCandidate {
    temporary_table: Name,
    original_table: Option<Name>,
    create_line: usize,
    copy_line: usize,
    drop_line: usize,
    replacement_line: usize,
    drop_statement_index: usize,
    replacement_statement_index: usize,
    foreign_keys_enabled_at_drop: Option<bool>,
    replacement_columns: Option<Vec<String>>,
    stage: CandidateStage,
}

impl RebuildCandidate {
    fn new(temporary_table: Name, create_line: usize, columns: Option<Vec<String>>) -> Self {
        Self {
            temporary_table,
            replacement_columns: columns,
            original_table: None,
            create_line,
            copy_line: 0,
            drop_line: 0,
            replacement_line: 0,
            drop_statement_index: 0,
            replacement_statement_index: 0,
            foreign_keys_enabled_at_drop: None,
            stage: CandidateStage::Created,
        }
    }

    fn finish(self, kind: RebuildKind) -> RebuildSequence {
        let original = self
            .original_table
            .expect("a dropped candidate has an original table");
        RebuildSequence {
            kind,
            temporary_table: self.temporary_table.canonical(),
            original_table: original.canonical(),
            create_line: self.create_line,
            copy_line: self.copy_line,
            drop_line: self.drop_line,
            replacement_line: self.replacement_line,
            foreign_keys_enabled_at_drop: self.foreign_keys_enabled_at_drop,
            drop_statement_index: self.drop_statement_index,
            replacement_statement_index: self.replacement_statement_index,
            temporary_table_parts: self.temporary_table.parts,
            original_table_parts: original.parts,
            replacement_columns: self.replacement_columns,
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

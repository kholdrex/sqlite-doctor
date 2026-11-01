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
    pub(crate) referenced_table_parts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub foreign_keys_pragmas_in_transactions: Vec<ForeignKeysPragmaInTransaction>,
    pub rebuilds: Vec<RebuildSequence>,
    pub delete_action_references: Vec<DeleteActionReference>,
    pub schema_objects: Vec<SchemaObject>,
    pub dropped_schema_objects: Vec<DroppedSchemaObject>,
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

    for (statement_index, statement) in statements.iter().enumerate() {
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
    for (statement_index, (statement, foreign_keys_enabled)) in
        statements.iter().zip(foreign_key_states).enumerate()
    {
        if let Some((created, _)) = created_table(statement) {
            // Recreating the dropped original under its own name is the
            // replacement step of the copy-out/copy-back shape Rails emits.
            if let Some(candidate) = candidates.iter_mut().rev().find(|candidate| {
                candidate.stage == CandidateStage::Dropped
                    && candidate
                        .original_table
                        .as_ref()
                        .is_some_and(|name| name.matches(&created))
            }) {
                candidate.replacement_line = statement.line;
                candidate.replacement_statement_index = statement_index;
                candidate.stage = CandidateStage::Recreated;
                continue;
            }

            // A replacement table with the same identity supersedes an older,
            // incomplete candidate. Later steps must not finish the old create.
            candidates.retain(|candidate| !candidate.temporary_table.matches(&created));
            candidates.push(RebuildCandidate::new(created, statement.line));
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
            report
                .rebuilds
                .push(candidate.finish(RebuildKind::Recreate));
            continue;
        }

        if let Some((temporary_table, original_table)) = copied_table(statement) {
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
            if let Some(candidate) = candidates.iter_mut().rev().find(|candidate| {
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
    let (table, _) = parse_name(tokens, index + 1)?;
    Some(SchemaObject {
        kind: SchemaObjectKind::Index,
        name: name.canonical(),
        table: table.canonical(),
        line: statement.line,
        statement_index,
        name_parts: name.parts,
        table_parts: table.parts,
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
    stage: CandidateStage,
}

impl RebuildCandidate {
    fn new(temporary_table: Name, create_line: usize) -> Self {
        Self {
            temporary_table,
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

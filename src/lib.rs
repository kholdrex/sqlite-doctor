use std::fmt;

mod scanner;

use scanner::TableChangeKind;

pub use scanner::{
    DeleteAction, DeleteActionReference, DroppedSchemaObject, ForeignKeysPragmaInTransaction,
    RebuildKind, RebuildSequence, ScanReport, SchemaObject, SchemaObjectKind, scan_sql,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HazardId {
    DeleteActionOnRebuild,
    ForeignKeysPragmaInsideTransaction,
    MissingSchemaObjectRestoration,
}

impl HazardId {
    pub const ALL: [Self; 3] = [
        Self::DeleteActionOnRebuild,
        Self::ForeignKeysPragmaInsideTransaction,
        Self::MissingSchemaObjectRestoration,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::DeleteActionOnRebuild => "SQLD001",
            Self::ForeignKeysPragmaInsideTransaction => "SQLD002",
            Self::MissingSchemaObjectRestoration => "SQLD003",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::DeleteActionOnRebuild => {
                "table rebuild may fire ON DELETE actions on dependent rows"
            }
            Self::ForeignKeysPragmaInsideTransaction => {
                "changing PRAGMA foreign_keys inside a transaction has no effect"
            }
            Self::MissingSchemaObjectRestoration => "table rebuild may drop an index or trigger",
        }
    }
}

impl fmt::Display for HazardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.title())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub hazard: HazardId,
    pub line: usize,
    pub related_line: Option<usize>,
    pub detail: String,
    pub remediation: String,
}

impl Finding {
    pub fn new(
        hazard: HazardId,
        line: usize,
        detail: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            hazard,
            line,
            related_line: None,
            detail: detail.into(),
            remediation: remediation.into(),
        }
    }

    pub fn with_related_line(mut self, line: usize) -> Self {
        self.related_line = Some(line);
        self
    }
}

/// `SQLD001` requires a complete rebuild and a matching `ON DELETE CASCADE`,
/// `SET NULL` or `SET DEFAULT` reference in the input.
pub fn diagnose(report: &ScanReport) -> Vec<Finding> {
    let mut findings = Vec::new();

    for rebuild in &report.rebuilds {
        if rebuild.foreign_keys_enabled_at_drop == Some(false) {
            continue;
        }
        for reference in &report.delete_action_references {
            let Some((dependent_table, referenced_tables)) =
                reference_at(report, reference, rebuild.drop_statement_index)
            else {
                continue;
            };
            // A reference points at a table in the dependent's own schema.
            let same_schema = rebuild
                .original_schema
                .as_ref()
                .is_none_or(|schema| *schema == reference.dependent_identity.0);
            if !same_schema
                || !referenced_tables
                    .iter()
                    .any(|table| name_parts_match(table, &rebuild.original_table_parts))
            {
                continue;
            }
            findings.push(
                Finding::new(
                    HazardId::DeleteActionOnRebuild,
                    rebuild.drop_line,
                    format!(
                        "Dependent table `{}` references rebuilt table `{}` with {} on line {}; dropping the original table {} while foreign-key enforcement is active.",
                        dependent_table,
                        rebuild.original_table,
                        reference.action.sql(),
                        reference.line,
                        match reference.action {
                            DeleteAction::Cascade => "can delete dependent rows",
                            DeleteAction::SetNull => "can set dependent foreign keys to NULL",
                            DeleteAction::SetDefault =>
                                "can reset dependent foreign keys to their default",
                        }
                    ),
                    "Disable foreign-key enforcement before starting the transaction, rebuild the table, and run PRAGMA foreign_key_check before COMMIT; roll back if it reports violations. Restore enforcement after the transaction ends.",
                )
                .with_related_line(reference.line),
            );
        }
    }

    for rebuild in &report.rebuilds {
        let at = rebuild.drop_statement_index;
        for schema_object in &report.schema_objects {
            let tables = tables_at(report, schema_object, at).names;
            if !exists_at(report, schema_object, at, false)
                || !in_schema_of(rebuild, &schema_object.table_schema)
                || !tables
                    .iter()
                    .any(|table| name_parts_match(table, &rebuild.original_table_parts))
            {
                continue;
            }
            let same_object = |other: &SchemaObject| {
                other.kind == schema_object.kind && other.identity == schema_object.identity
            };

            // Skipped only when the earlier object is certainly still there.
            if !created(report, schema_object, true) {
                continue;
            }

            let restored = report.schema_objects.iter().any(|candidate| {
                let restored_before_rename = rebuild.kind == RebuildKind::Rename
                    && candidate.statement_index > at
                    && candidate.statement_index < rebuild.replacement_statement_index
                    && name_parts_match(&candidate.table_parts, &rebuild.temporary_table_parts);
                // Only while the replacement table exists: a later rebuild
                // of the same table starts from a table without the object.
                let restored_after_rename = candidate.statement_index
                    > rebuild.replacement_statement_index
                    && replacement_names_at(report, rebuild, candidate.statement_index)
                        .iter()
                        .any(|table| {
                            name_parts_match(
                                effective_table_parts(candidate, &report.rebuilds),
                                table,
                            )
                        })
                    && !dropped_between(report, rebuild, candidate.statement_index);
                // A rollback that takes the restoration with it leaves the
                // object missing, unless it also undoes the drop.
                let kept = candidate
                    .undone_at
                    .is_none_or(|undone| drop_undone_at(report, rebuild) == Some(undone));
                same_object(candidate)
                    && kept
                    && !candidate.table_shadowed
                    && (restored_before_rename || restored_after_rename)
                    && created(report, candidate, false)
            });
            if restored {
                continue;
            }

            // Skip a plain-column index if the replacement omits one of its
            // keys under every name the key has had.
            let column_removed = schema_object.columns.as_ref().is_some_and(|columns| {
                replacement_columns(report, rebuild).is_some_and(|kept| {
                    columns.iter().any(|column| {
                        !column_names(report, schema_object, column, at)
                            .iter()
                            .any(|name| kept.contains(name))
                    })
                })
            });
            if column_removed {
                continue;
            }

            findings.push(
                Finding::new(
                    HazardId::MissingSchemaObjectRestoration,
                    rebuild.drop_line,
                    format!(
                        "Dropping `{}` removes {} `{}` from line {}. The input does not recreate it.",
                        rebuild.original_table,
                        schema_object.kind.label(),
                        schema_object.name,
                        schema_object.line
                    ),
                    format!(
                        "Recreate {} `{}` on the replacement table after dropping the original, or remove it explicitly before the rebuild if the omission is intentional.",
                        schema_object.kind.label(),
                        schema_object.name
                    ),
                )
                .with_related_line(schema_object.line),
            );
        }
    }

    for pragma in &report.foreign_keys_pragmas_in_transactions {
        findings.push(Finding::new(
            HazardId::ForeignKeysPragmaInsideTransaction,
            pragma.line,
            format!(
                "SQLite ignores attempts to turn PRAGMA foreign_keys {} while a transaction is active.",
                if pragma.requested_enabled { "ON" } else { "OFF" }
            ),
            if pragma.requested_enabled {
                "Move PRAGMA foreign_keys=ON outside the transaction: before BEGIN or after COMMIT."
            } else {
                "Set PRAGMA foreign_keys=OFF before BEGIN, run PRAGMA foreign_key_check before COMMIT, and restore the previous setting after the transaction ends."
            },
        ));
    }

    findings.sort_by_key(|finding| finding.line);
    findings
}

/// Whether a change made by statement `statement_index` is in effect when
/// statement `at` runs.
fn holds_at(statement_index: usize, undone_at: Option<usize>, at: usize) -> bool {
    statement_index < at && undone_at.is_none_or(|undone| undone > at)
}

/// Whether `rebuild` dropped a table in `schema`, as far as the drop tells.
fn in_schema_of(rebuild: &RebuildSequence, schema: &str) -> bool {
    rebuild
        .original_schema
        .as_ref()
        .is_none_or(|original| original == schema)
}

/// The dependent's name and the names its parent may have when statement `at`
/// runs. A renamed dependent keeps its references, and SQLite rewrites a
/// reference when its parent is renamed, unless `legacy_alter_table` is on
/// and foreign keys are off. `None` if the dependent was dropped, lost the
/// referencing column, or its creation was rolled back by then.
fn reference_at(
    report: &ScanReport,
    reference: &DeleteActionReference,
    at: usize,
) -> Option<(String, Vec<Vec<String>>)> {
    if !holds_at(reference.statement_index, reference.undone_at, at) {
        return None;
    }
    let (schema, declared) = &reference.dependent_identity;
    // The dependent's names and the statement that gave it each.
    let mut names = vec![(reference.statement_index, declared.clone())];
    // After a rename that may have hit another table of the dependent's
    // name, a drop cannot be matched to the dependent, so the reference
    // stays.
    let mut dependent_known = true;
    let mut referenced = vec![reference.referenced_table_parts.clone()];

    for change in &report.table_changes {
        if change.statement_index <= reference.statement_index
            || !holds_at(change.statement_index, change.undone_at, at)
        {
            continue;
        }
        let (changed_schema, table) = &change.table;
        let dependent = &names.last().expect("the declared name").1;
        match &change.kind {
            TableChangeKind::Dropped => {
                if dependent_known && changed_schema == schema && table == dependent {
                    return None;
                }
            }
            TableChangeKind::RenamedTo {
                name,
                rewrites_references,
                unqualified,
                shadowed,
            } => {
                let unsure = *shadowed || (*unqualified && changed_schema != schema);
                let renames_parent = |parent: &Vec<String>| parent.last() == Some(table);
                if unsure || rewrites_references.is_none() {
                    if referenced.iter().any(renames_parent) {
                        referenced.push(vec![name.clone()]);
                    }
                } else if changed_schema == schema && *rewrites_references == Some(true) {
                    for parent in referenced
                        .iter_mut()
                        .filter(|parent| renames_parent(parent))
                    {
                        *parent = vec![name.clone()];
                    }
                }
                if table == dependent {
                    if unsure {
                        dependent_known = false;
                    } else if changed_schema == schema {
                        names.push((change.statement_index, name.clone()));
                    }
                }
            }
        }
    }

    let column_dropped = report.column_changes.iter().any(|change| {
        let dependent = names
            .iter()
            .rev()
            .find(|(since, _)| *since < change.statement_index)
            .map(|(_, name)| name);
        change.to.is_none()
            && Some(&change.from) == reference.column.as_ref()
            && change.statement_index > reference.statement_index
            && change.statement_index < at
            && change.table_parts.last() == dependent
            && (change.table_parts.len() == 1 || change.table_parts[0] == *schema)
    });
    if column_dropped {
        return None;
    }

    let dependent = &names.last().expect("the declared name").1;
    let dependent_table = if dependent == declared {
        reference.dependent_table.clone()
    } else if schema == "main" {
        dependent.clone()
    } else {
        format!("{schema}.{dependent}")
    };
    Some((dependent_table, referenced))
}

/// The `ROLLBACK` that undid the drop of a rebuild, if one did.
fn drop_undone_at(report: &ScanReport, rebuild: &RebuildSequence) -> Option<usize> {
    report
        .table_changes
        .iter()
        .find(|change| change.statement_index == rebuild.drop_statement_index)
        .and_then(|change| change.undone_at)
}

/// Whether the declaration created the object. IF NOT EXISTS does nothing
/// while an object of that name exists, and SQLite keeps the first definition.
/// Where the scanner cannot tell whether the earlier object is still there,
/// `assume_gone` decides.
fn created(report: &ScanReport, schema_object: &SchemaObject, assume_gone: bool) -> bool {
    !schema_object.if_not_exists
        || !report.schema_objects.iter().any(|earlier| {
            earlier.kind == schema_object.kind
                && earlier.identity == schema_object.identity
                && earlier.statement_index < schema_object.statement_index
                && created(report, earlier, assume_gone)
                && exists_at(report, earlier, schema_object.statement_index, assume_gone)
        })
}

/// Whether an index or trigger is there when statement `at` runs: created and
/// not rolled back, not dropped, not gone with its table, and not followed by
/// a plain CREATE of the same name on the same table, which succeeds only
/// once the object is gone and then stands for it. Where a rename leaves the
/// object's table uncertain, `assume_gone` says whether a drop of any name
/// the table may have ends the object.
fn exists_at(
    report: &ScanReport,
    schema_object: &SchemaObject,
    at: usize,
    assume_gone: bool,
) -> bool {
    let tables_then = |statement: usize| {
        let tables = tables_at(report, schema_object, statement);
        match (tables.certain, assume_gone) {
            (true, _) => vec![tables.current],
            (false, true) => tables.names,
            (false, false) => Vec::new(),
        }
    };
    let created_again = report.schema_objects.iter().any(|later| {
        later.kind == schema_object.kind
            && later.identity == schema_object.identity
            && later.statement_index > schema_object.statement_index
            && !later.if_not_exists
            && holds_at(later.statement_index, later.undone_at, at)
            && tables_then(later.statement_index).iter().any(|table| {
                name_parts_match(table, effective_table_parts(later, &report.rebuilds))
            })
    });
    let dropped = report.dropped_schema_objects.iter().any(|dropped| {
        dropped.kind == schema_object.kind
            && dropped.identity == schema_object.identity
            && dropped.statement_index > schema_object.statement_index
            && holds_at(dropped.statement_index, dropped.undone_at, at)
    });
    let table_dropped = report.table_changes.iter().any(|change| {
        change.kind == TableChangeKind::Dropped
            && change.statement_index > schema_object.statement_index
            && holds_at(change.statement_index, change.undone_at, at)
            && change.table.0 == schema_object.table_schema
            && tables_then(change.statement_index)
                .iter()
                .any(|table| table.last() == Some(&change.table.1))
    });
    let lost_in_rebuild = report.rebuilds.iter().any(|rebuild| {
        rebuild.drop_statement_index > schema_object.statement_index
            && holds_at(
                rebuild.drop_statement_index,
                drop_undone_at(report, rebuild),
                at,
            )
            && in_schema_of(rebuild, &schema_object.table_schema)
            && tables_then(rebuild.drop_statement_index)
                .iter()
                .any(|table| name_parts_match(table, &rebuild.original_table_parts))
    });
    holds_at(schema_object.statement_index, schema_object.undone_at, at)
        && !dropped
        && !table_dropped
        && !lost_in_rebuild
        && !created_again
}

/// The columns of the table that takes the original's place, under every name
/// a `RENAME COLUMN` gave them: on the replacement before a rename swap, or on
/// the recreated table before the rows are copied back.
fn replacement_columns(report: &ScanReport, rebuild: &RebuildSequence) -> Option<Vec<String>> {
    let mut columns = rebuild.replacement_columns.clone()?;
    for change in &report.column_changes {
        let on_replacement = match rebuild.kind {
            RebuildKind::Rename => {
                change.statement_index < rebuild.replacement_statement_index
                    && name_parts_match(&change.table_parts, &rebuild.temporary_table_parts)
            }
            RebuildKind::Recreate => {
                change.statement_index > rebuild.replacement_statement_index
                    && name_parts_match(&change.table_parts, &rebuild.original_table_parts)
            }
        };
        if let Some(to) = &change.to
            && on_replacement
            && columns.contains(&change.from)
        {
            columns.push(to.clone());
        }
    }
    Some(columns)
}

/// Every name an indexed column may have when statement `at` runs, following
/// `ALTER TABLE ... RENAME COLUMN` on the index's table under the name the
/// table has at that statement.
fn column_names(
    report: &ScanReport,
    schema_object: &SchemaObject,
    column: &str,
    at: usize,
) -> Vec<String> {
    let mut names = vec![column.to_string()];
    for change in &report.column_changes {
        if let Some(to) = &change.to
            && change.statement_index > schema_object.statement_index
            && change.statement_index < at
            && names.contains(&change.from)
            && tables_at(report, schema_object, change.statement_index)
                .names
                .iter()
                .any(|table| name_parts_match(table, &change.table_parts))
        {
            names.push(to.clone());
        }
    }
    names
}

/// The names the table that took the original's place may have when statement
/// `at` runs, following renames after the rebuild.
fn replacement_names_at(
    report: &ScanReport,
    rebuild: &RebuildSequence,
    at: usize,
) -> Vec<Vec<String>> {
    let mut names = vec![rebuild.original_table_parts.clone()];
    for change in &report.table_changes {
        let TableChangeKind::RenamedTo { name, shadowed, .. } = &change.kind else {
            continue;
        };
        if change.statement_index <= rebuild.replacement_statement_index
            || !holds_at(change.statement_index, change.undone_at, at)
            || !in_schema_of(rebuild, &change.table.0)
        {
            continue;
        }
        let renamed = |table: &Vec<String>| table.last() == Some(&change.table.1);
        if *shadowed {
            if names.iter().any(renamed) {
                names.push(vec![name.clone()]);
            }
        } else {
            for table in names.iter_mut().filter(|table| renamed(table)) {
                *table = vec![name.clone()];
            }
        }
    }
    names
}

/// Whether the table that took the original's place is dropped again before
/// statement `to`, by a plain drop or by a later rebuild.
fn dropped_between(report: &ScanReport, rebuild: &RebuildSequence, to: usize) -> bool {
    let from = rebuild.replacement_statement_index;
    let dropped = report.table_changes.iter().any(|change| {
        let (schema, name) = &change.table;
        change.kind == TableChangeKind::Dropped
            && change.statement_index > from
            && holds_at(change.statement_index, change.undone_at, to)
            && in_schema_of(rebuild, schema)
            && replacement_names_at(report, rebuild, change.statement_index)
                .iter()
                .any(|table| table.last() == Some(name))
    });
    dropped
        || report.rebuilds.iter().any(|later| {
            later.drop_statement_index > from
                && holds_at(
                    later.drop_statement_index,
                    drop_undone_at(report, later),
                    to,
                )
                && replacement_names_at(report, rebuild, later.drop_statement_index)
                    .iter()
                    .any(|table| name_parts_match(&later.original_table_parts, table))
        })
}

/// Where an index or trigger is when a statement runs.
struct ObjectTables {
    /// Its table, following renames in order.
    current: Vec<String>,
    /// Every name the table may have: a rename that may have hit another
    /// table of that name adds one instead of replacing the old one.
    names: Vec<Vec<String>>,
    /// No such rename applied, so `current` is the table.
    certain: bool,
}

/// The table of an index or trigger when statement `at` runs: the object stays
/// on its table through `ALTER TABLE ... RENAME TO`.
fn tables_at(report: &ScanReport, schema_object: &SchemaObject, at: usize) -> ObjectTables {
    let current = effective_table_parts(schema_object, &report.rebuilds).to_vec();
    let mut tables = ObjectTables {
        names: vec![current.clone()],
        current,
        certain: true,
    };
    for change in &report.table_changes {
        let TableChangeKind::RenamedTo {
            name,
            unqualified,
            shadowed,
            ..
        } = &change.kind
        else {
            continue;
        };
        if change.statement_index <= schema_object.statement_index
            || !holds_at(change.statement_index, change.undone_at, at)
        {
            continue;
        }
        let (schema, table) = &change.table;
        let renamed = |candidate: &Vec<String>| candidate.last() == Some(table);
        if *schema == schema_object.table_schema && !shadowed {
            for candidate in tables.names.iter_mut().filter(|name| renamed(name)) {
                *candidate = vec![name.clone()];
            }
            if renamed(&tables.current) {
                tables.current = vec![name.clone()];
            }
        } else if *unqualified && tables.names.iter().any(renamed) {
            // The rename may have hit a temp table of that name, or the
            // object's table in another schema than the one resolved.
            tables.names.push(vec![name.clone()]);
            tables.certain = false;
        }
    }
    tables
}

fn effective_table_parts<'a>(
    schema_object: &'a SchemaObject,
    rebuilds: &'a [RebuildSequence],
) -> &'a [String] {
    rebuilds
        .iter()
        .rev()
        .find(|rebuild| {
            rebuild.kind == RebuildKind::Rename
                && schema_object.statement_index > rebuild.drop_statement_index
                && schema_object.statement_index < rebuild.replacement_statement_index
                && name_parts_match(&schema_object.table_parts, &rebuild.temporary_table_parts)
        })
        .map_or(&schema_object.table_parts, |rebuild| {
            rebuild.original_table_parts.as_slice()
        })
}

fn name_parts_match(left: &[String], right: &[String]) -> bool {
    left == right
        || ((left.len() == 1 || right.len() == 1)
            && left.last().is_some_and(|left_leaf| {
                right
                    .last()
                    .is_some_and(|right_leaf| left_leaf == right_leaf)
            }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hazard_codes_are_stable() {
        assert_eq!(
            HazardId::ALL.map(HazardId::code),
            ["SQLD001", "SQLD002", "SQLD003"]
        );
    }

    #[test]
    fn display_includes_code_and_title() {
        assert_eq!(
            HazardId::ForeignKeysPragmaInsideTransaction.to_string(),
            "SQLD002: changing PRAGMA foreign_keys inside a transaction has no effect"
        );
    }

    #[test]
    fn a_finding_has_no_related_line_until_one_is_set() {
        let finding = Finding::new(HazardId::DeleteActionOnRebuild, 12, "detail", "help");
        assert_eq!(finding.related_line, None);
        assert_eq!(finding.with_related_line(3).related_line, Some(3));
    }
}

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
            if !referenced_tables
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
            let (_, tables) = tables_at(report, schema_object, at);
            if !exists_at(report, schema_object, at)
                || !tables
                    .iter()
                    .any(|table| name_parts_match(table, &rebuild.original_table_parts))
            {
                continue;
            }
            let same_object = |other: &SchemaObject| {
                other.kind == schema_object.kind && other.identity == schema_object.identity
            };

            if !created(report, schema_object) {
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
                    && name_parts_match(
                        effective_table_parts(candidate, &report.rebuilds),
                        &rebuild.original_table_parts,
                    )
                    && !dropped_between(
                        report,
                        &rebuild.original_table_parts,
                        rebuild.replacement_statement_index,
                        candidate.statement_index,
                    );
                // A rollback that takes the restoration with it leaves the
                // object missing, unless it also undoes the drop.
                let kept = candidate
                    .undone_at
                    .is_none_or(|undone| drop_undone_at(report, rebuild) == Some(undone));
                same_object(candidate)
                    && kept
                    && !candidate.table_shadowed
                    && (restored_before_rename || restored_after_rename)
                    && created(report, candidate)
            });
            if restored {
                continue;
            }

            // Skip a plain-column index if the replacement omits one of its
            // keys under every name the key has had.
            let column_removed = schema_object.columns.as_ref().is_some_and(|columns| {
                rebuild.replacement_columns.as_ref().is_some_and(|kept| {
                    columns.iter().any(|column| {
                        !column_names(report, schema_object, &tables, column, at)
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

/// The dependent's name and the names its parent may have when statement `at`
/// runs: SQLite rewrites a reference when its parent is renamed, and a renamed
/// dependent keeps its references. `None` if the dependent was dropped or its
/// creation rolled back by then.
fn reference_at(
    report: &ScanReport,
    reference: &DeleteActionReference,
    at: usize,
) -> Option<(String, Vec<Vec<String>>)> {
    if !holds_at(reference.statement_index, reference.undone_at, at) {
        return None;
    }
    let identity = &reference.dependent_identity;
    let since_declared = || {
        report.table_changes.iter().filter(|change| {
            change.statement_index > reference.statement_index && change.statement_index < at
        })
    };

    // A rename involving the dependent's name keeps the reference: a later
    // drop may then hit a different table.
    let renamed = since_declared().any(|change| match &change.kind {
        TableChangeKind::RenamedTo { name, .. } => {
            change.table == *identity || (change.table.0 == identity.0 && *name == identity.1)
        }
        TableChangeKind::Dropped => false,
    });
    let dropped = since_declared().any(|change| {
        change.kind == TableChangeKind::Dropped
            && change.undone_at.is_none_or(|undone| undone > at)
            && change.table == *identity
    });
    if dropped && !renamed {
        return None;
    }

    let mut dependent = identity.1.clone();
    // The old name stays a candidate in case the rename did not take place.
    let mut referenced = vec![reference.referenced_table_parts.clone()];
    for change in since_declared() {
        let TableChangeKind::RenamedTo {
            name, unqualified, ..
        } = &change.kind
        else {
            continue;
        };
        let (schema, table) = &change.table;
        if change.undone_at.is_some_and(|undone| undone < at) {
            continue;
        }
        if *schema == identity.0 && *table == dependent {
            dependent = name.clone();
        }
        // An unqualified rename may apply in the dependent's schema even
        // when it was resolved to another one.
        if (*unqualified || *schema == identity.0)
            && referenced.iter().any(|parent| parent.last() == Some(table))
        {
            referenced.push(vec![name.clone()]);
        }
    }

    let dependent_table = if dependent == identity.1 {
        reference.dependent_table.clone()
    } else if identity.0 == "main" {
        dependent
    } else {
        format!("{}.{dependent}", identity.0)
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
fn created(report: &ScanReport, schema_object: &SchemaObject) -> bool {
    !schema_object.if_not_exists
        || !report.schema_objects.iter().any(|earlier| {
            earlier.kind == schema_object.kind
                && earlier.identity == schema_object.identity
                && earlier.statement_index < schema_object.statement_index
                && created(report, earlier)
                && exists_at(report, earlier, schema_object.statement_index)
        })
}

/// Whether an index or trigger is there when statement `at` runs: created and
/// not rolled back, not dropped, not gone with its table, and not followed by
/// a plain CREATE of the same name on the same table, which succeeds only
/// once the object is gone and then stands for it.
fn exists_at(report: &ScanReport, schema_object: &SchemaObject, at: usize) -> bool {
    let table_at = |statement: usize| tables_at(report, schema_object, statement).0;
    let created_again = report.schema_objects.iter().any(|later| {
        later.kind == schema_object.kind
            && later.identity == schema_object.identity
            && later.statement_index > schema_object.statement_index
            && !later.if_not_exists
            && holds_at(later.statement_index, later.undone_at, at)
            && name_parts_match(
                &table_at(later.statement_index),
                effective_table_parts(later, &report.rebuilds),
            )
    });
    let dropped = report.dropped_schema_objects.iter().any(|dropped| {
        dropped.kind == schema_object.kind
            && dropped.identity == schema_object.identity
            && dropped.statement_index > schema_object.statement_index
            && holds_at(dropped.statement_index, dropped.undone_at, at)
    });
    // The object lives in the schema of its table.
    let table_dropped = report.table_changes.iter().any(|change| {
        change.kind == TableChangeKind::Dropped
            && change.statement_index > schema_object.statement_index
            && holds_at(change.statement_index, change.undone_at, at)
            && change.table.0 == schema_object.identity.0
            && table_at(change.statement_index).last() == Some(&change.table.1)
    });
    let lost_in_rebuild = report.rebuilds.iter().any(|rebuild| {
        let original = &rebuild.original_table_parts;
        rebuild.drop_statement_index > schema_object.statement_index
            && holds_at(
                rebuild.drop_statement_index,
                drop_undone_at(report, rebuild),
                at,
            )
            && (original.len() == 1 || original[0] == schema_object.identity.0)
            && name_parts_match(&table_at(rebuild.drop_statement_index), original)
    });
    holds_at(schema_object.statement_index, schema_object.undone_at, at)
        && !dropped
        && !table_dropped
        && !lost_in_rebuild
        && !created_again
}

/// Every name an indexed column may have when statement `at` runs, following
/// `ALTER TABLE ... RENAME COLUMN` on the index's table.
fn column_names(
    report: &ScanReport,
    schema_object: &SchemaObject,
    tables: &[Vec<String>],
    column: &str,
    at: usize,
) -> Vec<String> {
    let mut names = vec![column.to_string()];
    for rename in &report.column_renames {
        if rename.statement_index > schema_object.statement_index
            && rename.statement_index < at
            && names.contains(&rename.from)
            && tables
                .iter()
                .any(|table| name_parts_match(table, &rename.table_parts))
        {
            names.push(rename.to.clone());
        }
    }
    names
}

/// Whether `table` is dropped after statement `from` and before statement
/// `to`, by a plain drop or by a later rebuild.
fn dropped_between(report: &ScanReport, table: &[String], from: usize, to: usize) -> bool {
    let dropped = report.table_changes.iter().any(|change| {
        let (schema, name) = &change.table;
        change.kind == TableChangeKind::Dropped
            && change.statement_index > from
            && holds_at(change.statement_index, change.undone_at, to)
            && name_parts_match(&[schema.clone(), name.clone()], table)
    });
    dropped
        || report.rebuilds.iter().any(|rebuild| {
            rebuild.drop_statement_index > from
                && rebuild.drop_statement_index < to
                && name_parts_match(&rebuild.original_table_parts, table)
        })
}

/// The table of an index or trigger when statement `at` runs, and every name
/// that table may have then: the object stays on its table through
/// `ALTER TABLE ... RENAME TO`, and the old names stay candidates in case a
/// rename did not take place.
fn tables_at(
    report: &ScanReport,
    schema_object: &SchemaObject,
    at: usize,
) -> (Vec<String>, Vec<Vec<String>>) {
    let mut current = effective_table_parts(schema_object, &report.rebuilds).to_vec();
    let mut names = vec![current.clone()];
    for change in &report.table_changes {
        let TableChangeKind::RenamedTo {
            name,
            unqualified,
            shadowed,
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
        // The object lives in the schema of its table.
        let renames = |candidate: &Vec<String>| {
            candidate.last() == Some(table) && (*unqualified || *schema == schema_object.identity.0)
        };
        // A rename that may have hit a temp table of the same name adds a
        // candidate but does not move the object.
        if renames(&current) && !shadowed {
            current = vec![name.clone()];
        }
        if names.iter().any(renames) {
            names.push(vec![name.clone()]);
        }
    }
    (current, names)
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

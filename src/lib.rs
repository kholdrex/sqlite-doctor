use std::fmt;

mod scanner;

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
                "PRAGMA foreign_keys inside a transaction is a no-op"
            }
            Self::MissingSchemaObjectRestoration => {
                "table rebuild may omit restoration of indexes or triggers"
            }
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

/// Converts scanner evidence into actionable diagnostics.
///
/// `SQLD001` requires both a complete rebuild and an explicit `ON DELETE
/// CASCADE`, `SET NULL` or `SET DEFAULT` reference in the same input. This deliberately avoids claiming knowledge of
/// schema state that the scanner cannot see.
pub fn diagnose(report: &ScanReport) -> Vec<Finding> {
    let mut findings = Vec::new();

    for rebuild in &report.rebuilds {
        for reference in &report.delete_action_references {
            if reference.statement_index < rebuild.drop_statement_index
                && rebuild.foreign_keys_enabled_at_drop != Some(false)
                && !dependent_dropped_before(report, reference, rebuild)
                && name_parts_match(
                    &reference.referenced_table_parts,
                    &rebuild.original_table_parts,
                )
            {
                findings.push(
                    Finding::new(
                        HazardId::DeleteActionOnRebuild,
                        rebuild.drop_line,
                        format!(
                            "Dependent table `{}` references rebuilt table `{}` with {} on line {}; dropping the original table {} while foreign-key enforcement is active.",
                            reference.dependent_table,
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
                        "Disable foreign-key enforcement before starting the transaction, perform the rebuild, then restore enforcement and run PRAGMA foreign_key_check; prefer a native ALTER TABLE or staged migration when available.",
                    )
                    .with_related_line(reference.line),
                );
            }
        }
    }

    for rebuild in &report.rebuilds {
        for schema_object in &report.schema_objects {
            let schema_table_parts = effective_table_parts(schema_object, &report.rebuilds);
            if schema_object.statement_index >= rebuild.drop_statement_index
                || !name_parts_match(schema_table_parts, &rebuild.original_table_parts)
            {
                continue;
            }

            let superseded = report.schema_objects.iter().any(|candidate| {
                candidate.kind == schema_object.kind
                    && candidate.statement_index > schema_object.statement_index
                    && candidate.statement_index < rebuild.drop_statement_index
                    && name_parts_match(&candidate.name_parts, &schema_object.name_parts)
                    && name_parts_match(
                        effective_table_parts(candidate, &report.rebuilds),
                        schema_table_parts,
                    )
            });
            if superseded {
                continue;
            }

            let explicitly_removed = report.dropped_schema_objects.iter().any(|dropped| {
                dropped.kind == schema_object.kind
                    && dropped.statement_index > schema_object.statement_index
                    && dropped.statement_index < rebuild.drop_statement_index
                    && name_parts_match(&dropped.name_parts, &schema_object.name_parts)
            });
            let removed_by_earlier_rebuild = report.rebuilds.iter().any(|earlier_rebuild| {
                earlier_rebuild.drop_statement_index > schema_object.statement_index
                    && earlier_rebuild.drop_statement_index < rebuild.drop_statement_index
                    && name_parts_match(&earlier_rebuild.original_table_parts, schema_table_parts)
            });
            if explicitly_removed || removed_by_earlier_rebuild {
                continue;
            }

            let restored = report.schema_objects.iter().any(|candidate| {
                let same_identity = candidate.kind == schema_object.kind
                    && name_parts_match(&candidate.name_parts, &schema_object.name_parts);
                let restored_before_rename = rebuild.kind == RebuildKind::Rename
                    && candidate.statement_index > rebuild.drop_statement_index
                    && candidate.statement_index < rebuild.replacement_statement_index
                    && name_parts_match(&candidate.table_parts, &rebuild.temporary_table_parts);
                let restored_after_rename = candidate.statement_index
                    > rebuild.replacement_statement_index
                    && name_parts_match(
                        effective_table_parts(candidate, &report.rebuilds),
                        &rebuild.original_table_parts,
                    );
                same_identity && (restored_before_rename || restored_after_rename)
            });
            if restored {
                continue;
            }

            // An index on a column the replacement table no longer has cannot
            // be restored; removing the column removes the index on purpose.
            let column_removed = schema_object.columns.as_ref().is_some_and(|columns| {
                rebuild
                    .replacement_columns
                    .as_ref()
                    .is_some_and(|kept| columns.iter().any(|column| !kept.contains(column)))
            });
            if column_removed {
                continue;
            }

            findings.push(
                Finding::new(
                    HazardId::MissingSchemaObjectRestoration,
                    rebuild.drop_line,
                    format!(
                        "The {} `{}` created on line {} is destroyed when rebuilt table `{}` is dropped and is not recreated under the same name later in this input.",
                        schema_object.kind.label(),
                        schema_object.name,
                        schema_object.line,
                        rebuild.original_table
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
            "Set PRAGMA foreign_keys=OFF before starting the transaction, restore it after the migration, and run PRAGMA foreign_key_check.",
        ));
    }

    findings.sort_by_key(|finding| finding.line);
    findings
}

/// The dependent table was dropped after the reference was declared and before
/// the parent's drop, and no rollback undid that drop in between. A table
/// created again later declares its references anew. A rename of that table in
/// the same window keeps the reference, since the drop may hit another table.
fn dependent_dropped_before(
    report: &ScanReport,
    reference: &DeleteActionReference,
    rebuild: &RebuildSequence,
) -> bool {
    let window =
        |index: usize| index > reference.statement_index && index < rebuild.drop_statement_index;
    let renamed = report.renamed_tables.iter().any(|rename| {
        window(rename.statement_index)
            && (rename.from == reference.dependent_identity
                || rename.to == reference.dependent_identity)
    });
    !renamed
        && report.dropped_tables.iter().any(|dropped| {
            window(dropped.statement_index)
                && dropped
                    .undone_at
                    .is_none_or(|undone| undone > rebuild.drop_statement_index)
                && dropped.table == reference.dependent_identity
        })
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

pub fn initial_hazards() -> Vec<HazardId> {
    vec![
        HazardId::DeleteActionOnRebuild,
        HazardId::ForeignKeysPragmaInsideTransaction,
        HazardId::MissingSchemaObjectRestoration,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hazard_codes_are_stable() {
        let codes: Vec<_> = initial_hazards().into_iter().map(HazardId::code).collect();
        assert_eq!(codes, vec!["SQLD001", "SQLD002", "SQLD003"]);
    }

    #[test]
    fn display_includes_code_and_title() {
        assert_eq!(
            HazardId::ForeignKeysPragmaInsideTransaction.to_string(),
            "SQLD002: PRAGMA foreign_keys inside a transaction is a no-op"
        );
    }

    #[test]
    fn finding_constructor_populates_fields() {
        let finding = Finding::new(
            HazardId::DeleteActionOnRebuild,
            12,
            "Detected rebuild sequence with dependent child tables.",
            "Use a safer staged migration or avoid rebuild when adding the constraint.",
        );

        assert_eq!(finding.hazard, HazardId::DeleteActionOnRebuild);
        assert_eq!(finding.line, 12);
        assert_eq!(finding.related_line, None);
        assert!(finding.detail.contains("rebuild sequence"));
        assert!(finding.remediation.contains("safer staged migration"));
    }
}

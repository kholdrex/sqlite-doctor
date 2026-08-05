use std::fmt;

mod scanner;

pub use scanner::{
    CascadeReference, ForeignKeysPragmaInTransaction, RebuildSequence, ScanReport, scan_sql,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HazardId {
    CascadeDeleteOnRebuild,
    ForeignKeysPragmaInsideTransaction,
    MissingSchemaObjectRestoration,
}

impl HazardId {
    pub fn code(self) -> &'static str {
        match self {
            Self::CascadeDeleteOnRebuild => "SQLD001",
            Self::ForeignKeysPragmaInsideTransaction => "SQLD002",
            Self::MissingSchemaObjectRestoration => "SQLD003",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::CascadeDeleteOnRebuild => "table rebuild may trigger ON DELETE CASCADE data loss",
            Self::ForeignKeysPragmaInsideTransaction => {
                "PRAGMA foreign_keys inside a transaction is a no-op"
            }
            Self::MissingSchemaObjectRestoration => {
                "table rebuild may omit restoration of indexes, triggers, or views"
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
/// `SQLD001` requires both a complete rebuild and an explicit cascading
/// reference in the same input. This deliberately avoids claiming knowledge of
/// schema state that the scanner cannot see.
pub fn diagnose(report: &ScanReport) -> Vec<Finding> {
    let mut findings = Vec::new();

    for rebuild in &report.rebuilds {
        for reference in &report.cascade_references {
            if reference.statement_index < rebuild.drop_statement_index
                && rebuild.foreign_keys_enabled_at_drop != Some(false)
                && name_parts_match(
                    &reference.referenced_table_parts,
                    &rebuild.original_table_parts,
                )
            {
                findings.push(
                    Finding::new(
                        HazardId::CascadeDeleteOnRebuild,
                        rebuild.drop_line,
                        format!(
                            "Dependent table `{}` references rebuilt table `{}` with ON DELETE CASCADE on line {}; dropping the original table can delete dependent rows while foreign-key enforcement is active.",
                            reference.dependent_table, rebuild.original_table, reference.line
                        ),
                        "Disable foreign-key enforcement before starting the transaction, perform the rebuild, then restore enforcement and run PRAGMA foreign_key_check; prefer a native ALTER TABLE or staged migration when available.",
                    )
                    .with_related_line(reference.line),
                );
            }
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
        HazardId::CascadeDeleteOnRebuild,
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
            HazardId::CascadeDeleteOnRebuild,
            12,
            "Detected rebuild sequence with dependent child tables.",
            "Use a safer staged migration or avoid rebuild when adding the constraint.",
        );

        assert_eq!(finding.hazard, HazardId::CascadeDeleteOnRebuild);
        assert_eq!(finding.line, 12);
        assert_eq!(finding.related_line, None);
        assert!(finding.detail.contains("rebuild sequence"));
        assert!(finding.remediation.contains("safer staged migration"));
    }
}

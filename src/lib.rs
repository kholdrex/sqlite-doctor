use std::fmt;

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
    pub detail: String,
    pub remediation: String,
}

impl Finding {
    pub fn new(
        hazard: HazardId,
        detail: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            hazard,
            detail: detail.into(),
            remediation: remediation.into(),
        }
    }
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
            "Detected rebuild sequence with dependent child tables.",
            "Use a safer staged migration or avoid rebuild when adding the constraint.",
        );

        assert_eq!(finding.hazard, HazardId::CascadeDeleteOnRebuild);
        assert!(finding.detail.contains("rebuild sequence"));
        assert!(finding.remediation.contains("safer staged migration"));
    }
}

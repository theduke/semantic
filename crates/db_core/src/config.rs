#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationMismatchPolicy {
    Fail,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub struct DbConfig {
    pub migration_mismatch_policy: MigrationMismatchPolicy,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            migration_mismatch_policy: MigrationMismatchPolicy::Fail,
        }
    }
}

/// Per-call controls for database writes.
///
/// These settings are deliberately not stored on the database: a relaxed import
/// must never weaken validation for later, unrelated writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteSettings {
    /// Validate that references point at an existing entity of an allowed class.
    pub validate_foreign_keys: bool,
}

impl Default for WriteSettings {
    fn default() -> Self {
        Self {
            validate_foreign_keys: true,
        }
    }
}

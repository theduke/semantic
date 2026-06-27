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

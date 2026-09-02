use semantic_db_core::DbConfig;

/// Physical representation used by the PostgreSQL backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostgresLayout {
    /// Lossless semantic objects stored in backend-owned tables.
    Semantic,
    /// Strict one-class-per-table relational storage.
    Relational,
}

/// Whether PostgreSQL schema objects may be changed by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostgresSchemaOwnership {
    Managed,
    ReadOnly,
}

/// Stable identity policy for relational tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostgresIdentityPolicy {
    PrimaryKey,
    PrimaryKeyOrUniqueNotNull,
}

/// Construction options for the PostgreSQL backend.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PostgresBackendOptions {
    pub layout: PostgresLayout,
    pub ownership: PostgresSchemaOwnership,
    pub metadata_schema: String,
    pub schemas: Vec<String>,
    pub db_config: DbConfig,
    pub transaction_retry_count: u32,
    pub identity_policy: PostgresIdentityPolicy,
    pub allow_legacy_discovery_ids: bool,
}

impl PostgresBackendOptions {
    pub fn semantic_managed() -> Self {
        Self {
            layout: PostgresLayout::Semantic,
            ownership: PostgresSchemaOwnership::Managed,
            metadata_schema: "_semantic".to_string(),
            schemas: Vec::new(),
            db_config: DbConfig::default(),
            transaction_retry_count: 3,
            identity_policy: PostgresIdentityPolicy::PrimaryKey,
            allow_legacy_discovery_ids: false,
        }
    }

    pub fn semantic_read_only(schema: impl Into<String>) -> Self {
        let mut options = Self::semantic_managed();
        options.ownership = PostgresSchemaOwnership::ReadOnly;
        options.metadata_schema = schema.into();
        options
    }

    pub fn relational_managed(schema: impl Into<String>) -> Self {
        let mut options = Self::semantic_managed();
        options.layout = PostgresLayout::Relational;
        options.metadata_schema = schema.into();
        options
    }

    pub fn relational_discovery(schemas: Vec<String>) -> Self {
        let mut options = Self::semantic_managed();
        options.layout = PostgresLayout::Relational;
        options.ownership = PostgresSchemaOwnership::ReadOnly;
        options.schemas = schemas;
        options
    }

    pub fn with_metadata_schema(mut self, schema: impl Into<String>) -> Self {
        self.metadata_schema = schema.into();
        self
    }

    pub fn with_db_config(mut self, config: DbConfig) -> Self {
        self.db_config = config;
        self
    }

    pub fn with_transaction_retry_count(mut self, retries: u32) -> Self {
        self.transaction_retry_count = retries;
        self
    }

    pub fn with_identity_policy(mut self, policy: PostgresIdentityPolicy) -> Self {
        self.identity_policy = policy;
        self
    }

    pub fn with_legacy_discovery_ids(mut self, enabled: bool) -> Self {
        self.allow_legacy_discovery_ids = enabled;
        self
    }

    pub(crate) fn validate(&self) -> Result<(), semantic_db_core::DbError> {
        if self.metadata_schema.is_empty() || self.metadata_schema.contains('\0') {
            return Err(semantic_db_core::DbError::InvalidQuery(
                "PostgreSQL metadata schema must be a non-empty identifier".to_string(),
            ));
        }
        if self.layout == PostgresLayout::Semantic
            && self.ownership == PostgresSchemaOwnership::ReadOnly
            && !self.schemas.is_empty()
        {
            return Err(semantic_db_core::DbError::InvalidQuery(
                "semantic read-only mode uses metadata_schema, not discovery schemas".to_string(),
            ));
        }
        if self.layout == PostgresLayout::Relational
            && self.ownership == PostgresSchemaOwnership::ReadOnly
            && self.schemas.is_empty()
        {
            return Err(semantic_db_core::DbError::InvalidQuery(
                "relational discovery requires at least one schema".to_string(),
            ));
        }
        Ok(())
    }
}

impl Default for PostgresBackendOptions {
    fn default() -> Self {
        Self::semantic_managed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_constructors_set_layout_and_ownership() {
        let managed = PostgresBackendOptions::semantic_managed();
        assert_eq!(managed.layout, PostgresLayout::Semantic);
        assert_eq!(managed.ownership, PostgresSchemaOwnership::Managed);

        let discovery = PostgresBackendOptions::relational_discovery(vec!["public".into()]);
        assert_eq!(discovery.layout, PostgresLayout::Relational);
        assert_eq!(discovery.ownership, PostgresSchemaOwnership::ReadOnly);
        assert!(discovery.validate().is_ok());
    }
}

use std::collections::{BTreeMap, BTreeSet};

use fnv::FnvHashMap;
use semantic_data::schema::{
    IndexSchema as DataIndexSchema,
    attribute::attribute_type::AttributeType,
    class::class_type::ClassType,
    core::type_def::TypeDef,
    core::type_node::Type,
    lowered::{DataType, LowerError},
    record::record_type::RecordType,
};
use semantic_data::value::Value;

use crate::catalog::{
    LocalAttrId, LocalClassId, LocalCollectionId, LocalFieldId, LocalIndexId, LocalRecordTypeId,
    LocalRelationId, LocalTypeDefId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSet {
    pub qualified_name: String,
    pub plain_name: String,
    pub underscore_name: String,
}

#[derive(Debug, Clone)]
pub struct AttributeSchema {
    pub lid: LocalAttrId,
    pub names: NameSet,
    pub attribute: AttributeType,
}

#[derive(Debug, Clone)]
pub struct RecordTypeSchema {
    pub lid: LocalRecordTypeId,
    pub names: NameSet,
    pub id: String,
    pub name: String,
    pub record: RecordType,
}

#[derive(Debug, Clone)]
pub struct TypeDefSchema {
    pub lid: LocalTypeDefId,
    pub names: NameSet,
    pub type_def: TypeDef,
    /// Storage disposition derived by lowering `type_def` against the catalog.
    pub data: TypeDefData,
}

/// Whether a catalog type definition can be inhabited by stored values.
///
/// Derived (never persisted): the catalog re-lowers every definition after
/// each schema change, so the tag always reflects the current catalog.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDefData {
    /// Not lowered yet. Only observable transiently while a DDL batch is
    /// being applied.
    Pending,
    /// The definition lowers to a storable data type.
    Storable(DataType),
    /// The definition cannot hold stored values: interface-only definitions
    /// (functions, interfaces, streams, ...) and legacy data definitions that
    /// were persisted before lowering was enforced.
    Unstorable(LowerError),
}

impl TypeDefData {
    pub fn data_type(&self) -> Option<&DataType> {
        match self {
            Self::Storable(data_type) => Some(data_type),
            Self::Pending | Self::Unstorable(_) => None,
        }
    }

    pub fn error(&self) -> Option<&LowerError> {
        match self {
            Self::Unstorable(error) => Some(error),
            Self::Pending | Self::Storable(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClassSchema {
    pub lid: LocalClassId,
    pub names: NameSet,
    pub class: ClassType,
    pub attributes: BTreeMap<String, LocalAttrId>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum CollectionKind {
    Untyped,
    Schema,
    Polymorphic,
}

pub use semantic_data::query::IntegrityMode;

#[derive(Debug, Clone)]
pub struct CollectionSchema {
    pub lid: LocalCollectionId,
    pub name: String,
    pub kind: CollectionKind,
    pub integrity_mode: IntegrityMode,
    pub internal: bool,

    field_aliases: FnvHashMap<String, String>,
    field_types: FnvHashMap<String, Type>,
    field_ids: FnvHashMap<String, LocalFieldId>,
    field_names_by_id: FnvHashMap<LocalFieldId, String>,
    attr_by_field_id: FnvHashMap<LocalFieldId, LocalAttrId>,
    /// Fields whose values are computed at read time and cannot be written.
    pub computed_fields: BTreeSet<String>,
    closed_fields: bool,
}

impl CollectionSchema {
    pub(crate) fn new(
        lid: LocalCollectionId,
        name: String,
        kind: CollectionKind,
        integrity_mode: IntegrityMode,
        internal: bool,
        field_aliases: FnvHashMap<String, String>,
        field_types: FnvHashMap<String, Type>,
        field_ids: FnvHashMap<String, LocalFieldId>,
        field_names_by_id: FnvHashMap<LocalFieldId, String>,
        attr_by_field_id: FnvHashMap<LocalFieldId, LocalAttrId>,
        computed_fields: BTreeSet<String>,
        closed_fields: bool,
    ) -> Self {
        Self {
            lid,
            name,
            kind,
            integrity_mode,
            internal,
            field_aliases,
            field_types,
            field_ids,
            field_names_by_id,
            attr_by_field_id,
            computed_fields,
            closed_fields,
        }
    }

    pub fn canonical_field_name<'a>(&'a self, field: &'a str) -> &'a str {
        self.field_aliases
            .get(field)
            .map(String::as_str)
            .unwrap_or(field)
    }

    pub fn field_type(&self, canonical_field: &str) -> Option<&Type> {
        self.field_types.get(canonical_field)
    }

    pub fn field_id(&self, field_name_or_alias: &str) -> Option<LocalFieldId> {
        let canonical = self.canonical_field_name(field_name_or_alias);
        self.field_ids.get(canonical).copied()
    }

    pub fn field_name_by_id(&self, field_id: LocalFieldId) -> Option<&str> {
        self.field_names_by_id.get(&field_id).map(String::as_str)
    }

    pub fn attr_for_field_id(&self, field_id: LocalFieldId) -> Option<LocalAttrId> {
        self.attr_by_field_id.get(&field_id).copied()
    }

    pub fn fields(&self) -> impl Iterator<Item = (LocalFieldId, &str)> {
        self.field_names_by_id
            .iter()
            .map(|(field_id, name)| (*field_id, name.as_str()))
    }

    pub fn is_closed_field_set(&self) -> bool {
        self.closed_fields
    }

    pub fn knows_field(&self, canonical_field: &str) -> bool {
        self.field_types.contains_key(canonical_field)
    }

    /// Internal `__`-prefixed collections store raw field names that are
    /// never resolved through attribute aliases.
    pub fn is_raw_system_collection(&self) -> bool {
        self.internal && self.name.starts_with("__")
    }
}

#[derive(Debug, Clone)]
pub struct IndexSchema {
    pub lid: LocalIndexId,
    pub schema: DataIndexSchema,
    pub collection: LocalCollectionId,
    pub canonical_field: String,
    pub field_id: Option<LocalFieldId>,
    pub attr_id: Option<LocalAttrId>,
}

impl IndexSchema {
    /// Canonical field names of the key columns after the first one.
    pub fn extra_columns(&self) -> impl Iterator<Item = &str> {
        self.schema
            .extra_key_paths
            .iter()
            .filter_map(|path| path.segments.first().map(String::as_str))
    }

    /// Canonical field names of all key columns, in key order.
    pub fn columns(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.canonical_field.as_str()).chain(self.extra_columns())
    }

    /// Number of key columns.
    pub fn column_count(&self) -> usize {
        1 + self.schema.extra_key_paths.len()
    }

    pub fn is_composite(&self) -> bool {
        self.schema.is_composite()
    }

    pub fn is_partial(&self) -> bool {
        self.schema.is_partial()
    }

    /// Single-column index covering every row with the column: its keys are
    /// the plain column values.
    pub fn is_simple(&self) -> bool {
        !self.is_composite() && !self.is_partial()
    }

    /// The partial index predicate as a query expression.
    pub fn predicate_expr(&self) -> Option<crate::Expr> {
        self.schema.predicate.clone().map(crate::Expr::from)
    }

    /// Whether `object` belongs to the rows this index covers (always true
    /// for non-partial indexes).
    pub fn covers(&self, object: &semantic_data::value::Object) -> bool {
        self.predicate_expr()
            .is_none_or(|predicate| crate::evaluate_filter_expr(object, &predicate))
    }

    /// Key value of `object` in an equality or range index, or `None` when
    /// the row has no entry.
    ///
    /// Rows without the first column, and rows outside a partial index's
    /// predicate, have no entry. A single-column key is the column value; a
    /// composite key is the list of column values with missing columns as
    /// `Void`. Its order-preserving encoding (a list token framing the
    /// concatenated column tokens) makes equality on leading columns plus a
    /// range on the next column one contiguous key range. The same value
    /// identifies the key for unique checks.
    pub fn key_value(&self, object: &semantic_data::value::Object) -> Option<Value> {
        if !self.schema.kind.is_value_index() {
            return None;
        }
        let first = object.get(&self.canonical_field)?;
        if !self.covers(object) {
            return None;
        }
        if !self.is_composite() {
            return Some(first.clone());
        }
        let mut values = Vec::with_capacity(self.column_count());
        values.push(first.clone());
        values.extend(
            self.extra_columns()
                .map(|column| object.get(column).cloned().unwrap_or(Value::Void)),
        );
        Some(Value::List(values))
    }

    /// Distinct tokens of `object` in a full-text index, or `None` for
    /// other index kinds.
    ///
    /// The tokens of all key columns (their string values and the strings
    /// of list values) are combined; rows outside a partial index's
    /// predicate have none. Each token is one index entry.
    pub fn text_tokens(
        &self,
        object: &semantic_data::value::Object,
    ) -> Option<std::collections::BTreeSet<String>> {
        if !self.schema.kind.is_full_text() {
            return None;
        }
        let mut tokens = std::collections::BTreeSet::new();
        if self.covers(object) {
            for value in self.columns().filter_map(|column| object.get(column)) {
                self.schema.analyzer.value_tokens(value, &mut tokens);
            }
        }
        Some(tokens)
    }

    /// Whether `old` and `new` agree on every key column and on the partial
    /// index predicate, so they derive the same entries.
    pub fn indexed_columns_equal(
        &self,
        old: &semantic_data::value::Object,
        new: &semantic_data::value::Object,
    ) -> bool {
        self.columns()
            .all(|column| old.get(column) == new.get(column))
            && (!self.is_partial() || self.covers(old) == self.covers(new))
    }

    /// Column values of a key produced by [`Self::key_value`], with `Void`
    /// (missing) columns as `None`.
    pub fn key_columns(&self, key: Value) -> Option<Vec<Option<Value>>> {
        let values = if self.is_composite() {
            match key {
                Value::List(values) if values.len() == self.column_count() => values,
                _ => return None,
            }
        } else {
            vec![key]
        };
        Some(
            values
                .into_iter()
                .map(|value| (!matches!(value, Value::Void)).then_some(value))
                .collect(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct RelationshipSchema {
    pub lid: LocalRelationId,
    pub relationship: semantic_data::schema::RelationType,
}

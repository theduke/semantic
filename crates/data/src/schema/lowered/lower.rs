//! Lowering from surface [`Type`]s to storable [`DataType`]s.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    DataKind, DataType, FieldData, ListData, MapData, NumberRepr, RecordData, RecordRest,
    TemporalRepr, TupleData, VariantCaseData, VariantData, VariantPayloadData,
};
use crate::schema::{
    AttributeType, ClassConstraint, ClassType, FloatWidth, IntWidth, NumberType, RecordType,
    TemporalType, Type, TypeDef, TypeKind, TypeRef, UIntWidth, VariantPayload, VariantTag,
    VariantType,
};

/// Maximum nesting of lowered nodes (mirrors the stored-value validator).
const MAX_DEPTH: usize = 128;
/// Upper bound on lowered nodes, guarding against exponential inlining of
/// heavily shared definitions.
const MAX_NODES: usize = 100_000;

/// Resolves named definitions while lowering.
///
/// Returned definitions must carry their canonical name (`TypeDef::name`,
/// `AttributeType::id`, `ClassType::id`); back-references and cycle detection
/// are keyed by it.
pub trait TypeResolver {
    /// Resolves a `Named` type reference.
    fn resolve_type_def(&self, name: &str) -> Option<&TypeDef>;

    /// Resolves an attribute referenced by an embedded class shape.
    fn resolve_attribute(&self, id: &str) -> Option<&AttributeType> {
        match &self.resolve_type_def(id)?.ty.kind {
            TypeKind::Attribute(attribute) => Some(attribute),
            _ => None,
        }
    }

    /// Resolves a base class referenced by an embedded class shape.
    fn resolve_class(&self, id: &str) -> Option<&ClassType> {
        match &self.resolve_type_def(id)?.ty.kind {
            TypeKind::Class(class) => Some(class),
            _ => None,
        }
    }
}

/// Lowers a surface type in a data position.
pub fn lower_type(ty: &Type, resolver: &dyn TypeResolver) -> Result<DataType, LowerError> {
    Lowerer::new(resolver).lower(ty)
}

/// Lowers the body of a named definition.
///
/// Self-references through a data constructor become
/// [`DataKind::Recursive`] back-references to `type_def.name`.
pub fn lower_type_def(
    type_def: &TypeDef,
    resolver: &dyn TypeResolver,
) -> Result<DataType, LowerError> {
    if !type_def.params.is_empty() {
        return Err(LowerError::new(LowerErrorKind::GenericDefinition {
            name: type_def.name.clone(),
        }));
    }
    let mut lowerer = Lowerer::new(resolver);
    lowerer.enter(&type_def.name);
    let lowered = lowerer.definition(type_def);
    lowerer.stack.pop();
    lowered
}

/// A type that cannot be lowered to a storable data type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowerError {
    /// Location of the offending node, outermost first.
    pub path: Vec<LowerPathSegment>,
    pub kind: LowerErrorKind,
}

impl LowerError {
    pub fn new(kind: LowerErrorKind) -> Self {
        Self {
            path: Vec::new(),
            kind,
        }
    }

    fn within(mut self, segment: LowerPathSegment) -> Self {
        self.path.insert(0, segment);
        self
    }

    /// Renders the error below a caller-provided context, e.g.
    /// `attribute 'x' list item: function types cannot be stored`.
    pub fn in_context(&self, context: &str) -> String {
        if self.path.is_empty() {
            format!("{context}: {}", self.kind)
        } else {
            format!("{context} {self}")
        }
    }
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, segment) in self.path.iter().enumerate() {
            if index > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{segment}")?;
        }
        if !self.path.is_empty() {
            f.write_str(": ")?;
        }
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for LowerError {}

/// One step of the path to an offending node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LowerPathSegment {
    OptionalInner,
    ListItem,
    TupleItem(usize),
    TupleRest,
    MapKey,
    MapValue,
    Field(String),
    AdditionalField,
    UnionVariant(usize),
    IntersectionPart(usize),
    VariantCase(String),
    ResultOk,
    ResultErr,
    /// A resolved `Named` reference.
    Named(String),
    /// An inline or class-member attribute.
    Attribute(String),
    /// A base class of an embedded class shape.
    BaseClass(String),
}

impl std::fmt::Display for LowerPathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OptionalInner => f.write_str("optional inner"),
            Self::ListItem => f.write_str("list item"),
            Self::TupleItem(index) => write!(f, "tuple item {index}"),
            Self::TupleRest => f.write_str("tuple rest"),
            Self::MapKey => f.write_str("map key"),
            Self::MapValue => f.write_str("map value"),
            Self::Field(name) => write!(f, "field '{name}'"),
            Self::AdditionalField => f.write_str("additional field"),
            Self::UnionVariant(index) => write!(f, "union variant {index}"),
            Self::IntersectionPart(index) => write!(f, "intersection part {index}"),
            Self::VariantCase(name) => write!(f, "variant case '{name}'"),
            Self::ResultOk => f.write_str("result ok"),
            Self::ResultErr => f.write_str("result err"),
            Self::Named(name) => write!(f, "type '{name}'"),
            Self::Attribute(id) => write!(f, "attribute '{id}'"),
            Self::BaseClass(id) => write!(f, "base class '{id}'"),
        }
    }
}

/// Why a type cannot be lowered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LowerErrorKind {
    /// A behavioural or uninhabitable kind (function, interface, handle,
    /// stream, never, unknown, opaque).
    NotStorable {
        kind: &'static str,
    },
    /// A number representation without a stored-value counterpart.
    UnrepresentableNumber {
        repr: &'static str,
    },
    /// A temporal representation without a stored-value counterpart.
    UnrepresentableTemporal {
        repr: &'static str,
    },
    /// A `Named` reference that does not resolve.
    UnresolvedType {
        name: String,
    },
    /// A `Named` reference with generic arguments.
    GenericArguments {
        name: String,
    },
    /// A generic definition used in a data position.
    GenericDefinition {
        name: String,
    },
    /// A definition that refers to itself without a data constructor in
    /// between (e.g. `A = B`, `B = Optional<A>`).
    AliasCycle {
        name: String,
    },
    /// An intersection with a part that is not a record.
    IntersectionNotRecord,
    /// Intersection parts that declare incompatible fields or rest types.
    IntersectionConflict {
        field: Option<String>,
    },
    TooDeep,
    TooLarge,
}

impl std::fmt::Display for LowerErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStorable { kind } => write!(f, "{kind} types cannot be stored"),
            Self::UnrepresentableNumber { repr } => {
                write!(f, "number type '{repr}' has no stored value representation")
            }
            Self::UnrepresentableTemporal { repr } => {
                write!(
                    f,
                    "temporal type '{repr}' has no stored value representation"
                )
            }
            Self::UnresolvedType { name } => write!(f, "type '{name}' is not defined"),
            Self::GenericArguments { name } => write!(
                f,
                "generic type arguments are not supported (applied to '{name}')"
            ),
            Self::GenericDefinition { name } => {
                write!(f, "generic type '{name}' cannot be stored")
            }
            Self::AliasCycle { name } => write!(
                f,
                "type '{name}' refers to itself without an intervening data structure"
            ),
            Self::IntersectionNotRecord => {
                f.write_str("only intersections of record types can be stored")
            }
            Self::IntersectionConflict { field: Some(field) } => {
                write!(f, "intersection parts declare conflicting field '{field}'")
            }
            Self::IntersectionConflict { field: None } => {
                f.write_str("intersection parts declare conflicting additional fields")
            }
            Self::TooDeep => write!(f, "type nesting exceeds {MAX_DEPTH} levels"),
            Self::TooLarge => write!(f, "lowered type exceeds {MAX_NODES} nodes"),
        }
    }
}

struct Lowerer<'a> {
    resolver: &'a dyn TypeResolver,
    /// Definitions being expanded, with the structural depth at entry.
    stack: Vec<(String, usize)>,
    /// Number of data constructors on the current path.
    structural: usize,
    depth: usize,
    nodes: usize,
}

impl<'a> Lowerer<'a> {
    fn new(resolver: &'a dyn TypeResolver) -> Self {
        Self {
            resolver,
            stack: Vec::new(),
            structural: 0,
            depth: 0,
            nodes: 0,
        }
    }

    fn enter(&mut self, name: &str) {
        self.stack.push((name.to_string(), self.structural));
    }

    /// Returns a back-reference if `name` is already being expanded.
    fn back_reference(&self, name: &str) -> Option<Result<DataType, LowerError>> {
        let (_, entry) = self.stack.iter().rev().find(|(open, _)| open == name)?;
        Some(if *entry == self.structural {
            Err(LowerError::new(LowerErrorKind::AliasCycle {
                name: name.to_string(),
            }))
        } else {
            Ok(DataType::new(DataKind::Recursive(name.to_string())))
        })
    }

    fn lower(&mut self, ty: &Type) -> Result<DataType, LowerError> {
        self.depth += 1;
        self.nodes += 1;
        let result = if self.depth > MAX_DEPTH {
            Err(LowerError::new(LowerErrorKind::TooDeep))
        } else if self.nodes > MAX_NODES {
            Err(LowerError::new(LowerErrorKind::TooLarge))
        } else {
            self.lower_node(ty)
        };
        self.depth -= 1;
        result
    }

    /// Lowers a child reached through a data constructor.
    fn structural(&mut self, ty: &Type, segment: LowerPathSegment) -> Result<DataType, LowerError> {
        self.structural += 1;
        let result = self.lower(ty).map_err(|err| err.within(segment));
        self.structural -= 1;
        result
    }

    fn lower_node(&mut self, ty: &Type) -> Result<DataType, LowerError> {
        let kind = match &ty.kind {
            TypeKind::Any(_) => DataKind::Any,
            TypeKind::Null(_) => DataKind::Null,
            TypeKind::Bool(_) => DataKind::Bool,
            TypeKind::Char(_) => DataKind::Char,
            TypeKind::Number(number) => DataKind::Number(lower_number(number)?),
            TypeKind::String(string) => DataKind::String(string.clone()),
            TypeKind::Bytes(bytes) => DataKind::Bytes(bytes.clone()),
            TypeKind::Temporal(temporal) => DataKind::Temporal(lower_temporal(temporal)?),
            TypeKind::Uuid => DataKind::Uuid,
            TypeKind::IpAddr(ip) => DataKind::IpAddr(ip.clone()),
            TypeKind::Json => DataKind::Json,
            TypeKind::Optional(optional) => DataKind::Optional(Box::new(
                self.lower(&optional.inner)
                    .map_err(|err| err.within(LowerPathSegment::OptionalInner))?,
            )),
            TypeKind::Array(array) => DataKind::List(ListData {
                items: Box::new(self.structural(&array.items, LowerPathSegment::ListItem)?),
                length: array.length.clone(),
                distinct: false,
            }),
            TypeKind::List(list) => DataKind::List(ListData {
                items: Box::new(self.structural(&list.items, LowerPathSegment::ListItem)?),
                length: None,
                distinct: false,
            }),
            TypeKind::Set(set) => DataKind::List(ListData {
                items: Box::new(self.structural(&set.items, LowerPathSegment::ListItem)?),
                length: None,
                distinct: true,
            }),
            TypeKind::Tuple(tuple) => {
                let mut items = Vec::with_capacity(tuple.items.len());
                for (index, item) in tuple.items.iter().enumerate() {
                    items.push(self.structural(item, LowerPathSegment::TupleItem(index))?);
                }
                let rest = match &tuple.rest {
                    Some(rest) => Some(Box::new(
                        self.structural(rest, LowerPathSegment::TupleRest)?,
                    )),
                    None => None,
                };
                DataKind::Tuple(TupleData { items, rest })
            }
            TypeKind::Map(map) => DataKind::Map(MapData {
                keys: Box::new(self.structural(&map.keys, LowerPathSegment::MapKey)?),
                values: Box::new(self.structural(&map.values, LowerPathSegment::MapValue)?),
                ordered: map.ordered,
            }),
            TypeKind::Record(record) => DataKind::Record(self.record(record)?),
            TypeKind::Class(class) => DataKind::Record(self.class(class)?),
            TypeKind::Union(union) => {
                let mut variants = Vec::with_capacity(union.variants.len());
                for (index, variant) in union.variants.iter().enumerate() {
                    variants.push(
                        self.lower(variant)
                            .map_err(|err| err.within(LowerPathSegment::UnionVariant(index)))?,
                    );
                }
                DataKind::Union(variants)
            }
            TypeKind::Intersection(intersection) => {
                let mut lowered = self.intersection(&intersection.variants)?;
                lowered.constraints.extend(ty.constraints.iter().cloned());
                return Ok(lowered);
            }
            TypeKind::Variant(variant) => DataKind::Variant(self.variant(variant)?),
            TypeKind::Enum(enumeration) => DataKind::Enum(enumeration.clone()),
            TypeKind::Result(result) => DataKind::Variant(VariantData {
                tag: VariantTag::ExternallyTagged,
                cases: vec![
                    VariantCaseData {
                        name: "ok".to_string(),
                        payload: VariantPayloadData::Newtype(Box::new(
                            self.structural(&result.ok, LowerPathSegment::ResultOk)?,
                        )),
                        discriminant: None,
                    },
                    VariantCaseData {
                        name: "err".to_string(),
                        payload: VariantPayloadData::Newtype(Box::new(
                            self.structural(&result.err, LowerPathSegment::ResultErr)?,
                        )),
                        discriminant: None,
                    },
                ],
            }),
            TypeKind::Ref(reference) => DataKind::Ref(reference.clone()),
            TypeKind::Extension(extension) => DataKind::Extension(extension.clone()),
            TypeKind::Attribute(attribute) => {
                let mut lowered = self
                    .attribute(attribute)
                    .map_err(|err| err.within(LowerPathSegment::Attribute(attribute.id.clone())))?;
                lowered.constraints.extend(ty.constraints.iter().cloned());
                return Ok(lowered);
            }
            TypeKind::Named(reference) => {
                let mut lowered = self.named(reference)?;
                lowered.constraints.extend(ty.constraints.iter().cloned());
                return Ok(lowered);
            }
            TypeKind::Function(_) => return not_storable("function"),
            TypeKind::Interface(_) => return not_storable("interface"),
            TypeKind::Handle(_) => return not_storable("handle"),
            TypeKind::Stream(_) => return not_storable("stream"),
            TypeKind::Never(_) => return not_storable("never"),
            TypeKind::Unknown(_) => return not_storable("unknown"),
            TypeKind::Opaque(_) => return not_storable("opaque"),
        };
        Ok(DataType {
            kind,
            constraints: ty.constraints.clone(),
        })
    }

    fn attribute(&mut self, attribute: &AttributeType) -> Result<DataType, LowerError> {
        let mut lowered = self.lower(&attribute.ty)?;
        lowered
            .constraints
            .extend(attribute.constraints.iter().cloned());
        Ok(lowered)
    }

    fn named(&mut self, reference: &TypeRef) -> Result<DataType, LowerError> {
        if !reference.args.is_empty() {
            return Err(LowerError::new(LowerErrorKind::GenericArguments {
                name: reference.name.clone(),
            }));
        }
        let Some(type_def) = self.resolver.resolve_type_def(&reference.name) else {
            return Err(LowerError::new(LowerErrorKind::UnresolvedType {
                name: reference.name.clone(),
            }));
        };
        let segment = LowerPathSegment::Named(type_def.name.clone());
        if !type_def.params.is_empty() {
            return Err(LowerError::new(LowerErrorKind::GenericDefinition {
                name: type_def.name.clone(),
            }));
        }
        if let Some(back_reference) = self.back_reference(&type_def.name) {
            return back_reference.map_err(|err| err.within(segment));
        }
        self.enter(&type_def.name);
        let lowered = self.definition(type_def);
        self.stack.pop();
        lowered.map_err(|err| err.within(segment))
    }

    /// Lowers a definition body. Attribute definitions lower to their value
    /// type; the caller already names the attribute in error paths.
    fn definition(&mut self, type_def: &TypeDef) -> Result<DataType, LowerError> {
        let TypeKind::Attribute(attribute) = &type_def.ty.kind else {
            return self.lower(&type_def.ty);
        };
        let mut lowered = self.attribute(attribute)?;
        lowered
            .constraints
            .extend(type_def.ty.constraints.iter().cloned());
        Ok(lowered)
    }

    fn record(&mut self, record: &RecordType) -> Result<RecordData, LowerError> {
        let mut fields = BTreeMap::new();
        for (name, field) in &record.fields {
            let ty = self.structural(&field.ty, LowerPathSegment::Field(name.clone()))?;
            fields.insert(
                name.clone(),
                FieldData {
                    ty,
                    required: field.required,
                    computed: false,
                    default: field.default.clone(),
                },
            );
        }
        let rest = match (&record.additional, record.open) {
            (Some(additional), _) => RecordRest::Additional(Box::new(
                self.structural(additional, LowerPathSegment::AdditionalField)?,
            )),
            (None, true) => RecordRest::Open,
            (None, false) => RecordRest::Closed,
        };
        Ok(RecordData {
            fields,
            rest,
            class: None,
        })
    }

    /// Lowers an embedded class shape to a record keyed by canonical
    /// attribute ID.
    ///
    /// Mirrors stored-value validation: base classes contribute their
    /// attributes first, unresolved attributes and base classes are skipped,
    /// and class field constraints attach to the resolved field.
    fn class(&mut self, class: &ClassType) -> Result<RecordData, LowerError> {
        let mut fields = BTreeMap::new();
        let mut seen = BTreeSet::from([class.id.clone()]);
        self.class_fields(class, &mut fields, &mut seen)?;
        Ok(RecordData {
            fields,
            rest: if class.strict_schema {
                RecordRest::Closed
            } else {
                RecordRest::Open
            },
            class: Some(class.id.clone()),
        })
    }

    fn class_fields(
        &mut self,
        class: &ClassType,
        fields: &mut BTreeMap<String, FieldData>,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), LowerError> {
        let resolver = self.resolver;
        for base in class.inherits.iter().chain(&class.extends) {
            if let Some(base_class) = resolver.resolve_class(&base.id)
                && seen.insert(base_class.id.clone())
            {
                self.class_fields(base_class, fields, seen).map_err(|err| {
                    err.within(LowerPathSegment::BaseClass(base_class.id.clone()))
                })?;
            }
        }
        for class_attr in class.attributes.values() {
            let Some(attribute) = resolver.resolve_attribute(&class_attr.attribute.id) else {
                continue;
            };
            let segment = LowerPathSegment::Attribute(attribute.id.clone());
            self.structural += 1;
            let lowered = match self.back_reference(&attribute.id) {
                Some(back_reference) => back_reference,
                None => {
                    self.enter(&attribute.id);
                    let lowered = self.attribute(attribute);
                    self.stack.pop();
                    lowered
                }
            };
            self.structural -= 1;
            let mut ty = lowered.map_err(|err| err.within(segment))?;
            if let Some(inherited) = fields.get(&attribute.id) {
                ty.constraints
                    .extend(inherited.ty.constraints.iter().cloned());
            }
            ty.constraints
                .extend(class_attr.constraints.iter().cloned());
            fields.insert(
                attribute.id.clone(),
                FieldData {
                    ty,
                    required: class_attr.required,
                    computed: class_attr.computed.is_some(),
                    default: None,
                },
            );
        }
        for constraint in &class.constraints {
            let ClassConstraint::Field {
                attribute,
                constraint,
            } = constraint
            else {
                continue;
            };
            let canonical = resolver
                .resolve_attribute(&attribute.id)
                .map(|attribute| attribute.id.clone())
                .or_else(|| self.class_alias_field(class, &attribute.id, &mut BTreeSet::new()));
            if let Some(field) = canonical.and_then(|id| fields.get_mut(&id)) {
                field.ty.constraints.push(constraint.clone());
            }
        }
        Ok(())
    }

    /// Resolves a class attribute alias (including inherited aliases) to the
    /// canonical attribute ID.
    fn class_alias_field(
        &self,
        class: &ClassType,
        alias: &str,
        seen: &mut BTreeSet<String>,
    ) -> Option<String> {
        if !seen.insert(class.id.clone()) {
            return None;
        }
        if let Some(class_attr) = class.attributes.get(alias) {
            return Some(
                self.resolver
                    .resolve_attribute(&class_attr.attribute.id)
                    .map(|attribute| attribute.id.clone())
                    .unwrap_or_else(|| class_attr.attribute.id.clone()),
            );
        }
        class
            .inherits
            .iter()
            .chain(&class.extends)
            .filter_map(|base| self.resolver.resolve_class(&base.id))
            .find_map(|base| self.class_alias_field(base, alias, seen))
    }

    fn variant(&mut self, variant: &VariantType) -> Result<VariantData, LowerError> {
        let mut cases = Vec::with_capacity(variant.variants.len());
        for case in &variant.variants {
            let segment = || LowerPathSegment::VariantCase(case.name.clone());
            let payload = match &case.payload {
                VariantPayload::Unit => VariantPayloadData::Unit,
                VariantPayload::Tuple(items) => {
                    let mut lowered = Vec::with_capacity(items.len());
                    for (index, item) in items.iter().enumerate() {
                        lowered.push(
                            self.structural(item, LowerPathSegment::TupleItem(index))
                                .map_err(|err| err.within(segment()))?,
                        );
                    }
                    VariantPayloadData::Tuple(lowered)
                }
                VariantPayload::Record(record) => {
                    self.structural += 1;
                    let lowered = self.record(record);
                    self.structural -= 1;
                    VariantPayloadData::Record(lowered.map_err(|err| err.within(segment()))?)
                }
                VariantPayload::Newtype(inner) => {
                    VariantPayloadData::Newtype(Box::new(self.structural(inner, segment())?))
                }
            };
            cases.push(VariantCaseData {
                name: case.name.clone(),
                payload,
                discriminant: case.discriminant.clone(),
            });
        }
        Ok(VariantData {
            tag: variant.tag.clone(),
            cases,
        })
    }

    /// Merges an intersection whose parts all lower to records.
    fn intersection(&mut self, parts: &[Type]) -> Result<DataType, LowerError> {
        let mut merged = RecordData {
            fields: BTreeMap::new(),
            rest: RecordRest::Open,
            class: None,
        };
        let mut constraints = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let segment = || LowerPathSegment::IntersectionPart(index);
            let lowered = self.lower(part).map_err(|err| err.within(segment()))?;
            let DataKind::Record(record) = lowered.kind else {
                return Err(
                    LowerError::new(LowerErrorKind::IntersectionNotRecord).within(segment())
                );
            };
            constraints.extend(lowered.constraints);
            merge_record(&mut merged, record).map_err(|err| err.within(segment()))?;
        }
        Ok(DataType {
            kind: DataKind::Record(merged),
            constraints,
        })
    }
}

fn merge_record(into: &mut RecordData, record: RecordData) -> Result<(), LowerError> {
    for (name, field) in record.fields {
        match into.fields.get_mut(&name) {
            Some(existing) if existing.ty == field.ty => {
                existing.required |= field.required;
                existing.computed |= field.computed;
                if existing.default.is_none() {
                    existing.default = field.default;
                }
            }
            Some(_) => {
                return Err(LowerError::new(LowerErrorKind::IntersectionConflict {
                    field: Some(name),
                }));
            }
            None => {
                into.fields.insert(name, field);
            }
        }
    }
    into.rest = match (
        std::mem::replace(&mut into.rest, RecordRest::Open),
        record.rest,
    ) {
        (RecordRest::Closed, _) | (_, RecordRest::Closed) => RecordRest::Closed,
        (RecordRest::Additional(a), RecordRest::Additional(b)) if a != b => {
            return Err(LowerError::new(LowerErrorKind::IntersectionConflict {
                field: None,
            }));
        }
        (RecordRest::Additional(a), _) | (_, RecordRest::Additional(a)) => {
            RecordRest::Additional(a)
        }
        (RecordRest::Open, RecordRest::Open) => RecordRest::Open,
    };
    Ok(())
}

fn not_storable<T>(kind: &'static str) -> Result<T, LowerError> {
    Err(LowerError::new(LowerErrorKind::NotStorable { kind }))
}

fn lower_number(number: &NumberType) -> Result<NumberRepr, LowerError> {
    let unrepresentable = |repr| {
        Err(LowerError::new(LowerErrorKind::UnrepresentableNumber {
            repr,
        }))
    };
    match number {
        NumberType::Int(IntWidth::I256) => unrepresentable("i256"),
        NumberType::UInt(UIntWidth::U256) => unrepresentable("u256"),
        NumberType::Float(FloatWidth::F80) => unrepresentable("f80"),
        NumberType::Float(FloatWidth::F128) => unrepresentable("f128"),
        NumberType::Float(FloatWidth::Decimal32) => unrepresentable("decimal32"),
        NumberType::Float(FloatWidth::Decimal64) => unrepresentable("decimal64"),
        NumberType::Float(FloatWidth::Decimal128) => unrepresentable("decimal128"),
        NumberType::Int(width) => Ok(NumberRepr::Int(width.clone())),
        NumberType::UInt(width) => Ok(NumberRepr::UInt(width.clone())),
        NumberType::Float(width) => Ok(NumberRepr::Float(width.clone())),
        NumberType::Unspecified => Ok(NumberRepr::Unspecified),
        NumberType::BigInt(_) => unrepresentable("big_int"),
        NumberType::BigUInt(_) => unrepresentable("big_uint"),
        NumberType::Decimal(_) => unrepresentable("decimal"),
        NumberType::Rational(_) => unrepresentable("rational"),
        NumberType::Complex(_) => unrepresentable("complex"),
    }
}

fn lower_temporal(temporal: &TemporalType) -> Result<TemporalRepr, LowerError> {
    let unrepresentable = |repr| {
        Err(LowerError::new(LowerErrorKind::UnrepresentableTemporal {
            repr,
        }))
    };
    match temporal {
        TemporalType::Date => Ok(TemporalRepr::Date),
        TemporalType::Time => Ok(TemporalRepr::Time),
        TemporalType::DateTime => Ok(TemporalRepr::DateTime),
        TemporalType::Duration => Ok(TemporalRepr::Duration),
        // Timestamps are stored as date-times: the Postgres backend maps
        // `timestamp`/`timestamptz` columns to this kind and reads them back as
        // `Value::DateTime`. Unit and time-zone details are surface metadata.
        TemporalType::Timestamp(_) => Ok(TemporalRepr::DateTime),
        TemporalType::Period => unrepresentable("period"),
        TemporalType::Instant => unrepresentable("instant"),
    }
}

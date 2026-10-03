//! Lossless Semantic representations of the public query AST and DDL payloads.
//!
//! Records are closed objects using the existing Facet field names. Enums are
//! snake-case strings; variants with payloads are single-key objects such as
//! `{"select": {...}}`, and unit variants are strings. Newtypes encode their
//! inner value directly; field paths are lists of `field`/`index` variants.
//! Unit schema structs are empty objects. Actual Semantic `Value` and `Object`
//! payloads pass through without conversion, preserving every value kind.
//!
//! Optional fields accept omission and null. The exception is `Option<Value>`:
//! omission means `None`, while a present null means `Some(Value::Null)`.
//! Their encoders omit absent values. Other optional fields encode null when
//! absent. Record/default annotations describe every accepted omission.
//!
//! Each `SemanticType` implementation returns a named reference. [`definitions`]
//! provides the complete finite graph, including the distinct general-expression
//! model used by schema constraints and defaults. Consumers must resolve those
//! references; eagerly expanding recursive expressions cannot terminate.
//! [`crate::bundles::query::package`] installs these definitions by migration.

#[macro_use]
mod support;
mod expression;
mod path;
#[path = "query.rs"]
mod query_codecs;
#[path = "schema.rs"]
mod schema_codecs;

use crate::schema::{Type, TypeKind};
use crate::value::{FromValue, FromValueError, IntoValue, Object, SemanticType, Value};
use crate::{expr, query, schema, value};
use std::collections::BTreeMap;
use support::*;

pub const MODULE_NAME: &str = "query";

/// All named definitions reachable from the public query AST.
/// References deliberately stop expansion, making this graph finite despite recursion.
pub fn definitions() -> BTreeMap<String, schema::TypeDef> {
    let definitions = vec![
        definition::<expr::BetweenExpr>(),
        definition::<expr::BinaryExpr>(),
        definition::<expr::BinaryOperator>(),
        definition::<expr::CallArg>(),
        definition::<expr::CallExpr>(),
        definition::<expr::Callee>(),
        definition::<expr::CaseBranch>(),
        definition::<expr::CaseExpr>(),
        definition::<expr::CastExpr>(),
        definition::<expr::Delete>(),
        definition::<expr::ExistsExpr>(),
        definition::<expr::Expr>(),
        definition::<expr::FieldAccessExpr>(),
        definition::<expr::FromItem>(),
        definition::<expr::IfExpr>(),
        definition::<expr::InExpr>(),
        definition::<expr::InSet>(),
        definition::<expr::IndexAccessExpr>(),
        definition::<expr::IsNullExpr>(),
        definition::<expr::JoinExpr>(),
        definition::<expr::JoinKind>(),
        definition::<expr::LambdaExpr>(),
        definition::<expr::LambdaParam>(),
        definition::<expr::LetBinding>(),
        definition::<expr::LetExpr>(),
        definition::<expr::LikeExpr>(),
        definition::<expr::LikeKind>(),
        definition::<expr::ListExpr>(),
        definition::<expr::LiteralExpr>(),
        definition::<expr::MapEntryExpr>(),
        definition::<expr::MapExpr>(),
        definition::<expr::NullsOrder>(),
        definition::<expr::OrderByExpr>(),
        definition::<expr::ParameterRef>(),
        definition::<expr::Query>(),
        definition::<expr::RefExpr>(),
        definition::<expr::RegexExpr>(),
        definition::<expr::Select>(),
        definition::<expr::SelectExpr>(),
        definition::<expr::SubqueryExpr>(),
        definition::<expr::TupleExpr>(),
        definition::<expr::UnaryExpr>(),
        definition::<expr::UnaryOperator>(),
        definition::<expr::Update>(),
        definition::<expr::UpdateAssignment>(),
        definition::<expr::VariableRef>(),
        definition::<expr::WindowFrame>(),
        definition::<expr::WindowFrameBound>(),
        definition::<expr::WindowFrameUnits>(),
        definition::<expr::WindowSpec>(),
        definition::<query::AggregateOp>(),
        definition::<query::Assignment>(),
        definition::<query::BinaryOp>(),
        definition::<query::DdlBatch>(),
        definition::<query::DdlCollectionKind>(),
        definition::<query::DdlOperation>(),
        definition::<query::DdlQuery>(),
        definition::<query::DeleteQuery>(),
        definition::<query::Expr>(),
        definition::<query::FieldFormat>(),
        definition::<query::FunctionArg<query::Expr>>(),
        definition::<query::InsertQuery>(),
        definition::<query::InsertSource>(),
        definition::<query::IntegrityMode>(),
        definition::<query::JoinCondition>(),
        definition::<query::JoinQuery>(),
        definition::<query::JoinSource>(),
        definition::<query::JoinType>(),
        definition::<query::Operand>(),
        definition::<query::OrderBy>(),
        definition::<query::PatternMatchKind>(),
        definition::<query::Query>(),
        definition::<query::QueryField>(),
        definition::<query::QueryInput>(),
        definition::<query::SelectQuery>(),
        definition::<query::SortDirection>(),
        definition::<query::TextAnalyzer>(),
        definition::<query::TextMatchMode>(),
        definition::<query::TextQueryFormat>(),
        definition::<query::UnaryOp>(),
        definition::<query::UpdateQuery>(),
        definition::<schema::Annotation>(),
        definition::<schema::AnnotationValue>(),
        definition::<schema::AnyType>(),
        definition::<schema::ArrayType>(),
        definition::<schema::AttributeRef>(),
        definition::<schema::AttributeType>(),
        definition::<schema::BigIntType>(),
        definition::<schema::BigUIntType>(),
        definition::<schema::BoolType>(),
        definition::<schema::BytesEncoding>(),
        definition::<schema::BytesType>(),
        definition::<schema::CharType>(),
        definition::<schema::Charset>(),
        definition::<schema::ClassAttribute>(),
        definition::<schema::ClassConstraint>(),
        definition::<schema::ClassRef>(),
        definition::<schema::ClassType>(),
        definition::<schema::ComplexType>(),
        definition::<schema::Constraint>(),
        definition::<schema::DecimalEncoding>(),
        definition::<schema::DecimalType>(),
        definition::<schema::core::meta::Deprecation>(),
        definition::<schema::EntityRef>(),
        definition::<schema::EnumRepr>(),
        definition::<schema::EnumType>(),
        definition::<schema::EnumVariant>(),
        definition::<schema::ExtensionType>(),
        definition::<schema::Field>(),
        definition::<schema::FloatWidth>(),
        definition::<schema::FunctionParam>(),
        definition::<schema::FunctionType>(),
        definition::<schema::HandleMode>(),
        definition::<schema::HandleType>(),
        definition::<schema::IndexKind>(),
        definition::<schema::IntWidth>(),
        definition::<schema::InterfaceMethod>(),
        definition::<schema::InterfaceType>(),
        definition::<schema::IntersectionType>(),
        definition::<schema::IpAddrType>(),
        definition::<schema::constraints::constraint::LegacyReferenceConstraint>(),
        definition::<schema::LengthSpec>(),
        definition::<schema::ListType>(),
        definition::<schema::MapType>(),
        definition::<schema::Meta>(),
        definition::<schema::NeverType>(),
        definition::<schema::NullType>(),
        definition::<schema::NumberBound>(),
        definition::<schema::NumberType>(),
        definition::<schema::OnDelete>(),
        definition::<schema::OpaqueType>(),
        definition::<schema::OptionalType>(),
        definition::<schema::RationalType>(),
        definition::<schema::RecordType>(),
        definition::<schema::RelationIndexingMode>(),
        definition::<schema::RelationMode>(),
        definition::<schema::RelationType>(),
        definition::<schema::ResultType>(),
        definition::<schema::SetType>(),
        definition::<schema::StreamType>(),
        definition::<schema::StringFormat>(),
        definition::<schema::StringType>(),
        definition::<schema::TemporalType>(),
        definition::<schema::TimeUnit>(),
        definition::<schema::TimeZoneSpec>(),
        definition::<schema::TimestampType>(),
        definition::<schema::TransportFormat>(),
        definition::<schema::TupleType>(),
        definition::<schema::Type>(),
        definition::<schema::TypeDef>(),
        definition::<schema::TypeKind>(),
        definition::<schema::TypeParam>(),
        definition::<schema::TypeRef>(),
        definition::<schema::UIntWidth>(),
        definition::<schema::UnicodeNormalization>(),
        definition::<schema::UnionType>(),
        definition::<schema::UnknownType>(),
        definition::<schema::VariantCase>(),
        definition::<schema::VariantPayload>(),
        definition::<schema::VariantTag>(),
        definition::<schema::VariantType>(),
        definition::<schema::Visibility>(),
        definition::<value::FieldPath>(),
        definition::<value::PathSegment>(),
    ];
    definitions
        .into_iter()
        .map(|definition| (definition.name.clone(), definition))
        .collect()
}

#[cfg(test)]
mod tests;

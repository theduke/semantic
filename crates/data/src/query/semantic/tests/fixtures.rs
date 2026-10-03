use super::*;
trait Sample {
    fn samples() -> Vec<Self>
    where
        Self: Sized;
    fn sample() -> Self
    where
        Self: Sized,
    {
        Self::samples().remove(0)
    }
}
macro_rules! scalar { ($($ty:ty => $value:expr),* $(,)?) => { $(impl Sample for $ty { fn samples() -> Vec<Self> { vec![$value] } })* }; }
scalar!(String => "sample".into(), bool => true, usize => 2, u32 => 3, u64 => 4, i32 => -2, i64 => -3, Value => dynamic_values(), Object => [("payload".into(), dynamic_values())].into_iter().collect());
impl<T: Sample> Sample for Box<T> {
    fn samples() -> Vec<Self> {
        vec![Box::new(T::sample())]
    }
}
impl<T: Sample> Sample for Option<T> {
    fn samples() -> Vec<Self> {
        vec![Some(T::sample())]
    }
}
impl<T: Sample> Sample for Vec<T> {
    fn samples() -> Vec<Self> {
        vec![vec![T::sample()]]
    }
}
impl<T: Sample> Sample for BTreeMap<String, T> {
    fn samples() -> Vec<Self> {
        vec![[("sample".into(), T::sample())].into_iter().collect()]
    }
}
macro_rules! record_samples { ($ty:path, {$($field:ident : $fty:ty),* $(,)?}) => { impl Sample for $ty { fn samples() -> Vec<Self> { vec![Self { $($field: <$fty>::sample()),* }] } } }; }
macro_rules! variant_sample { ($v:ident) => { Self::$v }; ($v:ident, ($ty:ty)) => { Self::$v(<$ty>::sample()) }; ($v:ident, {$($f:ident : $ty:ty),* $(,)?}) => { Self::$v { $($f: <$ty>::sample()),* } }; }
macro_rules! variant_samples {
    ($ty:path, {$first:ident $(($firstty:ty))? $({$($firstfield:ident : $firstfty:ty),* $(,)?})? $(, $variant:ident $(($vty:ty))? $({$($field:ident : $fty:ty),* $(,)?})?)* $(,)?}) => {
        impl Sample for $ty {
            fn sample() -> Self { variant_sample!($first $(, ($firstty))? $(, {$($firstfield: $firstfty),*})?) }
            fn samples() -> Vec<Self> { vec![Self::sample(), $(variant_sample!($variant $(, ($vty))? $(, {$($field: $fty),*})?)),*] }
        }
    };
}

impl Sample for value::FieldPath {
    fn samples() -> Vec<Self> {
        vec![Self(vec![
            value::PathSegment::Field("field".into()),
            value::PathSegment::Index(2),
        ])]
    }
}

record_samples!(expr::BetweenExpr, {value: expr::Expr, lower: expr::Expr, upper: expr::Expr, negated: bool});
record_samples!(expr::BinaryExpr, {op: expr::BinaryOperator, left: expr::Expr, right: expr::Expr});
variant_samples!(expr::BinaryOperator, {Add, Sub, Mul, Div, Mod, Pow, Eq, Ne, Lt, Le, Gt, Ge, And, Or, BitAnd, BitOr, BitXor, ShiftLeft, ShiftRight, Concat, Coalesce});
variant_samples!(expr::CallArg, {Positional(expr::Expr), Named {name: String, value: expr::Expr}});
record_samples!(expr::CallExpr, {callee: expr::Callee, args: Vec<expr::CallArg>, over: Option<expr::WindowSpec>});
variant_samples!(expr::Callee, {Name(Vec<String>), Expr(expr::Expr)});
record_samples!(expr::CaseBranch, {when: expr::Expr, then_expr: expr::Expr});
record_samples!(expr::CaseExpr, {operand: Option<expr::Expr>, branches: Vec<expr::CaseBranch>, else_expr: Option<expr::Expr>});
record_samples!(expr::CastExpr, {expr: expr::Expr, to: schema::Type, safe: bool});
record_samples!(expr::Delete, {from: Vec<String>, selection: Option<expr::Expr>, returning: Vec<expr::SelectExpr>, limit: Option<u64>});
record_samples!(expr::ExistsExpr, {query: Box<expr::Select>, negated: bool});
variant_samples!(expr::Expr, {Literal(expr::LiteralExpr), Ref(expr::RefExpr), Unary(Box<expr::UnaryExpr>), Binary(Box<expr::BinaryExpr>), Call(Box<expr::CallExpr>), FieldAccess(Box<expr::FieldAccessExpr>), IndexAccess(Box<expr::IndexAccessExpr>), Tuple(expr::TupleExpr), List(expr::ListExpr), Map(expr::MapExpr), Cast(Box<expr::CastExpr>), If(Box<expr::IfExpr>), Case(Box<expr::CaseExpr>), Let(Box<expr::LetExpr>), Lambda(Box<expr::LambdaExpr>), Between(Box<expr::BetweenExpr>), In(Box<expr::InExpr>), Like(Box<expr::LikeExpr>), Regex(Box<expr::RegexExpr>), IsNull(Box<expr::IsNullExpr>), Exists(Box<expr::ExistsExpr>), Query(Box<expr::Query>), Subquery(Box<expr::SubqueryExpr>)});
record_samples!(expr::FieldAccessExpr, {target: expr::Expr, field: String});
variant_samples!(expr::FromItem, {Table {name: Vec<String>, alias: Option<String>}, Subquery {query: Box<expr::Select>, alias: String}, Join(expr::JoinExpr)});
record_samples!(expr::IfExpr, {condition: expr::Expr, then_expr: expr::Expr, else_expr: expr::Expr});
record_samples!(expr::InExpr, {value: expr::Expr, set: expr::InSet, negated: bool});
variant_samples!(expr::InSet, {Exprs(Vec<expr::Expr>), Subquery(Box<expr::Select>)});
record_samples!(expr::IndexAccessExpr, {target: expr::Expr, index: expr::Expr});
record_samples!(expr::IsNullExpr, {value: expr::Expr, negated: bool});
record_samples!(expr::JoinExpr, {left: Box<expr::FromItem>, right: Box<expr::FromItem>, kind: expr::JoinKind, on: Option<expr::Expr>});
variant_samples!(expr::JoinKind, {Inner, Left, Right, Full, Cross});
record_samples!(expr::LambdaExpr, {params: Vec<expr::LambdaParam>, body: expr::Expr});
record_samples!(expr::LambdaParam, {name: String, ty: Option<schema::Type>});
record_samples!(expr::LetBinding, {name: String, value: expr::Expr});
record_samples!(expr::LetExpr, {bindings: Vec<expr::LetBinding>, body: expr::Expr});
record_samples!(expr::LikeExpr, {kind: expr::LikeKind, value: expr::Expr, pattern: expr::Expr, escape: Option<expr::Expr>, case_insensitive: bool, negated: bool});
variant_samples!(expr::LikeKind, {Like, SimilarTo});
record_samples!(expr::ListExpr, {items: Vec<expr::Expr>});
record_samples!(expr::LiteralExpr, {value: value::Value});
record_samples!(expr::MapEntryExpr, {key: expr::Expr, value: expr::Expr});
record_samples!(expr::MapExpr, {entries: Vec<expr::MapEntryExpr>});
variant_samples!(expr::NullsOrder, {First, Last});
record_samples!(expr::OrderByExpr, {expr: expr::Expr, direction: query::SortDirection, nulls: Option<expr::NullsOrder>});
record_samples!(expr::ParameterRef, {name: String});
variant_samples!(expr::Query, {Select(expr::Select), Update(expr::Update), Delete(expr::Delete)});
variant_samples!(expr::RefExpr, {Identifier(String), Qualified(Vec<String>), Parameter(expr::ParameterRef), Variable(expr::VariableRef), CurrentRow});
record_samples!(expr::RegexExpr, {value: expr::Expr, pattern: expr::Expr, case_insensitive: bool, negated: bool});
record_samples!(expr::Select, {projection: Vec<expr::SelectExpr>, from: Vec<expr::FromItem>, selection: Option<expr::Expr>, group_by: Vec<expr::Expr>, having: Option<expr::Expr>, order_by: Vec<expr::OrderByExpr>, limit: Option<u64>, offset: Option<u64>, distinct: bool});
record_samples!(expr::SelectExpr, {expr: expr::Expr, alias: Option<String>});
record_samples!(expr::SubqueryExpr, {query: Box<expr::Select>});
record_samples!(expr::TupleExpr, {items: Vec<expr::Expr>});
record_samples!(expr::UnaryExpr, {op: expr::UnaryOperator, operand: expr::Expr});
variant_samples!(expr::UnaryOperator, {Plus, Minus, Not, BitNot});
record_samples!(expr::Update, {table: Vec<String>, assignments: Vec<expr::UpdateAssignment>, selection: Option<expr::Expr>, returning: Vec<expr::SelectExpr>, limit: Option<u64>});
record_samples!(expr::UpdateAssignment, {target: Vec<String>, value: expr::Expr});
record_samples!(expr::VariableRef, {name: String});
record_samples!(expr::WindowFrame, {units: expr::WindowFrameUnits, start: expr::WindowFrameBound, end: Option<expr::WindowFrameBound>});
variant_samples!(expr::WindowFrameBound, {UnboundedPreceding, Preceding(u64), CurrentRow, Following(u64), UnboundedFollowing});
variant_samples!(expr::WindowFrameUnits, {Rows, Range, Groups});
record_samples!(expr::WindowSpec, {partition_by: Vec<expr::Expr>, order_by: Vec<expr::OrderByExpr>, frame: Option<expr::WindowFrame>});
variant_samples!(query::AggregateOp, {Count, Sum, Avg, Min, Max});
record_samples!(query::Assignment, {path: value::FieldPath, value: query::Expr});
variant_samples!(query::BinaryOp, {Add, Sub, Mul, Div, Mod, Concat, And, Or, Eq, NotEq, Lt, Lte, Gt, Gte, In});
record_samples!(query::DdlBatch, {operations: Vec<query::DdlOperation>});
variant_samples!(query::DdlCollectionKind, {Untyped, Schema, Polymorphic});
variant_samples!(query::DdlOperation, {UpsertAttribute {attribute: schema::AttributeType}, DeleteAttribute {id: String}, UpsertTypeDef {type_def: schema::TypeDef}, DeleteTypeDef {name: String}, UpsertRecordType {id: String, name: String, record: schema::RecordType}, DeleteRecordType {id: String}, UpsertClass {class: schema::ClassType}, DeleteClass {id: String}, UpsertCollection {name: String, kind: query::DdlCollectionKind, integrity_mode: query::IntegrityMode}, DeleteCollection {name: String}, UpsertIndex {name: String, collection: String, field: String, unique: bool, kind: schema::IndexKind, extra_fields: Vec<String>, predicate: Option<query::Expr>, analyzer: query::TextAnalyzer}, DeleteIndex {name: String, collection: String}, UpsertRelationship {relationship: schema::RelationType}, DeleteRelationship {id: String}, SetAutoIndex {enabled: bool}});
record_samples!(query::DdlQuery, {batch: query::DdlBatch});
record_samples!(query::DeleteQuery, {collection: Option<String>, predicate: Option<query::Expr>, limit: Option<query::Expr>, returning: Vec<query::QueryField>, field_format: query::FieldFormat});
variant_samples!(query::Expr, {Operand(query::Operand), ProjectionRef(Box<query::Expr>), Unary {op: query::UnaryOp, expr: Box<query::Expr>}, Binary {op: query::BinaryOp, left: Box<query::Expr>, right: Box<query::Expr>}, IfElse {cond: Box<query::Expr>, then_expr: Box<query::Expr>, else_expr: Box<query::Expr>}, Coalesce(Vec<query::Expr>), Function {name: String, args: Vec<query::FunctionArg<query::Expr>>}, Aggregate {op: query::AggregateOp, distinct: bool, arg: Box<query::FunctionArg<query::Expr>>}, InList {expr: Box<query::Expr>, list: Vec<query::Expr>, negated: bool}, Subquery(Box<query::SelectQuery>), Between {expr: Box<query::Expr>, low: Box<query::Expr>, high: Box<query::Expr>, negated: bool}, PatternMatch {kind: query::PatternMatchKind, expr: Box<query::Expr>, pattern: Box<query::Expr>, case_insensitive: bool, negated: bool}, RegexMatch {expr: Box<query::Expr>, pattern: Box<query::Expr>, case_insensitive: bool, negated: bool}, TextMatch {exprs: Vec<query::Expr>, query: Box<query::Expr>, mode: query::TextMatchMode, analyzer: query::TextAnalyzer}, IsNull {expr: Box<query::Expr>, negated: bool}, Exists {query: Box<query::SelectQuery>, negated: bool}, RelationExists {relation: Box<query::Expr>, source: Box<query::Expr>, target: Box<query::Expr>, transitive: bool, max_depth: Option<Box<query::Expr>>}});
variant_samples!(query::FieldFormat, {Qualified, Underscore, Plain});
variant_samples!(query::FunctionArg<query::Expr>, {Expr(query::Expr), Wildcard});
record_samples!(query::InsertQuery, {collection: Option<String>, columns: Vec<String>, source: query::InsertSource, returning: Vec<query::QueryField>, field_format: query::FieldFormat});
variant_samples!(query::InsertSource, {Objects(Vec<value::Object>), Values(Vec<Vec<query::Expr>>), Select(query::SelectQuery)});
variant_samples!(query::IntegrityMode, {Permissive, StrictRegisteredSchema});
variant_samples!(query::JoinCondition, {OnExpr(query::Expr), UsingFields {left: value::FieldPath, right: value::FieldPath}});
record_samples!(query::JoinQuery, {source: query::JoinSource, alias: Option<String>, join_type: query::JoinType, condition: query::JoinCondition, predicate: Option<query::Expr>});
record_samples!(query::JoinSource, {collection: Option<String>, class: Option<String>});
variant_samples!(query::JoinType, {Inner, Left, Right, Full});
variant_samples!(query::Operand, {Field(value::FieldPath), Literal(value::Value), Parameter(String)});
record_samples!(query::OrderBy, {expr: query::Expr, direction: query::SortDirection});
variant_samples!(query::PatternMatchKind, {Like, SimilarTo});
variant_samples!(query::Query, {Select(query::SelectQuery), Insert(query::InsertQuery), Update(query::UpdateQuery), Delete(query::DeleteQuery), Ddl(query::DdlQuery)});
record_samples!(query::QueryField, {expr: Box<query::Expr>, alias: Option<String>, wildcard: Option<value::FieldPath>});
variant_samples!(query::QueryInput, {Ast(query::Query), AstWithParams {query: query::Query, params: BTreeMap<String, value::Value>}, Text {format: query::TextQueryFormat, query: String, params: BTreeMap<String, value::Value>}});
record_samples!(query::SelectQuery, {collection: Option<String>, source_alias: Option<String>, joins: Vec<query::JoinQuery>, predicate: Option<query::Expr>, projection: Vec<query::QueryField>, distinct: bool, group_by: Vec<query::Expr>, having: Option<query::Expr>, order_by: Vec<query::OrderBy>, offset: query::Expr, limit: Option<query::Expr>, field_format: query::FieldFormat});
variant_samples!(query::SortDirection, {Asc, Desc});
record_samples!(query::TextAnalyzer, {stemming: bool, min_token_len: u32});
variant_samples!(query::TextMatchMode, {All, Any});
variant_samples!(query::TextQueryFormat, {Sql, Prql});
variant_samples!(query::UnaryOp, {Not, Neg});
record_samples!(query::UpdateQuery, {collection: Option<String>, predicate: Option<query::Expr>, assignments: Vec<query::Assignment>, limit: Option<query::Expr>, returning: Vec<query::QueryField>, field_format: query::FieldFormat});
record_samples!(schema::Annotation, {key: String, value: schema::AnnotationValue});
variant_samples!(schema::AnnotationValue, {Bool(bool), Number(String), String(String), List(Vec<schema::AnnotationValue>), Map(BTreeMap<String, schema::AnnotationValue>)});
impl Sample for schema::AnyType {
    fn samples() -> Vec<Self> {
        vec![Self]
    }
}
record_samples!(schema::ArrayType, {items: Box<schema::Type>, length: Option<schema::LengthSpec>});
record_samples!(schema::AttributeRef, {id: String});
record_samples!(schema::AttributeType, {id: String, name: String, ty: schema::Type, constraints: Vec<schema::Constraint>, meta: schema::Meta});
record_samples!(schema::BigIntType, {min_bits: Option<u32>, max_bits: Option<u32>});
record_samples!(schema::BigUIntType, {min_bits: Option<u32>, max_bits: Option<u32>});
impl Sample for schema::BoolType {
    fn samples() -> Vec<Self> {
        vec![Self]
    }
}
variant_samples!(schema::BytesEncoding, {Raw, Base64, Base64Url, Hex, Ascii85});
record_samples!(schema::BytesType, {encoding: Option<schema::BytesEncoding>});
record_samples!(schema::CharType, {unicode_scalar: bool});
variant_samples!(schema::Charset, {Utf8, Utf16, Ascii, Latin1, Custom(String)});
record_samples!(schema::ClassAttribute, {attribute: schema::AttributeRef, required: bool, ui_order: Option<u32>, computed: Option<expr::Expr>, default: Option<expr::Expr>, constraints: Vec<schema::Constraint>, meta: schema::Meta});
variant_samples!(schema::ClassConstraint, {Field {attribute: schema::AttributeRef, constraint: schema::Constraint}, MultiFieldExpr {expr: expr::Expr, description: Option<String>}});
record_samples!(schema::ClassRef, {id: String});
record_samples!(schema::ClassType, {id: String, name: String, inherits: Option<schema::ClassRef>, extends: Vec<schema::ClassRef>, strict_schema: bool, creatable_in_ui: Option<bool>, include_in_ui_listings: Option<bool>, attributes: BTreeMap<String, schema::ClassAttribute>, constraints: Vec<schema::ClassConstraint>, meta: schema::Meta});
record_samples!(schema::ComplexType, {component: schema::FloatWidth});
variant_samples!(schema::Constraint, {Min(schema::NumberBound), Max(schema::NumberBound), MultipleOf(String), Length(schema::LengthSpec), Pattern(String), Prefix(String), Suffix(String), Contains(value::Value), Precision {precision: u32, scale: u32}, Charset(schema::Charset), Collation(String), TimeZone(schema::TimeZoneSpec), MinItems(u64), MaxItems(u64), MinProperties(u64), MaxProperties(u64), RequiredFields(Vec<String>), KeyPattern(String), Unique, Distinct, PrimaryKey, LegacyForeignKey(schema::constraints::constraint::LegacyReferenceConstraint), Index {name: Option<String>, fields: Vec<String>, unique: bool}, DefaultValue {value: value::Value}, DefaultExpr {expr: expr::Expr}, Transport {format: schema::TransportFormat, media_type: Option<String>}});
variant_samples!(schema::DecimalEncoding, {Base10, Scientific, BinaryCodedDecimal, String});
record_samples!(schema::DecimalType, {precision: Option<u32>, scale: Option<i32>, encoding: schema::DecimalEncoding});
record_samples!(schema::core::meta::Deprecation, {note: Option<String>});
record_samples!(schema::EntityRef, {target: Option<String>, legacy_args: Vec<schema::Type>, on_delete: schema::OnDelete});
variant_samples!(schema::EnumRepr, {String, Int});
record_samples!(schema::EnumType, {repr: schema::EnumRepr, variants: Vec<schema::EnumVariant>});
record_samples!(schema::EnumVariant, {name: String, value: Option<i64>, symbol: Option<String>, meta: schema::Meta});
record_samples!(schema::ExtensionType, {namespace: String, name: String, payload: BTreeMap<String, String>});
record_samples!(schema::Field, {ty: schema::Type, required: bool, readonly: bool, writeonly: bool, default: Option<value::Value>, meta: schema::Meta});
variant_samples!(schema::FloatWidth, {F16, F32, F64, F80, F128, Decimal32, Decimal64, Decimal128});
record_samples!(schema::FunctionParam, {name: Option<String>, ty: schema::Type});
record_samples!(schema::FunctionType, {params: Vec<schema::FunctionParam>, results: Vec<schema::Type>, throws: Option<Box<schema::Type>>, async_fn: bool});
variant_samples!(schema::HandleMode, {Own, Borrow});
record_samples!(schema::HandleType, {interface: schema::TypeRef, mode: schema::HandleMode});
variant_samples!(schema::IndexKind, {Equality, PathEquality, Range, FullText});
variant_samples!(schema::IntWidth, {I8, I16, I24, I32, I40, I48, I56, I64, I128, I256});
record_samples!(schema::InterfaceMethod, {name: String, signature: schema::FunctionType});
record_samples!(schema::InterfaceType, {methods: Vec<schema::InterfaceMethod>});
record_samples!(schema::IntersectionType, {variants: Vec<schema::Type>});
variant_samples!(schema::IpAddrType, {V4, V6, Any});
record_samples!(schema::constraints::constraint::LegacyReferenceConstraint, {to: schema::TypeRef, fields: Vec<String>});
variant_samples!(schema::LengthSpec, {Exactly(u64), Range {min: Option<u64>, max: Option<u64>}});
record_samples!(schema::ListType, {items: Box<schema::Type>});
record_samples!(schema::MapType, {keys: Box<schema::Type>, values: Box<schema::Type>, ordered: bool});
record_samples!(schema::Meta, {title: Option<String>, description: Option<String>, id: Option<String>, deprecated: Option<schema::core::meta::Deprecation>, aliases: Vec<String>, examples: Vec<value::Value>, tags: Vec<String>, docs_url: Option<String>, annotations: BTreeMap<String, String>});
impl Sample for schema::NeverType {
    fn samples() -> Vec<Self> {
        vec![Self]
    }
}
impl Sample for schema::NullType {
    fn samples() -> Vec<Self> {
        vec![Self]
    }
}
variant_samples!(schema::NumberBound, {Inclusive(String), Exclusive(String)});
variant_samples!(schema::NumberType, {Int(schema::IntWidth), UInt(schema::UIntWidth), Float(schema::FloatWidth), BigInt(schema::BigIntType), BigUInt(schema::BigUIntType), Decimal(schema::DecimalType), Rational(schema::RationalType), Complex(schema::ComplexType), Unspecified});
variant_samples!(schema::OnDelete, {Restrict, Cascade});
record_samples!(schema::OpaqueType, {id: String, domain: Option<String>, repr: Option<String>});
record_samples!(schema::OptionalType, {inner: Box<schema::Type>});
record_samples!(schema::RationalType, {numerator: Option<Box<schema::NumberType>>, denominator: Option<Box<schema::NumberType>>});
record_samples!(schema::RecordType, {fields: BTreeMap<String, schema::Field>, open: bool, additional: Option<Box<schema::Type>>, required_order: Option<Vec<String>>});
variant_samples!(schema::RelationIndexingMode, {Disabled, Enabled});
variant_samples!(schema::RelationMode, {Embedded {attribute: String}, External});
record_samples!(schema::RelationType, {id: String, name: String, source_collection: String, mode: schema::RelationMode, indexing_mode: schema::RelationIndexingMode, meta: schema::Meta});
record_samples!(schema::ResultType, {ok: Box<schema::Type>, err: Box<schema::Type>});
record_samples!(schema::SetType, {items: Box<schema::Type>});
record_samples!(schema::StreamType, {element: Box<schema::Type>, end: Option<Box<schema::Type>>});
variant_samples!(schema::StringFormat, {Email, Uri, Url, Hostname, Regex, Uuid, Base64, Hex, Ascii, Utf8, JsonPointer, JsonPath, Sql, Custom(String)});
record_samples!(schema::StringType, {format: Option<schema::StringFormat>, normalization: Option<schema::UnicodeNormalization>});
variant_samples!(schema::TemporalType, {Date, Time, DateTime, Duration, Period, Timestamp(schema::TimestampType), Instant});
variant_samples!(schema::TimeUnit, {Seconds, Millis, Micros, Nanos});
variant_samples!(schema::TimeZoneSpec, {Required, Forbidden, Allowed, Specific(String)});
record_samples!(schema::TimestampType, {unit: schema::TimeUnit, timezone: schema::TimeZoneSpec});
variant_samples!(schema::TransportFormat, {Json, Json5, Cbor, MessagePack, Bincode, Avro, Protobuf, Flatbuffers, Arrow, Parquet, Xml, Yaml, Toml, Csv, Sql, Custom(String)});
record_samples!(schema::TupleType, {items: Vec<schema::Type>, rest: Option<Box<schema::Type>>});
record_samples!(schema::Type, {kind: schema::TypeKind, constraints: Vec<schema::Constraint>, annotations: Vec<schema::Annotation>});
record_samples!(schema::TypeDef, {name: String, module: Option<String>, params: Vec<schema::TypeParam>, ty: schema::Type, visibility: schema::Visibility, meta: schema::Meta});
variant_samples!(schema::TypeKind, {Any(schema::AnyType), Never(schema::NeverType), Unknown(schema::UnknownType), Null(schema::NullType), Bool(schema::BoolType), Char(schema::CharType), Number(schema::NumberType), String(schema::StringType), Bytes(schema::BytesType), Temporal(schema::TemporalType), Uuid, IpAddr(schema::IpAddrType), Json, Optional(schema::OptionalType), Array(schema::ArrayType), List(schema::ListType), Tuple(schema::TupleType), Map(schema::MapType), Set(schema::SetType), Record(schema::RecordType), Attribute(Box<schema::AttributeType>), Class(schema::ClassType), Union(schema::UnionType), Intersection(schema::IntersectionType), Variant(schema::VariantType), Enum(schema::EnumType), Result(schema::ResultType), Function(schema::FunctionType), Interface(schema::InterfaceType), Handle(schema::HandleType), Stream(schema::StreamType), Opaque(schema::OpaqueType), Extension(schema::ExtensionType), Named(schema::TypeRef), Ref(schema::EntityRef)});
record_samples!(schema::TypeParam, {name: String, bounds: Vec<schema::TypeRef>, default: Option<schema::Type>});
record_samples!(schema::TypeRef, {name: String, args: Vec<schema::Type>});
variant_samples!(schema::UIntWidth, {U8, U16, U24, U32, U40, U48, U56, U64, U128, U256});
variant_samples!(schema::UnicodeNormalization, {Nfc, Nfd, Nfkc, Nfkd});
record_samples!(schema::UnionType, {variants: Vec<schema::Type>});
impl Sample for schema::UnknownType {
    fn samples() -> Vec<Self> {
        vec![Self]
    }
}
record_samples!(schema::VariantCase, {name: String, payload: schema::VariantPayload, discriminant: Option<value::Value>, meta: schema::Meta});
variant_samples!(schema::VariantPayload, {Unit, Tuple(Vec<schema::Type>), Record(schema::RecordType), Newtype(Box<schema::Type>)});
variant_samples!(schema::VariantTag, {ExternallyTagged, InternallyTagged {field: String}, AdjacentlyTagged {tag_field: String, data_field: String}, Untagged});
record_samples!(schema::VariantType, {tag: schema::VariantTag, variants: Vec<schema::VariantCase>});
variant_samples!(schema::Visibility, {Public, Internal, Private});
variant_samples!(value::PathSegment, {Field(String), Index(usize)});
#[test]
fn all_public_ast_and_payload_variants_round_trip() {
    for sample in <expr::BetweenExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::BinaryExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::BinaryOperator>::samples() {
        round_trip(sample);
    }
    for sample in <expr::CallArg>::samples() {
        round_trip(sample);
    }
    for sample in <expr::CallExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::Callee>::samples() {
        round_trip(sample);
    }
    for sample in <expr::CaseBranch>::samples() {
        round_trip(sample);
    }
    for sample in <expr::CaseExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::CastExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::Delete>::samples() {
        round_trip(sample);
    }
    for sample in <expr::ExistsExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::Expr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::FieldAccessExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::FromItem>::samples() {
        round_trip(sample);
    }
    for sample in <expr::IfExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::InExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::InSet>::samples() {
        round_trip(sample);
    }
    for sample in <expr::IndexAccessExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::IsNullExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::JoinExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::JoinKind>::samples() {
        round_trip(sample);
    }
    for sample in <expr::LambdaExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::LambdaParam>::samples() {
        round_trip(sample);
    }
    for sample in <expr::LetBinding>::samples() {
        round_trip(sample);
    }
    for sample in <expr::LetExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::LikeExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::LikeKind>::samples() {
        round_trip(sample);
    }
    for sample in <expr::ListExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::LiteralExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::MapEntryExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::MapExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::NullsOrder>::samples() {
        round_trip(sample);
    }
    for sample in <expr::OrderByExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::ParameterRef>::samples() {
        round_trip(sample);
    }
    for sample in <expr::Query>::samples() {
        round_trip(sample);
    }
    for sample in <expr::RefExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::RegexExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::Select>::samples() {
        round_trip(sample);
    }
    for sample in <expr::SelectExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::SubqueryExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::TupleExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::UnaryExpr>::samples() {
        round_trip(sample);
    }
    for sample in <expr::UnaryOperator>::samples() {
        round_trip(sample);
    }
    for sample in <expr::Update>::samples() {
        round_trip(sample);
    }
    for sample in <expr::UpdateAssignment>::samples() {
        round_trip(sample);
    }
    for sample in <expr::VariableRef>::samples() {
        round_trip(sample);
    }
    for sample in <expr::WindowFrame>::samples() {
        round_trip(sample);
    }
    for sample in <expr::WindowFrameBound>::samples() {
        round_trip(sample);
    }
    for sample in <expr::WindowFrameUnits>::samples() {
        round_trip(sample);
    }
    for sample in <expr::WindowSpec>::samples() {
        round_trip(sample);
    }
    for sample in <query::AggregateOp>::samples() {
        round_trip(sample);
    }
    for sample in <query::Assignment>::samples() {
        round_trip(sample);
    }
    for sample in <query::BinaryOp>::samples() {
        round_trip(sample);
    }
    for sample in <query::DdlBatch>::samples() {
        round_trip(sample);
    }
    for sample in <query::DdlCollectionKind>::samples() {
        round_trip(sample);
    }
    for sample in <query::DdlOperation>::samples() {
        round_trip(sample);
    }
    for sample in <query::DdlQuery>::samples() {
        round_trip(sample);
    }
    for sample in <query::DeleteQuery>::samples() {
        round_trip(sample);
    }
    for sample in <query::Expr>::samples() {
        round_trip(sample);
    }
    for sample in <query::FieldFormat>::samples() {
        round_trip(sample);
    }
    for sample in <query::FunctionArg<query::Expr>>::samples() {
        round_trip(sample);
    }
    for sample in <query::InsertQuery>::samples() {
        round_trip(sample);
    }
    for sample in <query::InsertSource>::samples() {
        round_trip(sample);
    }
    for sample in <query::IntegrityMode>::samples() {
        round_trip(sample);
    }
    for sample in <query::JoinCondition>::samples() {
        round_trip(sample);
    }
    for sample in <query::JoinQuery>::samples() {
        round_trip(sample);
    }
    for sample in <query::JoinSource>::samples() {
        round_trip(sample);
    }
    for sample in <query::JoinType>::samples() {
        round_trip(sample);
    }
    for sample in <query::Operand>::samples() {
        round_trip(sample);
    }
    for sample in <query::OrderBy>::samples() {
        round_trip(sample);
    }
    for sample in <query::PatternMatchKind>::samples() {
        round_trip(sample);
    }
    for sample in <query::Query>::samples() {
        round_trip(sample);
    }
    for sample in <query::QueryField>::samples() {
        round_trip(sample);
    }
    for sample in <query::QueryInput>::samples() {
        round_trip(sample);
    }
    for sample in <query::SelectQuery>::samples() {
        round_trip(sample);
    }
    for sample in <query::SortDirection>::samples() {
        round_trip(sample);
    }
    for sample in <query::TextAnalyzer>::samples() {
        round_trip(sample);
    }
    for sample in <query::TextMatchMode>::samples() {
        round_trip(sample);
    }
    for sample in <query::TextQueryFormat>::samples() {
        round_trip(sample);
    }
    for sample in <query::UnaryOp>::samples() {
        round_trip(sample);
    }
    for sample in <query::UpdateQuery>::samples() {
        round_trip(sample);
    }
    for sample in <schema::Annotation>::samples() {
        round_trip(sample);
    }
    for sample in <schema::AnnotationValue>::samples() {
        round_trip(sample);
    }
    for sample in <schema::AnyType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ArrayType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::AttributeRef>::samples() {
        round_trip(sample);
    }
    for sample in <schema::AttributeType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::BigIntType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::BigUIntType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::BoolType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::BytesEncoding>::samples() {
        round_trip(sample);
    }
    for sample in <schema::BytesType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::CharType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::Charset>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ClassAttribute>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ClassConstraint>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ClassRef>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ClassType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ComplexType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::Constraint>::samples() {
        round_trip(sample);
    }
    for sample in <schema::DecimalEncoding>::samples() {
        round_trip(sample);
    }
    for sample in <schema::DecimalType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::core::meta::Deprecation>::samples() {
        round_trip(sample);
    }
    for sample in <schema::EntityRef>::samples() {
        round_trip(sample);
    }
    for sample in <schema::EnumRepr>::samples() {
        round_trip(sample);
    }
    for sample in <schema::EnumType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::EnumVariant>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ExtensionType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::Field>::samples() {
        round_trip(sample);
    }
    for sample in <schema::FloatWidth>::samples() {
        round_trip(sample);
    }
    for sample in <schema::FunctionParam>::samples() {
        round_trip(sample);
    }
    for sample in <schema::FunctionType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::HandleMode>::samples() {
        round_trip(sample);
    }
    for sample in <schema::HandleType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::IndexKind>::samples() {
        round_trip(sample);
    }
    for sample in <schema::IntWidth>::samples() {
        round_trip(sample);
    }
    for sample in <schema::InterfaceMethod>::samples() {
        round_trip(sample);
    }
    for sample in <schema::InterfaceType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::IntersectionType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::IpAddrType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::constraints::constraint::LegacyReferenceConstraint>::samples() {
        round_trip(sample);
    }
    for sample in <schema::LengthSpec>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ListType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::MapType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::Meta>::samples() {
        round_trip(sample);
    }
    for sample in <schema::NeverType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::NullType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::NumberBound>::samples() {
        round_trip(sample);
    }
    for sample in <schema::NumberType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::OnDelete>::samples() {
        round_trip(sample);
    }
    for sample in <schema::OpaqueType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::OptionalType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::RationalType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::RecordType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::RelationIndexingMode>::samples() {
        round_trip(sample);
    }
    for sample in <schema::RelationMode>::samples() {
        round_trip(sample);
    }
    for sample in <schema::RelationType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::ResultType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::SetType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::StreamType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::StringFormat>::samples() {
        round_trip(sample);
    }
    for sample in <schema::StringType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TemporalType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TimeUnit>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TimeZoneSpec>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TimestampType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TransportFormat>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TupleType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::Type>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TypeDef>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TypeKind>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TypeParam>::samples() {
        round_trip(sample);
    }
    for sample in <schema::TypeRef>::samples() {
        round_trip(sample);
    }
    for sample in <schema::UIntWidth>::samples() {
        round_trip(sample);
    }
    for sample in <schema::UnicodeNormalization>::samples() {
        round_trip(sample);
    }
    for sample in <schema::UnionType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::UnknownType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::VariantCase>::samples() {
        round_trip(sample);
    }
    for sample in <schema::VariantPayload>::samples() {
        round_trip(sample);
    }
    for sample in <schema::VariantTag>::samples() {
        round_trip(sample);
    }
    for sample in <schema::VariantType>::samples() {
        round_trip(sample);
    }
    for sample in <schema::Visibility>::samples() {
        round_trip(sample);
    }
    for sample in <value::FieldPath>::samples() {
        round_trip(sample);
    }
    for sample in <value::PathSegment>::samples() {
        round_trip(sample);
    }
}

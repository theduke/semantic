use super::*;

ast_record!(schema::Annotation, "semantic:schema:Annotation", {
    key: String => "key" [required],
    value: schema::AnnotationValue => "value" [required],
});

ast_variant!(schema::AnnotationValue, "semantic:schema:AnnotationValue", {
    Bool => "bool" (inner: bool),
    Number => "number" (inner: String),
    String => "string" (inner: String),
    List => "list" (inner: Vec<schema::AnnotationValue>),
    Map => "map" (inner: BTreeMap<String, schema::AnnotationValue>),
});

ast_unit!(schema::AnyType, "semantic:schema:AnyType");

ast_record!(schema::ArrayType, "semantic:schema:ArrayType", {
    items: Box<schema::Type> => "items" [required],
    length: Option<schema::LengthSpec> => "length" [default None],
});

ast_record!(schema::AttributeRef, "semantic:schema:AttributeRef", {
    id: String => "id" [required],
});

ast_record!(schema::AttributeType, "semantic:schema:AttributeType", {
    id: String => "id" [required],
    name: String => "name" [required],
    ty: schema::Type => "ty" [required],
    constraints: Vec<schema::Constraint> => "constraints" [required],
    meta: schema::Meta => "meta" [required],
});

ast_record!(schema::BigIntType, "semantic:schema:BigIntType", {
    min_bits: Option<u32> => "min_bits" [default None],
    max_bits: Option<u32> => "max_bits" [default None],
});

ast_record!(schema::BigUIntType, "semantic:schema:BigUIntType", {
    min_bits: Option<u32> => "min_bits" [default None],
    max_bits: Option<u32> => "max_bits" [default None],
});

ast_unit!(schema::BoolType, "semantic:schema:BoolType");

ast_enum!(schema::BytesEncoding, "semantic:schema:BytesEncoding", {
    Raw => "raw",
    Base64 => "base64",
    Base64Url => "base64_url",
    Hex => "hex",
    Ascii85 => "ascii85",
});

ast_record!(schema::BytesType, "semantic:schema:BytesType", {
    encoding: Option<schema::BytesEncoding> => "encoding" [default None],
});

ast_record!(schema::CharType, "semantic:schema:CharType", {
    unicode_scalar: bool => "unicode_scalar" [required],
});

ast_variant!(schema::Charset, "semantic:schema:Charset", {
    Utf8 => "utf8",
    Utf16 => "utf16",
    Ascii => "ascii",
    Latin1 => "latin1",
    Custom => "custom" (inner: String),
});

ast_record!(schema::ClassAttribute, "semantic:schema:ClassAttribute", {
    attribute: schema::AttributeRef => "attribute" [required],
    required: bool => "required" [required],
    ui_order: Option<u32> => "ui_order" [default None],
    computed: Option<expr::Expr> => "computed" [default None],
    default: Option<expr::Expr> => "default" [default None],
    constraints: Vec<schema::Constraint> => "constraints" [required],
    meta: schema::Meta => "meta" [required],
});

ast_variant!(schema::ClassConstraint, "semantic:schema:ClassConstraint", {
    Field => "field" {
        attribute: schema::AttributeRef => "attribute" [required],
        constraint: schema::Constraint => "constraint" [required],
    },
    MultiFieldExpr => "multi_field_expr" {
        expr: expr::Expr => "expr" [required],
        description: Option<String> => "description" [default None],
    },
});

ast_record!(schema::ClassRef, "semantic:schema:ClassRef", {
    id: String => "id" [required],
});

ast_record!(schema::ClassType, "semantic:schema:ClassType", {
    id: String => "id" [required],
    name: String => "name" [required],
    inherits: Option<schema::ClassRef> => "inherits" [default None],
    extends: Vec<schema::ClassRef> => "extends" [required],
    strict_schema: bool => "semantic:class:strict_schema" [default Default::default()],
    creatable_in_ui: Option<bool> => "semantic:ui:creatable_in_ui" [default None],
    include_in_ui_listings: Option<bool> => "semantic:ui:include_in_listings" [default None],
    attributes: BTreeMap<String, schema::ClassAttribute> => "attributes" [required],
    constraints: Vec<schema::ClassConstraint> => "constraints" [required],
    meta: schema::Meta => "meta" [required],
});

ast_record!(schema::ComplexType, "semantic:schema:ComplexType", {
    component: schema::FloatWidth => "component" [required],
});

ast_variant!(schema::Constraint, "semantic:schema:Constraint", {
    Min => "min" (inner: schema::NumberBound),
    Max => "max" (inner: schema::NumberBound),
    MultipleOf => "multiple_of" (inner: String),
    Length => "length" (inner: schema::LengthSpec),
    Pattern => "pattern" (inner: String),
    Prefix => "prefix" (inner: String),
    Suffix => "suffix" (inner: String),
    Contains => "contains" (inner: value::Value),
    Precision => "precision" {
        precision: u32 => "precision" [required],
        scale: u32 => "scale" [required],
    },
    Charset => "charset" (inner: schema::Charset),
    Collation => "collation" (inner: String),
    TimeZone => "time_zone" (inner: schema::TimeZoneSpec),
    MinItems => "min_items" (inner: u64),
    MaxItems => "max_items" (inner: u64),
    MinProperties => "min_properties" (inner: u64),
    MaxProperties => "max_properties" (inner: u64),
    RequiredFields => "required_fields" (inner: Vec<String>),
    KeyPattern => "key_pattern" (inner: String),
    Unique => "unique",
    Distinct => "distinct",
    PrimaryKey => "primary_key",
    LegacyForeignKey => "foreign_key" (inner: schema::constraints::constraint::LegacyReferenceConstraint),
    Index => "index" {
        name: Option<String> => "name" [default None],
        fields: Vec<String> => "fields" [required],
        unique: bool => "unique" [required],
    },
    DefaultValue => "default_value" {
        value: value::Value => "value" [required],
    },
    DefaultExpr => "default_expr" {
        expr: expr::Expr => "expr" [required],
    },
    Transport => "transport" {
        format: schema::TransportFormat => "format" [required],
        media_type: Option<String> => "media_type" [default None],
    },
});

ast_enum!(schema::DecimalEncoding, "semantic:schema:DecimalEncoding", {
    Base10 => "base10",
    Scientific => "scientific",
    BinaryCodedDecimal => "binary_coded_decimal",
    String => "string",
});

ast_record!(schema::DecimalType, "semantic:schema:DecimalType", {
    precision: Option<u32> => "precision" [default None],
    scale: Option<i32> => "scale" [default None],
    encoding: schema::DecimalEncoding => "encoding" [required],
});

ast_record!(schema::core::meta::Deprecation, "semantic:schema:Deprecation", {
    note: Option<String> => "note" [default None],
});

ast_record!(schema::EntityRef, "semantic:schema:EntityRef", {
    target: Option<String> => "name" [default None],
    legacy_args: Vec<schema::Type> => "args" [default Default::default()],
    on_delete: schema::OnDelete => "on_delete" [default schema::OnDelete::Restrict],
});

ast_enum!(schema::EnumRepr, "semantic:schema:EnumRepr", {
    String => "string",
    Int => "int",
});

ast_record!(schema::EnumType, "semantic:schema:EnumType", {
    repr: schema::EnumRepr => "repr" [required],
    variants: Vec<schema::EnumVariant> => "variants" [required],
});

ast_record!(schema::EnumVariant, "semantic:schema:EnumVariant", {
    name: String => "name" [required],
    value: Option<i64> => "value" [default None],
    symbol: Option<String> => "symbol" [default None],
    meta: schema::Meta => "meta" [required],
});

ast_record!(schema::ExtensionType, "semantic:schema:ExtensionType", {
    namespace: String => "namespace" [required],
    name: String => "name" [required],
    payload: BTreeMap<String, String> => "payload" [required],
});

ast_record!(schema::Field, "semantic:schema:Field", {
    ty: schema::Type => "ty" [required],
    required: bool => "required" [required],
    readonly: bool => "readonly" [required],
    writeonly: bool => "writeonly" [required],
    default: Option<value::Value> => "default" [presence],
    meta: schema::Meta => "meta" [required],
});

ast_enum!(schema::FloatWidth, "semantic:schema:FloatWidth", {
    F16 => "f16",
    F32 => "f32",
    F64 => "f64",
    F80 => "f80",
    F128 => "f128",
    Decimal32 => "decimal32",
    Decimal64 => "decimal64",
    Decimal128 => "decimal128",
});

ast_record!(schema::FunctionParam, "semantic:schema:FunctionParam", {
    name: Option<String> => "name" [default None],
    ty: schema::Type => "ty" [required],
});

ast_record!(schema::FunctionType, "semantic:schema:FunctionType", {
    params: Vec<schema::FunctionParam> => "params" [required],
    results: Vec<schema::Type> => "results" [required],
    throws: Option<Box<schema::Type>> => "throws" [default None],
    async_fn: bool => "async_fn" [required],
});

ast_enum!(schema::HandleMode, "semantic:schema:HandleMode", {
    Own => "own",
    Borrow => "borrow",
});

ast_record!(schema::HandleType, "semantic:schema:HandleType", {
    interface: schema::TypeRef => "interface" [required],
    mode: schema::HandleMode => "mode" [required],
});

ast_enum!(schema::IndexKind, "semantic:schema:IndexKind", {
    Equality => "equality",
    PathEquality => "path_equality",
    Range => "range",
    FullText => "full_text",
});

ast_enum!(schema::IntWidth, "semantic:schema:IntWidth", {
    I8 => "i8",
    I16 => "i16",
    I24 => "i24",
    I32 => "i32",
    I40 => "i40",
    I48 => "i48",
    I56 => "i56",
    I64 => "i64",
    I128 => "i128",
    I256 => "i256",
});

ast_record!(schema::InterfaceMethod, "semantic:schema:InterfaceMethod", {
    name: String => "name" [required],
    signature: schema::FunctionType => "signature" [required],
});

ast_record!(schema::InterfaceType, "semantic:schema:InterfaceType", {
    methods: Vec<schema::InterfaceMethod> => "methods" [required],
});

ast_record!(schema::IntersectionType, "semantic:schema:IntersectionType", {
    variants: Vec<schema::Type> => "variants" [required],
});

ast_enum!(schema::IpAddrType, "semantic:schema:IpAddrType", {
    V4 => "v4",
    V6 => "v6",
    Any => "any",
});

ast_record!(schema::constraints::constraint::LegacyReferenceConstraint, "semantic:schema:LegacyReferenceConstraint", {
    to: schema::TypeRef => "to" [required],
    fields: Vec<String> => "fields" [required],
});

ast_variant!(schema::LengthSpec, "semantic:schema:LengthSpec", {
    Exactly => "exactly" (inner: u64),
    Range => "range" {
        min: Option<u64> => "min" [default None],
        max: Option<u64> => "max" [default None],
    },
});

ast_record!(schema::ListType, "semantic:schema:ListType", {
    items: Box<schema::Type> => "items" [required],
});

ast_record!(schema::MapType, "semantic:schema:MapType", {
    keys: Box<schema::Type> => "keys" [required],
    values: Box<schema::Type> => "values" [required],
    ordered: bool => "ordered" [required],
});

ast_record!(schema::Meta, "semantic:schema:Meta", {
    title: Option<String> => "title" [default None],
    description: Option<String> => "description" [default None],
    id: Option<String> => "id" [default None],
    deprecated: Option<schema::core::meta::Deprecation> => "deprecated" [default None],
    aliases: Vec<String> => "aliases" [required],
    examples: Vec<value::Value> => "examples" [required],
    tags: Vec<String> => "tags" [required],
    docs_url: Option<String> => "docs_url" [default None],
    annotations: BTreeMap<String, String> => "annotations" [required],
});

ast_unit!(schema::NeverType, "semantic:schema:NeverType");

ast_unit!(schema::NullType, "semantic:schema:NullType");

ast_variant!(schema::NumberBound, "semantic:schema:NumberBound", {
    Inclusive => "inclusive" (inner: String),
    Exclusive => "exclusive" (inner: String),
});

ast_variant!(schema::NumberType, "semantic:schema:NumberType", {
    Int => "int" (inner: schema::IntWidth),
    UInt => "u_int" (inner: schema::UIntWidth),
    Float => "float" (inner: schema::FloatWidth),
    BigInt => "big_int" (inner: schema::BigIntType),
    BigUInt => "big_u_int" (inner: schema::BigUIntType),
    Decimal => "decimal" (inner: schema::DecimalType),
    Rational => "rational" (inner: schema::RationalType),
    Complex => "complex" (inner: schema::ComplexType),
    Unspecified => "unspecified",
});

ast_enum!(schema::OnDelete, "semantic:schema:OnDelete", {
    Restrict => "restrict",
    Cascade => "cascade",
});

ast_record!(schema::OpaqueType, "semantic:schema:OpaqueType", {
    id: String => "id" [required],
    domain: Option<String> => "domain" [default None],
    repr: Option<String> => "repr" [default None],
});

ast_record!(schema::OptionalType, "semantic:schema:OptionalType", {
    inner: Box<schema::Type> => "inner" [required],
});

ast_record!(schema::RationalType, "semantic:schema:RationalType", {
    numerator: Option<Box<schema::NumberType>> => "numerator" [default None],
    denominator: Option<Box<schema::NumberType>> => "denominator" [default None],
});

ast_record!(schema::RecordType, "semantic:schema:RecordType", {
    fields: BTreeMap<String, schema::Field> => "fields" [required],
    open: bool => "open" [required],
    additional: Option<Box<schema::Type>> => "additional" [default None],
    required_order: Option<Vec<String>> => "required_order" [default None],
});

ast_enum!(schema::RelationIndexingMode, "semantic:schema:RelationIndexingMode", {
    Disabled => "disabled",
    Enabled => "enabled",
});

ast_variant!(schema::RelationMode, "semantic:schema:RelationMode", {
    Embedded => "embedded" {
        attribute: String => "attribute" [required],
    },
    External => "external",
});

ast_record!(schema::RelationType, "semantic:schema:RelationType", {
    id: String => "id" [required],
    name: String => "name" [required],
    source_collection: String => "source_collection" [required],
    mode: schema::RelationMode => "mode" [required],
    indexing_mode: schema::RelationIndexingMode => "indexing_mode" [required],
    meta: schema::Meta => "meta" [required],
});

ast_record!(schema::ResultType, "semantic:schema:ResultType", {
    ok: Box<schema::Type> => "ok" [required],
    err: Box<schema::Type> => "err" [required],
});

ast_record!(schema::SetType, "semantic:schema:SetType", {
    items: Box<schema::Type> => "items" [required],
});

ast_record!(schema::StreamType, "semantic:schema:StreamType", {
    element: Box<schema::Type> => "element" [required],
    end: Option<Box<schema::Type>> => "end" [default None],
});

ast_variant!(schema::StringFormat, "semantic:schema:StringFormat", {
    Email => "email",
    Uri => "uri",
    Url => "url",
    Hostname => "hostname",
    Regex => "regex",
    Uuid => "uuid",
    Base64 => "base64",
    Hex => "hex",
    Ascii => "ascii",
    Utf8 => "utf8",
    JsonPointer => "json_pointer",
    JsonPath => "json_path",
    Sql => "sql",
    Custom => "custom" (inner: String),
});

ast_record!(schema::StringType, "semantic:schema:StringType", {
    format: Option<schema::StringFormat> => "format" [default None],
    normalization: Option<schema::UnicodeNormalization> => "normalization" [default None],
});

ast_variant!(schema::TemporalType, "semantic:schema:TemporalType", {
    Date => "date",
    Time => "time",
    DateTime => "date_time",
    Duration => "duration",
    Period => "period",
    Timestamp => "timestamp" (inner: schema::TimestampType),
    Instant => "instant",
});

ast_enum!(schema::TimeUnit, "semantic:schema:TimeUnit", {
    Seconds => "seconds",
    Millis => "millis",
    Micros => "micros",
    Nanos => "nanos",
});

ast_variant!(schema::TimeZoneSpec, "semantic:schema:TimeZoneSpec", {
    Required => "required",
    Forbidden => "forbidden",
    Allowed => "allowed",
    Specific => "specific" (inner: String),
});

ast_record!(schema::TimestampType, "semantic:schema:TimestampType", {
    unit: schema::TimeUnit => "unit" [required],
    timezone: schema::TimeZoneSpec => "timezone" [required],
});

ast_variant!(schema::TransportFormat, "semantic:schema:TransportFormat", {
    Json => "json",
    Json5 => "json5",
    Cbor => "cbor",
    MessagePack => "message_pack",
    Bincode => "bincode",
    Avro => "avro",
    Protobuf => "protobuf",
    Flatbuffers => "flatbuffers",
    Arrow => "arrow",
    Parquet => "parquet",
    Xml => "xml",
    Yaml => "yaml",
    Toml => "toml",
    Csv => "csv",
    Sql => "sql",
    Custom => "custom" (inner: String),
});

ast_record!(schema::TupleType, "semantic:schema:TupleType", {
    items: Vec<schema::Type> => "items" [required],
    rest: Option<Box<schema::Type>> => "rest" [default None],
});

ast_record!(schema::Type, "semantic:schema:Type", {
    kind: schema::TypeKind => "kind" [required],
    constraints: Vec<schema::Constraint> => "constraints" [required],
    annotations: Vec<schema::Annotation> => "annotations" [required],
});

ast_record!(schema::TypeDef, "semantic:schema:TypeDef", {
    name: String => "name" [required],
    module: Option<String> => "module" [default None],
    params: Vec<schema::TypeParam> => "params" [required],
    ty: schema::Type => "ty" [required],
    visibility: schema::Visibility => "visibility" [required],
    meta: schema::Meta => "meta" [required],
});

ast_variant!(schema::TypeKind, "semantic:schema:TypeKind", {
    Any => "any" (inner: schema::AnyType),
    Never => "never" (inner: schema::NeverType),
    Unknown => "unknown" (inner: schema::UnknownType),
    Null => "null" (inner: schema::NullType),
    Bool => "bool" (inner: schema::BoolType),
    Char => "char" (inner: schema::CharType),
    Number => "number" (inner: schema::NumberType),
    String => "string" (inner: schema::StringType),
    Bytes => "bytes" (inner: schema::BytesType),
    Temporal => "temporal" (inner: schema::TemporalType),
    Uuid => "uuid",
    IpAddr => "ip_addr" (inner: schema::IpAddrType),
    Json => "json",
    Optional => "optional" (inner: schema::OptionalType),
    Array => "array" (inner: schema::ArrayType),
    List => "list" (inner: schema::ListType),
    Tuple => "tuple" (inner: schema::TupleType),
    Map => "map" (inner: schema::MapType),
    Set => "set" (inner: schema::SetType),
    Record => "record" (inner: schema::RecordType),
    Attribute => "attribute" (inner: Box<schema::AttributeType>),
    Class => "class" (inner: schema::ClassType),
    Union => "union" (inner: schema::UnionType),
    Intersection => "intersection" (inner: schema::IntersectionType),
    Variant => "variant" (inner: schema::VariantType),
    Enum => "enum" (inner: schema::EnumType),
    Result => "result" (inner: schema::ResultType),
    Function => "function" (inner: schema::FunctionType),
    Interface => "interface" (inner: schema::InterfaceType),
    Handle => "handle" (inner: schema::HandleType),
    Stream => "stream" (inner: schema::StreamType),
    Opaque => "opaque" (inner: schema::OpaqueType),
    Extension => "extension" (inner: schema::ExtensionType),
    Named => "named" (inner: schema::TypeRef),
    Ref => "ref" (inner: schema::EntityRef),
});

ast_record!(schema::TypeParam, "semantic:schema:TypeParam", {
    name: String => "name" [required],
    bounds: Vec<schema::TypeRef> => "bounds" [required],
    default: Option<schema::Type> => "default" [default None],
});

ast_record!(schema::TypeRef, "semantic:schema:TypeRef", {
    name: String => "name" [required],
    args: Vec<schema::Type> => "args" [required],
});

ast_enum!(schema::UIntWidth, "semantic:schema:UIntWidth", {
    U8 => "u8",
    U16 => "u16",
    U24 => "u24",
    U32 => "u32",
    U40 => "u40",
    U48 => "u48",
    U56 => "u56",
    U64 => "u64",
    U128 => "u128",
    U256 => "u256",
});

ast_enum!(schema::UnicodeNormalization, "semantic:schema:UnicodeNormalization", {
    Nfc => "nfc",
    Nfd => "nfd",
    Nfkc => "nfkc",
    Nfkd => "nfkd",
});

ast_record!(schema::UnionType, "semantic:schema:UnionType", {
    variants: Vec<schema::Type> => "variants" [required],
});

ast_unit!(schema::UnknownType, "semantic:schema:UnknownType");

ast_record!(schema::VariantCase, "semantic:schema:VariantCase", {
    name: String => "name" [required],
    payload: schema::VariantPayload => "payload" [required],
    discriminant: Option<value::Value> => "discriminant" [presence],
    meta: schema::Meta => "meta" [required],
});

ast_variant!(schema::VariantPayload, "semantic:schema:VariantPayload", {
    Unit => "unit",
    Tuple => "tuple" (inner: Vec<schema::Type>),
    Record => "record" (inner: schema::RecordType),
    Newtype => "newtype" (inner: Box<schema::Type>),
});

ast_variant!(schema::VariantTag, "semantic:schema:VariantTag", {
    ExternallyTagged => "externally_tagged",
    InternallyTagged => "internally_tagged" {
        field: String => "field" [required],
    },
    AdjacentlyTagged => "adjacently_tagged" {
        tag_field: String => "tag_field" [required],
        data_field: String => "data_field" [required],
    },
    Untagged => "untagged",
});

ast_record!(schema::VariantType, "semantic:schema:VariantType", {
    tag: schema::VariantTag => "tag" [required],
    variants: Vec<schema::VariantCase> => "variants" [required],
});

ast_enum!(schema::Visibility, "semantic:schema:Visibility", {
    Public => "public",
    Internal => "internal",
    Private => "private",
});

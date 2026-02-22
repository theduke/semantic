#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TypeKind {
    Any(crate::schema::primitives::any_type::AnyType),
    Never(crate::schema::primitives::never_type::NeverType),
    Unknown(crate::schema::primitives::unknown_type::UnknownType),

    Null(crate::schema::primitives::null_type::NullType),
    Bool(crate::schema::primitives::bool_type::BoolType),
    Char(crate::schema::primitives::char_type::CharType),

    Number(crate::schema::primitives::number_type::NumberType),
    String(crate::schema::primitives::string_type::StringType),
    Bytes(crate::schema::primitives::bytes_type::BytesType),
    Temporal(crate::schema::primitives::temporal_type::TemporalType),

    Uuid,
    IpAddr(crate::schema::primitives::ip_addr_type::IpAddrType),
    Json,

    Optional(crate::schema::collections::optional_type::OptionalType),

    Array(crate::schema::collections::array_type::ArrayType),
    List(crate::schema::collections::list_type::ListType),
    Tuple(crate::schema::collections::tuple_type::TupleType),
    Map(crate::schema::collections::map_type::MapType),
    Set(crate::schema::collections::set_type::SetType),

    Record(crate::schema::record::record_type::RecordType),
    Attribute(Box<crate::schema::attribute::attribute_type::AttributeType>),
    Class(crate::schema::class::class_type::ClassType),

    Union(crate::schema::union::union_type::UnionType),
    Intersection(crate::schema::intersection::intersection_type::IntersectionType),
    Variant(crate::schema::variant::variant_type::VariantType),
    Enum(crate::schema::r#enum::enum_type::EnumType),

    Result(crate::schema::algebraic::result_type::ResultType),
    Function(crate::schema::behavior::function_type::FunctionType),
    Interface(crate::schema::behavior::interface_type::InterfaceType),
    Handle(crate::schema::handle::handle_type::HandleType),
    Stream(crate::schema::behavior::stream_type::StreamType),

    Opaque(crate::schema::primitives::opaque_type::OpaqueType),
    Extension(crate::schema::core::extension_type::ExtensionType),

    Ref(crate::schema::core::type_ref::TypeRef),
}

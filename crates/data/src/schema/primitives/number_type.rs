#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum NumberType {
    Int(crate::schema::primitives::int_width::IntWidth),
    UInt(crate::schema::primitives::uint_width::UIntWidth),
    Float(crate::schema::primitives::float_width::FloatWidth),
    BigInt(crate::schema::primitives::big_int_type::BigIntType),
    BigUInt(crate::schema::primitives::big_uint_type::BigUIntType),
    Decimal(crate::schema::primitives::decimal_type::DecimalType),
    Rational(crate::schema::primitives::rational_type::RationalType),
    Complex(crate::schema::primitives::complex_type::ComplexType),
    Unspecified,
}

// Lets the derive macros' `::semantic_data` paths resolve inside this crate.
extern crate self as semantic_data;

pub mod value;
pub use value::*;

pub mod filestore;
pub mod import;
pub mod jobs;
pub mod plugin;
pub mod vdb;

pub mod builtin;

pub mod bundles;

pub mod attr;

pub mod expr;
pub mod query;

pub mod schema;

pub enum CoreValueKind {
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    I256,
    U8,
    U16,
    U32,
    U64,
    U128,
    U256,
    F32,
    F64,

    Bytes,
    List,
    Map,
    Object,
}

pub enum ValueKind {
    // Core
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    I256,
    U8,
    U16,
    U32,
    U64,
    U128,
    U256,
    F32,
    F64,
    Bytes,

    Uuid,
    IpAddr,

    Duration,
    Time,
    Date,
    DateTime,
    String,
}

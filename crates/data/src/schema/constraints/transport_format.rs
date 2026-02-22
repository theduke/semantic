#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TransportFormat {
    Json,
    Json5,
    Cbor,
    MessagePack,
    Bincode,
    Avro,
    Protobuf,
    Flatbuffers,
    Arrow,
    Parquet,
    Xml,
    Yaml,
    Toml,
    Csv,
    Sql,
    Custom(String),
}

use factordb::{schema::DbSchema, AnyError};

pub mod media;

pub fn json_from_slice<T: serde::de::DeserializeOwned>(
    slice: &[u8],
) -> Result<T, serde_path_to_error::Error<serde_json::Error>> {
    let jd = &mut serde_json::Deserializer::from_slice(slice);
    serde_path_to_error::deserialize(jd)
}

pub fn generate_db_schema_typescript_definitions(schema: &DbSchema) -> Result<String, AnyError> {
    let ts = factor_tools::typescript::schema_to_typescript(&schema, None)?;
    Ok(ts)
}

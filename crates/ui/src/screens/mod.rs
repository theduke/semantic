mod catalog;
mod collection;
mod entity;
mod home;
mod query;

pub use catalog::CatalogScreen;
pub use collection::CollectionScreen;
pub use entity::EntityScreen;
pub use home::HomeScreen;
pub use query::QueryScreen;

fn value_string(value: &semantic_data::value::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{value:?}"))
}

use factordb::{query::migrate, Id, Ident};

pub mod api;
pub mod base;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PluginSchema {
    pub id: Id,
    pub name: String,
    pub description: Option<String>,
    pub db: factordb::schema::DbSchema,
}

pub trait PluginDescriptor {
    const NAME: &'static str;
    const IDENT: Ident = Ident::new_static(Self::NAME);

    fn schema() -> PluginSchema;

    fn build_upsert_migration() -> migrate::Migration {
        let schema = Self::schema();

        let attrs = schema.db.attributes.into_iter().map(|attr| {
            migrate::SchemaAction::AttributeUpsert(migrate::AttributeUpsert { schema: attr })
        });
        let entities = schema.db.entities.into_iter().map(|attr| {
            migrate::SchemaAction::EntityUpsert(migrate::EntityUpsert { schema: attr })
        });

        let actions = attrs.chain(entities).collect();

        migrate::Migration { actions }
    }
}

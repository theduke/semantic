use factordb::{
    prelude::{AttributeSchema, ValueType},
    query::{
        expr::Expr,
        migrate::{self, Migration},
        select::Select,
    },
    schema::{builtin::AttrIdent, AttributeDescriptor, EntityAttribute, EntityDescriptor},
    Attribute, Entity, Id,
};
use serde::{Deserialize, Serialize};

use crate::{
    base::AttrComment,
    plugin::{Plugin, PluginDescriptor, PluginSchema},
};

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "plugin_code")]
pub struct AttrPluginCode(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "plugin_runtime")]
pub struct AttrPluginRuntime(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "plugin_strict_validation")]
pub struct AttrPluginStrictValidation(bool);

#[derive(Serialize, Deserialize, Entity, Clone, Debug, PartialEq, Eq)]
#[factor(namespace = "semantic")]
pub struct PluginSource {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: String,

    #[factor(attr = AttrPluginRuntime)]
    #[serde(rename = "semantic/plugin_runtime")]
    pub runtime: Option<String>,

    #[factor(attr = AttrPluginCode)]
    #[serde(rename = "semantic/plugin_code")]
    pub code: Option<String>,

    #[factor(attr = AttrPluginStrictValidation)]
    #[serde(rename = "semantic/plugin_strict_validation", default)]
    pub strict_validation: bool,

    #[factor(attr = AttrComment)]
    #[serde(rename = "semantic/comment")]
    pub comment: Option<String>,
}

impl PluginSource {
    pub fn query_all() -> Select {
        Select::new().with_filter(Expr::is_entity::<PluginSource>())
    }
}

pub struct SemanticCorePlugin;

impl PluginDescriptor for SemanticCorePlugin {
    const NAME: &'static str = "semantic/core";
    const IDENT: factordb::Ident = factordb::Ident::new_static(Self::NAME);

    fn new() -> crate::plugin::DynPlugin {
        std::sync::Arc::new(Self)
    }
}

impl Plugin for SemanticCorePlugin {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn schema(&self) -> PluginSchema {
        PluginSchema {
            name: Self::NAME.into(),
            description: None,
            import_matchers: Vec::new(),
            db: Some(factordb::schema::DbSchema {
                attributes: vec![
                    AttrPluginCode::schema(),
                    AttrPluginRuntime::schema(),
                    AttrPluginStrictValidation::schema(),
                ],
                entities: vec![PluginSource::schema()],
                indexes: vec![],
            }),
        }
    }

    fn migrations(&self) -> Vec<factordb::query::migrate::Migration> {
        let create = Migration::with_name("semantic/core/v1".to_string())
            .attr_create(AttrPluginCode::schema())
            .attr_create(AttrPluginRuntime::schema())
            .entity_create(factordb::schema::EntitySchema {
                id: Id::nil(),
                ident: PluginSource::QUALIFIED_NAME.to_string(),
                title: Some("Plugin Source".to_string()),
                description: None,
                attributes: vec![
                    EntityAttribute {
                        attribute: AttrIdent::IDENT,
                        cardinality: factordb::schema::Cardinality::Required,
                    },
                    EntityAttribute {
                        attribute: AttrPluginRuntime::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrPluginCode::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                ],
                extends: vec![],
                strict: false,
            });

        let add_comment = Migration::with_name("add_comment_attr_to_pluginsource".to_string())
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: PluginSource::IDENT.to_string(),
                    attribute: AttrComment::IDENT.to_string(),
                    cardinality: factordb::schema::Cardinality::Optional,
                    default_value: None,
                },
            ));

        let create_strict_validation = Migration::with_name("create_strict_validation".to_string())
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: AttrPluginStrictValidation::QUALIFIED_NAME.to_string(),
                title: Some("Strict Plugin Validation".to_string()),
                description: None,
                value_type: ValueType::Bool,
                unique: false,
                index: false,
                strict: true,
            });

        let add_strict_validation =
            Migration::with_name("add_strict_validation_to_pluginsource".to_string()).action(
                migrate::SchemaAction::EntityAttributeAdd(migrate::EntityAttributeAdd {
                    entity: PluginSource::IDENT.to_string(),
                    attribute: AttrPluginStrictValidation::IDENT.to_string(),
                    cardinality: factordb::schema::Cardinality::Required,
                    default_value: Some(false.into()),
                }),
            );

        vec![
            create,
            add_comment,
            create_strict_validation,
            add_strict_validation,
        ]
    }
}

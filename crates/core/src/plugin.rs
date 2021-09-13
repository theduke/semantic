use factordb::{
    query::{migrate, select::Item},
    Id, Ident,
};
use url::Url;

/// Describes how an importer can handle a url.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum ImportSupport {
    /// Importer has dedicated (specific) support for the url.
    Dedicated { priority: u64 },
    /// Importer has generic support for the url.
    Generic { priority: u64 },
    /// Importer might support the url, but needs to do a more expensive check.
    MaybeSupported { priority: u64 },
}

impl PartialOrd for ImportSupport {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ImportSupport {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        match (self, other) {
            (
                ImportSupport::Dedicated { priority: a },
                ImportSupport::Dedicated { priority: b },
            ) => a.cmp(b),
            (ImportSupport::Dedicated { .. }, ImportSupport::Generic { .. }) => Ordering::Greater,
            (ImportSupport::Dedicated { .. }, ImportSupport::MaybeSupported { .. }) => {
                Ordering::Greater
            }
            (ImportSupport::Generic { .. }, ImportSupport::Dedicated { .. }) => Ordering::Less,
            (ImportSupport::Generic { priority: a }, ImportSupport::Generic { priority: b }) => {
                a.cmp(b)
            }
            (ImportSupport::Generic { .. }, ImportSupport::MaybeSupported { .. }) => {
                Ordering::Equal
            }
            (ImportSupport::MaybeSupported { .. }, ImportSupport::Dedicated { .. }) => {
                Ordering::Less
            }
            (ImportSupport::MaybeSupported { .. }, ImportSupport::Generic { .. }) => Ordering::Less,
            (
                ImportSupport::MaybeSupported { priority: a },
                ImportSupport::MaybeSupported { priority: b },
            ) => a.cmp(b),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ImporterMatch {
    pub plugin: String,
    pub support: ImportSupport,
}

pub struct ImportMatches {
    pub matches: Vec<ImporterMatch>,
}

impl ImportMatches {
    pub fn sort(&mut self) {
        self.matches.sort_by(|a, b| a.support.cmp(&b.support))
    }

    pub fn best(&self) -> Option<&ImporterMatch> {
        self.matches.first()
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ImportRelatedUrl {
    pub label: String,
    pub url: Url,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ImportOutput {
    pub plugin: String,

    /// Potentially nested items.
    pub items: Vec<Item>,
    /// The url where more items can be retrieved.
    pub load_more_url: Option<url::Url>,
    pub related_urls: Vec<ImportRelatedUrl>,
}

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

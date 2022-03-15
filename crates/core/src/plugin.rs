use factordb::{
    prelude::IdOrIdent,
    query::{migrate, select::Item},
    schema::DbSchema,
    AnyError,
};
use futures::future::BoxFuture;
use url::Url;

/// Describes how an importer can handle a url.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum UrlSupport {
    /// Importer has dedicated (specific) support for the url.
    Dedicated,
    /// Importer has generic support for the url.
    Generic { priority: u64 },
    /// Importer might support the url, but needs to do a more expensive check.
    MaybeSupported,
}

impl PartialOrd for UrlSupport {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for UrlSupport {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        match (self, other) {
            (UrlSupport::Dedicated, UrlSupport::Dedicated) => std::cmp::Ordering::Equal,
            (UrlSupport::Dedicated { .. }, UrlSupport::Generic { .. }) => Ordering::Greater,
            (UrlSupport::Dedicated { .. }, UrlSupport::MaybeSupported { .. }) => Ordering::Greater,
            (UrlSupport::Generic { .. }, UrlSupport::Dedicated { .. }) => Ordering::Less,
            (UrlSupport::Generic { priority: a }, UrlSupport::Generic { priority: b }) => a.cmp(b),
            (UrlSupport::Generic { .. }, UrlSupport::MaybeSupported { .. }) => Ordering::Equal,
            (UrlSupport::MaybeSupported { .. }, UrlSupport::Dedicated { .. }) => Ordering::Less,
            (UrlSupport::MaybeSupported { .. }, UrlSupport::Generic { .. }) => Ordering::Less,
            (UrlSupport::MaybeSupported, UrlSupport::MaybeSupported) => std::cmp::Ordering::Equal,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct UrlSupportMatch {
    pub plugin: String,
    pub support: UrlSupport,
}

pub struct UrlSupportMatches {
    pub matches: Vec<UrlSupportMatch>,
}

impl UrlSupportMatches {
    pub fn sort(&mut self) {
        self.matches.sort_by(|a, b| a.support.cmp(&b.support))
    }

    pub fn best(&self) -> Option<&UrlSupportMatch> {
        self.matches.first()
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RelatedUrl {
    pub label: String,
    pub url: Url,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct FetchUrlJob {
    pub url: Url,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct FetchUrlOutput {
    /// Potentially nested items.
    #[serde(default)]
    pub items: Vec<Item>,
    /// The url where more items can be retrieved.
    pub load_more_url: Option<RelatedUrl>,
    #[serde(default)]
    pub related_urls: Vec<RelatedUrl>,

    #[serde(default)]
    pub related_items: Vec<Item>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ImportJob {
    pub url: Url,
    pub import_media: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ImportOutput {
    pub items: Vec<Item>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PluginSchema {
    pub name: String,
    pub description: Option<String>,
    pub db: Option<factordb::schema::DbSchema>,
    #[serde(default)]
    pub import_matchers: Vec<ImportMatcherRule>,
}

impl PluginSchema {
    pub fn find_import_match(&self, url: &Url) -> Option<UrlSupport> {
        self.import_matchers
            .iter()
            .filter_map(|rule| {
                if rule.matcher.is_match(url) {
                    Some(rule.support.clone())
                } else {
                    None
                }
            })
            .max()
    }
}

// Describes what URLs a plugin can import.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ImportMatcher {
    /// Potentially supports all urls.
    /// Should be rarely used, eg for generic fallback importers that support
    /// all web pages
    All,
    /// Support for specific domains.
    Domains { domains: Vec<String> },
    // TODO: regex?
}

impl ImportMatcher {
    pub fn is_match(&self, url: &Url) -> bool {
        match self {
            Self::All => true,
            Self::Domains { domains } => domains
                .iter()
                .any(|domain| Some(domain.as_str()) == url.domain()),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ImportMatcherRule {
    pub matcher: ImportMatcher,
    pub support: UrlSupport,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct JavascriptRuntimeSpec {
    pub code: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum PluginRuntimeSpec {
    Javascript(JavascriptRuntimeSpec),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PluginSpec {
    pub runtime: PluginRuntimeSpec,
}

/// Static definition of a plugin.
///
/// Provides various plugin metadata.
pub trait PluginDescriptor {
    const NAME: &'static str;
    const IDENT: IdOrIdent = IdOrIdent::new_static(Self::NAME);

    fn new() -> DynPlugin;
}

/// A backend plugin that runs in the semantic backend.
///
/// Can be backed by different plugin runtimes.
pub trait Plugin {
    fn name(&self) -> &str;
    fn schema(&self) -> PluginSchema;
    fn migrations(&self) -> Vec<migrate::Migration>;

    /// Quickly check if a URL is supported.
    #[allow(unused_variables)]
    fn fetch_url_support(&self, url: &Url) -> Option<UrlSupport> {
        None
    }

    #[allow(unused_variables)]
    fn fetch_url(
        &self,
        job: FetchUrlJob,
    ) -> BoxFuture<'static, Result<Option<FetchUrlOutput>, AnyError>> {
        Box::pin(async move { Ok(None) })
    }

    #[allow(unused_variables)]
    fn import(&self, job: ImportJob) -> BoxFuture<'static, Result<Option<ImportOutput>, AnyError>> {
        Box::pin(async move { Ok(None) })
    }

    fn stop(&self) -> Result<(), AnyError> {
        Ok(())
    }
}

pub type DynPlugin = std::sync::Arc<dyn Plugin + Send + Sync>;

/// Build an upsert migration based on a database schema.
///
/// This is useful for test environments where you just want to create the
/// schema without having to run the individual migrations.
pub fn build_upsert_migration(schema: &DbSchema) -> migrate::Migration {
    let attrs = schema.attributes.iter().map(|attr| {
        migrate::SchemaAction::AttributeUpsert(migrate::AttributeUpsert {
            schema: attr.clone(),
        })
    });
    let entities = schema.entities.iter().map(|entity| {
        migrate::SchemaAction::EntityUpsert(migrate::EntityUpsert {
            schema: entity.clone(),
        })
    });

    let actions = attrs.chain(entities).collect();

    migrate::Migration {
        name: None,
        actions,
    }
}

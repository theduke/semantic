use factdb::{
    macros::{Attribute, Class},
    AttrIdent, DataMap, Id, Timestamp,
};

use super::{
    AttrCreatedAt, AttrDescription, AttrImportedAt, AttrTextContent, AttrTitle, AttrUpdatedAt,
    AttrUrl,
};

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Language",
    name = "code_snippet_language"
)]
pub struct AttrCodeSnippetLanguage(String);

#[derive(serde::Serialize, serde::Deserialize, Class, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct CodeSnippet {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<String>,

    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: Option<url::Url>,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,

    #[factor(attr = AttrDescription)]
    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[factor(attr = AttrTextContent)]
    #[serde(rename = "semantic/text_content")]
    pub text_content: String,

    #[factor(attr = AttrCodeSnippetLanguage)]
    #[serde(rename = "semantic/code_snippet_language")]
    pub language: Option<String>,

    #[factor(attr = AttrCreatedAt)]
    #[serde(rename = "semantic/created_at")]
    pub created_at: Option<Timestamp>,

    #[factor(attr = AttrUpdatedAt)]
    #[serde(rename = "semantic/updated_at")]
    pub updated_at: Option<Timestamp>,

    #[factor(attr = AttrImportedAt)]
    #[serde(rename = "semantic/imported_at")]
    pub imported_at: Option<Timestamp>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}

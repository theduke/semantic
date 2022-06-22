use factordb::prelude::{
    Attribute, AttributeDescriptor, AttributeSchema, DataMap, Entity, Id, Timestamp, ValueType,
};
use serde::{Deserialize, Serialize};

use super::{AttrDescription, AttrIdent, AttrName, AttrUrl};

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "given_name", title = "Given name")]
pub struct AttrGivenName(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "family_name", title = "Family name")]
pub struct AttrFamilyName(Id);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "birthdate", title = "Date of birth")]
pub struct AttrBirthDate(Id);

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gender {
    Male,
    Female,
}

impl AttributeDescriptor for Gender {
    const NAMESPACE: &'static str = "semantic";
    const PLAIN_NAME: &'static str = "gender";
    const QUALIFIED_NAME: &'static str = "semantic/gender";

    const IDENT: factordb::prelude::IdOrIdent =
        factordb::prelude::IdOrIdent::new_static(Self::QUALIFIED_NAME);

    type Type = Self;

    fn schema() -> AttributeSchema {
        AttributeSchema {
            id: Id::nil(),
            ident: Self::QUALIFIED_NAME.to_string(),
            title: Some("Gender".to_string()),
            description: None,
            value_type: ValueType::Union(vec![
                ValueType::Const("male".into()),
                ValueType::Const("female".into()),
            ]),
            unique: false,
            index: false,
            strict: false,
        }
    }
}

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantic", title = "Person")]
pub struct Person {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<String>,

    #[factor(attr = AttrDescription)]
    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: Option<url::Url>,

    #[factor(attr = AttrName)]
    #[serde(rename = "semantic/name")]
    pub name: Option<String>,

    #[factor(attr = AttrGivenName)]
    #[serde(rename = "semantic/given_name")]
    pub given_name: Option<String>,

    #[factor(attr = AttrFamilyName)]
    #[serde(rename = "semantic/family_name")]
    pub family_name: Option<String>,

    #[factor(attr = AttrBirthDate)]
    #[serde(rename = "semantic/birthdate")]
    pub birthdate: Option<Timestamp>,

    #[factor(attr = Gender)]
    #[serde(rename = "semantic/gender")]
    pub gender: Option<Gender>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}

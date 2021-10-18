use serde::{Deserialize, Serialize};

use factordb::{
    data::{DataMap, Timestamp},
    query::{expr::Expr, mutate::Mutate, select::Select},
    schema::{builtin::AttrType, AttributeDescriptor, EntityDescriptor},
    Attribute, Entity, Id,
};

use super::AttrDateTime;

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Weight")]
pub struct AttrWeight(f64);

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct WeightLogEntry {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrWeight)]
    #[serde(rename = "semantic/weight")]
    pub weight: f64,

    #[factor(attr = AttrDateTime)]
    #[serde(rename = "semantic/datetime")]
    pub datetime: Timestamp,
}

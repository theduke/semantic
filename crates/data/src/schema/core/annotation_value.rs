use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum AnnotationValue {
    Bool(bool),
    Number(String),
    String(String),
    List(Vec<AnnotationValue>),
    Map(BTreeMap<String, AnnotationValue>),
}

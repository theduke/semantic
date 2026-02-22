#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Annotation {
    pub key: String,
    pub value: crate::schema::core::annotation_value::AnnotationValue,
}

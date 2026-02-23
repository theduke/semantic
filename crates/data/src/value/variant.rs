use super::Value;

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantValue {
    pub r#type: Option<String>,
    pub variant: String,
    pub value: Value,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantValueRef<'a> {
    pub r#type: Option<&'a str>,
    pub variant: &'a str,
    pub value: &'a Value,
}

impl<'a> VariantValueRef<'a> {
    pub fn into_owned(self) -> VariantValue {
        VariantValue {
            r#type: self.r#type.map(|s| s.to_string()),
            variant: self.variant.to_string(),
            value: self.value.clone(),
        }
    }
}

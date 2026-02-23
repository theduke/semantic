#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum PathSegment {
    Field(String),
    Index(usize),
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[facet(transparent)]
pub struct FieldPath(pub Vec<PathSegment>);

impl FieldPath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_fields(parts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(
            parts
                .into_iter()
                .map(|part| PathSegment::Field(part.into()))
                .collect(),
        )
    }

    pub fn push_field(&mut self, field: impl Into<String>) {
        self.0.push(PathSegment::Field(field.into()));
    }

    pub fn push_index(&mut self, index: usize) {
        self.0.push(PathSegment::Index(index));
    }

    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }
}

impl From<Vec<PathSegment>> for FieldPath {
    fn from(value: Vec<PathSegment>) -> Self {
        Self(value)
    }
}

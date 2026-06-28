use std::rc::Rc;

use dioxus::prelude::Element;
use semantic_data::builtin::DEFAULT_COLLECTION;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityTarget {
    pub collection: Option<String>,
    pub id: String,
}

impl EntityTarget {
    pub fn collection_or_default(&self) -> &str {
        self.collection.as_deref().unwrap_or(DEFAULT_COLLECTION)
    }

    pub fn default_collection(id: impl Into<String>) -> Self {
        Self {
            collection: None,
            id: id.into(),
        }
    }

    pub fn new(collection: Option<String>, id: impl Into<String>) -> Self {
        Self {
            collection,
            id: id.into(),
        }
    }

    pub fn is_default_collection(&self) -> bool {
        self.collection
            .as_deref()
            .is_none_or(|collection| collection == DEFAULT_COLLECTION)
    }
}

#[derive(Clone, Default)]
pub struct EntityNavigation {
    pub href: Option<EntityHrefBuilder>,
    pub open: Option<EntityOpenHandler>,
    pub link_renderer: Option<EntityLinkRenderer>,
}

pub type EntityHrefBuilder = Rc<dyn Fn(&EntityTarget) -> Option<String>>;
pub type EntityOpenHandler = Rc<dyn Fn(EntityTarget)>;
pub type EntityLinkRenderer = Rc<dyn Fn(EntityTarget, Element) -> Element>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_target_collection_or_default_uses_default_for_none() {
        assert_eq!(
            EntityTarget::default_collection("id-1").collection_or_default(),
            DEFAULT_COLLECTION
        );
        assert_eq!(
            EntityTarget::new(Some("custom".to_string()), "id-1").collection_or_default(),
            "custom"
        );
    }
}

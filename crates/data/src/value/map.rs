use super::Value;

#[derive(facet::Facet, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[facet(transparent)]
pub struct Map(std::collections::BTreeMap<Value, Value>);

impl Map {
    pub fn new() -> Self {
        Self(std::collections::BTreeMap::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &Value) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn insert(&mut self, key: impl Into<Value>, value: impl Into<Value>) -> Option<Value> {
        self.0.insert(key.into(), value.into())
    }

    pub fn remove(&mut self, key: &Value) -> Option<Value> {
        self.0.remove(key)
    }

    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, Value, Value> {
        self.0.iter()
    }

    pub fn into_btree(self) -> std::collections::BTreeMap<Value, Value> {
        self.0
    }
}

impl Default for Map {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for Map {
    type Target = std::collections::BTreeMap<Value, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Map {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

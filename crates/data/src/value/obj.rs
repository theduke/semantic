use std::collections::BTreeMap;

use super::Value;

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[facet(transparent)]
pub struct Object(BTreeMap<String, Value>);

impl Object {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.0.get_mut(key)
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        self.0.insert(key.into(), value.into())
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.0.remove(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, String, Value> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> std::collections::btree_map::IterMut<'_, String, Value> {
        self.0.iter_mut()
    }

    pub fn into_btree(self) -> BTreeMap<String, Value> {
        self.0
    }
}

impl std::ops::Deref for Object {
    type Target = BTreeMap<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Object {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<BTreeMap<String, Value>> for Object {
    fn from(value: BTreeMap<String, Value>) -> Self {
        Self(value)
    }
}

impl FromIterator<(String, Value)> for Object {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Object {
    type Item = (String, Value);
    type IntoIter = std::collections::btree_map::IntoIter<String, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Object {
    type Item = (&'a String, &'a Value);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut Object {
    type Item = (&'a String, &'a mut Value);
    type IntoIter = std::collections::btree_map::IterMut<'a, String, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

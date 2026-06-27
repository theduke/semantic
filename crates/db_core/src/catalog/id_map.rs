use fnv::FnvHashMap;

use crate::catalog::{NameSet, nameset_for_identifier};

/// Dense map keyed by both a local numeric id and a stable string key.
#[derive(Debug, Clone)]
pub struct IdMap<ID, V> {
    values: Vec<Option<V>>,
    qualified_keys: FnvHashMap<String, ID>,
    plain_keys: FnvHashMap<String, Vec<ID>>,
    underscore_keys: FnvHashMap<String, Vec<ID>>,
}

impl<ID, V> IdMap<ID, V>
where
    ID: Into<usize> + From<usize> + Copy + PartialEq,
{
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            qualified_keys: FnvHashMap::default(),
            plain_keys: FnvHashMap::default(),
            underscore_keys: FnvHashMap::default(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (ID, &V)> {
        self.values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_ref().map(|v| (ID::from(i), v)))
    }

    pub fn insert(&mut self, names: NameSet, builder: impl FnOnce(ID) -> V) -> ID {
        if let Some(existing) = self.get_key_id(&names.qualified_name) {
            let value = builder(existing);
            self.values[Into::<usize>::into(existing)] = Some(value);
            self.clear_id_aliases(existing);
            self.insert_aliases(existing, &names);
            return existing;
        }
        let index = ID::from(self.values.len());
        let value = builder(index);
        self.values.push(Some(value));
        self.insert_aliases(index, &names);
        index
    }

    pub fn insert_fixed(&mut self, id: ID, names: NameSet, value: V) {
        let index: usize = id.into();
        if self.values.len() <= index {
            self.values.resize_with(index + 1, || None);
        }
        self.values[index] = Some(value);
        self.clear_id_aliases(id);
        self.insert_aliases(id, &names);
    }

    pub fn get_key(&self, key: &str) -> Option<&V> {
        let index: usize = self.get_key_id(key)?.into();
        self.values.get(index).and_then(|v| v.as_ref())
    }

    pub fn get_key_id(&self, key: &str) -> Option<ID> {
        let ids = self.get_key_ids(key);
        if ids.len() == 1 {
            return ids.first().copied();
        }
        None
    }

    pub fn get_key_ids(&self, key: &str) -> Vec<ID> {
        if let Some(id) = self.qualified_keys.get(key).copied() {
            return vec![id];
        }

        if let Some(ids) = self.plain_keys.get(key) {
            return ids.clone();
        }

        if let Some(ids) = self.underscore_keys.get(key) {
            return ids.clone();
        }

        if key.contains('.') || key.contains(':') || key.contains('_') {
            let normalized = nameset_for_identifier(key, None).qualified_name;
            if normalized != key
                && let Some(id) = self.qualified_keys.get(&normalized).copied()
            {
                return vec![id];
            }
        }

        vec![]
    }

    pub fn get(&self, index: ID) -> Option<&V> {
        let index: usize = index.into();
        self.values.get(index).and_then(|v| v.as_ref())
    }

    pub fn get_mut(&mut self, index: ID) -> Option<&mut V> {
        let index: usize = index.into();
        self.values.get_mut(index).and_then(|v| v.as_mut())
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get_key_id(key).is_some()
    }

    pub fn next_id(&self) -> ID {
        ID::from(self.values.len())
    }

    pub fn remove_key(&mut self, key: &str) -> Option<V> {
        let id = self.get_key_id(key)?;
        let idx: usize = id.into();
        self.clear_id_aliases(id);
        self.values.get_mut(idx).and_then(Option::take)
    }

    fn insert_aliases(&mut self, id: ID, names: &NameSet) {
        self.qualified_keys.insert(names.qualified_name.clone(), id);
        self.plain_keys
            .entry(names.plain_name.clone())
            .or_default()
            .push(id);
        self.underscore_keys
            .entry(names.underscore_name.clone())
            .or_default()
            .push(id);
    }

    fn clear_id_aliases(&mut self, id: ID) {
        self.qualified_keys.retain(|_, value| *value != id);
        self.plain_keys.retain(|_, ids| {
            ids.retain(|value| *value != id);
            !ids.is_empty()
        });
        self.underscore_keys.retain(|_, ids| {
            ids.retain(|value| *value != id);
            !ids.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::{IdMap, NameSet};

    #[test]
    fn resolves_by_qualified_plain_and_underscore() {
        let mut map = IdMap::<usize, String>::new();
        let id = map.insert(
            NameSet {
                qualified_name: "semantic:title".to_string(),
                plain_name: "title".to_string(),
                underscore_name: "semantic_title".to_string(),
            },
            |_| "value".to_string(),
        );
        assert_eq!(id, 0);
        assert_eq!(map.get_key("semantic:title"), Some(&"value".to_string()));
        assert_eq!(map.get_key("title"), Some(&"value".to_string()));
        assert_eq!(map.get_key("semantic_title"), Some(&"value".to_string()));
    }

    #[test]
    fn returns_all_plain_alias_candidates_and_no_unique_match_when_ambiguous() {
        let mut map = IdMap::<usize, String>::new();
        let _ = map.insert(
            NameSet {
                qualified_name: "semantic:title".to_string(),
                plain_name: "title".to_string(),
                underscore_name: "semantic_title".to_string(),
            },
            |_| "semantic".to_string(),
        );
        let _ = map.insert(
            NameSet {
                qualified_name: "shared:blog:title".to_string(),
                plain_name: "title".to_string(),
                underscore_name: "shared_blog_title".to_string(),
            },
            |_| "shared".to_string(),
        );

        assert_eq!(map.get_key_id("title"), None);
        let candidates = map.get_key_ids("title");
        assert_eq!(candidates.len(), 2);
    }
}

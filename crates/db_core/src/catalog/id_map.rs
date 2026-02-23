use fnv::FnvHashMap;

/// Dense map keyed by both a local numeric id and a stable string key.
#[derive(Debug, Clone)]
pub struct IdMap<ID, V> {
    values: Vec<Option<V>>,
    keys: FnvHashMap<String, ID>,
}

impl<ID, V> IdMap<ID, V>
where
    ID: Into<usize> + From<usize> + Copy,
{
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            keys: FnvHashMap::default(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (ID, &V)> {
        self.values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_ref().map(|v| (ID::from(i), v)))
    }

    pub fn insert(&mut self, key: String, builder: impl FnOnce(ID) -> V) -> ID {
        if let Some(existing) = self.keys.get(&key).copied() {
            let value = builder(existing);
            self.values[Into::<usize>::into(existing)] = Some(value);
            return existing;
        }
        let index = ID::from(self.values.len());
        let value = builder(index);
        self.values.push(Some(value));
        self.keys.insert(key, index);
        index
    }

    pub fn insert_fixed(&mut self, id: ID, key: String, value: V) {
        let index: usize = id.into();
        if self.values.len() <= index {
            self.values.resize_with(index + 1, || None);
        }
        self.values[index] = Some(value);
        self.keys.insert(key, id);
    }

    pub fn get_key(&self, key: &str) -> Option<&V> {
        let index: usize = (*self.keys.get(key)?).into();
        self.values.get(index).and_then(|v| v.as_ref())
    }

    pub fn get_key_id(&self, key: &str) -> Option<ID> {
        self.keys.get(key).copied()
    }

    pub fn get(&self, index: ID) -> Option<&V> {
        let index: usize = index.into();
        self.values.get(index).and_then(|v| v.as_ref())
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.keys.contains_key(key)
    }

    pub fn next_id(&self) -> ID {
        ID::from(self.values.len())
    }

    pub fn remove_key(&mut self, key: &str) -> Option<V> {
        let id = self.keys.remove(key)?;
        let idx: usize = id.into();
        self.values.get_mut(idx).and_then(Option::take)
    }
}

use crate::schema::labels::*;
use semantic_data::value::{DateTime, Object, Value};
use semantic_rpc_core::RpcError;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionMode {
    #[default]
    Multiple,
    Exclusive,
}

impl SelectionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Multiple => MODE_MULTIPLE,
            Self::Exclusive => MODE_EXCLUSIVE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Label,
    Group,
}

impl LabelKind {
    pub fn class_id(&self) -> &'static str {
        match self {
            Self::Label => CLASS_ID,
            Self::Group => GROUP_CLASS_ID,
        }
    }
}

/// Shared metadata for assignable labels and nonselectable organizing groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub id: String,
    pub kind: LabelKind,
    pub name: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    /// An optional six-digit CSS hex color, including '#'.
    pub color: Option<String>,
    pub selection_mode: SelectionMode,
    pub created_at: Option<DateTime>,
    pub updated_at: Option<DateTime>,
}

impl Label {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: LabelKind::Label,
            name: name.into(),
            description: None,
            parent_id: None,
            color: None,
            selection_mode: SelectionMode::Multiple,
            created_at: None,
            updated_at: None,
        }
    }

    pub fn new_group(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: LabelKind::Group,
            ..Self::new(id, name)
        }
    }

    pub fn is_group(&self) -> bool {
        self.kind == LabelKind::Group
    }

    pub fn from_object(object: &Object) -> Result<Self, RpcError> {
        let mode = match optional_string(object, ATTR_SELECTION_MODE)?.as_deref() {
            None | Some(MODE_MULTIPLE) => SelectionMode::Multiple,
            Some(MODE_EXCLUSIVE) => SelectionMode::Exclusive,
            _ => return Err(RpcError::invalid_payload("Unknown label selection mode")),
        };
        Ok(Self {
            id: required_string(object, "id")?,
            kind: match required_string(object, "type")?.as_str() {
                CLASS_ID => LabelKind::Label,
                GROUP_CLASS_ID => LabelKind::Group,
                _ => return Err(RpcError::invalid_payload("Expected a label or label group")),
            },
            name: required_string(object, ATTR_NAME)?,
            description: optional_string(object, ATTR_DESCRIPTION)?,
            parent_id: optional_string(object, ATTR_PARENT)?,
            color: optional_string(object, ATTR_COLOR)?,
            selection_mode: mode,
            created_at: datetime(object, ATTR_CREATED_AT),
            updated_at: datetime(object, ATTR_UPDATED_AT),
        })
    }

    pub fn to_object(&self) -> Object {
        let mut object = Object::new();
        for (key, value) in [
            ("id", self.id.as_str()),
            ("type", self.kind.class_id()),
            (ATTR_NAME, &self.name),
        ] {
            object.insert(key, Value::String(value.into()));
        }
        if self.is_group() {
            object.insert(
                ATTR_SELECTION_MODE,
                Value::String(self.selection_mode.as_str().into()),
            );
        }
        for (key, value) in [
            (ATTR_DESCRIPTION, &self.description),
            (ATTR_PARENT, &self.parent_id),
            (ATTR_COLOR, &self.color),
        ] {
            if let Some(value) = value {
                object.insert(key, Value::String(value.clone()));
            }
        }
        for (key, value) in [
            (ATTR_CREATED_AT, self.created_at),
            (ATTR_UPDATED_AT, self.updated_at),
        ] {
            if let Some(value) = value {
                object.insert(key, Value::DateTime(value));
            }
        }
        object
    }
}

fn datetime(object: &Object, key: &str) -> Option<DateTime> {
    match object.get(key) {
        Some(Value::DateTime(value)) => Some(*value),
        _ => None,
    }
}

pub(crate) fn optional_string(object: &Object, key: &str) -> Result<Option<String>, RpcError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(RpcError::invalid_payload(format!("{key} must be a string"))),
    }
}

pub(crate) fn required_string(object: &Object, key: &str) -> Result<String, RpcError> {
    optional_string(object, key)?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| RpcError::invalid_payload(format!("{key} is required")))
}

pub fn valid_label_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

/// An indexed catalog shared by server validation and selection UIs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelCatalog {
    pub labels: BTreeMap<String, Label>,
}

impl LabelCatalog {
    pub fn new(labels: impl IntoIterator<Item = Label>) -> Self {
        Self {
            labels: labels
                .into_iter()
                .map(|label| (label.id.clone(), label))
                .collect(),
        }
    }

    pub fn path(&self, id: &str) -> String {
        let mut names = Vec::new();
        let mut seen = BTreeSet::new();
        let mut next = Some(id);
        while let Some(label) = next.and_then(|id| self.labels.get(id)) {
            if !seen.insert(&label.id) {
                break;
            }
            names.push(label.name.as_str());
            next = label.parent_id.as_deref();
        }
        names.reverse();
        names.join(" / ")
    }

    pub fn exclusive_group(&self, id: &str) -> Option<&str> {
        let parent = self.labels.get(id)?.parent_id.as_deref()?;
        let group = self.labels.get(parent)?;
        (group.is_group() && group.selection_mode == SelectionMode::Exclusive).then_some(parent)
    }

    pub fn validate_selection(&self, ids: &BTreeSet<String>) -> Result<(), RpcError> {
        let mut groups = BTreeSet::new();
        for id in ids {
            self.require_assignable(id)?;
            if let Some(group) = self.exclusive_group(id) {
                if !groups.insert(group) {
                    return Err(RpcError::new(
                        "label_conflict",
                        format!("Choose only one label in {}", self.path(group)),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Add a label, replacing any selected direct sibling in an exclusive group.
    pub fn select(&self, selected: &mut BTreeSet<String>, id: &str) -> Result<(), RpcError> {
        self.require_assignable(id)?;
        if let Some(group) = self.exclusive_group(id) {
            selected.retain(|other| self.exclusive_group(other) != Some(group));
        }
        selected.insert(id.into());
        Ok(())
    }

    fn require_assignable(&self, id: &str) -> Result<(), RpcError> {
        let label = self.labels.get(id).ok_or_else(|| {
            RpcError::new("label_not_found", format!("Label {id} no longer exists"))
        })?;
        if label.is_group() {
            return Err(RpcError::new(
                "label_group_not_assignable",
                format!(
                    "{} is a group. Remove its legacy assignment and select labels instead",
                    self.path(id)
                ),
            ));
        }
        Ok(())
    }

    pub fn validate_label(&self, label: &Label) -> Result<(), RpcError> {
        if !label.is_group() && label.selection_mode != SelectionMode::Multiple {
            return Err(RpcError::invalid_payload(
                "Only label groups can have exclusive child selection",
            ));
        }
        if label.id.trim().is_empty() || label.name.trim().is_empty() {
            return Err(RpcError::invalid_payload("A label needs an ID and a name"));
        }
        if label
            .color
            .as_deref()
            .is_some_and(|color| !valid_label_color(color))
        {
            return Err(RpcError::invalid_payload(
                "Color must be a six-digit hex color, such as #6366f1",
            ));
        }
        let mut seen = BTreeSet::from([label.id.as_str()]);
        let mut next = label.parent_id.as_deref();
        while let Some(id) = next {
            if !seen.insert(id) {
                return Err(RpcError::new(
                    "label_cycle",
                    "A label cannot be its own ancestor",
                ));
            }
            let parent = self
                .labels
                .get(id)
                .ok_or_else(|| RpcError::new("label_not_found", "Parent label no longer exists"))?;
            next = parent.parent_id.as_deref();
        }
        Ok(())
    }
}

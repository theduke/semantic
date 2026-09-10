use super::{InterfaceType, TypeDef};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub fn interface_fingerprint(
    interface: &InterfaceType,
    definitions: &BTreeMap<String, TypeDef>,
) -> Result<String, String> {
    let mut schema = json(interface)?;
    canonicalize(&mut schema);
    let mut pending = BTreeSet::new();
    references(&schema, &mut pending);
    let mut reachable = BTreeMap::new();
    while let Some(name) = pending.pop_first() {
        if reachable.contains_key(&name) {
            continue;
        }
        let definition = definitions
            .get(&name)
            .ok_or_else(|| format!("unresolved interface type '{name}'"))?;
        let mut value = json(definition)?;
        canonicalize(&mut value);
        references(&value, &mut pending);
        reachable.insert(name, value);
    }
    let bytes = serde_json::to_vec(&(schema, reachable)).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    hash.update(b"semantic.interface.fingerprint.v1\0");
    hash.update(bytes);
    Ok(format!("v1:{:x}", hash.finalize()))
}

fn json<'a, T: facet::Facet<'a>>(value: &T) -> Result<serde_json::Value, String> {
    let value = facet_json::to_string(value).map_err(|error| error.to_string())?;
    serde_json::from_str(&value).map_err(|error| error.to_string())
}

fn canonicalize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            // Preserve identity and semantic annotations; exclude display metadata.
            if let Some(serde_json::Value::Object(meta)) = object.get_mut("meta") {
                meta.retain(|key, _| key == "id" || key == "annotations");
            }
            for value in object.values_mut() {
                canonicalize(value);
            }
            if let Some(serde_json::Value::Array(methods)) = object.get_mut("methods") {
                methods.sort_by(|left, right| {
                    left.get("name")
                        .and_then(|v| v.as_str())
                        .cmp(&right.get("name").and_then(|v| v.as_str()))
                });
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                canonicalize(item);
            }
        }
        _ => {}
    }
}

fn references(value: &serde_json::Value, output: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(name) = object
                .get("ref")
                .and_then(|reference| reference.get("name"))
                .and_then(|name| name.as_str())
            {
                output.insert(name.into());
            }
            for value in object.values() {
                references(value, output);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                references(item, output);
            }
        }
        _ => {}
    }
}

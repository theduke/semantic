//! Authoritative read-only interface lookup and versioned canonical fingerprints.
use super::Catalog;
use crate::{CoreError, normalize_package_definition};
use semantic_data::schema::{InterfaceType, SchemaVersion, TypeDef};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ResolvedInterface {
    pub interface: InterfaceType,
    pub package_version: Option<SchemaVersion>,
    pub fingerprint: String,
    pub definitions: BTreeMap<String, TypeDef>,
}

impl Catalog {
    pub fn resolve_interface(
        &self,
        package: &str,
        module: &str,
        contract: Option<&str>,
        name: &str,
    ) -> Result<ResolvedInterface, CoreError> {
        let package = self
            .package_by_name(package)
            .ok_or_else(|| CoreError::new("interface package not installed"))?;
        let package = normalize_package_definition(package)?;
        let module = if package.root.name == module {
            Some(&package.root)
        } else {
            package.modules.get(module)
        }
        .ok_or_else(|| CoreError::new("interface module not found"))?;
        let interface = match contract {
            None => module.interfaces.get(name),
            Some(contract) => module
                .contracts
                .get(contract)
                .and_then(|contract| contract.interfaces.get(name))
                .map(|item| &item.interface),
        }
        .ok_or_else(|| CoreError::new("interface not found"))?
        .clone();
        let mut definitions = BTreeMap::<String, TypeDef>::new();
        for (_, package) in self.packages() {
            let package = normalize_package_definition(package)?;
            for module in std::iter::once(&package.root).chain(package.modules.values()) {
                for definition in module.types.values() {
                    let key = super::nameset_for_identifier(&definition.name, Some(&module.name))
                        .qualified_name;
                    if definitions
                        .insert(key.clone(), definition.clone())
                        .is_some()
                    {
                        return Err(CoreError::new(format!(
                            "ambiguous interface type reference '{key}'"
                        )));
                    }
                }
            }
        }
        if let Some(contract_name) = contract {
            for definition in module.contracts[contract_name].types.values() {
                let key = super::nameset_for_identifier(&definition.name, Some(&module.name))
                    .qualified_name;
                if definitions
                    .insert(key.clone(), definition.clone())
                    .is_some()
                {
                    return Err(CoreError::new(format!("ambiguous contract type '{key}'")));
                }
            }
        }
        let fingerprint = fingerprint(&interface, &definitions)?;
        Ok(ResolvedInterface {
            interface,
            package_version: package.version,
            fingerprint,
            definitions,
        })
    }
}

fn fingerprint(
    interface: &InterfaceType,
    definitions: &BTreeMap<String, TypeDef>,
) -> Result<String, CoreError> {
    semantic_data::schema::interface_fingerprint(interface, definitions).map_err(CoreError::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::schema::{
        FunctionType, InterfaceMethod, Meta, Type, TypeKind, TypeRef, Visibility,
    };

    fn method(name: &str, ty: Type) -> InterfaceMethod {
        InterfaceMethod {
            name: name.into(),
            signature: FunctionType {
                params: vec![],
                results: vec![ty],
                throws: None,
                async_fn: true,
            },
        }
    }

    #[test]
    fn fingerprint_is_order_independent_and_follows_recursive_references() {
        let reference = Type::new(TypeKind::Ref(TypeRef::new("test:Node")));
        let definition = TypeDef {
            name: "Node".into(),
            module: Some("test".into()),
            params: vec![],
            ty: reference.clone(),
            visibility: Visibility::Public,
            meta: Meta::default(),
        };
        let mut definitions = BTreeMap::from([("test:Node".into(), definition)]);
        let interface = InterfaceType {
            methods: vec![method("b", reference), method("a", Type::new_bool())],
        };
        assert!(fingerprint(&interface, &BTreeMap::new()).is_err());
        let hash = fingerprint(&interface, &definitions).unwrap();
        let mut reordered = interface.clone();
        reordered.methods.reverse();
        assert_eq!(hash, fingerprint(&reordered, &definitions).unwrap());
        definitions.get_mut("test:Node").unwrap().meta.description = Some("display only".into());
        assert_eq!(hash, fingerprint(&interface, &definitions).unwrap());
        definitions.get_mut("test:Node").unwrap().ty = Type::new_bool();
        assert_ne!(hash, fingerprint(&interface, &definitions).unwrap());
    }

    #[test]
    fn resolves_builtin_root_interfaces() {
        let mut catalog = Catalog::new();
        catalog.upsert_package(semantic_data::import::package());
        let resolved = catalog
            .resolve_interface("semantic.import", "v1", None, "Source")
            .unwrap();
        assert_eq!(resolved.interface.methods.len(), 2);
        assert!(resolved.fingerprint.starts_with("v1:"));
    }
}

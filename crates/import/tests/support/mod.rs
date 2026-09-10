use semantic_data::Value;
use semantic_import::GenericUrlPlugin;
use semantic_plugin::{Plugin, PluginInstanceContext};
use semantic_rpc::interface::*;
use std::sync::Arc;

pub fn exports() -> Vec<ImplementationDescriptor> {
    ["Source", "Fetcher", "Importer"]
        .into_iter()
        .map(|name| ImplementationDescriptor {
            export: name.to_lowercase(),
            interface: InterfaceRef {
                package: "semantic.import".into(),
                module: "v1".into(),
                contract: None,
                name: name.into(),
            },
            package_version: "1.0.0".into(),
            fingerprint: semantic_data::schema::interface_fingerprint(
                &semantic_data::import::package().root.interfaces[name],
                &Default::default(),
            )
            .unwrap(),
        })
        .collect()
}

pub async fn implementation() -> Arc<dyn InterfaceImplementation> {
    GenericUrlPlugin::new(exports())
        .create(PluginInstanceContext {
            scope: "acceptance".into(),
            generation: 1,
            configuration: Value::Null,
            cancellation: CancellationToken::new(),
        })
        .await
        .unwrap()
}

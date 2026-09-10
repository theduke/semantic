//! Run with `cargo run -p semantic_import --example rust_url_plugin -- URL`.
//! A custom Rust factory, canonical exports, and a direct fetch without storage.
use futures_util::StreamExt;
use semantic_data::{
    Object,
    import::{SourceRequest, package},
    schema::interface_fingerprint,
};
use semantic_import::GenericUrlPlugin;
use semantic_plugin::{Plugin, PluginError, PluginInstanceContext, PluginManifest, PluginRegistry};
use semantic_rpc::interface::{
    InterfaceImplementation, InvocationArgument, InvocationContext, InvocationOutput,
    ValidatedInvocation,
};
use semantic_rpc_core::interface::{ImplementationDescriptor, InterfaceRef};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

/// Custom embedding code can capture HTTP clients or other scope-bound services
/// in its factory and return an implementation of the canonical interfaces.
struct CustomUrlPlugin {
    manifest: PluginManifest,
    inner: GenericUrlPlugin,
}
impl Plugin for CustomUrlPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn create<'a>(
        &'a self,
        context: PluginInstanceContext,
    ) -> Pin<
        Box<dyn Future<Output = Result<Arc<dyn InterfaceImplementation>, PluginError>> + Send + 'a>,
    > {
        eprintln!(
            "Creating {} for scope {} generation {}",
            self.manifest.id, context.scope, context.generation
        );
        self.inner.create(context)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let requested = std::env::args()
        .nth(1)
        .ok_or("usage: rust_url_plugin URL")?;
    let package = package();
    let exports = [
        ("source", "Source"),
        ("fetcher", "Fetcher"),
        ("importer", "Importer"),
    ]
    .into_iter()
    .map(|(export, name)| {
        Ok(ImplementationDescriptor {
            export: export.into(),
            interface: InterfaceRef {
                package: package.name.clone(),
                module: "v1".into(),
                contract: None,
                name: name.into(),
            },
            package_version: "1.0.0".into(),
            fingerprint: interface_fingerprint(&package.root.interfaces[name], &BTreeMap::new())?,
        })
    })
    .collect::<Result<Vec<_>, String>>()?;
    let inner = GenericUrlPlugin::new(exports);
    let mut manifest = inner.manifest().clone();
    manifest.id = "example.custom-url".into();
    manifest.title = "Custom Rust URL factory".into();
    let plugin = CustomUrlPlugin { manifest, inner };
    let cancellation = semantic_jobs::CancellationToken::new();
    // In an application, ScopePlugins creates this instance after conformance.
    let instance = plugin
        .create(PluginInstanceContext {
            scope: "example".into(),
            generation: 1,
            configuration: semantic_data::Value::Null,
            cancellation: cancellation.clone(),
        })
        .await?;
    let mut registry = PluginRegistry::new();
    registry.register(plugin)?;
    let request = SourceRequest {
        url: requested,
        options: Object::new(),
    };
    let output = instance
        .invoke(
            ValidatedInvocation {
                export: "fetcher".into(),
                method: "fetch".into(),
                arguments: vec![InvocationArgument::Value(request.to_value())],
            },
            InvocationContext {
                generation: 1,
                cancellation,
            },
        )
        .await?;
    let InvocationOutput::Stream(stream) = output else {
        return Err("expected fetch stream".into());
    };
    let mut stream = semantic_import::decode_stream(stream);
    while let Some(frame) = stream.next().await {
        match frame? {
            semantic_import::ContentFrame::Item(semantic_import::ContentEvent::FileStart(
                metadata,
            )) => println!(
                "{} ({})",
                metadata.filename.as_deref().unwrap_or("file"),
                metadata.mime_type
            ),
            semantic_import::ContentFrame::End(summary) => println!(
                "Fetched {} items / {} bytes; no database or blob writes",
                summary.items, summary.bytes
            ),
            _ => {}
        }
    }
    Ok(())
}

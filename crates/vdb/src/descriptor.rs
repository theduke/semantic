use semantic_db_core::catalog::Catalog;
use semantic_rpc_core::interface::{ImplementationDescriptor, InterfaceRef};

use crate::{INTERFACE_NAME, MODULE_NAME, PACKAGE_NAME, VdbError};

/// Resolve the published VDB interface and its canonical fingerprint.
pub fn implementation_descriptor(
    catalog: &Catalog,
    export: &str,
) -> Result<ImplementationDescriptor, VdbError> {
    let resolved = catalog
        .resolve_interface(PACKAGE_NAME, MODULE_NAME, None, INTERFACE_NAME)
        .map_err(|error| VdbError {
            code: "invalid_interface".into(),
            message: error.to_string(),
        })?;
    Ok(ImplementationDescriptor {
        export: export.into(),
        interface: InterfaceRef {
            package: PACKAGE_NAME.into(),
            module: MODULE_NAME.into(),
            contract: None,
            name: INTERFACE_NAME.into(),
        },
        package_version: resolved
            .package_version
            .map(|version| {
                let mut value = format!("{}.{}.{}", version.major, version.minor, version.patch);
                if let Some(pre) = version.pre {
                    value.push('-');
                    value.push_str(&pre);
                }
                if let Some(build) = version.build {
                    value.push('+');
                    value.push_str(&build);
                }
                value
            })
            .unwrap_or_else(|| "1.0.0".into()),
        fingerprint: resolved.fingerprint,
    })
}

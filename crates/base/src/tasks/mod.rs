//! Optional task tracking. Install base and comments before this package.
//! Task hierarchy uses the shared `semantic:parent` attribute.
mod commands;
mod migration_v1;
mod model;
pub mod schema;
mod service;
pub use commands::*;
pub use model::*;
pub use service::*;
pub const PACKAGE_NAME: &str = "semantic.tasks";
pub fn package() -> semantic_data::schema::Package {
    crate::domain_support::with_content_dependency(crate::domain_support::package(
        PACKAGE_NAME,
        "tasks",
        schema::attributes(),
        schema::classes(),
        migration_v1::migration(),
    ))
}
#[derive(Clone, Copy, Debug, Default)]
pub struct TasksPackage;
impl<Ctx: TaskContext, E: From<semantic_rpc_core::RpcError>>
    semantic_rpc_core::RuntimePackage<Ctx, E> for TasksPackage
{
    fn schema(&self) -> semantic_data::schema::Package {
        package()
    }
    fn commands(&self) -> Vec<Box<dyn semantic_rpc_core::DynCommand<Ctx, E>>> {
        commands()
    }
}

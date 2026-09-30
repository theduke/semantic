//! Optional comments attached to collection-qualified entity targets.
//! Install base first. Reply hierarchy uses the shared `semantic:parent` attribute.
mod commands;
mod migration_v1;
mod model;
pub mod schema;
mod service;
pub use commands::*;
pub use model::*;
pub use service::*;
pub const PACKAGE_NAME: &str = "semantic.comments";
pub fn package() -> semantic_data::schema::Package {
    crate::domain_support::with_content_dependency(crate::domain_support::package(
        PACKAGE_NAME,
        "comments",
        schema::attributes(),
        schema::classes(),
        migration_v1::migration(),
    ))
}
#[derive(Clone, Copy, Debug, Default)]
pub struct CommentsPackage;
impl<Ctx: CommentContext, E: From<semantic_rpc_core::RpcError>>
    semantic_rpc_core::RuntimePackage<Ctx, E> for CommentsPackage
{
    fn schema(&self) -> semantic_data::schema::Package {
        package()
    }
    fn commands(&self) -> Vec<Box<dyn semantic_rpc_core::DynCommand<Ctx, E>>> {
        commands()
    }
}

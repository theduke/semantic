use semantic_data::schema::Package;

use crate::DynCommand;

/// A schema package and its executable command handlers.
///
/// Wrap typed handlers in [`crate::CommandAdapter`] to return heterogeneous commands.
/// The context is supplied by the host application when invoking a command.
pub trait RuntimePackage<Ctx> {
    /// Build the schema definition, including any package migrations.
    fn schema(&self) -> Package;

    /// Build owned handlers to register with the host application.
    fn commands(&self) -> Vec<Box<dyn DynCommand<Ctx>>>;
}

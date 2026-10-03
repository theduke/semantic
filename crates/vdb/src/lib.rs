//! Read-only virtual databases exported through the `semantic.vdb/v1` interface.
//!
//! Implement [`VirtualDatabase`] and wrap an instance factory in
//! [`VirtualDatabasePlugin`]. Exposed schemas are runtime definitions: they do
//! not install packages or migrate the host database.

mod author;
mod descriptor;
mod helpers;

pub use author::{EntityStream, VirtualDatabase, VirtualDatabasePlugin};
pub use descriptor::implementation_descriptor;
pub use helpers::{classify_simple, simple_eq_value};
pub use semantic_data::vdb::*;
pub use semantic_rpc::interface::CancellationToken;

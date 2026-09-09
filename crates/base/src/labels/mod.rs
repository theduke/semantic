//! Hierarchical labels. All writes through these helpers are serialized within this process
//! and committed as one database batch. Direct database writes bypass label invariants;
//! distributed hosts must provide external serialization for label operations.
//!
//! Labels and nonselectable groups share metadata and hierarchy through [`Label`]
//! and [`LabelKind`]. Membership helpers match source collection and ID; generic
//! indexed relation predicates currently use bare endpoint IDs and cannot distinguish
//! identically named entities in different collections. Use [`labels_for_entity`]
//! for collection-aware label lookup.
mod commands;
mod model;
mod service;

pub use commands::*;
pub use model::*;
pub use service::*;
#[cfg(test)]
mod tests;

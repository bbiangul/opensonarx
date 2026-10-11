//! URL-structure helpers. They live in `oi-types` so that every layer derives
//! a source family from a URL the same way, including a node that receives a
//! claim from another node and must not take the sender's word for its source.

pub use oi_types::source::{host, source_family};

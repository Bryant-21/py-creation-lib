pub mod bindings;
pub mod bulk;
pub mod bulk_schema;
pub mod database;
pub mod dir_index;
pub mod embedder;
pub mod embeddings;
pub mod error;
pub mod fts5;
pub mod nif_indexer;
pub mod pragmas;
pub mod query;
pub mod records_indexer;
pub mod registry;
pub mod schema;
pub mod tokenizer;

pub use bindings::register_module;

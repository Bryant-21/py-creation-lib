pub mod types;

#[allow(dead_code)]
pub mod generated {
    use crate::schema::types::*;
    include!(concat!(env!("OUT_DIR"), "/schema_generated.rs"));
}

pub mod registry;

pub use registry::{FieldPlanEntry, NifSchema, SCHEMA};
pub use types::*;

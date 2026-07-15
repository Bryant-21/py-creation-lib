mod parser;
mod skyrim;
mod types;
mod validator;
mod writer;
mod yaml;

#[cfg(test)]
mod tests;

pub use parser::{NvnmError, parse_nvnm};
pub use skyrim::{
    SkyrimNvnmConversion, SkyrimNvnmConversionBatch, SkyrimNvnmConversionFailure,
    SkyrimNvnmConversionReport, convert_skyrim_nvnm_set_to_fo4,
    convert_skyrim_nvnm_set_to_fo4_lossy,
};
pub(crate) use skyrim::{collect_skyrim_nvnm_form_ids, rewrite_skyrim_nvnm_form_ids};
pub use types::{
    NvnmCoverEntry, NvnmCoverTriangleMapping, NvnmDoorRef, NvnmEdgeLink, NvnmGrid, NvnmGridCell,
    NvnmParent, NvnmPayload, NvnmTriangle, NvnmVertex, NvnmWaypoint,
};
pub use validator::{
    ValidationError, ValidationErrorKind, ValidationReport, validate_navmesh_set,
    validate_plugin_navmeshes,
};
pub use writer::write_nvnm;
pub use yaml::{nvnm_from_yaml, nvnm_to_yaml};

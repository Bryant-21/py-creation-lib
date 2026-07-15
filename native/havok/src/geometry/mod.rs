pub mod matching;
pub mod normals;
pub mod qem;
pub mod section_merge;
pub mod skinning;
pub mod weight_transfer;

pub use matching::{MatchResult, center_vertices, centroid, match_nearest, mean_nearest_distance};
pub use normals::{compute_flat_normals, compute_normals};
pub use qem::{DecimatedMesh, decimate};
pub use section_merge::{MeshSection, merge_sections, split_sections};
pub use skinning::{
    Mat4, SkinWeight, apply_lbs, mat4_identity, mat4_inverse_affine, mat4_mul, normalize_weights,
    transform_point,
};
pub use weight_transfer::{KdTree, transfer_weights};

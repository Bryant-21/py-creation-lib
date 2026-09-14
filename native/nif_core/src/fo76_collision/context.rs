use std::sync::OnceLock;

use havok_native::collision::SourcePhysicsSystemContext;
use serde_json::Value;

use super::{
    ExtractedCollisionBody, HAVOK_SCALE, SourceBodyMetadata, source_body_metadata_from_parts,
    usable_source_preview_mesh,
};

pub(crate) struct SourceCollisionContext<'a> {
    source: SourcePhysicsSystemContext<'a>,
    summary_value: OnceLock<Option<Value>>,
}

impl<'a> SourceCollisionContext<'a> {
    pub(crate) fn new(blob: &'a [u8]) -> Self {
        Self {
            source: SourcePhysicsSystemContext::new(blob),
            summary_value: OnceLock::new(),
        }
    }

    pub(crate) fn body_count(&self) -> usize {
        self.source.body_transforms().len()
    }

    pub(crate) fn mass_distribution(
        &self,
        body_id: usize,
    ) -> Option<havok_native::collision::SourceMassDistribution> {
        self.source
            .mass_distributions()
            .get(body_id)
            .copied()
            .flatten()
    }

    pub(crate) fn body_metadata(&self, body_id: usize) -> SourceBodyMetadata {
        let source_transform = self
            .source
            .body_transforms()
            .get(body_id)
            .copied()
            .flatten();
        let Ok(summary) = self.source.collision_summary() else {
            return SourceBodyMetadata {
                position: source_transform.map(|value| value.position),
                orientation: source_transform.map(|value| value.orientation),
                ..SourceBodyMetadata::default()
            };
        };
        let has_mass_dist = self.mass_distribution(body_id).is_some();
        let value = self
            .summary_value
            .get_or_init(|| serde_json::from_str::<Value>(summary).ok());
        let Some(value) = value else {
            return SourceBodyMetadata {
                position: source_transform.map(|value| value.position),
                orientation: source_transform.map(|value| value.orientation),
                has_ref_mass_distribution: has_mass_dist,
                is_dynamic: has_mass_dist,
                ..SourceBodyMetadata::default()
            };
        };
        source_body_metadata_from_parts(source_transform, has_mass_dist, value, body_id)
    }

    pub(crate) fn extract_body(&self, body_id: usize) -> Result<ExtractedCollisionBody, String> {
        let metadata = self.body_metadata(body_id);
        let source_primitive = self
            .source
            .direct_source_primitive(body_id)
            .ok()
            .flatten()
            .map(|shape| (shape, self.body_count()));
        let source_polytopes = self.source.source_polytopes(body_id).unwrap_or_default();
        let source_compound_children = self
            .source
            .source_compound_children(body_id)
            .unwrap_or_default();
        let source_compressed_mesh = self
            .source
            .direct_raw_compressed_mesh(body_id)
            .unwrap_or_default();
        let has_exact_source_shape = source_primitive.is_some()
            || !source_polytopes.is_empty()
            || !source_compound_children.is_empty()
            || source_compressed_mesh.is_some();
        let meshes = match self.source.preview_meshes(HAVOK_SCALE, body_id) {
            Ok(meshes) => meshes
                .into_iter()
                .filter(usable_source_preview_mesh)
                .collect(),
            Err(_) if has_exact_source_shape => Vec::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok(ExtractedCollisionBody {
            body_id,
            meshes,
            source_polytopes,
            source_compound_children,
            source_compressed_mesh,
            source_primitive,
            layer: metadata.layer,
            material_crc: metadata.material_crc,
            is_dynamic: metadata.is_dynamic,
        })
    }
}

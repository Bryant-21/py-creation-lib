use std::collections::HashMap;

use crate::error::{HavokError, HavokResult};
use crate::hkx::HkxFile;

use super::corpus;
use super::hooks::{ConversionContext, CustomHookRegistry};
use super::ops::{Patch, PatchOperation};
use super::version::{
    HavokVersion, UNIMPLEMENTED_PATCH_PACKAGE_IDS, get_version, get_version_chain,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchDirection {
    Upgrade,
    Downgrade,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchStep {
    pub version: HavokVersion,
    pub direction: PatchDirection,
    pub patches: Vec<Patch>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchRoute {
    pub source: HavokVersion,
    pub target: HavokVersion,
    pub upgrading: bool,
    pub steps: Vec<PatchStep>,
}

#[derive(Default)]
pub struct PatchManager {
    patches: HashMap<u8, Vec<Patch>>,
    hooks: CustomHookRegistry,
    corpus_complete: bool,
}

impl PatchManager {
    pub fn new() -> Self {
        Self {
            corpus_complete: true,
            ..Self::default()
        }
    }

    pub fn with_native_corpus() -> Self {
        let mut manager = Self::new();
        corpus::register_native_patches(&mut manager);
        corpus::register_native_hooks(&mut manager.hooks);
        manager.corpus_complete = false;
        manager
    }

    /// Mark the corpus as complete — applied conversions will not be refused
    /// by the parity gate. Used by `havok_convert_bytes` to drive the
    /// patch-chain route opportunistically; callers accept that some semantic
    /// hooks may be no-ops where the C++ SDK had bytewise transforms.
    pub fn force_corpus_complete(&mut self) {
        self.corpus_complete = true;
    }

    pub fn is_corpus_complete(&self) -> bool {
        self.corpus_complete
    }

    pub fn register(&mut self, version_id: u8, patch: Patch) {
        self.patches.entry(version_id).or_default().push(patch);
    }

    pub fn register_all(&mut self, version_id: u8, patches: Vec<Patch>) {
        self.patches.entry(version_id).or_default().extend(patches);
    }

    pub fn register_hook<F>(&mut self, name: impl Into<String>, hook: F)
    where
        F: Fn(&mut ConversionContext<'_>) -> HavokResult<()> + Send + Sync + 'static,
    {
        self.hooks.register(name, hook);
    }

    pub fn register_native_hooks(&mut self) {
        corpus::register_native_hooks(&mut self.hooks);
    }

    pub fn patch_count(&self, version_id: u8) -> usize {
        self.patches.get(&version_id).map_or(0, Vec::len)
    }

    pub fn route(&self, source: u8, target: u8) -> HavokResult<PatchRoute> {
        let chain = get_version_chain(source, target)?;
        let upgrading = target >= source;
        let step_versions: &[HavokVersion] = if source == target {
            &[]
        } else if upgrading {
            &chain[1..]
        } else {
            &chain[..chain.len() - 1]
        };
        if let Some(version) = step_versions
            .iter()
            .find(|version| UNIMPLEMENTED_PATCH_PACKAGE_IDS.contains(&version.id))
        {
            return Err(HavokError::ConversionNotImplemented {
                source_version: source,
                target_version: target,
                route: format!("{}->{}", chain[0].name, chain[chain.len() - 1].name),
                reason: format!(
                    "unimplemented patch package for {} ({})",
                    version.name, version.id
                ),
            });
        }

        let steps = step_versions
            .iter()
            .map(|version| PatchStep {
                version: *version,
                direction: if upgrading {
                    PatchDirection::Upgrade
                } else {
                    PatchDirection::Downgrade
                },
                patches: self
                    .patches
                    .get(&version.id)
                    .map(|patches| {
                        if upgrading {
                            patches.clone()
                        } else {
                            patches.iter().rev().map(Patch::inverse).collect()
                        }
                    })
                    .unwrap_or_default(),
            })
            .collect();

        Ok(PatchRoute {
            source: get_version(source)?,
            target: get_version(target)?,
            upgrading,
            steps,
        })
    }

    pub fn convert_hkx(&self, hkx: &mut HkxFile, source: u8, target: u8) -> HavokResult<()> {
        let route = self.route(source, target)?;
        if source == target {
            return Ok(());
        }
        if !self.corpus_complete {
            return Err(crate::error::HavokError::UnportedEdgeCase {
                route: super::route_name(source, target, "packfile").to_string(),
                edge_case: "native patch corpus parity".to_string(),
                detail: "native patch corpus is intentionally incomplete; refusing changed conversion state until Python corpus parity is ported".to_string(),
            });
        }

        let route_name = format!("{}->{}", route.source.name, route.target.name);
        for step in &route.steps {
            for patch in &step.patches {
                let matched_indices: Vec<usize> = hkx
                    .objects()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, object)| patch.matches_object(object).then_some(index))
                    .collect();
                for index in matched_indices {
                    self.apply_patch_to_object(hkx, index, patch, source, target, &route_name)?;
                }
            }
        }

        hkx.set_contents_version(route.target.name);
        Ok(())
    }

    fn apply_patch_to_object(
        &self,
        hkx: &mut HkxFile,
        index: usize,
        patch: &Patch,
        source: u8,
        target: u8,
        route_name: &str,
    ) -> HavokResult<()> {
        for operation in &patch.operations {
            match operation {
                PatchOperation::CustomHook { name, .. } => {
                    let mut context =
                        ConversionContext::new_for_object(hkx, source, target, route_name, index);
                    self.hooks.invoke(name, &mut context)?;
                }
                operation => operation.apply_to_object(&mut hkx.objects_mut()[index])?,
            }
        }
        let object = &mut hkx.objects_mut()[index];
        object.class_name = patch.new.class_name.clone();
        object.signature = patch.new.version.max(0) as u32;
        Ok(())
    }
}

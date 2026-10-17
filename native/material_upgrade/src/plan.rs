//! Scan sources into upgrade units, decide what happens to each and name outputs.

use std::collections::{BTreeMap, HashMap, HashSet};

use rayon::prelude::*;
use serde::Serialize;

use crate::bgsm_v22::is_already_v2x;
use crate::classify::{ClassHints, MaterialClass, classify_material, envmap_is_metal};
use crate::kernels::{
    NEUTRAL, SmoothnessCurve, Tint, cube_tint, envmap_metal_mask, gloss_is_flat, is_neutral, tinted_metal_srgb,
};
use crate::options::{MetalMode, UpgradeOptions};
use crate::sources::{AssetSources, normalize, texture_key};
use crate::terrain::TerrainLayerSource;

pub const SHARED_DIR: &str = "textures/materialupgrade";
pub const DIELECTRIC_R: &str = "textures/materialupgrade/dielectric_r.dds";
pub const METAL_R: &str = "textures/materialupgrade/metal_r.dds";
/// Below this peak mask no sRGB byte of `_d` moves by more than ~1, so no darkened copy is written.
pub const METAL_DIFFUSE_MIN_MASK: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum UnitKind {
    Material,
    NifTextureSet,
    TerrainLayer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnitStatus {
    Upgrade,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unit {
    pub kind: UnitKind,
    /// `materials/x.bgsm`, `meshes/x.nif#<block>` or `ltex:<plugin>:<object id>`.
    pub key: String,
    pub class: MaterialClass,
    pub diffuse: String,
    pub normal: String,
    pub smoothspec: String,
    pub smoothness_scalar: f32,
    pub translucent: bool,
    pub metal_envmap: bool,
    /// FO4 envmap cube; its colour tints metal `_r` (FO4 had no other metal colour).
    pub envmap: String,
    pub metal_tint: Tint,
    pub envmap_mask_scale: f32,
    pub flat_gloss: bool,
    /// Highest per-texel envmap mask of `_s` at EnvironmentMappingMaskScale 1.
    pub envmap_mask_peak: f32,
    pub status: UnitStatus,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
    pub reflectance: String,
    pub lighting: String,
    /// Metal-darkened copy of `diffuse`; empty when no texel is metal.
    pub metal_diffuse: String,
    /// Upgraded BGSM / NIF / terrain-bank prefix written into the output mod.
    pub output: String,
    #[serde(skip)]
    pub nif_texture_sets: Vec<usize>,
}

impl Unit {
    fn new(kind: UnitKind, key: String, class: MaterialClass) -> Self {
        Self {
            kind,
            key,
            class,
            diffuse: String::new(),
            normal: String::new(),
            smoothspec: String::new(),
            smoothness_scalar: 1.0,
            translucent: false,
            metal_envmap: false,
            envmap: String::new(),
            metal_tint: NEUTRAL,
            envmap_mask_scale: 1.0,
            flat_gloss: false,
            envmap_mask_peak: 0.0,
            status: UnitStatus::Upgrade,
            reason: None,
            warnings: Vec::new(),
            reflectance: String::new(),
            lighting: String::new(),
            metal_diffuse: String::new(),
            output: String::new(),
            nif_texture_sets: Vec::new(),
        }
    }

    pub fn skip(&mut self, reason: impl Into<String>) {
        self.status = UnitStatus::Skipped;
        self.reason = Some(reason.into());
    }

    pub fn is_upgrade(&self) -> bool {
        self.status == UnitStatus::Upgrade
    }

    fn inputs(&self) -> impl Iterator<Item = &String> {
        [&self.key, &self.diffuse, &self.normal, &self.smoothspec].into_iter()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReflectanceSource {
    Constant(f32),
    TintedMetal(Tint),
    SpecMask(String, Tint),
    /// `spec: None` = metal everywhere; with `diffuse` the F0 hue blends `tint` with it.
    EnvmapMask { spec: Option<String>, diffuse: Option<String>, tint: Tint, mask_scale: f32 },
    UserMask(std::path::PathBuf, Tint),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextureJob {
    Lighting { out: String, spec: Option<String>, normal: Option<String>, curve: SmoothnessCurve, flat_value: f32, sss: f32 },
    Reflectance { out: String, source: ReflectanceSource },
    /// `spec: None` = metal everywhere (envmap metal without `_s`).
    MetalDiffuse { out: String, diffuse: String, spec: Option<String>, mask_scale: f32 },
    TerrainLayer { prefix: String, layer: TerrainLayerSource, curve: SmoothnessCurve, flat_value: f32, f0: f32, size: u32 },
}

impl TextureJob {
    pub fn outputs(&self) -> Vec<String> {
        match self {
            TextureJob::Lighting { out, .. } | TextureJob::Reflectance { out, .. } | TextureJob::MetalDiffuse { out, .. } => {
                vec![out.clone()]
            }
            TextureJob::TerrainLayer { prefix, .. } => {
                ["d", "n", "l", "r"].iter().map(|m| format!("{prefix}_source_{m}.dds")).collect()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Summary {
    pub units: usize,
    pub upgrade: usize,
    pub skipped: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub by_class: BTreeMap<String, usize>,
    pub skip_reasons: BTreeMap<String, usize>,
    pub texture_outputs: usize,
    pub warnings: usize,
}

pub struct Plan {
    pub units: Vec<Unit>,
    pub textures: Vec<TextureJob>,
    /// LTEX/TXST and worldspace LAND read for `--terrain`; the sidecars need them after execution.
    pub terrain: Option<std::sync::Arc<crate::terrain::TerrainRecords>>,
}

impl Plan {
    pub fn summary(&self) -> Summary {
        let mut s = Summary { units: self.units.len(), ..Default::default() };
        for u in &self.units {
            *s.by_kind.entry(serde_json::to_value(u.kind).unwrap().as_str().unwrap().to_string()).or_default() += 1;
            s.warnings += u.warnings.len();
            if u.is_upgrade() {
                s.upgrade += 1;
                *s.by_class.entry(u.class.name().to_string()).or_default() += 1;
            } else {
                s.skipped += 1;
                let reason = u.reason.clone().unwrap_or_default();
                let bucket = reason.split(':').next().unwrap_or("").to_string();
                *s.skip_reasons.entry(bucket).or_default() += 1;
            }
        }
        s.texture_outputs = self.textures.iter().map(|t| t.outputs().len()).sum();
        s
    }
}

fn prefix_allowed(opts: &UpgradeOptions, key: &str) -> bool {
    opts.include_prefixes.is_empty() || opts.include_prefixes.iter().any(|p| key.starts_with(&normalize(p)))
}

/// Only assets an upgradable source supplies are planned; an overlay's own copy wins and stays as
/// authored, unless the caller names it in `overlay_upgrade_paths`.
fn upgradable(opts: &UpgradeOptions, sources: &AssetSources, key: &str) -> bool {
    prefix_allowed(opts, key)
        && sources.winning_source(key).is_some_and(|i| {
            (opts.base_sources.len()..opts.base_sources.len() + opts.sources.len()).contains(&i)
                || opts.overlay_upgrade_paths.iter().any(|path| normalize(path) == key)
        })
}

fn stem_without(path: &str, suffix: &str) -> String {
    path.strip_suffix(suffix).unwrap_or_else(|| path.strip_suffix(".dds").unwrap_or(path)).to_string()
}

fn clean(s: &str) -> String {
    s.trim_end_matches('\0').to_string()
}

// ------------------------------------------------------------------ scanning

fn material_unit(sources: &AssetSources, key: &str) -> Unit {
    let bytes = match sources.read(key) {
        Ok(bytes) => bytes,
        Err(err) => {
            let mut u = Unit::new(UnitKind::Material, key.to_string(), MaterialClass::Other);
            u.skip(format!("unreadable: {err}"));
            return u;
        }
    };
    let data = match materials_native::bgsm::parse(&bytes) {
        Ok(data) => data,
        Err(err) => {
            let mut u = Unit::new(UnitKind::Material, key.to_string(), MaterialClass::Other);
            u.skip(format!("unreadable: {err}"));
            return u;
        }
    };
    let envmap = clean(data.EnvmapTexture.as_deref().unwrap_or(""));
    let class = classify_material(&ClassHints {
        diffuse: &clean(&data.DiffuseTexture),
        root_material: &clean(&data.RootMaterialPath),
        envmap: &envmap,
        material_path: key,
        skin_tint: data.SkinTint,
        facegen: data.Facegen,
        hair: data.Hair,
        tree: data.Tree,
    });
    let mut u = Unit::new(UnitKind::Material, key.to_string(), class);
    u.diffuse = texture_key(&data.DiffuseTexture);
    u.normal = texture_key(&data.NormalTexture);
    u.smoothspec = texture_key(&data.SmoothSpecTexture);
    u.smoothness_scalar = data.Smoothness;
    u.metal_envmap = envmap_is_metal(&envmap);
    if u.metal_envmap {
        u.envmap = texture_key(&envmap);
        u.envmap_mask_scale = data.header.env_mapping_mask_scale.unwrap_or(1.0);
    }
    u.translucent = class == MaterialClass::Organic && (data.SubsurfaceLighting == Some(true) || data.Tree);
    u.output = key.to_string();
    if is_already_v2x(&data) {
        u.skip("already BGSM v20+");
    }
    if ENGINE_SCREEN_MATERIALS.contains(&key) {
        u.skip("engine screen material");
    }
    if data.EmitEnabled || data.Glowmap {
        u.warnings.push("emissive: LumEmittance 100 is exposure-referred, review brightness".into());
    }
    u
}

/// Drawn by the engine's own Pip-Boy screen technique, which reads the vanilla scalars (SpecularMult, Smoothness,
/// emittance), never PBR maps: upgraded, the screen glares white and loses its glow.
const ENGINE_SCREEN_MATERIALS: [&str; 2] =
    ["materials/animobjects/pipboy/pipboyscreen.bgsm", "materials/interface/pipscreenterminals.bgsm"];

/// A NIF texture set whose shader names a BGSM: (nif key, texture set block, material key).
type MaterialLink = (String, usize, String);

fn nif_units(sources: &AssetSources, key: &str) -> (Vec<Unit>, Vec<MaterialLink>) {
    use nif_core_native::model::{NifFile, NifValue};
    let Ok(bytes) = sources.read(key) else { return Default::default() };
    let Ok(nif) = NifFile::from_bytes(&bytes, None) else { return Default::default() };
    let (mut units, mut links) = (Vec::new(), Vec::new());
    for shader in nif.blocks.iter().filter(|b| b.type_name == "BSLightingShaderProperty") {
        let name = match shader.get_field("Name") {
            Some(NifValue::String(name)) => normalize(name),
            _ => String::new(),
        };
        let Some(set_id) = shader.get_field("Texture Set").map(NifValue::as_i64).filter(|id| *id >= 0) else {
            continue;
        };
        if name.ends_with(".bgsm") {
            // Only FO4's envmap shaders: other BGSM-named sets are bound from the BGSM as they are.
            let envmap_type = shader.get_field("Shader Type").map(NifValue::as_i64) == Some(1);
            let envmap_flag = shader.get_field("Shader Flags 1").map(NifValue::as_i64).is_some_and(|f| f & (1 << 7) != 0);
            if envmap_type || envmap_flag {
                links.push((key.to_string(), set_id as usize, name));
            }
            continue;
        }
        if name.ends_with(".bgem") {
            continue;
        }
        let Some(NifValue::Array(textures)) = nif.get_block(set_id as usize).and_then(|b| b.get_field("Textures")) else {
            continue;
        };
        let slot = |i: usize| match textures.get(i) {
            Some(NifValue::String(path)) => texture_key(path),
            _ => String::new(),
        };
        let (diffuse, normal, smoothspec) = (slot(0), slot(1), slot(7));
        if diffuse.is_empty() {
            continue;
        }
        let class = classify_material(&ClassHints { diffuse: &diffuse, material_path: key, ..Default::default() });
        let mut u = Unit::new(UnitKind::NifTextureSet, format!("{key}#{set_id}"), class);
        u.diffuse = diffuse;
        u.normal = normal;
        u.smoothspec = smoothspec;
        u.output = key.to_string();
        u.nif_texture_sets.push(set_id as usize);
        if !slot(9).is_empty() {
            u.skip("already has a reflectance slot");
        } else if textures.len() < 10 {
            u.skip("texture set has fewer than 10 slots");
        }
        units.push(u);
    }
    (units, links)
}

/// The FO4 engine and FO4CS still read the inline texture set and shader type of a NIF whose
/// shader names a BGSM, so each such set mirrors its upgraded material.
fn material_linked_nif_units(units: &[Unit], links: &[MaterialLink]) -> Vec<Unit> {
    let materials: HashMap<&str, &Unit> =
        units.iter().filter(|u| u.is_upgrade() && u.kind == UnitKind::Material).map(|u| (u.key.as_str(), u)).collect();
    links
        .iter()
        .filter_map(|(nif, set_id, material)| {
            let mut u = (*materials.get(material.as_str())?).clone();
            u.kind = UnitKind::NifTextureSet;
            u.key = format!("{nif}#{set_id}");
            u.output = nif.clone();
            u.nif_texture_sets = vec![*set_id];
            u.warnings.clear();
            Some(u)
        })
        .collect()
}

fn decide_flatness(sources: &AssetSources, units: &mut [Unit]) {
    let specs: HashSet<String> = units
        .iter()
        .filter(|u| u.is_upgrade() && !u.smoothspec.is_empty())
        .map(|u| u.smoothspec.clone())
        .collect();
    let flat: HashMap<String, Option<(bool, f32)>> = specs
        .into_par_iter()
        .map(|spec| {
            let verdict = sources
                .read(&spec)
                .ok()
                .and_then(|bytes| directxtex_native::read_dds_rgba_image_bytes(&bytes).ok())
                .map(|img| (gloss_is_flat(&img.rgba), envmap_metal_mask(&img.rgba, 1.0).fold(0.0, f32::max)));
            (spec, verdict)
        })
        .collect();
    for u in units.iter_mut().filter(|u| u.is_upgrade()) {
        match flat.get(&u.smoothspec).copied().flatten() {
            Some((is_flat, peak)) => {
                u.flat_gloss = is_flat;
                u.envmap_mask_peak = peak;
            }
            None => {
                if !u.smoothspec.is_empty() {
                    u.warnings.push(format!("smoothspec missing: {}", u.smoothspec));
                }
                u.smoothspec.clear();
                u.flat_gloss = true;
            }
        }
        if u.flat_gloss {
            u.warnings.push("flat or missing _s: class default smoothness".into());
        }
    }
}

fn decide_metal_tint(sources: &AssetSources, units: &mut [Unit]) {
    let cubes: HashSet<String> = units
        .iter()
        .filter(|u| u.is_upgrade() && !u.envmap.is_empty())
        .map(|u| u.envmap.clone())
        .collect();
    let tints: HashMap<String, Option<Tint>> = cubes
        .into_par_iter()
        .map(|cube| {
            let tint = sources
                .read(&cube)
                .ok()
                .and_then(|bytes| directxtex_native::read_dds_rgba_image_bytes(&bytes).ok())
                .map(|img| cube_tint(&img.rgba));
            (cube, tint)
        })
        .collect();
    for u in units.iter_mut().filter(|u| u.is_upgrade() && !u.envmap.is_empty()) {
        match tints.get(&u.envmap).copied().flatten() {
            Some(tint) => u.metal_tint = tint,
            None => u.warnings.push(format!("envmap unreadable, metal stays grey: {}", u.envmap)),
        }
    }
}

fn tint_tag(tint: Tint) -> String {
    let [r, g, b] = tinted_metal_srgb(tint);
    format!("{r:02x}{g:02x}{b:02x}")
}

// --------------------------------------------------------------- selection

/// The upgrade-vs-replace rule: a unit is not upgraded when its material, any of
/// its source textures or any texture it would write is in `excluded` (the set of
/// data paths being replaced by native versions). Compute the replace set first,
/// then pass it here.
pub fn apply_exclusions(units: &mut [Unit], excluded: &HashSet<String>) {
    if excluded.is_empty() {
        return;
    }
    for u in units.iter_mut().filter(|u| u.is_upgrade()) {
        let per_asset_outputs = [&u.reflectance, &u.lighting, &u.metal_diffuse]
            .into_iter()
            .filter(|p| !p.starts_with(SHARED_DIR));
        let hit = u
            .inputs()
            .chain(per_asset_outputs)
            .chain(std::iter::once(&u.output))
            .filter(|p| !p.is_empty())
            .any(|p| excluded.contains(&normalize(p.split('#').next().unwrap_or(p))));
        if hit {
            u.skip("replaced by native asset");
        }
    }
}

fn apply_class_filter(units: &mut [Unit], opts: &UpgradeOptions) {
    let enabled: HashSet<MaterialClass> = opts.classes.iter().copied().collect();
    for u in units.iter_mut().filter(|u| u.is_upgrade()) {
        if !enabled.contains(&u.class) {
            u.skip(format!("class disabled: {}", u.class.name()));
        }
    }
}

// ------------------------------------------------------------------ naming

struct Namer<'a> {
    sources: &'a AssetSources,
    claimed: HashMap<String, String>,
}

impl<'a> Namer<'a> {
    /// First claimant of `base` (by identity) keeps it; a different identity gets
    /// a suffixed name. Names that already exist in the sources are never reused.
    fn claim(&mut self, base: &str, identity: &str, alt_suffix: &str) -> String {
        let mut candidate = base.to_string();
        let mut n = 0;
        loop {
            match self.claimed.get(&candidate) {
                Some(owner) if owner == identity => return candidate,
                None if !self.sources.contains(&candidate) => {
                    self.claimed.insert(candidate.clone(), identity.to_string());
                    return candidate;
                }
                _ => {
                    n += 1;
                    let stem = base.strip_suffix(alt_suffix).unwrap_or(base);
                    candidate = format!("{stem}_u{n}{alt_suffix}");
                }
            }
        }
    }
}

fn quantize(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 100.0).round() as u32
}

fn assign_outputs(units: &mut [Unit], opts: &UpgradeOptions, sources: &AssetSources) -> Vec<TextureJob> {
    let mut namer = Namer { sources, claimed: HashMap::new() };
    let mut jobs: BTreeMap<String, TextureJob> = BTreeMap::new();
    for u in units.iter_mut().filter(|u| u.is_upgrade() && u.kind != UnitKind::TerrainLayer) {
        let curve = SmoothnessCurve { scalar: u.smoothness_scalar, gain: opts.smoothness_gain, knee: opts.knee };
        let sss = if u.translucent { 1.0 } else { 0.0 };
        let flat_value = u.class.default_smoothness();
        let sss_tag = if u.translucent { "_sss" } else { "" };

        u.lighting = if u.flat_gloss {
            let out = format!("{SHARED_DIR}/smooth_{:02}{sss_tag}_l.dds", quantize(flat_value * opts.smoothness_gain));
            jobs.entry(out.clone()).or_insert_with(|| TextureJob::Lighting {
                out: out.clone(),
                spec: None,
                normal: None,
                curve,
                flat_value: (flat_value * opts.smoothness_gain).clamp(0.0, 1.0),
                sss,
            });
            out
        } else {
            let base = format!("{}{sss_tag}_l.dds", stem_without(&u.smoothspec, "_s.dds"));
            let identity = format!("{}|{}|{sss}|{}", u.smoothspec, quantize(u.smoothness_scalar), u.normal);
            let out = namer.claim(&base, &identity, "_l.dds");
            jobs.entry(out.clone()).or_insert_with(|| TextureJob::Lighting {
                out: out.clone(),
                spec: Some(u.smoothspec.clone()),
                normal: (!u.normal.is_empty()).then(|| u.normal.clone()),
                curve,
                flat_value,
                sss,
            });
            out
        };

        let user_mask = match (&opts.metal_mode, &opts.metal_mask_dir) {
            (MetalMode::UserMask, Some(dir)) => {
                let rel = stem_without(u.diffuse.strip_prefix("textures/").unwrap_or(&u.diffuse), "_d.dds");
                Some(dir.join(format!("{rel}_metal.dds"))).filter(|p| p.is_file())
            }
            _ => None,
        };
        let blend_diffuse = sources.contains(&u.diffuse).then(|| u.diffuse.clone());
        u.reflectance = match opts.metal_mode {
            // FO4's _s decides per texel how much envmap shows; that share becomes metal.
            MetalMode::Envmap if u.metal_envmap && (!u.smoothspec.is_empty() || blend_diffuse.is_some()) => {
                let spec = (!u.smoothspec.is_empty()).then(|| u.smoothspec.clone());
                let stem = match &spec {
                    Some(s) => stem_without(s, "_s.dds"),
                    None => stem_without(&u.diffuse, "_d.dds"),
                };
                let identity = format!(
                    "{}|{}|envmap|{}|{}",
                    u.smoothspec,
                    blend_diffuse.as_deref().unwrap_or(""),
                    tint_tag(u.metal_tint),
                    u.envmap_mask_scale
                );
                let out = namer.claim(&format!("{stem}_r.dds"), &identity, "_r.dds");
                jobs.entry(out.clone()).or_insert_with(|| TextureJob::Reflectance {
                    out: out.clone(),
                    source: ReflectanceSource::EnvmapMask {
                        spec,
                        diffuse: blend_diffuse.clone(),
                        tint: u.metal_tint,
                        mask_scale: u.envmap_mask_scale,
                    },
                });
                out
            }
            MetalMode::Envmap if u.metal_envmap && !is_neutral(u.metal_tint) => {
                let out = format!("{SHARED_DIR}/metal_{}_r.dds", tint_tag(u.metal_tint));
                jobs.entry(out.clone())
                    .or_insert_with(|| TextureJob::Reflectance { out: out.clone(), source: ReflectanceSource::TintedMetal(u.metal_tint) });
                out
            }
            MetalMode::Envmap if u.metal_envmap => {
                jobs.entry(METAL_R.into()).or_insert(TextureJob::Reflectance {
                    out: METAL_R.into(),
                    source: ReflectanceSource::Constant(crate::kernels::METAL_F0),
                });
                METAL_R.into()
            }
            MetalMode::Mask if u.class == MaterialClass::Metal && !u.flat_gloss => {
                let base = format!("{}_r.dds", stem_without(&u.smoothspec, "_s.dds"));
                let out = namer.claim(&base, &format!("{}|mask|{}", u.smoothspec, tint_tag(u.metal_tint)), "_r.dds");
                u.warnings.push("metal mask from _s.R is low precision".into());
                jobs.entry(out.clone()).or_insert_with(|| TextureJob::Reflectance {
                    out: out.clone(),
                    source: ReflectanceSource::SpecMask(u.smoothspec.clone(), u.metal_tint),
                });
                out
            }
            MetalMode::UserMask if user_mask.is_some() => {
                let mask = user_mask.unwrap();
                let base = format!("{}_r.dds", stem_without(&u.diffuse, "_d.dds"));
                let out = namer.claim(&base, &format!("{}|{}", mask.display(), tint_tag(u.metal_tint)), "_r.dds");
                jobs.entry(out.clone()).or_insert_with(|| TextureJob::Reflectance {
                    out: out.clone(),
                    source: ReflectanceSource::UserMask(mask, u.metal_tint),
                });
                out
            }
            _ => {
                jobs.entry(DIELECTRIC_R.into()).or_insert(TextureJob::Reflectance {
                    out: DIELECTRIC_R.into(),
                    source: ReflectanceSource::Constant(crate::kernels::DIELECTRIC_F0),
                });
                DIELECTRIC_R.into()
            }
        };

        // Some(None): metal everywhere; Some(Some(_s)): per-texel envmap mask.
        let metal_spec = if opts.metal_mode != MetalMode::Envmap || !u.metal_envmap || !sources.contains(&u.diffuse) {
            None
        } else if u.smoothspec.is_empty() {
            Some(None)
        } else if u.envmap_mask_peak * u.envmap_mask_scale >= METAL_DIFFUSE_MIN_MASK {
            Some(Some(u.smoothspec.clone()))
        } else {
            None
        };
        if let Some(spec) = metal_spec {
            let base = format!("{}_metal_d.dds", stem_without(&u.diffuse, "_d.dds"));
            let identity = format!("{}|{}|{}", u.diffuse, spec.as_deref().unwrap_or(""), u.envmap_mask_scale);
            let out = namer.claim(&base, &identity, "_metal_d.dds");
            jobs.entry(out.clone()).or_insert_with(|| TextureJob::MetalDiffuse {
                out: out.clone(),
                diffuse: u.diffuse.clone(),
                spec,
                mask_scale: u.envmap_mask_scale,
            });
            u.metal_diffuse = out;
        }
    }
    for u in units.iter_mut().filter(|u| u.is_upgrade() && u.kind == UnitKind::TerrainLayer) {
        let prefix = stem_without(&u.diffuse, "_d.dds");
        u.output = format!("{prefix}_source");
        u.lighting = format!("{prefix}_source_l.dds");
        let layer = TerrainLayerSource {
            diffuse: u.diffuse.clone(),
            normal: u.normal.clone(),
            smoothspec: (!u.flat_gloss).then(|| u.smoothspec.clone()),
        };
        let curve = SmoothnessCurve { scalar: 1.0, gain: opts.smoothness_gain, knee: opts.knee };
        jobs.entry(u.output.clone()).or_insert_with(|| TextureJob::TerrainLayer {
            prefix: u.output.clone(),
            layer,
            curve,
            flat_value: (u.class.default_smoothness() * opts.smoothness_gain).clamp(0.0, 1.0),
            f0: crate::terrain_btd4::terrain_reflectance(&u.diffuse),
            size: opts.terrain_layer_size,
        });
    }
    jobs.into_values().collect()
}

// -------------------------------------------------------------------- plan

pub fn build_plan(opts: &UpgradeOptions, sources: &AssetSources) -> Result<Plan, String> {
    opts.validate()?;
    let mut units: Vec<Unit> = Vec::new();
    let mut links: Vec<MaterialLink> = Vec::new();
    if opts.include_materials {
        let keys: Vec<String> = sources
            .keys_with("materials/", ".bgsm")
            .into_iter()
            .filter(|k| upgradable(opts, sources, k))
            .collect();
        units.par_extend(keys.par_iter().map(|key| material_unit(sources, key)));
    }
    if opts.include_nifs {
        let keys: Vec<String> = sources
            .keys_with("meshes/", ".nif")
            .into_iter()
            .filter(|k| upgradable(opts, sources, k))
            .collect();
        let nested: Vec<(Vec<Unit>, Vec<MaterialLink>)> = keys.par_iter().map(|key| nif_units(sources, key)).collect();
        for (nif, nif_links) in nested {
            units.extend(nif);
            links.extend(nif_links);
        }
    }
    let mut terrain = None;
    if opts.include_terrain {
        let (terrain_units, records) = crate::terrain::terrain_units(&opts.plugins, sources)?;
        units.extend(terrain_units);
        terrain = Some(std::sync::Arc::new(records));
    }
    units.sort_by(|a, b| (a.kind, &a.key).cmp(&(b.kind, &b.key)));
    apply_class_filter(&mut units, opts);
    decide_flatness(sources, &mut units);
    if opts.metal_mode != MetalMode::Off {
        decide_metal_tint(sources, &mut units);
    }
    let excluded: HashSet<String> = opts.exclude_paths.iter().map(|p| normalize(p)).collect();
    // Exclusions are checked against inputs first so replaced assets never claim names.
    apply_exclusions(&mut units, &excluded);
    let textures = assign_outputs(&mut units, opts, sources);
    let linked = material_linked_nif_units(&units, &links);
    units.extend(linked);
    units.sort_by(|a, b| (a.kind, &a.key).cmp(&(b.kind, &b.key)));
    // ...and again against the per-asset outputs they would write.
    apply_exclusions(&mut units, &excluded);
    let live: HashSet<&String> = units
        .iter()
        .filter(|u| u.is_upgrade())
        .flat_map(|u| [&u.lighting, &u.reflectance, &u.metal_diffuse, &u.output])
        .collect();
    let textures = textures
        .into_iter()
        .filter(|job| job.outputs().iter().any(|o| live.contains(o)) || matches!(job, TextureJob::TerrainLayer { prefix, .. } if live.contains(prefix)))
        .collect();
    Ok(Plan { units, textures, terrain })
}

impl Unit {
    pub(crate) fn for_test_or_terrain(kind: UnitKind, key: &str, class: MaterialClass) -> Self {
        Unit::new(kind, key.to_string(), class)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::bgsm_v22::tests::fo4_bgsm;

    fn write_dds(path: &std::path::Path, rgba: &[u8], w: u32, h: u32) {
        directxtex_native::write_dds_rgba_image(path, w, h, rgba, "BC5_UNORM", true).unwrap();
    }

    pub(crate) fn gradient_spec(w: u32, h: u32) -> Vec<u8> {
        (0..w * h).flat_map(|i| [200u8, (i * 255 / (w * h)) as u8, 0, 255]).collect()
    }

    /// Two materials sharing one `_s`, one with a different Smoothness scalar,
    /// a flat-spec material and a v22 material.
    pub(crate) fn fixture() -> (tempfile::TempDir, UpgradeOptions) {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        let write_bgsm = |rel: &str, mutate: &dyn Fn(&mut materials_native::bgsm::BgsmData)| {
            let mut m = fo4_bgsm();
            mutate(&mut m);
            let path = data.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, materials_native::bgsm::write(&m)).unwrap();
        };
        write_bgsm("Materials/Props/Can.bgsm", &|m| {
            m.DiffuseTexture = "Props/Can_d.dds".into();
            m.SmoothSpecTexture = "Props/Can_s.dds".into();
            m.Smoothness = 1.0;
        });
        write_bgsm("Materials/Props/CanDim.bgsm", &|m| {
            m.DiffuseTexture = "Props/Can_d.dds".into();
            m.SmoothSpecTexture = "Props/Can_s.dds".into();
            m.Smoothness = 0.5;
        });
        write_bgsm("Materials/Architecture/Concrete.bgsm", &|m| {
            m.DiffuseTexture = "Architecture/Concrete_d.dds".into();
            m.SmoothSpecTexture = "Architecture/Concrete_s.dds".into();
            m.RootMaterialPath = "Template/BrickTemplate_Wet.bgsm".into();
        });
        write_bgsm("Materials/Props/Already.bgsm", &|m| m.header.version = 22);
        write_bgsm("Materials/Actors/Character/Head.bgsm", &|m| m.SkinTint = true);
        write_bgsm("Materials/AnimObjects/PipBoy/PipBoyScreen.bgsm", &|m| {
            m.DiffuseTexture = "AnimObjects/PipBoy/PipBoyScreen_d.dds".into();
        });
        let tex = data.join("Textures");
        std::fs::create_dir_all(tex.join("Props")).unwrap();
        std::fs::create_dir_all(tex.join("Architecture")).unwrap();
        write_dds(&tex.join("Props/Can_s.dds"), &gradient_spec(16, 16), 16, 16);
        write_dds(&tex.join("Architecture/Concrete_s.dds"), &[200u8, 240, 0, 255].repeat(256), 16, 16);
        let opts = UpgradeOptions { sources: vec![data], output: tmp.path().join("out"), ..Default::default() };
        (tmp, opts)
    }

    fn unit<'a>(plan: &'a Plan, key: &str) -> &'a Unit {
        plan.units.iter().find(|u| u.key == key).unwrap()
    }

    #[test]
    fn plan_classifies_names_and_skips() {
        let (_tmp, opts) = fixture();
        let sources = AssetSources::open(&opts.sources).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        let can = unit(&plan, "materials/props/can.bgsm");
        assert!(can.is_upgrade());
        assert_eq!(can.lighting, "textures/props/can_l.dds");
        assert_eq!(can.reflectance, DIELECTRIC_R);
        let dim = unit(&plan, "materials/props/candim.bgsm");
        assert_eq!(dim.lighting, "textures/props/can_u1_l.dds", "different scalar needs its own _l");
        let concrete = unit(&plan, "materials/architecture/concrete.bgsm");
        assert_eq!(concrete.class, MaterialClass::Stone);
        assert!(concrete.flat_gloss);
        assert_eq!(concrete.lighting, "textures/materialupgrade/smooth_21_l.dds");
        assert_eq!(unit(&plan, "materials/props/already.bgsm").reason.as_deref(), Some("already BGSM v20+"));
        assert_eq!(
            unit(&plan, "materials/actors/character/head.bgsm").reason.as_deref(),
            Some("class disabled: skin")
        );
        assert_eq!(
            unit(&plan, "materials/animobjects/pipboy/pipboyscreen.bgsm").reason.as_deref(),
            Some("engine screen material")
        );
        let summary = plan.summary();
        assert_eq!(summary.upgrade, 3);
        assert_eq!(summary.skipped, 3);
        // dielectric_r, can_l, can_u1_l, smooth_21_l
        assert_eq!(plan.textures.len(), 4);
    }

    #[test]
    fn metal_modes_pick_reflectance() {
        let (_tmp, mut opts) = fixture();
        let sources = AssetSources::open(&opts.sources).unwrap();
        opts.metal_mode = MetalMode::Mask;
        opts.classes = MaterialClass::DEFAULT_ENABLED.to_vec();
        let plan = build_plan(&opts, &sources).unwrap();
        // metal-class with a varying _s gets a per-texel mask; other classes stay dielectric
        assert_eq!(unit(&plan, "materials/props/can.bgsm").reflectance, "textures/props/can_r.dds");
        assert_eq!(unit(&plan, "materials/architecture/concrete.bgsm").reflectance, DIELECTRIC_R);
    }

    #[test]
    fn overlay_materials_are_never_upgraded() {
        let (tmp, mut opts) = fixture();
        // A converted mod ships its own pre-v22 Can.bgsm and a material FO4 lacks.
        let overlay = tmp.path().join("Converted");
        for rel in ["Materials/Props/Can.bgsm", "Materials/WhiteSpring/Gold.bgsm"] {
            let path = overlay.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, materials_native::bgsm::write(&fo4_bgsm())).unwrap();
        }
        opts.overlay_sources = vec![overlay];
        let all: Vec<_> = opts.sources.iter().chain(&opts.overlay_sources).cloned().collect();
        let sources = AssetSources::open(&all).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        assert!(!plan.units.iter().any(|u| u.key == "materials/props/can.bgsm"), "the overlay's copy wins");
        assert!(!plan.units.iter().any(|u| u.key == "materials/whitespring/gold.bgsm"));
        assert!(unit(&plan, "materials/props/candim.bgsm").is_upgrade(), "FO4-only materials still upgrade");
    }

    #[test]
    fn named_overlay_leftovers_are_upgraded() {
        let (tmp, mut opts) = fixture();
        let overlay = tmp.path().join("Converted");
        for rel in ["Materials/Clothes/Leftover.bgsm", "Materials/WhiteSpring/Gold.bgsm"] {
            let path = overlay.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, materials_native::bgsm::write(&fo4_bgsm())).unwrap();
        }
        opts.overlay_sources = vec![overlay];
        opts.overlay_upgrade_paths = vec!["Materials\\Clothes\\Leftover.bgsm".into()];
        let all: Vec<_> = opts.sources.iter().chain(&opts.overlay_sources).cloned().collect();
        let sources = AssetSources::open(&all).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        assert!(unit(&plan, "materials/clothes/leftover.bgsm").is_upgrade());
        assert!(!plan.units.iter().any(|u| u.key == "materials/whitespring/gold.bgsm"));
    }

    #[test]
    fn base_sources_feed_textures_but_are_never_upgraded() {
        let (tmp, mut opts) = fixture();
        let mod_dir = tmp.path().join("MyMod");
        let path = mod_dir.join("Materials/MyMod/Thing.bgsm");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, materials_native::bgsm::write(&fo4_bgsm())).unwrap();
        opts.base_sources = std::mem::replace(&mut opts.sources, vec![mod_dir]);
        let all: Vec<_> = opts.base_sources.iter().chain(&opts.sources).cloned().collect();
        let sources = AssetSources::open(&all).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        assert!(unit(&plan, "materials/mymod/thing.bgsm").is_upgrade(), "its textures come from the base Data");
        assert!(plan.units.iter().all(|u| u.key == "materials/mymod/thing.bgsm"), "base materials stay as they are");
    }

    #[test]
    fn replace_set_wins_over_upgrade() {
        let (_tmp, mut opts) = fixture();
        let sources = AssetSources::open(&opts.sources).unwrap();
        // The replace set names one material directly and one only through its texture.
        opts.exclude_paths = vec!["materials/props/can.bgsm".into(), "textures/architecture/concrete_s.dds".into()];
        let plan = build_plan(&opts, &sources).unwrap();
        assert_eq!(unit(&plan, "materials/props/can.bgsm").reason.as_deref(), Some("replaced by native asset"));
        assert_eq!(
            unit(&plan, "materials/architecture/concrete.bgsm").reason.as_deref(),
            Some("replaced by native asset")
        );
        let dim = unit(&plan, "materials/props/candim.bgsm");
        assert!(dim.is_upgrade(), "not in the replace set -> upgraded");
        assert_eq!(dim.lighting, "textures/props/can_l.dds", "replaced units claim no names");
    }

    #[test]
    fn replace_set_hits_per_asset_outputs() {
        let (_tmp, mut opts) = fixture();
        let sources = AssetSources::open(&opts.sources).unwrap();
        opts.exclude_paths = vec!["textures/props/can_l.dds".into()];
        let plan = build_plan(&opts, &sources).unwrap();
        assert_eq!(unit(&plan, "materials/props/can.bgsm").reason.as_deref(), Some("replaced by native asset"));
        assert!(plan.textures.iter().all(|t| !t.outputs().contains(&"textures/props/can_l.dds".to_string())));
    }
}

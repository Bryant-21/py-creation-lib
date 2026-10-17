//! Write a plan into the output mod. `_d`/`_n` are never re-encoded in place; the
//! raw-terrain bank (whose contract demands BC3 sRGB / BC5 SNORM at one size) writes
//! new copies of layer diffuse and normal maps, and envmap metals get a darkened
//! `_metal_d` copy (the FO4 `_d` stays shared with vanilla meshes).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::bgsm_v22::upgrade_bgsm;
use crate::kernels::{
    Tint, constant_lighting, normal_map_ao, constant_reflectance, is_neutral, lighting_from_specgloss, reflectance_from_mask,
    blended_reflectance_from_mask, darken_metal_diffuse, envmap_metal_mask, resize_rgba8, spec_metal_mask, terrain_layer_lighting, tinted_reflectance_from_mask, user_metal_mask,
};
use crate::options::UpgradeOptions;
use crate::plan::{Plan, ReflectanceSource, TextureJob, Unit, UnitKind};
use crate::sources::{AssetSources, material_texture_ref};

pub const LIGHTING_FORMAT: &str = "BC1_UNORM";
pub const REFLECTANCE_FORMAT: &str = "BC4_UNORM";
/// Coloured metal `_r`, as FO76 ships it; FO4CS reads `.rgb` as linear F0 through the sRGB view.
pub const TINTED_REFLECTANCE_FORMAT: &str = "BC1_UNORM_SRGB";
pub const TERRAIN_FORMATS: [(&str, &str); 4] =
    [("d", "BC3_UNORM_SRGB"), ("n", "BC5_SNORM"), ("l", "BC3_UNORM"), ("r", terrain_native::raw_terrain::REFLECTANCE_FORMAT)];
pub const REPORT_PATH: &str = "materialupgrade/upgrade_report.json";
pub const TERRAIN_MANIFEST_PATH: &str = "materialupgrade/terrain_layers.json";

#[derive(Debug, Clone, Serialize, Default)]
pub struct RunResult {
    pub written: Vec<String>,
    pub errors: Vec<(String, String)>,
    pub cancelled: bool,
}

struct Ctx<'a> {
    sources: &'a AssetSources,
    out: &'a Path,
}

impl Ctx<'_> {
    fn path(&self, key: &str) -> PathBuf {
        self.out.join(key)
    }

    fn decode(&self, key: &str) -> Result<directxtex_native::DdsRgbaImage, String> {
        let bytes = self.sources.read(key)?;
        directxtex_native::read_dds_rgba_image_bytes(&bytes).map_err(|err| format!("decode {key}: {err}"))
    }

    fn write_rgba(&self, key: &str, w: u32, h: u32, rgba: &[u8], format: &str) -> Result<(), String> {
        directxtex_native::write_dds_rgba_image(&self.path(key), w, h, rgba, format, true)
            .map_err(|err| format!("encode {key}: {err}"))
    }

    fn write_reflectance_mask(&self, key: &str, w: u32, h: u32, mask: impl Iterator<Item = f32>, tint: Tint) -> Result<(), String> {
        if is_neutral(tint) {
            self.write_rgba(key, w, h, &reflectance_from_mask(mask), REFLECTANCE_FORMAT)
        } else {
            self.write_rgba(key, w, h, &tinted_reflectance_from_mask(mask, tint), TINTED_REFLECTANCE_FORMAT)
        }
    }
}

/// The darkened `_d` keeps its source encoding (the bytes stay sRGB-encoded either way).
fn diffuse_format(dxgi: u32) -> &'static str {
    match dxgi {
        71 => "BC1_UNORM",
        72 => "BC1_UNORM_SRGB",
        74 => "BC2_UNORM",
        75 => "BC2_UNORM_SRGB",
        77 => "BC3_UNORM",
        78 => "BC3_UNORM_SRGB",
        98 => "BC7_UNORM",
        99 => "BC7_UNORM_SRGB",
        28 => "R8G8B8A8_UNORM",
        29 => "R8G8B8A8_UNORM_SRGB",
        87 => "B8G8R8A8_UNORM",
        _ if directxtex_native::is_srgb_dxgi_format(dxgi) => "BC7_UNORM_SRGB",
        _ => "BC7_UNORM",
    }
}

/// Writes normal-map AO into `lighting`'s G channel (at the `_l` size). Without a GPU device AO
/// stays 1.0, as before this existed, and the run says so once.
fn bake_normal_ao(ctx: &Ctx<'_>, normal: &str, lighting: &mut [u8], width: u32, height: u32) -> Result<(), String> {
    let Ok(n) = ctx.decode(normal) else { return Ok(()) };
    let ao = match normal_map_ao(&n.rgba, n.width, n.height) {
        Ok(ao) => ao,
        Err(err) => {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| eprintln!("material upgrade: normal-map AO skipped, _l AO stays 1.0 ({err})"));
            return Ok(());
        }
    };
    let gray: Vec<u8> = ao.iter().flat_map(|&v| { let g = (v * 255.0 + 0.5) as u8; [g, g, g, 255] }).collect();
    let gray = resize_rgba8(&gray, n.width, n.height, width, height);
    for (px, g) in lighting.chunks_exact_mut(4).zip(gray.chunks_exact(4)) {
        px[1] = g[0];
    }
    Ok(())
}

fn run_job(ctx: &Ctx<'_>, job: &TextureJob) -> Result<Vec<String>, String> {
    match job {
        TextureJob::Lighting { out, spec: Some(spec), normal, curve, sss, .. } => {
            let img = ctx.decode(spec)?;
            let mut lighting = lighting_from_specgloss(&img.rgba, *curve, *sss);
            if let Some(normal) = normal {
                bake_normal_ao(ctx, normal, &mut lighting, img.width, img.height)?;
            }
            ctx.write_rgba(out, img.width, img.height, &lighting, LIGHTING_FORMAT)?;
        }
        TextureJob::Lighting { out, spec: None, flat_value, sss, .. } => {
            ctx.write_rgba(out, 4, 4, &constant_lighting(*flat_value, *sss, 4, 4), LIGHTING_FORMAT)?;
        }
        TextureJob::Reflectance { out, source: ReflectanceSource::Constant(f0) } => {
            ctx.write_rgba(out, 4, 4, &constant_reflectance(*f0, 4, 4), REFLECTANCE_FORMAT)?;
        }
        TextureJob::Reflectance { out, source: ReflectanceSource::TintedMetal(tint) } => {
            ctx.write_reflectance_mask(out, 4, 4, std::iter::repeat_n(1.0, 16), *tint)?;
        }
        TextureJob::Reflectance { out, source: ReflectanceSource::SpecMask(spec, tint) } => {
            let img = ctx.decode(spec)?;
            ctx.write_reflectance_mask(out, img.width, img.height, spec_metal_mask(&img.rgba), *tint)?;
        }
        TextureJob::Reflectance { out, source: ReflectanceSource::EnvmapMask { spec, diffuse, tint, mask_scale } } => {
            let spec = spec.as_deref().map(|s| ctx.decode(s)).transpose()?;
            let diffuse = diffuse.as_deref().map(|d| ctx.decode(d)).transpose()?;
            let (w, h) = match (&spec, &diffuse) {
                (Some(s), _) => (s.width, s.height),
                (None, Some(d)) => (d.width, d.height),
                (None, None) => unreachable!("planner gives EnvmapMask a spec or a diffuse"),
            };
            let mask: Vec<f32> = match &spec {
                Some(s) => envmap_metal_mask(&s.rgba, *mask_scale).collect(),
                None => vec![1.0; (w * h) as usize],
            };
            match diffuse {
                Some(d) => {
                    let d = resize_rgba8(&d.rgba, d.width, d.height, w, h);
                    let rgba = blended_reflectance_from_mask(mask.into_iter(), *tint, &d);
                    ctx.write_rgba(out, w, h, &rgba, TINTED_REFLECTANCE_FORMAT)?;
                }
                None => ctx.write_reflectance_mask(out, w, h, mask.into_iter(), *tint)?,
            }
        }
        TextureJob::Reflectance { out, source: ReflectanceSource::UserMask(path, tint) } => {
            let img = directxtex_native::read_dds_rgba_image(path).map_err(|err| format!("{}: {err}", path.display()))?;
            ctx.write_reflectance_mask(out, img.width, img.height, user_metal_mask(&img.rgba), *tint)?;
        }
        TextureJob::MetalDiffuse { out, diffuse, spec, mask_scale } => {
            let d = ctx.decode(diffuse)?;
            let pixels = (d.width * d.height) as usize;
            let rgba = match spec {
                Some(spec) => {
                    let s = ctx.decode(spec)?;
                    let s = resize_rgba8(&s.rgba, s.width, s.height, d.width, d.height);
                    darken_metal_diffuse(&d.rgba, envmap_metal_mask(&s, *mask_scale))
                }
                None => darken_metal_diffuse(&d.rgba, std::iter::repeat_n(1.0, pixels)),
            };
            ctx.write_rgba(out, d.width, d.height, &rgba, diffuse_format(d.dxgi_format))?;
        }
        TextureJob::TerrainLayer { prefix, layer, curve, flat_value, f0, size } => {
            let s = *size;
            let diffuse = ctx.decode(&layer.diffuse)?;
            let d = resize_rgba8(&diffuse.rgba, diffuse.width, diffuse.height, s, s);
            ctx.write_rgba(&format!("{prefix}_source_d.dds"), s, s, &d, TERRAIN_FORMATS[0].1)?;

            let normal = ctx.decode(&layer.normal)?;
            let n = resize_rgba8(&normal.rgba, normal.width, normal.height, s, s);
            // DirectXTex maps UNORM input to [-1, 1] when it encodes an SNORM target.
            ctx.write_rgba(&format!("{prefix}_source_n.dds"), s, s, &n, TERRAIN_FORMATS[1].1)?;

            let spec = match &layer.smoothspec {
                Some(spec) => {
                    let img = ctx.decode(spec)?;
                    Some(resize_rgba8(&img.rgba, img.width, img.height, s, s))
                }
                None => None,
            };
            let l = terrain_layer_lighting(spec.as_deref(), *curve, *flat_value, (s * s) as usize);
            ctx.write_rgba(&format!("{prefix}_source_l.dds"), s, s, &l, TERRAIN_FORMATS[2].1)?;

            let r = terrain_native::raw_terrain::REFLECTANCE_SIZE;
            ctx.write_rgba(&format!("{prefix}_source_r.dds"), r, r, &constant_reflectance(*f0, r, r), TERRAIN_FORMATS[3].1)?;
        }
    }
    Ok(job.outputs())
}

fn write_material(ctx: &Ctx<'_>, unit: &Unit) -> Result<(), String> {
    let data = materials_native::bgsm::parse(&ctx.sources.read(&unit.key)?).map_err(|err| err.to_string())?;
    let mut upgraded = upgrade_bgsm(
        &data,
        &material_texture_ref(&unit.reflectance),
        &material_texture_ref(&unit.lighting),
        unit.translucent,
    );
    if !unit.metal_diffuse.is_empty() {
        upgraded.DiffuseTexture = material_texture_ref(&unit.metal_diffuse);
    }
    let path = ctx.path(&unit.output);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|err| err.to_string())?;
    std::fs::write(&path, materials_native::bgsm::write(&upgraded)).map_err(|err| format!("{}: {err}", path.display()))
}

/// NIF texture sets: slot 8 = `_l`, slot 9 = `_r` (FO4CS claims a texture set with
/// slot 9 filled as BGSM v22+ at draw time).
fn write_nif(ctx: &Ctx<'_>, key: &str, units: &[&Unit]) -> Result<(), String> {
    use nif_core_native::model::{NifFile, NifValue};
    let mut nif = NifFile::from_bytes(&ctx.sources.read(key)?, None).map_err(|err| format!("{key}: {err:?}"))?;
    for unit in units {
        for &set_id in &unit.nif_texture_sets {
            let block = nif.blocks.get_mut(set_id).ok_or_else(|| format!("{key}: block {set_id} missing"))?;
            let Some(NifValue::Array(textures)) = block.get_field("Textures").cloned() else {
                return Err(format!("{key}: block {set_id} has no Textures"));
            };
            let mut textures = textures;
            if textures.len() < 10 {
                return Err(format!("{key}: block {set_id} has {} slots", textures.len()));
            }
            let nif_path = |key: &str| NifValue::String(material_texture_ref(key).replace('/', "\\"));
            if !unit.metal_diffuse.is_empty() {
                textures[0] = nif_path(&unit.metal_diffuse);
            }
            // v22 has no envmap; FO4's cube would otherwise keep driving the old envmap technique.
            textures[4] = NifValue::String(String::new());
            textures[8] = nif_path(&unit.lighting);
            textures[9] = nif_path(&unit.reflectance);
            block.set_field("Textures", NifValue::Array(textures));
            clear_envmap_shader(&mut nif, set_id);
        }
    }
    let bytes = nif.to_bytes().map_err(|err| format!("{key}: {err:?}"))?;
    let path = ctx.path(key);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|err| err.to_string())?;
    std::fs::write(&path, bytes).map_err(|err| format!("{}: {err}", path.display()))
}

const SHADER_TYPE_ENVIRONMENT_MAP: u64 = 1;
const SF1_ENVIRONMENT_MAPPING: u64 = 1 << 7;

/// Shaders on `set_id` drop FO4's Environment Map type and flag, as converted FO76 meshes have them.
fn clear_envmap_shader(nif: &mut nif_core_native::model::NifFile, set_id: usize) {
    use nif_core_native::model::NifValue;
    let as_u64 = |v: Option<&NifValue>| match v {
        Some(NifValue::UInt(u)) => Some(*u),
        Some(NifValue::Int(i)) => Some(*i as u64),
        _ => None,
    };
    for shader in nif.blocks.iter_mut().filter(|b| b.type_name == "BSLightingShaderProperty") {
        if shader.get_field("Texture Set").map(NifValue::as_i64) != Some(set_id as i64) {
            continue;
        }
        if as_u64(shader.get_field("Shader Type")) == Some(SHADER_TYPE_ENVIRONMENT_MAP) {
            shader.set_field("Shader Type", NifValue::UInt(0));
        }
        if let Some(flags) = as_u64(shader.get_field("Shader Flags 1")) {
            shader.set_field("Shader Flags 1", NifValue::UInt(flags & !SF1_ENVIRONMENT_MAPPING));
        }
    }
}

/// Executes the plan. `progress(done, total, message)` returns false to cancel.
pub fn execute(
    plan: &Plan,
    opts: &UpgradeOptions,
    sources: &AssetSources,
    progress: &mut dyn FnMut(usize, usize, &str) -> bool,
) -> RunResult {
    let ctx = Ctx { sources, out: &opts.output };
    let mut result = RunResult::default();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(opts.workers.max(1)).build().expect("thread pool");

    let mut nif_groups: BTreeMap<&str, Vec<&Unit>> = BTreeMap::new();
    let materials: Vec<&Unit> = plan.units.iter().filter(|u| u.is_upgrade() && u.kind == UnitKind::Material).collect();
    for unit in plan.units.iter().filter(|u| u.is_upgrade() && u.kind == UnitKind::NifTextureSet) {
        nif_groups.entry(unit.output.as_str()).or_default().push(unit);
    }
    let total = plan.textures.len() + materials.len() + nif_groups.len();
    let mut done = 0usize;
    let chunk = (opts.workers.max(1) * 4).max(8);

    for jobs in plan.textures.chunks(chunk) {
        if !progress(done, total, "textures") {
            result.cancelled = true;
            return result;
        }
        let outcomes: Vec<_> = pool.install(|| jobs.par_iter().map(|job| (job, run_job(&ctx, job))).collect());
        for (job, outcome) in outcomes {
            match outcome {
                Ok(written) => result.written.extend(written),
                Err(err) => result.errors.push((job.outputs().join(","), err)),
            }
        }
        done += jobs.len();
    }
    for units in materials.chunks(chunk * 4) {
        if !progress(done, total, "materials") {
            result.cancelled = true;
            return result;
        }
        let outcomes: Vec<_> = pool.install(|| units.par_iter().map(|u| (u, write_material(&ctx, u))).collect());
        for (unit, outcome) in outcomes {
            match outcome {
                Ok(()) => result.written.push(unit.output.clone()),
                Err(err) => result.errors.push((unit.key.clone(), err)),
            }
        }
        done += units.len();
    }
    for (key, units) in &nif_groups {
        if !progress(done, total, "meshes") {
            result.cancelled = true;
            return result;
        }
        match write_nif(&ctx, key, units) {
            Ok(()) => result.written.push(key.to_string()),
            Err(err) => result.errors.push((key.to_string(), err)),
        }
        done += 1;
    }
    let terrain: BTreeMap<&str, &str> = plan
        .units
        .iter()
        .filter(|u| u.is_upgrade() && u.kind == UnitKind::TerrainLayer)
        .map(|u| (u.key.as_str(), u.output.as_str()))
        .collect();
    if !terrain.is_empty() {
        let manifest = serde_json::json!({
            "contract": "raw terrain bank: <prefix>_source_d.dds BC3_UNORM_SRGB (A height), _source_n.dds BC5_SNORM, _source_l.dds BC3_UNORM (R smoothness, G AO, A emission 0), _source_r.dds BC4_UNORM 256x256 (grey F0)",
            "layer_size": opts.terrain_layer_size,
            "layers": terrain,
        });
        let path = ctx.path(TERRAIN_MANIFEST_PATH);
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        match std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()) {
            Ok(()) => result.written.push(TERRAIN_MANIFEST_PATH.into()),
            Err(err) => result.errors.push((TERRAIN_MANIFEST_PATH.into(), err.to_string())),
        }
    }
    progress(total, total, "done");
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{DIELECTRIC_R, build_plan};

    /// The FO4 vault suit's number decal names a BGSM and also carries an Environment Map shader with
    /// the bronze cube inline; the upgraded mesh must mirror the upgraded material instead.
    #[test]
    fn bgsm_named_nif_set_mirrors_the_upgraded_material() {
        let fo4 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4");
        let nif_rel = "meshes/clothes/vault111suit/vault111suit.nif";
        if !fo4.join(nif_rel).is_file() {
            eprintln!("skipped, no extracted FO4 data");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        for rel in [
            nif_rel,
            "materials/clothes/vaultsuit/vaultsuitmnumber111dirty.bgsm",
            "textures/clothes/vaultsuit/vaultsuitnumber111dirty_d.dds",
            "textures/clothes/vaultsuit/vaultsuitnumber111dirty_n.dds",
            "textures/clothes/vaultsuit/vaultsuitnumber111dirty_s.dds",
            "textures/shared/cubemaps/mipblur_defaultoutside1_bronze.dds",
        ] {
            std::fs::create_dir_all(data.join(rel).parent().unwrap()).unwrap();
            std::fs::copy(fo4.join(rel), data.join(rel)).unwrap();
        }
        let mut opts = UpgradeOptions { sources: vec![data], output: tmp.path().join("out"), ..Default::default() };
        opts.metal_mode = crate::options::MetalMode::Envmap;
        opts.include_nifs = true;
        let sources = AssetSources::open(&opts.sources).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        let result = execute(&plan, &opts, &sources, &mut |_, _, _| true);
        assert!(result.errors.is_empty(), "{:?}", result.errors);

        use nif_core_native::model::{NifFile, NifValue};
        let nif = NifFile::from_bytes(&std::fs::read(opts.output.join(nif_rel)).unwrap(), None).unwrap();
        let shader = nif.get_block(66).unwrap();
        assert_eq!(shader.get_field("Shader Type").map(NifValue::as_i64), Some(0));
        assert_eq!(shader.get_field("Shader Flags 1").map(NifValue::as_i64).unwrap() & (1 << 7), 0);
        let Some(NifValue::Array(textures)) = nif.get_block(67).unwrap().get_field("Textures") else { panic!() };
        let slot = |i: usize| match &textures[i] { NifValue::String(s) => s.to_ascii_lowercase(), _ => String::new() };
        assert!(slot(0).ends_with("vaultsuitnumber111dirty_metal_d.dds"), "{}", slot(0));
        assert_eq!(slot(4), "");
        assert!(slot(8).ends_with("vaultsuitnumber111dirty_l.dds") && slot(9).ends_with("vaultsuitnumber111dirty_r.dds"), "{} {}", slot(8), slot(9));
    }

    fn noop(_: usize, _: usize, _: &str) -> bool {
        true
    }

    #[test]
    fn executes_plan_into_output_mod() {
        let (tmp, opts) = crate::plan::tests::fixture();
        let sources = AssetSources::open(&opts.sources).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        let result = execute(&plan, &opts, &sources, &mut noop);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(!result.cancelled);
        let out = tmp.path().join("out");

        let bgsm = materials_native::bgsm::parse(&std::fs::read(out.join("materials/props/can.bgsm")).unwrap()).unwrap();
        assert_eq!(bgsm.header.version, 22);
        assert_eq!(bgsm.LightingTexture.as_deref().map(|s| s.trim_end_matches('\0')), Some("props/can_l.dds"));
        assert_eq!(bgsm.SpecularTexture.as_deref().map(|s| s.trim_end_matches('\0')), Some("materialupgrade/dielectric_r.dds"));

        let probe = directxtex_native::read_dds_probe(&out.join("textures/props/can_l.dds")).unwrap();
        assert_eq!(probe.width, 16);
        let info = directxtex_native::read_dds_rgba_image(&out.join("textures/props/can_l.dds")).unwrap();
        assert_eq!(info.dxgi_format, 71, "BC1_UNORM: no alpha channel reaches FO4CS");
        let r = directxtex_native::read_dds_rgba_image(&out.join(DIELECTRIC_R)).unwrap();
        assert_eq!(r.dxgi_format, 80, "BC4_UNORM");
        assert_eq!((r.width, r.height), (4, 4));
        assert!((r.rgba[0] as i32 - 10).abs() <= 1);
        assert!(!out.join("textures/props/can_d.dds").exists(), "_d is never re-encoded");
        assert!(!out.join("materials/props/already.bgsm").exists());
    }

    #[test]
    fn metal_reflectance_follows_the_fo4_envmap_and_its_colour() {
        let (tmp, mut opts) = crate::plan::tests::fixture();
        let data = &opts.sources[0];
        let cubes = data.join("Textures/Shared/Cubemaps");
        std::fs::create_dir_all(&cubes).unwrap();
        directxtex_native::write_dds_rgba_image(&cubes.join("Copper_e.dds"), 4, 4, &[89u8, 37, 8, 255].repeat(16), "BC1_UNORM", true).unwrap();
        directxtex_native::write_dds_rgba_image(&cubes.join("MetalBrushed01Cube_e.dds"), 4, 4, &[93u8, 93, 93, 255].repeat(16), "BC1_UNORM", true).unwrap();
        for (name, cube) in [("Pot", "Shared/Cubemaps/Copper_e.dds"), ("Rail", "Shared/Cubemaps/MetalBrushed01Cube_e.dds")] {
            let mut m = crate::bgsm_v22::tests::fo4_bgsm();
            m.DiffuseTexture = format!("Props/{name}_d.dds");
            m.SmoothSpecTexture = "Props/Can_s.dds".into();
            m.EnvmapTexture = Some(cube.into());
            std::fs::write(data.join(format!("Materials/Props/{name}.bgsm")), materials_native::bgsm::write(&m)).unwrap();
        }
        opts.metal_mode = crate::options::MetalMode::Envmap;
        let sources = AssetSources::open(&opts.sources).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        let result = execute(&plan, &opts, &sources, &mut noop);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let unit = |key: &str| plan.units.iter().find(|u| u.key == key).unwrap();

        let read = |key: &str| directxtex_native::read_dds_rgba_image(&tmp.path().join("out").join(key)).unwrap();
        // Can_s: R 200 everywhere, G ramps 0..255 -> no envmap at the dull end, pure envmap at the glossy end.
        let pot = &unit("materials/props/pot.bgsm").reflectance;
        assert_eq!(pot, "textures/props/can_r.dds");
        let r = read(pot);
        assert_eq!((r.width, r.dxgi_format), (16, 72), "per-texel BC1_UNORM_SRGB, as FO76 ships coloured _r");
        let first = &r.rgba[..3];
        let last = &r.rgba[r.rgba.len() - 4..r.rgba.len() - 1];
        assert!(first[0].abs_diff(first[2]) <= 8, "dull texels stay dielectric grey, got {first:?}");
        assert!(last[0] > last[1] && last[1] > last[2] && last[0] > first[0] + 40, "glossy texels are copper, got {last:?}");

        let rail = &unit("materials/props/rail.bgsm").reflectance;
        assert_eq!(rail, "textures/props/can_u1_r.dds", "one _s under two envmaps needs two _r");
        assert_eq!(read(rail).dxgi_format, 80, "a grey envmap keeps the one-channel BC4");

        opts.metal_mode = crate::options::MetalMode::Mask;
        let plan = build_plan(&opts, &sources).unwrap();
        let pot = plan.units.iter().find(|u| u.key == "materials/props/pot.bgsm").unwrap();
        let rail = plan.units.iter().find(|u| u.key == "materials/props/rail.bgsm").unwrap();
        assert_ne!(pot.reflectance, rail.reflectance, "one _s under two cubes needs two _r");
    }

    #[test]
    fn envmap_metal_gets_a_darkened_diffuse() {
        let (tmp, mut opts) = crate::plan::tests::fixture();
        let data = opts.sources[0].clone();
        let tex = data.join("Textures/Props");
        directxtex_native::write_dds_rgba_image(&tex.join("Pot_d.dds"), 32, 32, &[180u8, 180, 180, 255].repeat(1024), "BC1_UNORM", true).unwrap();
        directxtex_native::write_dds_rgba_image(&tex.join("Matte_d.dds"), 4, 4, &[180u8, 180, 180, 255].repeat(16), "BC1_UNORM", true).unwrap();
        directxtex_native::write_dds_rgba_image(&tex.join("Matte_s.dds"), 16, 16, &crate::plan::tests::gradient_spec(16, 16).iter().enumerate().map(|(i, &v)| if i % 4 == 0 { 0 } else { v }).collect::<Vec<_>>(), "BC5_UNORM", true).unwrap();
        for (name, spec) in [("Pot", "Props/Can_s.dds"), ("Matte", "Props/Matte_s.dds")] {
            let mut m = crate::bgsm_v22::tests::fo4_bgsm();
            m.DiffuseTexture = format!("Props/{name}_d.dds");
            m.SmoothSpecTexture = spec.into();
            m.EnvmapTexture = Some("Shared/Cubemaps/MetalBrushed01Cube_e.dds".into());
            std::fs::write(data.join(format!("Materials/Props/{name}.bgsm")), materials_native::bgsm::write(&m)).unwrap();
        }
        opts.metal_mode = crate::options::MetalMode::Envmap;
        let sources = AssetSources::open(&opts.sources).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        let result = execute(&plan, &opts, &sources, &mut noop);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let unit = |key: &str| plan.units.iter().find(|u| u.key == key).unwrap();
        let out = tmp.path().join("out");

        let pot = unit("materials/props/pot.bgsm");
        assert_eq!(pot.metal_diffuse, "textures/props/pot_metal_d.dds");
        let bgsm = materials_native::bgsm::parse(&std::fs::read(out.join("materials/props/pot.bgsm")).unwrap()).unwrap();
        assert_eq!(bgsm.DiffuseTexture.trim_end_matches('\0'), "props/pot_metal_d.dds");
        let d = directxtex_native::read_dds_rgba_image(&out.join(&pot.metal_diffuse)).unwrap();
        assert_eq!((d.width, d.dxgi_format), (32, 71), "source size and format kept");
        assert!(d.rgba[0].abs_diff(180) <= 4, "dull texels keep their colour, got {}", d.rgba[0]);
        assert!(d.rgba[d.rgba.len() - 4] <= 4, "pure envmap texels go black, got {}", d.rgba[d.rgba.len() - 4]);
        assert_eq!(d.rgba[d.rgba.len() - 1], 255);

        let matte = unit("materials/props/matte.bgsm");
        assert!(matte.is_upgrade() && matte.metal_diffuse.is_empty(), "no spec -> no envmap -> no copy");
        assert!(!out.join("textures/props/matte_metal_d.dds").exists());

        opts.exclude_paths = vec!["textures/props/pot_metal_d.dds".into()];
        let plan = build_plan(&opts, &sources).unwrap();
        let pot = plan.units.iter().find(|u| u.key == "materials/props/pot.bgsm").unwrap();
        assert_eq!(pot.reason.as_deref(), Some("replaced by native asset"));
        assert!(plan.textures.iter().all(|t| !matches!(t, TextureJob::MetalDiffuse { .. })));
    }

    #[test]
    fn cancel_stops_before_writing() {
        let (tmp, opts) = crate::plan::tests::fixture();
        let sources = AssetSources::open(&opts.sources).unwrap();
        let plan = build_plan(&opts, &sources).unwrap();
        let result = execute(&plan, &opts, &sources, &mut |_, _, _| false);
        assert!(result.cancelled);
        assert!(!tmp.path().join("out/materials").exists());
    }

    #[test]
    fn terrain_bank_formats_and_srgb_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data/Textures/Landscape");
        std::fs::create_dir_all(&data).unwrap();
        directxtex_native::write_dds_rgba_image(&data.join("Dirt_d.dds"), 8, 8, &[188u8, 128, 64, 255].repeat(64), "BC1_UNORM_SRGB", true).unwrap();
        directxtex_native::write_dds_rgba_image(&data.join("Dirt_n.dds"), 8, 8, &[128u8, 128, 255, 255].repeat(64), "BC5_UNORM", true).unwrap();
        let sources = AssetSources::open(&[tmp.path().join("Data")]).unwrap();
        let opts = UpgradeOptions { sources: vec![], output: tmp.path().join("out"), ..Default::default() };
        let ctx = Ctx { sources: &sources, out: &opts.output };
        let job = TextureJob::TerrainLayer {
            prefix: "textures/landscape/dirt".into(),
            layer: crate::terrain::TerrainLayerSource {
                diffuse: "textures/landscape/dirt_d.dds".into(),
                normal: "textures/landscape/dirt_n.dds".into(),
                smoothspec: None,
            },
            curve: crate::kernels::SmoothnessCurve { scalar: 1.0, gain: 1.0, knee: None },
            flat_value: 0.2,
            f0: crate::kernels::METAL_F0,
            size: 16,
        };
        run_job(&ctx, &job).unwrap();
        let read = |m: &str| directxtex_native::read_dds_rgba_image(&tmp.path().join(format!("out/textures/landscape/dirt_source_{m}.dds"))).unwrap();
        let d = read("d");
        assert_eq!((d.width, d.dxgi_format), (16, 78), "BC3_UNORM_SRGB");
        // the sRGB source's stored bytes land unchanged under the sRGB header
        assert!((d.rgba[0] as i32 - 188).abs() <= 4, "sRGB payload kept, got {}", d.rgba[0]);
        let probe = directxtex_native::read_dds_probe(&tmp.path().join("out/textures/landscape/dirt_source_n.dds")).unwrap();
        assert_eq!(probe.dxgi_format, 84, "BC5_SNORM");
        // flat FO4 normal (128,128) must land near 0 in SNORM, not at +0.5
        let raw = std::fs::read(tmp.path().join("out/textures/landscape/dirt_source_n.dds")).unwrap();
        let block = &raw[raw.len() - 16..];
        assert!((block[0] as i8).unsigned_abs() <= 2 && (block[8] as i8).unsigned_abs() <= 2, "{block:?}");
        let l = read("l");
        assert_eq!(l.dxgi_format, 77, "BC3_UNORM");
        let r = read("r");
        assert_eq!((r.width, r.dxgi_format), (256, 80), "BC4_UNORM at the bank's one _r size");
        assert!((r.rgba[0] as i32 - 64).abs() <= 2, "metal F0 0.25, got {}", r.rgba[0]);
        assert!((l.rgba[0] as i32 - 51).abs() <= 2 && l.rgba[3] == 0, "smoothness, no emission");
    }
}

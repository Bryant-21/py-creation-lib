//! Load a placed ref's LOD NIF and extract one `ShapeDesc` per geom.
//!
//! Reads geometry + shader + textures via `nif_core` (the generic block-graph
//! reader — NOT a port of LODGen's `NiFile`) and resolves BGSM/BGEM materials
//! via the `materials` crate. Faithful port of the FO4 object path of:
//!   - `ParseNif`       LODApp.cs:1369-1388
//!   - `IterateNodes`   LODApp.cs:295-393
//!   - `ShapeDesc` ctor ShapeDesc.cs:235-1313
//!
//! Deviation from the C#: nif_core stores
//! `Shader Flags 1/2` as arrays of named flag strings and `Texture Clamp Mode`
//! as a named enum. The bit-arithmetic the C# performs on raw uints is ported by
//! decoding those named representations back to raw bits (`shader_flags` /
//! `clamp_mode`), mirroring nif_core/convert_file.rs `flag_name_bit`.

use crate::asset_source::{self, ResolvedAsset};
use crate::descriptors::BBox;
use crate::input::StaticDesc;
use crate::objects::geometry::LodGeometry;
use crate::objects::static_desc::{ShaderKind, ShapeDesc, ShapeFlags};
use crate::progress::QuadCtx;
use nif_core_native::model::{NifBlock, NifFile, NifValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Decoded-NIF cache
// ---------------------------------------------------------------------------
//
// parse_nif is called once PER PLACED REF (object pass) and per unique ref-key
// (atlas scan). The same LOD mesh is shared by many refs, and a coarse object
// quad (LOD16 = 16x16 cells, LOD32 = 32x32) re-loads each mesh again per quad —
// so one source .nif was decoded thousands of times. Every decode inflates the
// file into nif_core's generic block graph (an IndexMap per vertex); that torrent
// of short-lived allocation is the bulk of the object-LOD memory blowup (mimalloc
// retains the freed pages, so process RSS climbs to the peak CHURN, not the live
// set).
//
// Cache the decoded `Arc<NifFile>` keyed by resolved path: each unique mesh is
// decoded ONCE and shared across all refs/quads. `clear_nif_cache()` runs at each
// object-LOD level boundary (driver.rs) so resident memory stays bounded to a
// single level's unique meshes, never the whole worldspace.
static NIF_CACHE: OnceLock<Mutex<HashMap<ResolvedAsset, Arc<NifFile>>>> = OnceLock::new();

static MODEL_SHAPE_CACHE: OnceLock<Mutex<ModelShapeCache>> = OnceLock::new();

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ModelShapeCacheKey {
    path: ResolvedAsset,
    level: usize,
    base_flags: u32,
    is_billboard: bool,
    is_grass: bool,
    material_name: String,
    material_swap: Vec<(String, String)>,
}

struct ModelShapeCacheEntry {
    shapes: Arc<Vec<ShapeDesc>>,
    bytes: u64,
    last_used: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModelShapeCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub bytes: u64,
    pub evictions: u64,
    pub oversize: u64,
    pub shapes_prepared: u64,
    pub triangles_before: u64,
    pub triangles_after: u64,
}

impl ModelShapeCacheStats {
    pub(crate) fn is_empty(self) -> bool {
        self.hits == 0
            && self.misses == 0
            && self.entries == 0
            && self.bytes == 0
            && self.evictions == 0
            && self.oversize == 0
            && self.shapes_prepared == 0
            && self.triangles_before == 0
            && self.triangles_after == 0
    }
}

#[derive(Default)]
struct ModelShapeCache {
    entries: HashMap<ModelShapeCacheKey, ModelShapeCacheEntry>,
    bytes: u64,
    tick: u64,
    stats: ModelShapeCacheStats,
}

#[derive(Default)]
struct ModelShapePrepareStats {
    shapes_prepared: u64,
    triangles_before: u64,
    triangles_after: u64,
}

fn nif_cache() -> &'static Mutex<HashMap<ResolvedAsset, Arc<NifFile>>> {
    NIF_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn model_shape_cache() -> &'static Mutex<ModelShapeCache> {
    MODEL_SHAPE_CACHE.get_or_init(|| Mutex::new(ModelShapeCache::default()))
}

/// Load a NIF through the shared decode cache. The expensive `NifFile::load`
/// decode runs OUTSIDE the lock, so concurrent rayon workers decoding DIFFERENT
/// meshes never serialize; only the brief map get/insert is guarded. A rare
/// double-miss on the same path just decodes twice and keeps the first insert.
fn cached_load_nif(ctx: &QuadCtx, asset: &ResolvedAsset) -> anyhow::Result<Arc<NifFile>> {
    {
        let cache = nif_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.get(asset) {
            return Ok(Arc::clone(hit));
        }
    }
    let bytes = asset_source::read(&ctx.paths.data_dirs, asset)?;
    let nif =
        Arc::new(NifFile::from_bytes(&bytes, None).map_err(|error| anyhow::anyhow!("{error:?}"))?);
    let mut cache = nif_cache().lock().unwrap_or_else(|e| e.into_inner());
    Ok(Arc::clone(cache.entry(asset.clone()).or_insert(nif)))
}

/// Drop all cached decoded NIFs. Called at object-LOD level boundaries to bound
/// resident memory to one level's unique meshes; no-op if nothing is cached.
pub(crate) fn clear_nif_cache() {
    if let Some(cache) = NIF_CACHE.get() {
        cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

pub(crate) fn clear_model_shape_cache() {
    if let Some(cache) = MODEL_SHAPE_CACHE.get() {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.entries.clear();
        cache.bytes = 0;
    }
}

pub(crate) fn reset_model_shape_cache_stats() {
    if let Some(cache) = MODEL_SHAPE_CACHE.get() {
        cache.lock().unwrap_or_else(|e| e.into_inner()).stats = ModelShapeCacheStats::default();
    }
}

pub(crate) fn model_shape_cache_stats() -> ModelShapeCacheStats {
    let Some(cache) = MODEL_SHAPE_CACHE.get() else {
        return ModelShapeCacheStats::default();
    };
    let cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    ModelShapeCacheStats {
        entries: cache.entries.len(),
        bytes: cache.bytes,
        ..cache.stats
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Load `stat.lod_models[level]` and extract its shapes.
/// port: ParseNif (LODApp.cs:1369-1388) — read NIF, require NiNode root,
/// then IterateNodes from the root.
pub fn parse_nif(stat: &StaticDesc, level: usize, ctx: &QuadCtx) -> anyhow::Result<Vec<ShapeDesc>> {
    let model = match stat.lod_models.get(level).and_then(|m| m.as_ref()) {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    if !contains_ci(model, ".nif") {
        // .dds billboard refs belong to the tree/grass path; object path needs a NIF.
        return Ok(Vec::new());
    }
    let path = resolve_model_path(ctx, model)
        .ok_or_else(|| anyhow::anyhow!("LOD model not found in data_dirs: {model}"))?;
    let nif = cached_load_nif(ctx, &path).map_err(|e| anyhow::anyhow!("read {model}: {e}"))?;
    if nif.blocks.is_empty() || !is_ninode(&nif.blocks[0]) {
        anyhow::bail!("{model}: unexpected root node (not a NiNode)");
    }
    Ok(iterate_nif(&nif, stat, level, ctx))
}

/// Object-quad path with a bounded model-local shape cache. This avoids rebuilding
/// the same LOD mesh for every placed ref while still applying each ref's world
/// transform after cloning.
pub fn parse_nif_for_object_lod(
    stat: &StaticDesc,
    level: usize,
    ctx: &QuadCtx,
) -> anyhow::Result<Vec<ShapeDesc>> {
    let cache_mb = ctx.settings.objects.object_lod_model_cache_mb;
    if cache_mb == 0 {
        return parse_nif(stat, level, ctx);
    }

    let model = match stat.lod_models.get(level).and_then(|m| m.as_ref()) {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    if !contains_ci(model, ".nif") {
        return Ok(Vec::new());
    }
    let path = resolve_model_path(ctx, model)
        .ok_or_else(|| anyhow::anyhow!("LOD model not found in data_dirs: {model}"))?;
    let key = model_shape_cache_key(stat, level, path.clone());
    if let Some(shapes) = get_model_shape_cache_hit(&key, stat) {
        return Ok(shapes);
    }

    let nif = cached_load_nif(ctx, &path).map_err(|e| anyhow::anyhow!("read {model}: {e}"))?;
    if nif.blocks.is_empty() || !is_ninode(&nif.blocks[0]) {
        anyhow::bail!("{model}: unexpected root node (not a NiNode)");
    }

    let mut shapes = iterate_nif(&nif, stat, level, ctx);
    let prep_stats = prepare_model_shapes_for_cache(&mut shapes);
    let bytes = estimate_shapes_bytes(&shapes);
    insert_model_shape_cache(key, shapes.clone(), bytes, cache_mb, prep_stats);
    refresh_shapes_for_stat(&mut shapes, stat);
    Ok(shapes)
}

fn model_shape_cache_key(
    stat: &StaticDesc,
    level: usize,
    path: ResolvedAsset,
) -> ModelShapeCacheKey {
    ModelShapeCacheKey {
        path,
        level,
        base_flags: stat.base_flags,
        is_billboard: stat.is_billboard,
        is_grass: stat.is_grass,
        material_name: stat.material_name.clone(),
        material_swap: stat
            .material_swap
            .iter()
            .map(|(from, to)| (from.clone(), to.clone()))
            .collect(),
    }
}

fn get_model_shape_cache_hit(
    key: &ModelShapeCacheKey,
    stat: &StaticDesc,
) -> Option<Vec<ShapeDesc>> {
    let hit = {
        let mut cache = model_shape_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache.tick = cache.tick.wrapping_add(1);
        let tick = cache.tick;
        let hit = cache.entries.get_mut(key).map(|entry| {
            entry.last_used = tick;
            Arc::clone(&entry.shapes)
        });
        if hit.is_some() {
            cache.stats.hits += 1;
        } else {
            cache.stats.misses += 1;
        }
        hit
    }?;

    let mut shapes = hit.as_ref().clone();
    refresh_shapes_for_stat(&mut shapes, stat);
    Some(shapes)
}

fn insert_model_shape_cache(
    key: ModelShapeCacheKey,
    shapes: Vec<ShapeDesc>,
    bytes: u64,
    cache_mb: u64,
    prep_stats: ModelShapePrepareStats,
) {
    let budget = cache_mb.saturating_mul(1024 * 1024);
    let mut cache = model_shape_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache.stats.shapes_prepared += prep_stats.shapes_prepared;
    cache.stats.triangles_before += prep_stats.triangles_before;
    cache.stats.triangles_after += prep_stats.triangles_after;

    if budget == 0 || bytes > budget {
        cache.stats.oversize += 1;
        return;
    }

    cache.tick = cache.tick.wrapping_add(1);
    let tick = cache.tick;
    if let Some(old) = cache.entries.remove(&key) {
        cache.bytes = cache.bytes.saturating_sub(old.bytes);
    }
    cache.entries.insert(
        key,
        ModelShapeCacheEntry {
            shapes: Arc::new(shapes),
            bytes,
            last_used: tick,
        },
    );
    cache.bytes = cache.bytes.saturating_add(bytes);

    while cache.bytes > budget {
        let Some(lru_key) = cache
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        if let Some(evicted) = cache.entries.remove(&lru_key) {
            cache.bytes = cache.bytes.saturating_sub(evicted.bytes);
            cache.stats.evictions += 1;
        } else {
            break;
        }
    }
}

fn prepare_model_shapes_for_cache(shapes: &mut [ShapeDesc]) -> ModelShapePrepareStats {
    let mut prep = ModelShapePrepareStats::default();
    for shape in shapes {
        let before = shape.geometry.num_triangles() as u64;
        prep.shapes_prepared += 1;
        prep.triangles_before += before;

        prep.triangles_after += before;
    }
    prep
}

fn refresh_shapes_for_stat(shapes: &mut [ShapeDesc], stat: &StaticDesc) {
    let rotation = rotation_from_ref(stat.rot);
    for shape in shapes {
        shape.translation = stat.pos;
        shape.rotation = rotation;
        shape.enable_parent = stat.enable_parent;
        shape.ref_flags = stat.ref_flags;
    }
}

fn estimate_shapes_bytes(shapes: &[ShapeDesc]) -> u64 {
    shapes
        .iter()
        .map(|shape| {
            std::mem::size_of::<ShapeDesc>() as u64
                + string_bytes(&shape.name)
                + string_bytes(&shape.static_model)
                + shape.textures.iter().map(string_bytes).sum::<u64>()
                + shape.source_materials.iter().map(string_bytes).sum::<u64>()
                + string_bytes(&shape.textures_key)
                + vec_bytes(&shape.geometry.vertices)
                + vec_bytes(&shape.geometry.uvcoords)
                + vec_bytes(&shape.geometry.normals)
                + vec_bytes(&shape.geometry.tangents)
                + vec_bytes(&shape.geometry.bitangents)
                + vec_bytes(&shape.geometry.vertex_colors)
                + vec_bytes(&shape.geometry.triangles)
                + vec_bytes(&shape.segments)
        })
        .sum()
}

fn string_bytes(s: &String) -> u64 {
    s.capacity() as u64
}

fn vec_bytes<T>(v: &Vec<T>) -> u64 {
    v.capacity().saturating_mul(std::mem::size_of::<T>()) as u64
}

/// Walk an already-loaded NIF from its root NiNode (test seam + parse_nif body).
/// port: IterateNodes entry (LODApp.cs:1384-1385).
pub fn iterate_nif(
    nif: &NifFile,
    stat: &StaticDesc,
    level: usize,
    ctx: &QuadCtx,
) -> Vec<ShapeDesc> {
    if nif.blocks.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    iterate_nodes(
        nif,
        stat,
        level,
        &nif.blocks[0],
        mat4_identity(),
        1.0,
        ctx,
        &mut out,
    );
    out
}

// ---------------------------------------------------------------------------
// Node iteration (port: IterateNodes, LODApp.cs:295-393)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn iterate_nodes(
    nif: &NifFile,
    stat: &StaticDesc,
    level: usize,
    node: &NifBlock,
    parent_transform: Mat4,
    parent_scale: f32,
    ctx: &QuadCtx,
    out: &mut Vec<ShapeDesc>,
) {
    // port: LODApp.cs:298-307 — skip hidden / editormarker nodes.
    if node_hidden(node) {
        return;
    }
    let name = block_name(node);
    if contains_ci(&name, "editormarker") {
        return;
    }

    let model = stat
        .lod_models
        .get(level)
        .and_then(|m| m.as_ref())
        .cloned()
        .unwrap_or_default();

    // port: LODApp.cs:308-316 — accumulate transform unless BSFadeNode (and the
    // model isn't a "lod" model); ignoreTransRot list omitted (M1 has none).
    //
    // C#: parentTransform2 = parentNode.GetTransform() * parentTransform (ROW-vector,
    // child applied first). In our COLUMN-vector convention (v' = M·v, translation in
    // col 3), the equivalent is parent · node_local (verified numerically against the
    // C# Matrix44 algebra). The previous `node_local · parent` was the wrong order and
    // silently corrupted multi-node fixtures with rotation+translation.
    let mut transform = parent_transform;
    let skip_fade = node.type_name == "BSFadeNode" && !contains_ci(&model, "lod");
    if !skip_fade {
        transform = mat4_mul(&parent_transform, &node_local_transform(node));
    }
    let scale = node_scale(node) * parent_scale;

    for child_id in child_refs(node) {
        let Some(child) = block_of(nif, child_id) else {
            continue;
        };
        if is_ninode(child) {
            iterate_nodes(nif, stat, level, child, transform, scale, ctx, out);
            continue;
        }
        // port: LODApp.cs:336-369 — NiTriBasedGeom / BSTriShape, skip skinned/editormarker.
        let is_geom = child.type_name == "NiTriShape"
            || child.type_name == "NiTriStrips"
            || child.type_name.contains("BSTriShape")
            || child.type_name == "BSSubIndexTriShape"
            || child.type_name == "BSMeshLODTriShape"
            || child.type_name == "BSDynamicTriShape";
        if !is_geom {
            continue;
        }
        let skin = val_ref(child.get_field("Skin"))
            .or_else(|| val_ref(child.get_field("Skin Instance")))
            .unwrap_or(-1);
        if skin != -1 {
            continue;
        }
        if contains_ci(&block_name(child), "editormarker") {
            continue;
        }
        if let Some(mut shape) = build_shape_desc(nif, stat, level, child, ctx) {
            // Drop shapes that produced no geometry (ShapeDesc ctor early-returns,
            // or zero triangles after dedup — TransformShape would drop these too).
            if shape.geometry.num_triangles() == 0 {
                continue;
            }
            // Fold the geom block's OWN transform/scale into node_transform/node_scale,
            // matching C# TransformShape (LODApp.cs:870-871):
            //   num2    = geom.GetScale() * parentScale
            //   matrix7 = geom.GetTransform(parentScale) * parentTransform   (ROW-vector)
            // where geom.GetTransform(s) = Matrix44(geomRot, geomTrans * s, 1).
            // Column-vector equivalent: matrix7_col = parentTransform · geom_local
            // (geom applied first / rightmost), with the geom translation pre-scaled by
            // parentScale.
            let geom_local = geom_transform_scaled(child, scale);
            shape.node_transform = mat4_mul(&transform, &geom_local);
            shape.node_scale = node_scale(child) * scale;
            out.push(shape);
        }
    }
}

// ---------------------------------------------------------------------------
// ShapeDesc constructor (port: ShapeDesc.cs:235-1313, FO4 object path)
// ---------------------------------------------------------------------------

fn default_textures_fo4() -> [String; 10] {
    // port: ShapeDesc.cs:289-303 (FO4 default texture set).
    [
        "Textures\\Shared\\FlatWhite01_d.dds".to_string(),
        "Textures\\Shared\\FlatFlat_n.dds".to_string(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "Textures\\Shared\\White01_s.dds".to_string(),
        String::new(),
        String::new(),
    ]
}

/// Build one ShapeDesc from a geometry block. Returns None on unreadable geom or
/// missing UVs (matching the C# early-returns that leave an empty Geometry).
fn build_shape_desc(
    nif: &NifFile,
    stat: &StaticDesc,
    level: usize,
    geom: &NifBlock,
    ctx: &QuadCtx,
) -> Option<ShapeDesc> {
    let mut name = block_name(geom);
    let static_model = stat
        .lod_models
        .get(level)
        .and_then(|m| m.as_ref())
        .map(|m| m.to_lowercase())
        .unwrap_or_default();

    let mut flags = ShapeFlags::empty();
    let material = stat.material_name.clone();

    // port: ShapeDesc.cs:323-330 — passthru by name/material. (The C# also
    // prefixes Material with "passthru"; that mutation only feeds the
    // texture-key path and the contract ShapeDesc has no Material field, so it
    // is intentionally not tracked here — the passthru *flag* is what this
    // path produces.)
    if contains_ci(&name, "passthru") || contains_ci(&material, "passthru") {
        flags |= ShapeFlags::IS_PASSTHRU;
    }
    // port: ShapeDesc.cs:331-346 — group / billboard / grass / LOD flag.
    if stat.base_flags & 1 == 1 {
        flags |= ShapeFlags::IS_GROUP;
    }
    if stat.is_billboard {
        flags |= ShapeFlags::IS_BILLBOARD;
    }
    if stat.is_grass {
        flags |= ShapeFlags::IS_GRASS;
    } else {
        flags |= ShapeFlags::HAS_LOD_FLAG;
    }

    let mut alpha_threshold: u8 = 128;
    let mut alpha_flags: u16 = 4844;
    let mut backlight_power: f32 = 0.0;
    let mut grayscale_to_palette_scale: f32 = 1.0;
    let mut texture_clamp_mode: u32 = 0;
    let mut uv_scale = [1.0f32, 1.0];
    let mut uv_offset = [0.0f32, 0.0];

    // Per-ref translation/rotation (ShapeDesc.cs:358-367).
    let translation = stat.pos;
    let rotation = rotation_from_ref(stat.rot);

    // --- geometry ---
    let mut geometry = match extract_geometry(nif, geom) {
        Some(g) => g,
        None => return None,
    };
    if geometry.uvcoords.is_empty() {
        // port: ShapeDesc.cs:428-436 — skip shapes with no UVs.
        return None;
    }

    let mut textures = default_textures_fo4();
    let mut source_materials = Vec::new();
    let mut shader_type = ShaderKind::None;

    // --- shader properties (FO4 path; ShapeDesc.cs:437-996) ---
    // Try the BS shader slot first (Shader Property), then Alpha Property.
    let shader_ref = val_ref(geom.get_field("Shader Property")).unwrap_or(-1);
    let alpha_ref = val_ref(geom.get_field("Alpha Property")).unwrap_or(-1);

    if let Some(alpha) = block_of(nif, alpha_ref) {
        if alpha.type_name == "NiAlphaProperty" {
            // port: ShapeDesc.cs:446-454 — alpha (FO4 branch).
            flags |= ShapeFlags::IS_ALPHA;
            if let Some(t) = val_u64(alpha.get_field("Threshold")) {
                alpha_threshold = t as u8;
            }
            if let Some(f) = val_u64(alpha.get_field("Flags")) {
                alpha_flags = f as u16;
            }
        }
    }

    if let Some(shader) = block_of(nif, shader_ref) {
        match shader.type_name.as_str() {
            "BSEffectShaderProperty" => {
                shader_type = ShaderKind::Effect;
                read_effect_shader(
                    shader,
                    stat,
                    ctx,
                    &mut flags,
                    &mut textures,
                    &mut texture_clamp_mode,
                    &mut uv_scale,
                    &mut uv_offset,
                    &mut alpha_threshold,
                    &mut alpha_flags,
                    &mut source_materials,
                    &geometry,
                );
            }
            "BSLightingShaderProperty" => {
                shader_type = ShaderKind::Lighting;
                read_lighting_shader(
                    nif,
                    shader,
                    stat,
                    ctx,
                    &mut flags,
                    &mut textures,
                    &mut texture_clamp_mode,
                    &mut uv_scale,
                    &mut uv_offset,
                    &mut alpha_threshold,
                    &mut backlight_power,
                    &mut grayscale_to_palette_scale,
                    &mut source_materials,
                    &geometry,
                );
            }
            _ => {}
        }
    }

    // port: ShapeDesc.cs:1155-1188 — tree flag from base record flag 0x40.
    if stat.base_flags & 0x40 == 0x40 {
        flags |= ShapeFlags::IS_TREE;
        if contains_ci(&name, "FlatTrunk") {
            flags |= ShapeFlags::IS_FLAT_TRUNK;
        } else if contains_ci(&name, "Trunk") {
            flags |= ShapeFlags::IS_TRUNK;
        } else if contains_ci(&name, "Crown") {
            flags |= ShapeFlags::IS_CROWN;
        } else if flags.contains(ShapeFlags::IS_ALPHA) {
            if geometry.vertices.len() < 32 {
                flags |= ShapeFlags::IS_FLAT_TRUNK;
            } else {
                flags |= ShapeFlags::IS_CROWN;
            }
        } else {
            flags |= ShapeFlags::IS_TRUNK;
        }
    }

    // port: ShapeDesc.cs:1195-1207 — clamp UVs into [0,1] when clamp mode != WRAP.
    if texture_clamp_mode != 3 {
        for uv in &mut geometry.uvcoords {
            if texture_clamp_mode == 0 || texture_clamp_mode == 1 {
                uv[0] = uv[0].clamp(0.0, 1.0);
            }
            if texture_clamp_mode == 0 || texture_clamp_mode == 2 {
                uv[1] = uv[1].clamp(0.0, 1.0);
            }
        }
    }

    // port: ShapeDesc.cs:1209-1220 — dedup (skip for billboard/flat-trunk).
    if !flags.contains(ShapeFlags::IS_BILLBOARD) && !flags.contains(ShapeFlags::IS_FLAT_TRUNK) {
        geometry.remove_duplicate(true);
    }

    // port: ShapeDesc.cs:1221-1236 — drop duplicate triangles (insertion-ordered).
    drop_duplicate_triangles(&mut geometry);

    // port: ShapeDesc.cs:1282-1290 — vertex-color flag bookkeeping.
    if geometry.has_vertex_colors() {
        flags |= ShapeFlags::HAS_VERTEX_COLOR;
    } else {
        flags.remove(ShapeFlags::HAS_VERTEX_COLOR);
        flags.remove(ShapeFlags::HAS_VERTEX_ALPHA);
    }

    // port: ShapeDesc.cs:1300-1313 — normalize texture slots: blank non-.dds,
    // strip the leading `…Data\` prefix.
    for tex in &mut textures {
        if !contains_ci(tex, ".dds") {
            tex.clear();
        } else if let Some(stripped) = strip_data_prefix(tex) {
            *tex = stripped;
        }
    }

    name = name
        .replace("\\n", "")
        .replace("\\r", "")
        .trim()
        .to_string();

    Some(ShapeDesc {
        name,
        static_model,
        geometry,
        flags,
        textures,
        source_materials,
        textures_key: String::new(),
        texture_clamp_mode,
        alpha_threshold,
        alpha_flags,
        backlight_power,
        grayscale_to_palette_scale,
        enable_parent: stat.enable_parent,
        shader_type,
        x: 0.0,
        y: 0.0,
        bounding_box: BBox::empty(),
        segments: Vec::new(),
        uv_scale,
        uv_offset,
        ref_flags: stat.ref_flags,
        node_transform: mat4_identity(),
        node_scale: 1.0,
        translation,
        rotation,
        bto_translation: None,
        bto_scale: None,
    })
}

/// Resolve geometry from a geom block: inline Vertex Data, or via a Data ref.
/// port: ShapeDesc.cs:370-410.
fn extract_geometry(nif: &NifFile, geom: &NifBlock) -> Option<LodGeometry> {
    // Inline BSTriShape-family geometry.
    if matches!(geom.get_field("Vertex Data"), Some(NifValue::Array(a)) if !a.is_empty()) {
        return Some(geom_from_bstrishape(geom));
    }
    // NiTriShape / NiTriStrips → Data block.
    let data_ref = val_ref(geom.get_field("Data")).unwrap_or(-1);
    let data = block_of(nif, data_ref)?;
    let strips = data.type_name == "NiTriStripsData" || geom.type_name == "NiTriStrips";
    Some(geom_from_tridata(data, strips))
}

// ---------------------------------------------------------------------------
// BSEffectShaderProperty (port: ShapeDesc.cs:456-722, FO4 branch)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn read_effect_shader(
    shader: &NifBlock,
    stat: &StaticDesc,
    ctx: &QuadCtx,
    flags: &mut ShapeFlags,
    textures: &mut [String; 10],
    clamp: &mut u32,
    uv_scale: &mut [f32; 2],
    uv_offset: &mut [f32; 2],
    alpha_threshold: &mut u8,
    alpha_flags: &mut u16,
    source_materials: &mut Vec<String>,
    geometry: &LodGeometry,
) {
    // FO4: effect shader is always passthru (ShapeDesc.cs:458-465).
    *flags |= ShapeFlags::IS_PASSTHRU;

    if let Some(src) = nonempty(shader.get_field("Source Texture")) {
        textures[0] = src.to_lowercase();
    }
    if let Some(n) = nonempty(shader.get_field("Greyscale Texture")) {
        textures[3] = n.to_lowercase();
    }
    let sf2 = shader_flags(shader, "Shader Flags 2", false);
    let sf1 = shader_flags(shader, "Shader Flags 1", true);
    // port: ShapeDesc.cs:497-512 — vertex color / alpha bits.
    if sf2 & 0x20 == 0x20 && geometry.has_vertex_colors() {
        *flags |= ShapeFlags::HAS_VERTEX_COLOR;
    }
    if sf1 & 8 == 8 {
        *flags |= ShapeFlags::HAS_VERTEX_ALPHA;
    }
    if sf2 & 0x10 == 0x10 {
        *flags |= ShapeFlags::IS_DOUBLE_SIDED;
    }
    if sf1 & 0x10 == 0x10 && contains_ci(&textures[3], ".dds") {
        *flags |= ShapeFlags::IS_GREYSCALE_TO_PALETTE;
    }
    if sf1 & 0x20 == 0x20 && contains_ci(&textures[3], ".dds") {
        *flags |= ShapeFlags::IS_GREYSCALE_TO_ALPHA;
    }
    *clamp = clamp_mode(shader);
    if let Some(s) = uv2(shader.get_field("UV Scale")) {
        *uv_scale = s;
    }
    if let Some(o) = uv2(shader.get_field("UV Offset")) {
        *uv_offset = o;
    }

    // BGEM material (ShapeDesc.cs:538-700).
    if let Some(matname) = nonempty(shader.get_field("Name")) {
        let matname = matname.to_lowercase();
        if contains_ci(&matname, ".bgem") {
            source_materials.push(matname.clone());
            let resolved = resolve_material(stat, &matname);
            if let Some(bgem) = load_bgem(ctx, &resolved) {
                apply_bgem(
                    &bgem,
                    flags,
                    textures,
                    clamp,
                    uv_scale,
                    uv_offset,
                    alpha_threshold,
                    alpha_flags,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BSLightingShaderProperty (port: ShapeDesc.cs:723-986, FO4 branch)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn read_lighting_shader(
    nif: &NifFile,
    shader: &NifBlock,
    stat: &StaticDesc,
    ctx: &QuadCtx,
    flags: &mut ShapeFlags,
    textures: &mut [String; 10],
    clamp: &mut u32,
    uv_scale: &mut [f32; 2],
    uv_offset: &mut [f32; 2],
    alpha_threshold: &mut u8,
    backlight_power: &mut f32,
    grayscale_to_palette_scale: &mut f32,
    source_materials: &mut Vec<String>,
    geometry: &LodGeometry,
) {
    *clamp = clamp_mode(shader);
    if let Some(s) = uv2(shader.get_field("UV Scale")) {
        *uv_scale = s;
    }
    if let Some(o) = uv2(shader.get_field("UV Offset")) {
        *uv_offset = o;
    }
    let sf1 = shader_flags(shader, "Shader Flags 1", true);
    let sf2 = shader_flags(shader, "Shader Flags 2", false);
    // port: ShapeDesc.cs:755-781 — double-sided / decal / vertex-color / vertex-alpha.
    if sf2 & 0x10 == 0x10 {
        *flags |= ShapeFlags::IS_DOUBLE_SIDED;
    }
    if (sf1 & 0x4000000 == 0x4000000) || (sf1 & 0x8000000 == 0x8000000) {
        if !flags.contains(ShapeFlags::IS_PASSTHRU) {
            *flags |= ShapeFlags::IS_DECAL;
        }
    }
    if sf2 & 0x20 == 0x20 && geometry.has_vertex_colors() {
        *flags |= ShapeFlags::HAS_VERTEX_COLOR;
    }
    if sf1 & 8 == 8 {
        *flags |= ShapeFlags::HAS_VERTEX_ALPHA;
    }

    *backlight_power = val_f32(shader.get_field("Backlight Power")).unwrap_or(0.0);
    *grayscale_to_palette_scale =
        val_f32(shader.get_field("Grayscale to Palette Scale")).unwrap_or(1.0);

    // port: ShapeDesc.cs:804-814 — texture set slots.
    let ts_ref = val_ref(shader.get_field("Texture Set")).unwrap_or(-1);
    if let Some(ts) = block_of(nif, ts_ref) {
        for (i, t) in val_array(ts.get_field("Textures")).iter().enumerate() {
            if i >= 10 {
                break;
            }
            if let NifValue::String(s) = t {
                textures[i] = s.trim_end_matches('\0').to_lowercase().replace('/', "\\");
            }
        }
    }

    // port: ShapeDesc.cs:815-822 — greyscale flags from shader flag1 + slot 3.
    if sf1 & 0x10 == 0x10 && contains_ci(&textures[3], ".dds") {
        *flags |= ShapeFlags::IS_GREYSCALE_TO_PALETTE;
    }
    if sf1 & 0x20 == 0x20 && contains_ci(&textures[3], ".dds") {
        *flags |= ShapeFlags::IS_GREYSCALE_TO_ALPHA;
    }

    // BGSM material (ShapeDesc.cs:824-966).
    if let Some(matname) = nonempty(shader.get_field("Name")) {
        let matname = matname.to_lowercase();
        if contains_ci(&matname, ".bgsm") {
            source_materials.push(matname.clone());
            let resolved = resolve_material(stat, &matname);
            if let Some(bgsm) = load_bgsm(ctx, &resolved) {
                apply_bgsm(
                    &bgsm,
                    flags,
                    textures,
                    clamp,
                    uv_scale,
                    uv_offset,
                    alpha_threshold,
                    backlight_power,
                    grayscale_to_palette_scale,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Material swap + BGSM/BGEM read (via the `materials` crate)
// ---------------------------------------------------------------------------

/// Normalize a material filename (strip any `…Data\` prefix) and apply the
/// ref's material-swap map, honoring `*` wildcard rules.
/// port: ShapeDesc.cs:544-597 (BGEM) / :829-885 (BGSM).
fn resolve_material(stat: &StaticDesc, matname: &str) -> String {
    let mut name = strip_data_prefix(matname).unwrap_or_else(|| matname.to_string());
    if let Some(direct) = stat.material_swap.get(&name) {
        return direct.clone();
    }
    // Wildcard rules: a swap key with `*` matches as a regex `(.*)`.
    for (key, value) in &stat.material_swap {
        if let Some(repl) = wildcard_match(&name, key, value) {
            // BGSM additionally rejects a swap to tempartshader01 (ShapeDesc.cs:863).
            if !contains_ci(&repl, "tempartshader01.bgsm") {
                name = repl;
            }
            break;
        }
    }
    name
}

/// Match `name` against `key` (with `*` → `(.*)`); on match return `value` with
/// its `*` replaced by the captured group (when both contain `*`), else `value`.
/// port: the Regex `*`→`(.*)` swap (ShapeDesc.cs:563-579 / :848-867).
fn wildcard_match(name: &str, key: &str, value: &str) -> Option<String> {
    if !key.contains('*') {
        return None;
    }
    let lname = name.to_lowercase();
    let lkey = key.to_lowercase();
    let star = lkey.find('*')?;
    let prefix = &lkey[..star];
    let suffix = &lkey[star + 1..];
    if !lname.starts_with(prefix) || !lname.ends_with(suffix) {
        return None;
    }
    if lname.len() < prefix.len() + suffix.len() {
        return None;
    }
    let captured = &lname[prefix.len()..lname.len() - suffix.len()];
    if key.contains('*') && value.contains('*') {
        Some(value.replacen('*', captured, 1))
    } else {
        Some(value.to_string())
    }
}

fn load_bgsm(ctx: &QuadCtx, matname: &str) -> Option<materials_native::bgsm::BgsmData> {
    if matname.contains('*') {
        return None; // unexpanded wildcard — cannot resolve a concrete file.
    }
    let rel = if contains_ci(matname, "materials\\") {
        matname.to_string()
    } else {
        format!("Materials\\{matname}")
    };
    let path = resolve_data_path(ctx, &rel)?;
    let bytes = asset_source::read(&ctx.paths.data_dirs, &path).ok()?;
    materials_native::bgsm::parse(&bytes).ok()
}

fn load_bgem(ctx: &QuadCtx, matname: &str) -> Option<materials_native::bgem::BgemData> {
    if matname.contains('*') {
        return None;
    }
    let rel = if contains_ci(matname, "materials\\") {
        matname.to_string()
    } else {
        format!("Materials\\{matname}")
    };
    let path = resolve_data_path(ctx, &rel)?;
    let bytes = asset_source::read(&ctx.paths.data_dirs, &path).ok()?;
    materials_native::bgem::parse(&bytes).ok()
}

/// Apply a parsed BGSM to the shape state.
/// port: ShapeDesc.cs:886-966. Texture index map: [0]=Diffuse, [1]=Normal,
/// [2]=SmoothSpec→slot7, [3]=Greyscale→slot3.
#[allow(clippy::too_many_arguments)]
fn apply_bgsm(
    m: &materials_native::bgsm::BgsmData,
    flags: &mut ShapeFlags,
    textures: &mut [String; 10],
    clamp: &mut u32,
    uv_scale: &mut [f32; 2],
    uv_offset: &mut [f32; 2],
    alpha_threshold: &mut u8,
    backlight_power: &mut f32,
    grayscale_to_palette_scale: &mut f32,
) {
    // ShapeDesc.cs:888-891 — if diffuse is empty the C# logs and keeps slots; mirror by skipping.
    let diffuse = clean_tex(&m.DiffuseTexture);
    if diffuse.is_empty() {
        return;
    }
    textures[0] = diffuse;
    let normal = clean_tex(&m.NormalTexture);
    if !normal.is_empty() {
        textures[1] = normal;
    }
    let smooth_spec = clean_tex(&m.SmoothSpecTexture);
    if !smooth_spec.is_empty() {
        textures[7] = smooth_spec;
    } else if let Some(specular_fallback) = m
        .SpecularTexture
        .as_deref()
        .map(clean_tex)
        .filter(|texture| !texture.is_empty())
    {
        textures[7] = specular_fallback;
    }
    let greyscale = clean_tex(&m.GreyscaleTexture);
    if !greyscale.is_empty() {
        textures[3] = greyscale;
    }
    *clamp = clamp_from_tile(m.header.tile_u, m.header.tile_v);
    *uv_scale = [m.header.u_scale, m.header.v_scale];
    *uv_offset = [m.header.u_offset, m.header.v_offset];

    set_flag(flags, ShapeFlags::IS_ALPHA, m.header.alpha_test);
    set_flag(flags, ShapeFlags::IS_DOUBLE_SIDED, m.header.two_sided);
    set_flag(
        flags,
        ShapeFlags::IS_GREYSCALE_TO_PALETTE,
        m.header.grayscale_to_palette_color,
    );
    *grayscale_to_palette_scale = m.GrayscaleToPaletteScale;
    *alpha_threshold = m.header.alpha_test_ref;
    *backlight_power = m.BackLightPower.unwrap_or(0.0);

    // ShapeDesc.cs:947-957 — greyscale flags require a .dds slot-3 texture.
    if flags.contains(ShapeFlags::IS_GREYSCALE_TO_PALETTE) && !contains_ci(&textures[3], ".dds") {
        flags.remove(ShapeFlags::IS_GREYSCALE_TO_PALETTE);
    }
    if !flags.contains(ShapeFlags::IS_GREYSCALE_TO_PALETTE) {
        *grayscale_to_palette_scale = 1.0;
    }
    if flags.contains(ShapeFlags::IS_GREYSCALE_TO_ALPHA) && !contains_ci(&textures[3], ".dds") {
        flags.remove(ShapeFlags::IS_GREYSCALE_TO_ALPHA);
    }
    // ShapeDesc.cs:959-965 — decal from BGSM decal/decalnofade.
    if m.header.decal || m.header.decal_nofade {
        if !flags.contains(ShapeFlags::IS_PASSTHRU) {
            *flags |= ShapeFlags::IS_DECAL;
        }
    }
}

/// Apply a parsed BGEM to the shape state.
/// port: ShapeDesc.cs:598-695. BGEM texture map: [0]=Base→slot0, [3]=Normal→slot1,
/// [1]=Greyscale→slot3.
#[allow(clippy::too_many_arguments)]
fn apply_bgem(
    m: &materials_native::bgem::BgemData,
    flags: &mut ShapeFlags,
    textures: &mut [String; 10],
    clamp: &mut u32,
    uv_scale: &mut [f32; 2],
    uv_offset: &mut [f32; 2],
    alpha_threshold: &mut u8,
    alpha_flags: &mut u16,
) {
    let base = clean_tex(&m.BaseTexture);
    if !base.is_empty() {
        textures[0] = base;
    }
    let normal = clean_tex(&m.NormalTexture);
    if !normal.is_empty() {
        textures[1] = normal;
    }
    let greyscale = clean_tex(&m.GrayscaleTexture);
    if !greyscale.is_empty() {
        textures[3] = greyscale;
    }
    *clamp = clamp_from_tile(m.header.tile_u, m.header.tile_v);
    *uv_scale = [m.header.u_scale, m.header.v_scale];
    *uv_offset = [m.header.u_offset, m.header.v_offset];

    set_flag(flags, ShapeFlags::IS_DOUBLE_SIDED, m.header.two_sided);
    if contains_ci(&textures[3], ".dds") {
        set_flag(
            flags,
            ShapeFlags::IS_GREYSCALE_TO_PALETTE,
            m.header.grayscale_to_palette_color,
        );
    }
    *alpha_threshold = m.header.alpha_test_ref;
    // ShapeDesc.cs:657-661 — alpha flags packed from alpha blend modes.
    let mut af = 0x1000u32 | m.header.alpha_blend_mode0 as u32;
    af |= m.header.alpha_blend_mode1 << 1;
    af |= m.header.alpha_blend_mode2 << 5;
    af |= (m.header.alpha_test as u32) << 9;
    *alpha_flags = af as u16;
    set_flag(
        flags,
        ShapeFlags::IS_ALPHA,
        m.header.alpha_blend_mode0 != 0 || m.header.alpha_test,
    );

    if flags.contains(ShapeFlags::IS_GREYSCALE_TO_PALETTE) && !contains_ci(&textures[3], ".dds") {
        flags.remove(ShapeFlags::IS_GREYSCALE_TO_PALETTE);
    }
    if m.header.decal || m.header.decal_nofade {
        if !flags.contains(ShapeFlags::IS_PASSTHRU) {
            *flags |= ShapeFlags::IS_DECAL;
        }
    }
}

/// BGSM/BGEM clamp mode from tile flags: bit map matches FO4 Texture Clamp Mode
/// (tile=wrap, no-tile=clamp). 3=WRAP_S_WRAP_T … 0=CLAMP_S_CLAMP_T.
fn clamp_from_tile(tile_u: bool, tile_v: bool) -> u32 {
    (if tile_u { 2 } else { 0 }) | (if tile_v { 1 } else { 0 })
}

// ---------------------------------------------------------------------------
// Geometry helpers (extraction + dedup)
// ---------------------------------------------------------------------------

fn geom_from_bstrishape(shape: &NifBlock) -> LodGeometry {
    let mut g = LodGeometry::new();
    let data = val_array(shape.get_field("Vertex Data"));
    let has_colors = data.iter().any(|v| match v {
        NifValue::Struct(f) => f.contains_key("Vertex Colors"),
        _ => false,
    });
    for entry in data {
        let NifValue::Struct(f) = entry else { continue };
        g.vertices
            .push(vec3(f.get("Vertex")).unwrap_or([0.0, 0.0, 0.0]));
        g.uvcoords.push(uv2(f.get("UV")).unwrap_or([0.0, 0.0]));
        g.normals
            .push(vec3(f.get("Normal")).unwrap_or([0.0, 0.0, 1.0]));
        g.tangents
            .push(vec3(f.get("Tangent")).unwrap_or([1.0, 0.0, 0.0]));
        let bx = val_f32(f.get("Bitangent X")).unwrap_or(0.0);
        let by = val_f32(f.get("Bitangent Y")).unwrap_or(0.0);
        let bz = val_f32(f.get("Bitangent Z")).unwrap_or(0.0);
        g.bitangents.push([bx, by, bz]);
        if has_colors {
            g.vertex_colors
                .push(color4(f.get("Vertex Colors")).unwrap_or([1.0, 1.0, 1.0, 1.0]));
        }
    }
    g.triangles = read_triangles(shape.get_field("Triangles"));
    g
}

fn geom_from_tridata(data: &NifBlock, strips: bool) -> LodGeometry {
    let mut g = LodGeometry::new();
    for v in val_array(data.get_field("Vertices")) {
        g.vertices.push(vec3(Some(v)).unwrap_or([0.0, 0.0, 0.0]));
    }
    for n in val_array(data.get_field("Normals")) {
        g.normals.push(vec3(Some(n)).unwrap_or([0.0, 0.0, 1.0]));
    }
    for t in val_array(data.get_field("Tangents")) {
        g.tangents.push(vec3(Some(t)).unwrap_or([1.0, 0.0, 0.0]));
    }
    for b in val_array(data.get_field("Bitangents")) {
        g.bitangents.push(vec3(Some(b)).unwrap_or([0.0, 0.0, 0.0]));
    }
    if let Some(NifValue::Array(sets)) = data.get_field("UV Sets") {
        if let Some(NifValue::Array(set0)) = sets.first() {
            for uv in set0 {
                g.uvcoords.push(uv2(Some(uv)).unwrap_or([0.0, 0.0]));
            }
        }
    }
    for c in val_array(data.get_field("Vertex Colors")) {
        g.vertex_colors
            .push(color4(Some(c)).unwrap_or([1.0, 1.0, 1.0, 1.0]));
    }
    if strips {
        g.triangles = decode_strips(data);
    } else {
        g.triangles = read_triangles(data.get_field("Triangles"));
    }
    g
}

fn read_triangles(v: Option<&NifValue>) -> Vec<[u32; 3]> {
    val_array(v)
        .iter()
        .filter_map(|t| match t {
            NifValue::Struct(f) => Some([
                val_u64(f.get("v1"))? as u32,
                val_u64(f.get("v2"))? as u32,
                val_u64(f.get("v3"))? as u32,
            ]),
            NifValue::Array(a) if a.len() == 3 => Some([
                val_u64(Some(&a[0]))? as u32,
                val_u64(Some(&a[1]))? as u32,
                val_u64(Some(&a[2]))? as u32,
            ]),
            _ => None,
        })
        .collect()
}

fn decode_strips(data: &NifBlock) -> Vec<[u32; 3]> {
    let lengths: Vec<usize> = val_array(data.get_field("Strip Lengths"))
        .iter()
        .filter_map(|v| val_u64(Some(v)).map(|u| u as usize))
        .collect();
    let strips: Vec<Vec<usize>> = match data.get_field("Points") {
        Some(NifValue::Array(items)) if items.iter().all(|i| matches!(i, NifValue::Array(_))) => {
            items
                .iter()
                .map(|i| match i {
                    NifValue::Array(p) => p
                        .iter()
                        .filter_map(|x| val_u64(Some(x)).map(|u| u as usize))
                        .collect(),
                    _ => Vec::new(),
                })
                .collect()
        }
        Some(NifValue::Array(items)) => {
            let flat: Vec<usize> = items
                .iter()
                .filter_map(|x| val_u64(Some(x)).map(|u| u as usize))
                .collect();
            let mut out = Vec::new();
            let mut off = 0usize;
            for len in &lengths {
                let end = off.saturating_add(*len).min(flat.len());
                out.push(flat[off..end].to_vec());
                off = end;
            }
            out
        }
        _ => Vec::new(),
    };
    let mut tris = Vec::new();
    for points in &strips {
        for i in 0..points.len().saturating_sub(2) {
            let (a, b, c) = (points[i], points[i + 1], points[i + 2]);
            if a == b || b == c || a == c {
                continue;
            }
            if i % 2 == 0 {
                tris.push([a as u32, b as u32, c as u32]);
            } else {
                tris.push([b as u32, a as u32, c as u32]);
            }
        }
    }
    tris
}

/// Drop duplicate triangles (same v1_v2_v3 key), preserving first-seen order.
/// port: ShapeDesc.cs:1221-1236.
fn drop_duplicate_triangles(g: &mut LodGeometry) {
    let mut seen: std::collections::HashSet<(u32, u32, u32)> = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(g.triangles.len());
    for t in &g.triangles {
        let key = (t[0], t[1], t[2]);
        if seen.insert(key) {
            kept.push(*t);
        }
    }
    g.triangles = kept;
}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn nonempty(v: Option<&NifValue>) -> Option<String> {
    match v? {
        NifValue::String(s) => {
            let s = s.trim_end_matches('\0');
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
        _ => None,
    }
}

/// Trim a material-derived texture string of its trailing null terminator /
/// whitespace (a serialization artifact, not part of the path).
fn clean_tex(s: &str) -> String {
    s.trim_end_matches('\0').trim().to_string()
}

fn set_flag(flags: &mut ShapeFlags, flag: ShapeFlags, on: bool) {
    if on {
        *flags |= flag;
    } else {
        flags.remove(flag);
    }
}

/// Strip a leading `…Data\` / `…Data/` prefix (case-insensitive), returning
/// the remainder.  Handles both backslash (BSShaderTextureSet paths, which are
/// normalised by `read_lighting_shader` before reaching here) and forward-slash
/// (BGSM/BGEM texture strings via `clean_tex`, which are NOT slash-normalised).
///
/// port: the `Data\` strip in ShapeDesc.cs:1307-1310 + :544-555.
fn strip_data_prefix(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    // Try backslash form first (normalised by read_lighting_shader).
    if let Some(pos) = lower.rfind("data\\") {
        return Some(s[pos + 5..].to_string());
    }
    // Also handle forward-slash form (BGSM/BGEM textures not slash-normalised).
    if let Some(pos) = lower.rfind("data/") {
        return Some(s[pos + 5..].to_string());
    }
    None
}

/// Build a 3x3 rotation matrix from the ref's (rotX,rotY,rotZ) in radians,
/// using negated angles per ShapeDesc.cs:361-367 (R = Rx(-x)·Ry(-y)·Rz(-z)).
fn rotation_from_ref(rot: [f32; 3]) -> [[f32; 3]; 3] {
    let mx = rot_x(-rot[0]);
    let my = rot_y(-rot[1]);
    let mz = rot_z(-rot[2]);
    mat3_mul(&mat3_mul(&mx, &my), &mz)
}

fn rot_x(a: f32) -> [[f32; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}
fn rot_y(a: f32) -> [[f32; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}
fn rot_z(a: f32) -> [[f32; 3]; 3] {
    let (s, c) = a.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}
fn mat3_mul(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                out[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    out
}

// --- generic NifValue accessors (mirror nif_core/convert_file.rs private helpers) ---

fn val_f32(v: Option<&NifValue>) -> Option<f32> {
    match v? {
        NifValue::Float(f) => Some(*f as f32),
        NifValue::Int(i) => Some(*i as f32),
        NifValue::UInt(u) => Some(*u as f32),
        _ => None,
    }
}

fn val_ref(v: Option<&NifValue>) -> Option<i32> {
    match v? {
        NifValue::Ref(r) => Some(*r),
        NifValue::Int(i) => Some(*i as i32),
        NifValue::UInt(u) => Some(*u as i32),
        _ => None,
    }
}

fn val_u64(v: Option<&NifValue>) -> Option<u64> {
    match v? {
        NifValue::UInt(u) => Some(*u),
        NifValue::Int(i) if *i >= 0 => Some(*i as u64),
        NifValue::Ref(r) if *r >= 0 => Some(*r as u64),
        _ => None,
    }
}

fn val_string(v: Option<&NifValue>) -> Option<String> {
    match v? {
        NifValue::String(s) => Some(s.trim_end_matches('\0').to_string()),
        _ => None,
    }
}

fn val_array(v: Option<&NifValue>) -> &[NifValue] {
    match v {
        Some(NifValue::Array(items)) => items,
        _ => &[],
    }
}

fn vec3(v: Option<&NifValue>) -> Option<[f32; 3]> {
    match v? {
        NifValue::Vec3(a) => Some(*a),
        NifValue::Struct(fields) => Some([
            val_f32(fields.get("x")).unwrap_or(0.0),
            val_f32(fields.get("y")).unwrap_or(0.0),
            val_f32(fields.get("z")).unwrap_or(0.0),
        ]),
        _ => None,
    }
}

fn uv2(v: Option<&NifValue>) -> Option<[f32; 2]> {
    match v? {
        NifValue::Struct(fields) => Some([
            val_f32(fields.get("u")).unwrap_or(0.0),
            val_f32(fields.get("v")).unwrap_or(0.0),
        ]),
        NifValue::Vec3(a) => Some([a[0], a[1]]),
        _ => None,
    }
}

fn color4(v: Option<&NifValue>) -> Option<[f32; 4]> {
    match v? {
        NifValue::Color4(c) => Some(*c),
        NifValue::Struct(fields) => Some([
            val_f32(fields.get("r")).unwrap_or(1.0),
            val_f32(fields.get("g")).unwrap_or(1.0),
            val_f32(fields.get("b")).unwrap_or(1.0),
            val_f32(fields.get("a")).unwrap_or(1.0),
        ]),
        _ => None,
    }
}

fn block_of<'a>(nif: &'a NifFile, id: i32) -> Option<&'a NifBlock> {
    if id < 0 {
        return None;
    }
    nif.get_block(id as usize)
}

fn child_refs(node: &NifBlock) -> Vec<i32> {
    val_array(node.get_field("Children"))
        .iter()
        .filter_map(|v| val_ref(Some(v)))
        .collect()
}

fn is_ninode(block: &NifBlock) -> bool {
    matches!(
        block.type_name.as_str(),
        "NiNode"
            | "BSFadeNode"
            | "BSOrderedNode"
            | "BSMultiBoundNode"
            | "BSLeafAnimNode"
            | "NiSwitchNode"
            | "NiBillboardNode"
    )
}

fn node_hidden(block: &NifBlock) -> bool {
    val_u64(block.get_field("Flags"))
        .map(|f| f & 1 == 1)
        .unwrap_or(false)
}

fn node_scale(block: &NifBlock) -> f32 {
    val_f32(block.get_field("Scale")).unwrap_or(1.0)
}

fn block_name(block: &NifBlock) -> String {
    val_string(block.get_field("Name"))
        .unwrap_or_default()
        .replace("\\n", "")
        .replace("\\r", "")
        .trim()
        .to_string()
}

fn clamp_mode(block: &NifBlock) -> u32 {
    match block.get_field("Texture Clamp Mode") {
        Some(NifValue::String(s)) => {
            let key: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            match key.to_lowercase().as_str() {
                "clampsclampt" => 0,
                "clampswrapt" => 1,
                "wrapsclampt" => 2,
                "wrapswrapt" => 3,
                _ => 3,
            }
        }
        other => val_u64(other).map(|v| v as u32).unwrap_or(3),
    }
}

fn shader_flags(block: &NifBlock, field: &str, flags1: bool) -> u64 {
    match block.get_field(field) {
        Some(NifValue::UInt(b)) => *b,
        Some(NifValue::Int(b)) if *b >= 0 => *b as u64,
        Some(NifValue::Array(items)) => items
            .iter()
            .filter_map(|i| match i {
                NifValue::String(n) => flag_name_bit(n, flags1),
                _ => None,
            })
            .fold(0u64, |acc, bit| acc | (1u64 << bit)),
        Some(NifValue::String(n)) => flag_name_bit(n, flags1).map(|b| 1u64 << b).unwrap_or(0),
        _ => 0,
    }
}

fn flag_name_bit(name: &str, flags1: bool) -> Option<u64> {
    let key: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    if flags1 {
        match key.as_str() {
            "specular" => Some(0),
            "skinned" => Some(1),
            "vertexalpha" => Some(3),
            "greyscaletopalettecolor" => Some(4),
            "greyscaletopalettealpha" => Some(5),
            "usefalloff" => Some(6),
            "environmentmapping" => Some(7),
            "rgbfalloff" => Some(8),
            "castshadows" => Some(9),
            "face" => Some(10),
            "modelspacenormals" => Some(12),
            "refraction" => Some(15),
            "hair" => Some(18),
            "skintint" => Some(21),
            "ownemit" => Some(22),
            "decal" => Some(26),
            "dynamicdecal" => Some(27),
            "externalemittance" => Some(29),
            "softeffect" => Some(30),
            "zbuffertest" => Some(31),
            _ => None,
        }
    } else {
        match key.as_str() {
            "zbufferwrite" => Some(0),
            "lodobjects" => Some(2),
            "nofade" => Some(3),
            "doublesided" => Some(4),
            "vertexcolors" => Some(5),
            "glowmap" => Some(6),
            "transformchanged" => Some(7),
            _ => None,
        }
    }
}

// --- 4x4 matrix helpers (row-major) for node-transform accumulation ---

pub(crate) type Mat4 = [[f32; 4]; 4];

pub(crate) fn mat4_identity() -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[i][k] * b[k][j];
            }
            out[i][j] = s;
        }
    }
    out
}

fn node_local_transform(block: &NifBlock) -> Mat4 {
    let mut m = mat4_identity();
    if let Some(NifValue::Struct(rot)) = block.get_field("Rotation") {
        let g = |k: &str| val_f32(rot.get(k)).unwrap_or(0.0);
        m[0] = [g("m11"), g("m12"), g("m13"), 0.0];
        m[1] = [g("m21"), g("m22"), g("m23"), 0.0];
        m[2] = [g("m31"), g("m32"), g("m33"), 0.0];
    } else if let Some(NifValue::Matrix33(rot)) = block.get_field("Rotation") {
        for r in 0..3 {
            m[r] = [rot[r][0], rot[r][1], rot[r][2], 0.0];
        }
    }
    if let Some(t) = vec3(block.get_field("Translation")) {
        m[0][3] = t[0];
        m[1][3] = t[1];
        m[2][3] = t[2];
    }
    m
}

/// Column-vector transform for a geom block with its translation pre-scaled.
/// Port: C# `geom.GetTransform(scale)` = Matrix44(geomRotation, geomTranslation * scale, 1)
/// (NiAVObject.cs:199-202). The geom rotation is NOT scaled; only the translation is.
fn geom_transform_scaled(block: &NifBlock, scale: f32) -> Mat4 {
    let mut m = node_local_transform(block);
    m[0][3] *= scale;
    m[1][3] *= scale;
    m[2][3] *= scale;
    m
}

/// Resolve a LOD model path against the run's data_dirs.
///
/// FO4 base-record DistantLOD (MNAM) model paths are stored relative to the
/// `Meshes\` root (e.g. `LOD\Architecture\...`, `DLC03\LOD\...`) — they do NOT
/// include the `Meshes\` prefix, exactly like BGSM names omit `Materials\`. Try
/// the path as-is first (so a caller that already supplied a `Meshes\`-rooted
/// path — e.g. tests — still resolves), then with `Meshes\` prepended.
/// port: ParseNif's `niFile.Read(gameDir, staticModels[level])` (LODApp.cs:1376),
/// where the game's mesh root is the implicit base for the stored model path.
pub(crate) fn resolve_model_path(ctx: &QuadCtx, model: &str) -> Option<ResolvedAsset> {
    if let Some(p) = resolve_data_path(ctx, model) {
        return Some(p);
    }
    if !contains_ci(model, "meshes\\") && !contains_ci(model, "meshes/") {
        return resolve_data_path(ctx, &format!("Meshes\\{model}"));
    }
    None
}

/// Resolve a Data-relative path (backslash-separated, any case) against the
/// run's data_dirs. Returns the first existing match.
fn resolve_data_path(ctx: &QuadCtx, rel: &str) -> Option<ResolvedAsset> {
    asset_source::resolve(&ctx.paths.data_dirs, rel)
}

#[cfg(test)]
mod rooting_tests {
    use super::{apply_bgsm, resolve_model_path};
    use crate::game::Game;
    use crate::input::WorldspaceInput;
    use crate::objects::static_desc::ShapeFlags;
    use crate::progress::{LodPaths, QuadCtx};
    use crate::settings::LodSettings;

    /// A synthesized FO4 MNAM slot (`DLC03\LOD\Architecture\…_LOD.nif`, mixed
    /// case, `Meshes\`-relative without the prefix) must resolve to the on-disk
    /// FO76-style LOD mesh under `<data_dir>/meshes/dlc03/lod/architecture/…_lod.nif`
    /// (lowercase). This locks the rooting contract that ties the synthesize-MNAM
    /// phase + the Part-C output_dir search to lodgen's own resolver.
    #[test]
    fn synthesized_mnam_resolves_lowercase_lod_mesh_under_meshes() {
        let dir = std::env::temp_dir().join("lodgen_rooting_synth_mnam");
        let on_disk = dir
            .join("meshes")
            .join("dlc03")
            .join("lod")
            .join("architecture")
            .join("foo")
            .join("bar01_lod.nif");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(on_disk.parent().unwrap()).unwrap();
        std::fs::write(&on_disk, b"nif").unwrap();

        let world = WorldspaceInput::from_cells("TestW", vec![]);
        let settings = LodSettings::fo4_default();
        let game = Game::fo4();
        let paths = LodPaths {
            data_dirs: vec![dir.clone()],
            output_dir: dir.clone(),
            source_data_dir: None,
        };
        let ctx = QuadCtx {
            world: &world,
            settings: &settings,
            game: &game,
            paths: &paths,
            level: 4,
        };

        let resolved = resolve_model_path(&ctx, "DLC03\\LOD\\Architecture\\Foo\\Bar01_LOD.nif");
        assert!(
            resolved
                .as_ref()
                .and_then(|asset| asset.loose_path())
                .is_some_and(std::path::Path::is_file),
            "MNAM should resolve to the on-disk LOD mesh, got {resolved:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_test_bgsm() -> materials_native::bgsm::BgsmData {
        let mut bgsm = materials_native::bgsm::BgsmData::default();
        bgsm.DiffuseTexture = "textures\\trees\\pine_d.dds".to_string();
        bgsm
    }

    #[test]
    fn apply_bgsm_uses_source_specular_when_smoothspec_is_empty() {
        let mut bgsm = make_test_bgsm();
        bgsm.SmoothSpecTexture = " \0".to_string();
        bgsm.SpecularTexture = Some("textures\\trees\\pine_r.dds\0".to_string());

        let mut flags = ShapeFlags::empty();
        let mut textures: [String; 10] = std::array::from_fn(|_| String::new());
        let mut clamp = 0;
        let mut uv_scale = [0.0; 2];
        let mut uv_offset = [0.0; 2];
        let mut alpha_threshold = 0;
        let mut backlight_power = 0.0;
        let mut grayscale_to_palette_scale = 0.0;

        apply_bgsm(
            &bgsm,
            &mut flags,
            &mut textures,
            &mut clamp,
            &mut uv_scale,
            &mut uv_offset,
            &mut alpha_threshold,
            &mut backlight_power,
            &mut grayscale_to_palette_scale,
        );

        assert_eq!(textures[7], "textures\\trees\\pine_r.dds");
    }

    #[test]
    fn apply_bgsm_keeps_existing_smoothspec_over_specular_fallback() {
        let mut bgsm = make_test_bgsm();
        bgsm.SmoothSpecTexture = "textures\\trees\\pine_explicit_s.dds\0".to_string();
        bgsm.SpecularTexture = Some("textures\\trees\\pine_r.dds".to_string());

        let mut flags = ShapeFlags::empty();
        let mut textures: [String; 10] = std::array::from_fn(|_| String::new());
        let mut clamp = 0;
        let mut uv_scale = [0.0; 2];
        let mut uv_offset = [0.0; 2];
        let mut alpha_threshold = 0;
        let mut backlight_power = 0.0;
        let mut grayscale_to_palette_scale = 0.0;

        apply_bgsm(
            &bgsm,
            &mut flags,
            &mut textures,
            &mut clamp,
            &mut uv_scale,
            &mut uv_offset,
            &mut alpha_threshold,
            &mut backlight_power,
            &mut grayscale_to_palette_scale,
        );

        assert_eq!(textures[7], "textures\\trees\\pine_explicit_s.dds");
    }
}

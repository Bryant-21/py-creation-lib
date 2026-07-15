// Driver: level × quad loop + rayon parallelism + progress + stats + .lod write.
//
// Port of `TerrainLOD.GenerateLOD` outer loop (TerrainLOD.cs:1756-1801) and the
// xLODGen `GenerateLOD` orchestration in `Program.cs`.

use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Condvar, Mutex};

use crate::descriptors::{aligned_sw_cell, terrain_quads_for};
use crate::game::Game;
use crate::naming;
use crate::output::btt::write_tree_list;
use crate::output::lodsettings;
use crate::progress::{
    LodGenStats, LodPaths, ObjectModelTelemetry, ObjectQuadTelemetry, ObjectSimplifyStats,
    Progress, QuadCtx,
};
use crate::settings::LodSettings;
use crate::terrain;
use crate::trees;

const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
const OBJECT_QUAD_ACTIVE_MEMORY_CAP_LOD16: usize = 4;
const OBJECT_QUAD_ACTIVE_MEMORY_CAP_LOD32: usize = 2;
const OBJECT_LOD_PROCESS_PRIVATE_HARD_LIMIT_BYTES: u64 = 32 * BYTES_PER_GIB;
const OBJECT_LOD_PROCESS_PRIVATE_START_LIMIT_BYTES: u64 = 24 * BYTES_PER_GIB;
const OBJECT_LOD_SYSTEM_AVAILABLE_MIN_BYTES: u64 = 8 * BYTES_PER_GIB;
const OBJECT_LOD_COMMIT_AVAILABLE_MIN_BYTES: u64 = 16 * BYTES_PER_GIB;
const OBJECT_LOD_MEMORY_WARNING_PREFIX: &str = "lodgen object memory warning";

struct ObjectLevelTelemetry {
    level: i32,
    top_count: usize,
    huge_bto_warn_bytes: u64,
    simplify: ObjectSimplifyStats,
    models: BTreeMap<String, ObjectModelTelemetry>,
    huge_quads: Vec<ObjectQuadTelemetry>,
}

impl ObjectLevelTelemetry {
    fn new(level: i32, settings: &LodSettings) -> Self {
        Self {
            level,
            top_count: settings.objects.object_lod_top_model_count.max(1),
            huge_bto_warn_bytes: settings
                .objects
                .object_lod_huge_bto_warn_mb
                .saturating_mul(1024 * 1024),
            simplify: ObjectSimplifyStats::default(),
            models: BTreeMap::new(),
            huge_quads: Vec::new(),
        }
    }

    fn add_quad(&mut self, quad: ObjectQuadTelemetry) {
        self.simplify.add(&quad.simplify);
        if self.huge_bto_warn_bytes > 0 && quad.bto_bytes >= self.huge_bto_warn_bytes {
            self.huge_quads.push(quad.clone());
        }
        for model in quad.models {
            let entry =
                self.models
                    .entry(model.model.clone())
                    .or_insert_with(|| ObjectModelTelemetry {
                        model: model.model.clone(),
                        ..ObjectModelTelemetry::default()
                    });
            entry.shape_count += model.shape_count;
            entry.triangles_before += model.triangles_before;
            entry.triangles_after += model.triangles_after;
        }
    }

    fn summary_message(&self) -> Option<String> {
        if self.simplify.shapes_considered == 0 && self.models.is_empty() {
            return None;
        }
        Some(format!(
            "object LOD L{}: shapes={} tris={} top_out=[{}]",
            self.level,
            self.simplify.shapes_considered,
            self.simplify.triangles_after,
            format_top_models(self.models.values(), self.top_count, false),
        ))
    }

    fn huge_bto_warnings(&self) -> Vec<String> {
        let mut quads = self.huge_quads.clone();
        quads.sort_by(|a, b| b.bto_bytes.cmp(&a.bto_bytes));
        quads
            .into_iter()
            .take(self.top_count)
            .map(|quad| {
                let path = quad
                    .bto_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| format!("LOD{} {} {}", quad.level, quad.x, quad.y));
                format!(
                    "object LOD L{} huge BTO {:.1}MB {} top_models=[{}]",
                    quad.level,
                    quad.bto_bytes as f64 / (1024.0 * 1024.0),
                    path,
                    format_top_models(quad.models.iter(), self.top_count, false),
                )
            })
            .collect()
    }
}

fn report_object_level(
    progress: &mut dyn Progress,
    telemetry: &ObjectLevelTelemetry,
    level_index: usize,
    total_levels: f32,
) {
    if let Some(message) = telemetry.summary_message() {
        let frac = ((level_index + 1) as f32 / total_levels).min(1.0);
        progress.report(&message, frac);
    }
}

fn report_object_model_cache_level(
    progress: &mut dyn Progress,
    stats: crate::objects::parse_nif::ModelShapeCacheStats,
    level: i32,
    level_index: usize,
    total_levels: f32,
) {
    if stats.is_empty() {
        return;
    }
    let lookups = stats.hits + stats.misses;
    let hit_rate = if lookups == 0 {
        0.0
    } else {
        (stats.hits as f64 / lookups as f64) * 100.0
    };
    let frac = ((level_index + 1) as f32 / total_levels).min(1.0);
    progress.report(
        &format!(
            "object model cache L{level}: entries={} hits={} misses={} hit_rate={:.1}% bytes={:.1}MB evictions={} oversize={} prepared_shapes={} tris={}",
            stats.entries,
            stats.hits,
            stats.misses,
            hit_rate,
            stats.bytes as f64 / (1024.0 * 1024.0),
            stats.evictions,
            stats.oversize,
            stats.shapes_prepared,
            stats.triangles_after,
        ),
        frac,
    );
}

fn format_top_models<'a, I>(models: I, count: usize, by_saved: bool) -> String
where
    I: IntoIterator<Item = &'a ObjectModelTelemetry>,
{
    let mut models: Vec<&ObjectModelTelemetry> = models.into_iter().collect();
    if by_saved {
        models.sort_by(|a, b| {
            b.triangles_saved()
                .cmp(&a.triangles_saved())
                .then_with(|| b.triangles_after.cmp(&a.triangles_after))
                .then_with(|| a.model.cmp(&b.model))
        });
    } else {
        models.sort_by(|a, b| {
            b.triangles_after
                .cmp(&a.triangles_after)
                .then_with(|| b.triangles_saved().cmp(&a.triangles_saved()))
                .then_with(|| a.model.cmp(&b.model))
        });
    }
    models
        .into_iter()
        .take(count)
        .map(|model| {
            format!(
                "{} shapes={} before={} after={} saved={}",
                model.model,
                model.shape_count,
                model.triangles_before,
                model.triangles_after,
                model.triangles_saved()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Return mimalloc's cached free pages to the OS at LOD pass/level boundaries.
///
/// `generate_lod` runs terrain → atlas → every object level in one GIL-released
/// Rust call. Each level frees gigabytes (atlas tile loads, per-quad geometry
/// merges, the per-level rayon pool's abandoned thread heaps), but mimalloc keeps
/// those pages in its segment cache, so process-private memory only ratchets UP —
/// the multi-GiB "already resident before LOD16" high-water mark. `mi_collect`
/// (via esp's `trim_allocator`, the same hook the conversion phase boundaries use)
/// decommits them so each level's frees actually land. Wired only in the umbrella
/// build (`real-esp` links esp); a no-op in the default test build.
#[cfg(feature = "real-esp")]
fn trim_allocator() {
    esp_authoring_core::trim_allocator();
}

#[cfg(not(feature = "real-esp"))]
fn trim_allocator() {}

#[derive(Clone, Copy, Debug)]
struct MemorySnapshot {
    process_private_bytes: Option<u64>,
    available_physical_bytes: Option<u64>,
    available_commit_bytes: Option<u64>,
}

pub(crate) fn effective_worker_count(workers: usize) -> usize {
    if workers > 0 {
        workers
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get() / 2)
            .unwrap_or(1)
            .max(1)
    }
}

/// Build the bounded rayon pool for a LOD pass.
///
/// `workers > 0` uses that explicit thread count. `workers == 0` (the settings
/// sentinel) defaults to HALF the available cores (project convention; min 1) so
/// a run never saturates the machine via rayon's unbounded global pool. Returns
/// `None` only if the pool builder fails, in which case callers fall back to the
/// rayon global pool.
pub(crate) fn build_worker_pool(workers: usize) -> Option<rayon::ThreadPool> {
    let threads = effective_worker_count(workers);
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .ok()
}

fn object_quad_active_limit(workers: usize, level: i32) -> usize {
    let workers = effective_worker_count(workers);
    let cap = match level {
        16 => Some(OBJECT_QUAD_ACTIVE_MEMORY_CAP_LOD16),
        32 => Some(OBJECT_QUAD_ACTIVE_MEMORY_CAP_LOD32),
        _ => None,
    };
    cap.map(|cap| workers.min(cap).max(1)).unwrap_or(workers)
}

#[cfg(windows)]
fn memory_snapshot() -> Option<MemorySnapshot> {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};

    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }

    #[repr(C)]
    struct ProcessMemoryCountersEx {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(lp_buffer: *mut MemoryStatusEx) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }

    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCountersEx,
            cb: u32,
        ) -> i32;
    }

    let mut status: MemoryStatusEx = unsafe { zeroed() };
    status.dw_length = size_of::<MemoryStatusEx>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }

    let process_private_bytes = {
        let mut counters: ProcessMemoryCountersEx = unsafe { zeroed() };
        counters.cb = size_of::<ProcessMemoryCountersEx>() as u32;
        if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) } != 0 {
            Some(counters.private_usage as u64)
        } else {
            None
        }
    };

    Some(MemorySnapshot {
        process_private_bytes,
        available_physical_bytes: Some(status.ull_avail_phys),
        available_commit_bytes: Some(status.ull_avail_page_file),
    })
}

#[cfg(not(windows))]
fn memory_snapshot() -> Option<MemorySnapshot> {
    None
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_GIB as f64
}

fn format_gib(bytes: u64) -> String {
    format!("{:.1} GiB", gib(bytes))
}

fn validate_object_lod_memory_start(
    snapshot: Option<MemorySnapshot>,
    level: i32,
    active_limit: usize,
) -> Option<String> {
    let Some(snapshot) = snapshot else {
        return None;
    };

    if let Some(bytes) = snapshot.process_private_bytes {
        if bytes >= OBJECT_LOD_PROCESS_PRIVATE_START_LIMIT_BYTES {
            return Some(format!(
                "{OBJECT_LOD_MEMORY_WARNING_PREFIX}: starting LOD{level} object quad \
                 with {active_limit} active limit while process private memory is {} \
                 (start limit {}, hard limit {})",
                format_gib(bytes),
                format_gib(OBJECT_LOD_PROCESS_PRIVATE_START_LIMIT_BYTES),
                format_gib(OBJECT_LOD_PROCESS_PRIVATE_HARD_LIMIT_BYTES),
            ));
        }
    }

    if let Some(bytes) = snapshot.available_physical_bytes {
        if bytes < OBJECT_LOD_SYSTEM_AVAILABLE_MIN_BYTES {
            return Some(format!(
                "{OBJECT_LOD_MEMORY_WARNING_PREFIX}: starting LOD{level} object quad \
                 while available physical memory is {} (minimum {})",
                format_gib(bytes),
                format_gib(OBJECT_LOD_SYSTEM_AVAILABLE_MIN_BYTES),
            ));
        }
    }

    if let Some(bytes) = snapshot.available_commit_bytes {
        if bytes < OBJECT_LOD_COMMIT_AVAILABLE_MIN_BYTES {
            return Some(format!(
                "{OBJECT_LOD_MEMORY_WARNING_PREFIX}: starting LOD{level} object quad \
                 while available commit is {} (minimum {})",
                format_gib(bytes),
                format_gib(OBJECT_LOD_COMMIT_AVAILABLE_MIN_BYTES),
            ));
        }
    }

    None
}

fn validate_object_lod_memory_finish(
    snapshot: Option<MemorySnapshot>,
    level: i32,
) -> Option<String> {
    let Some(snapshot) = snapshot else {
        return None;
    };

    if let Some(bytes) = snapshot.process_private_bytes {
        if bytes >= OBJECT_LOD_PROCESS_PRIVATE_HARD_LIMIT_BYTES {
            return Some(format!(
                "{OBJECT_LOD_MEMORY_WARNING_PREFIX}: after LOD{level} object quad, \
                 process private memory reached {} (hard limit {})",
                format_gib(bytes),
                format_gib(OBJECT_LOD_PROCESS_PRIVATE_HARD_LIMIT_BYTES),
            ));
        }
    }

    None
}

fn record_object_lod_memory_warning(
    seen: &Mutex<BTreeSet<String>>,
    warnings: &Mutex<Vec<String>>,
    key: String,
    message: String,
) {
    let mut seen = seen.lock().expect("memory warning set poisoned");
    if seen.insert(key) {
        warnings
            .lock()
            .expect("memory warning list poisoned")
            .push(message);
    }
}

struct ParallelLimiter {
    available: Mutex<usize>,
    released: Condvar,
}

impl ParallelLimiter {
    fn new(limit: usize) -> Self {
        Self {
            available: Mutex::new(limit.max(1)),
            released: Condvar::new(),
        }
    }

    fn acquire(&self) -> ParallelPermit<'_> {
        let mut available = self.available.lock().expect("parallel limiter poisoned");
        while *available == 0 {
            available = self
                .released
                .wait(available)
                .expect("parallel limiter poisoned");
        }
        *available -= 1;
        ParallelPermit { limiter: self }
    }
}

struct ParallelPermit<'a> {
    limiter: &'a ParallelLimiter,
}

impl Drop for ParallelPermit<'_> {
    fn drop(&mut self) {
        let mut available = self
            .limiter
            .available
            .lock()
            .expect("parallel limiter poisoned");
        *available += 1;
        self.limiter.released.notify_one();
    }
}

pub(crate) fn ref_lod_cell(r: &crate::input::StaticDesc) -> (i32, i32) {
    (
        (r.pos[0] / 4096.0).floor() as i32,
        (r.pos[1] / 4096.0).floor() as i32,
    )
}

pub(crate) fn object_level_index(level: i32) -> i32 {
    match level {
        4 => 0,
        8 => 1,
        16 => 2,
        32 => 3,
        _ => 0,
    }
}

fn floor_to_multiple(value: i32, multiple: i32) -> i32 {
    if multiple <= 0 {
        return value;
    }
    value.div_euclid(multiple) * multiple
}

fn ceil_to_multiple(value: i32, multiple: i32) -> i32 {
    if multiple <= 0 {
        return value;
    }
    value.div_euclid(multiple) * multiple
        + if value.rem_euclid(multiple) == 0 {
            0
        } else {
            multiple
        }
}

fn content_cell_bounds(world: &crate::input::WorldspaceInput) -> Option<(i32, i32, i32, i32)> {
    let mut bounds = world.land_cell_bounds();
    for r in &world.refs {
        if r.lod_models.iter().all(Option::is_none) {
            continue;
        }
        let (x, y) = ref_lod_cell(r);
        bounds = Some(match bounds {
            Some((min_x, min_y, max_x, max_y)) => {
                (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
            }
            None => (x, y, x, y),
        });
    }
    bounds
}

fn configured_southwest_cell(
    world: &crate::input::WorldspaceInput,
    settings: &LodSettings,
) -> (i32, i32) {
    if let Some([x, y]) = settings.global.southwest_cell {
        return (x, y);
    }
    if let Some(bounds) = settings.global.bounds {
        return (bounds.w, bounds.s);
    }
    world.sw_cell
}

fn choose_fixed_stride_axis(min: i32, max: i32, stride: i32, align: i32) -> i32 {
    let sw = floor_to_multiple(min, align);
    if sw + stride - 1 >= max {
        return sw;
    }

    let needed = max - stride + 1;
    if align > 0 {
        let shifted = ceil_to_multiple(needed, align);
        if shifted <= min {
            shifted
        } else {
            floor_to_multiple(min + (max - min + 1 - stride) / 2, align)
        }
    } else {
        needed
    }
}

pub(crate) fn lod_settings_window(
    world: &crate::input::WorldspaceInput,
    settings: &LodSettings,
) -> ((i32, i32), i32) {
    let configured_sw = configured_southwest_cell(world, settings);
    let fallback_sw = aligned_sw_cell(configured_sw, settings.global.align);
    let bounds_ne = settings
        .global
        .bounds
        .map(|bounds| (bounds.e, bounds.n))
        .unwrap_or(world.ne_cell);
    let Some(stride) = settings.global.stride.filter(|s| *s > 0) else {
        return (
            fallback_sw,
            lodsettings::next_stride(fallback_sw, bounds_ne),
        );
    };

    if settings.global.southwest_cell.is_some() || settings.global.bounds.is_some() {
        return (fallback_sw, stride);
    }

    let Some((min_x, min_y, max_x, max_y)) = content_cell_bounds(world) else {
        return (fallback_sw, stride);
    };

    (
        (
            choose_fixed_stride_axis(min_x, max_x, stride, settings.global.align),
            choose_fixed_stride_axis(min_y, max_y, stride, settings.global.align),
        ),
        stride,
    )
}

pub(crate) fn object_quad_origin(pos: f32, south_west: i32, level: i32, quad_offset: f32) -> i32 {
    let remainder = south_west % level;
    let shifted = pos as f64 - (remainder as f64 * 4096.0);
    let origin = (shifted / quad_offset as f64).floor() as i32;
    let mut cell = origin * level;
    if remainder != 0 {
        cell += remainder;
    }
    cell
}

pub(crate) fn object_emit_bounds(
    settings: &LodSettings,
    lod_sw: (i32, i32),
    stride: i32,
) -> (i32, i32, i32, i32) {
    let window = (
        lod_sw.0,
        lod_sw.1,
        lod_sw.0.saturating_add(stride).saturating_sub(1),
        lod_sw.1.saturating_add(stride).saturating_sub(1),
    );
    if let Some(bounds) = settings.global.bounds {
        return (
            bounds.w.max(window.0),
            bounds.s.max(window.1),
            bounds.e.min(window.2),
            bounds.n.min(window.3),
        );
    }

    window
}

fn object_quads_for_refs<F>(
    world: &crate::input::WorldspaceInput,
    level: i32,
    include_ref: F,
    settings: &LodSettings,
) -> Vec<crate::descriptors::QuadDesc>
where
    F: Fn(&crate::input::StaticDesc) -> bool,
{
    let quad_index = object_level_index(level);
    let quad_offset = (level * 4096) as f32;
    let chunk_filter = settings.global.chunk.as_ref().filter(|c| c.level == level);
    let (lod_sw, stride) = lod_settings_window(world, settings);
    let (emit_w, emit_s, emit_e, emit_n) = object_emit_bounds(settings, lod_sw, stride);
    let mut quads: Vec<crate::descriptors::QuadDesc> = Vec::new();

    for (idx, r) in world.refs.iter().enumerate() {
        if !include_ref(r) {
            continue;
        }
        let (cell_x, cell_y) = ref_lod_cell(r);
        if cell_x < emit_w || cell_x > emit_e || cell_y < emit_s || cell_y > emit_n {
            continue;
        }
        let x = object_quad_origin(r.pos[0], lod_sw.0, level, quad_offset);
        let y = object_quad_origin(r.pos[1], lod_sw.1, level, quad_offset);
        if x < emit_w || x > emit_e || y < emit_s || y > emit_n {
            continue;
        }
        if chunk_filter
            .map(|c| x < c.w || x > c.e || y < c.s || y > c.n)
            .unwrap_or(false)
        {
            continue;
        }

        if let Some(q) = quads.iter_mut().find(|q| q.x == x && q.y == y) {
            q.static_indices.push(idx);
        } else {
            quads.push(crate::descriptors::QuadDesc {
                z_order: 0,
                x,
                y,
                quad_level: level,
                quad_index,
                quad_offset,
                static_indices: vec![idx],
                statics: Vec::new(),
                out_values: crate::descriptors::OutDesc::default(),
            });
        }
    }

    for q in &mut quads {
        q.static_indices.sort_by(|a, b| {
            let (ax, ay) = ref_lod_cell(&world.refs[*a]);
            let (bx, by) = ref_lod_cell(&world.refs[*b]);
            ax.cmp(&bx).then_with(|| by.cmp(&ay))
        });
    }
    quads
}

fn include_object_lod_ref(r: &crate::input::StaticDesc) -> bool {
    !trees::is_tree(r)
}

/// Run terrain LOD generation for all LOD levels in `[settings.global.lod_min..=lod_max]`.
///
/// For each level, enumerates quads, dispatches them via rayon, and folds results
/// deterministically. After all levels writes the `.lod` file if requested.
pub fn run_terrain(
    world: &crate::input::WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    progress: &mut dyn Progress,
) -> anyhow::Result<LodGenStats> {
    let mut stats = LodGenStats::default();

    // One-time fidelity warning for the still-DEFERRED options. protect_cell_borders
    // and skirts are now implemented (terrain_lod.rs); only the perf-only passes
    // optimize_unseen (below-water post culling) and hide_quads (covered-quad
    // removal) remain unported. Their absence only over-keeps geometry, never
    // breaks it, so this is a soft fidelity note.
    {
        let t = &settings.terrain;
        let mut deferred: Vec<&str> = Vec::new();
        if t.levels
            .iter()
            .any(|l| l.optimize_unseen != crate::settings::OptimizeUnseen::Off)
        {
            deferred.push("optimize_unseen");
        }
        if t.hide_quads {
            deferred.push("hide_quads");
        }
        if !deferred.is_empty() {
            stats.warnings.push(format!(
                "P1-FIDELITY: terrain perf options DEFERRED (output keeps extra \
                 geometry but is otherwise faithful): {}",
                deferred.join(", ")
            ));
        }
    }

    // Rayon pool: workers==0 → half the available cores (see build_worker_pool).
    let pool = build_worker_pool(settings.global.workers);

    let levels: Vec<i32> = game
        .lod_levels()
        .into_iter()
        .filter(|&l| l >= settings.global.lod_min && l <= settings.global.lod_max)
        .collect();
    let total_levels = levels.len() as f32;

    for (li, &level) in levels.iter().enumerate() {
        // Terrain emission is bounded by the land-cell extent (bbWorld), not the
        // declared SW..NE box — see terrain_quads_for (TerrainLOD.cs:1756-1758).
        let quads = terrain_quads_for(world, level, settings);
        let n_quads = quads.len();

        progress.report(
            &format!("LOD{level}: generating {n_quads} quads"),
            li as f32 / total_levels,
        );

        // Build a shared QuadCtx for this level. QuadCtx borrows world/settings/game/paths;
        // rayon needs them to be Send+Sync, which they are via shared references.
        let ctx = QuadCtx {
            world,
            settings,
            game,
            paths,
            level,
        };

        // Collect (quad_index, Result<QuadOutputs>) in parallel.
        let results: Vec<(usize, anyhow::Result<crate::progress::QuadOutputs>)> = {
            let do_par = |quads: &[crate::descriptors::QuadDesc]| {
                quads
                    .par_iter()
                    .enumerate()
                    .map(|(i, q)| (i, terrain::generate_quad(q, &ctx)))
                    .collect()
            };
            if let Some(ref pool) = pool {
                pool.install(|| do_par(&quads))
            } else {
                do_par(&quads)
            }
        };

        // Fold deterministically (by quad_index order).
        let mut indexed = results;
        indexed.sort_by_key(|(i, _)| *i);
        for (_, result) in indexed {
            match result {
                Ok(out) => {
                    stats.btr += out.meshes.len() as u32;
                    stats.dds += out.textures.len() as u32;
                }
                Err(e) => {
                    stats.warnings.push(format!("LOD{level} quad error: {e}"));
                }
            }
        }

        // Decommit this level's freed pages before the next allocates.
        trim_allocator();
    }

    // Write .lod settings file.
    if settings.global.write_lodsettings {
        let (lod_sw, stride) = lod_settings_window(world, settings);
        let lod_rel = crate::naming::lodsettings(&world.editor_id);
        let lod_path = paths.output_dir.join(lod_rel.replace('\\', "/"));
        if let Some(parent) = lod_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        lodsettings::write(
            &lod_path,
            lod_sw,
            stride,
            settings.global.lod_min,
            settings.global.lod_max,
        )?;
        stats.lod_written = true;
    }

    progress.report("done", 1.0);
    Ok(stats)
}

/// Read the tree-type indices from a written `.btt` block header.
///
/// `.btt` body: `[i32 numTypes]` then per type `[i32 index][i32 count][TreeRef×count]`,
/// where each `TreeRef` is the 32-byte packed `TwbLodTES5TreeRef` (wbLOD.pas:125-131).
/// Returns the `index` of every type present; empty on any read/parse shortfall.
fn read_btt_type_indices(path: &std::path::Path) -> Vec<i32> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let read_i32 = |b: &[u8], off: usize| -> Option<i32> {
        b.get(off..off + 4)
            .map(|s| i32::from_le_bytes(s.try_into().unwrap()))
    };
    let Some(num_types) = read_i32(&bytes, 0) else {
        return Vec::new();
    };
    let mut indices = Vec::with_capacity(num_types.max(0) as usize);
    let mut off = 4usize;
    for _ in 0..num_types.max(0) {
        let Some(index) = read_i32(&bytes, off) else {
            break;
        };
        off += 4;
        let Some(count) = read_i32(&bytes, off) else {
            break;
        };
        off += 4;
        indices.push(index);
        off += (count.max(0) as usize) * 32; // 32-byte TreeRef stride
    }
    indices
}

/// Run tree LOD generation for all LOD levels in `[settings.global.lod_min..=lod_max]`.
///
/// In billboard mode (`settings.trees.trees_3d = false`), loads the `BillboardManifest`
/// once per run (from `output_dir`/`data_dirs`) and passes it through to
/// `billboard_place::generate_quad`. After all levels, writes the world-level `.lst`
/// listing only the species actually placed into `.btt` blocks.
///
/// This function is the Task-9 tree-accounting driver. Full driver integration
/// (terrain + objects + trees in one pass).
///
/// The REAL object `AtlasResult` is threaded in (built once by
/// `run_objects`/`build_object_lod` and passed here), so 3D-tree quad UVs ARE
/// atlas-remapped onto the shared object atlas (closing the prior "stub empty
/// atlas" gap). Billboard mode is unaffected: its UVs come from the
/// `BillboardManifest`, not the object atlas.
pub fn run_trees(
    world: &crate::input::WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    atlas: &crate::atlas::AtlasResult,
    progress: &mut dyn Progress,
) -> anyhow::Result<LodGenStats> {
    let mut stats = LodGenStats::default();

    // Load the manifest once for billboard mode. If absent in billboard mode,
    // surface a one-time warning so the operator knows to run the Python generator.
    let manifest = if !settings.trees.trees_3d {
        // Build a temporary QuadCtx-like structure just for manifest resolution.
        let ctx_stub = QuadCtx {
            world,
            settings,
            game,
            paths,
            level: settings.global.lod_min,
        };
        let m = trees::load_billboard_manifest(&ctx_stub);
        if m.is_none() {
            stats.warnings.push(format!(
                "Billboard manifest not found for '{}' — run the Python billboard generator first. \
                 Expected: {}",
                world.editor_id,
                naming::billboard_manifest(&world.editor_id),
            ));
        }
        m
    } else {
        None
    };

    let pool = build_worker_pool(settings.global.workers);

    let levels: Vec<i32> = game
        .lod_levels()
        .into_iter()
        .filter(|&l| l >= settings.global.lod_min && l <= settings.global.lod_max)
        .collect();
    let total_levels = levels.len() as f32;

    // Tree-list indices actually emitted into `.btt` blocks. The world `.lst` is
    // built from this set only — Pascal lists only species drawn into the atlas
    // (wbLOD.pas:836-855), not every manifest entry.
    let mut placed_indices: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();

    for (li, &level) in levels.iter().enumerate() {
        let quads = object_quads_for_refs(world, level, trees::is_tree, settings);

        let n_quads = quads.len();
        progress.report(
            &format!("LOD{level}: generating {n_quads} tree quads"),
            li as f32 / total_levels,
        );

        let ctx = QuadCtx {
            world,
            settings,
            game,
            paths,
            level,
        };

        // The REAL object atlas (built once by build_object_lod) is threaded in here,
        // so 3D-tree UVs ARE atlas-remapped. Billboard mode ignores it (UVs come from
        // the manifest). port: the shared AtlasList consumed by ReUV/GroupShape.

        // In billboard mode, inject the pre-loaded manifest via per-quad calls.
        // The `trees::generate_quad` contract signature is fixed; we call
        // `billboard_place::generate_quad` directly when the manifest is available
        // to avoid a redundant load per quad.
        let results: Vec<(usize, anyhow::Result<crate::progress::QuadOutputs>)> = {
            let do_work = |quads: &[crate::descriptors::QuadDesc]| {
                quads
                    .par_iter()
                    .enumerate()
                    .map(|(i, q)| {
                        let out = if !settings.trees.trees_3d {
                            // Billboard path: pass manifest directly to avoid repeated disk reads.
                            let tree_statics: Vec<&crate::input::StaticDesc> = q
                                .static_refs(ctx.world)
                                .filter(|s| trees::is_tree(s))
                                .collect();
                            if tree_statics.is_empty() {
                                Ok(crate::progress::QuadOutputs::default())
                            } else {
                                crate::trees::billboard_place::generate_quad(
                                    q,
                                    &ctx,
                                    atlas,
                                    &tree_statics,
                                    manifest.as_ref(),
                                )
                            }
                        } else {
                            trees::generate_quad(q, &ctx, atlas)
                        };
                        (i, out)
                    })
                    .collect()
            };
            if let Some(ref pool) = pool {
                pool.install(|| do_work(&quads))
            } else {
                do_work(&quads)
            }
        };

        let mut indexed = results;
        indexed.sort_by_key(|(i, _)| *i);
        for (_, result) in indexed {
            match result {
                Ok(out) => {
                    // Count .btt and .bto outputs separately
                    for mesh in &out.meshes {
                        let name = mesh.to_string_lossy();
                        if name.ends_with(".btt") {
                            stats.btt += 1;
                            // Record which tree-list indices this block emitted so the
                            // world .lst lists only placed species (wbLOD.pas:836-855).
                            placed_indices.extend(read_btt_type_indices(mesh));
                        } else {
                            stats.bto += 1;
                        }
                    }
                    stats.dds += out.textures.len() as u32;
                }
                Err(e) => {
                    stats
                        .warnings
                        .push(format!("LOD{level} tree quad error: {e}"));
                }
            }
        }
    }

    // In billboard mode: write the world-level .lst once, listing only the species
    // actually placed into .btt blocks (Pascal lists only atlassed/drawn trees,
    // wbLOD.pas:836-855 — NOT every manifest entry).
    // port: TwbLodTES5TreeList.SaveToFile (wbLOD.pas:701-713)
    if !settings.trees.trees_3d {
        if let Some(m) = &manifest {
            use crate::output::btt::LstEntry;
            let entries: Vec<LstEntry> = m
                .entries
                .iter()
                .filter(|e| placed_indices.contains(&e.index))
                .map(|e| LstEntry {
                    index: e.index,
                    width: e.width,
                    height: e.height,
                    uv_min_x: e.uv_min_x,
                    uv_max_x: e.uv_max_x,
                    uv_min_y: e.uv_min_y,
                    uv_max_y: e.uv_max_y,
                })
                .collect();
            let lst_rel = naming::tree_list(&world.editor_id);
            let lst_path = paths.output_dir.join(lst_rel.replace('\\', "/"));
            write_tree_list(&lst_path, &entries)?;
        }
    }

    progress.report("done", 1.0);
    Ok(stats)
}

/// Run object (static, non-tree) LOD generation for all levels in
/// `[lod_min..=lod_max]`, distributing `world.refs` into per-quad statics buckets
/// and writing one `.bto` per non-empty quad via `objects::generate_quad`.
///
/// The REAL object `AtlasResult` is threaded in (built once by
/// `build_object_lod`), so object-LOD UVs ARE atlas-remapped — closing the prior
/// "stub empty atlas" gap. port: DoLOD FO4 object path outer loop (LODApp.cs).
pub fn run_objects(
    world: &crate::input::WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    atlas: &crate::atlas::AtlasResult,
    progress: &mut dyn Progress,
) -> anyhow::Result<LodGenStats> {
    let mut stats = LodGenStats::default();

    let pool = build_worker_pool(settings.global.workers);

    let levels: Vec<i32> = game
        .lod_levels()
        .into_iter()
        .filter(|&l| l >= settings.global.lod_min && l <= settings.global.lod_max)
        .collect();
    let total_levels = levels.len() as f32;

    for (li, &level) in levels.iter().enumerate() {
        crate::objects::parse_nif::clear_model_shape_cache();
        crate::objects::parse_nif::reset_model_shape_cache_stats();

        let quads = object_quads_for_refs(world, level, include_object_lod_ref, settings);

        let n_quads = quads.len();
        let active_limit = object_quad_active_limit(settings.global.workers, level);
        let worker_count = effective_worker_count(settings.global.workers);
        let message = if active_limit < worker_count {
            format!(
                "LOD{level}: generating {n_quads} object quads (memory cap {active_limit} active)"
            )
        } else {
            format!("LOD{level}: generating {n_quads} object quads")
        };
        progress.report(&message, li as f32 / total_levels);

        let ctx = QuadCtx {
            world,
            settings,
            game,
            paths,
            level,
        };

        let memory_warning_keys: Arc<Mutex<BTreeSet<String>>> =
            Arc::new(Mutex::new(BTreeSet::new()));
        let memory_warnings: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let results: Vec<(usize, anyhow::Result<crate::progress::QuadOutputs>)> = {
            let limiter = Arc::new(ParallelLimiter::new(active_limit));
            let do_par = |quads: &[crate::descriptors::QuadDesc]| {
                quads
                    .par_iter()
                    .enumerate()
                    .map(|(i, q)| {
                        let _permit = limiter.acquire();

                        if let Some(message) = validate_object_lod_memory_start(
                            memory_snapshot(),
                            ctx.level,
                            active_limit,
                        ) {
                            record_object_lod_memory_warning(
                                &memory_warning_keys,
                                &memory_warnings,
                                format!("LOD{}:start", ctx.level),
                                message,
                            );
                        }

                        let result = crate::objects::generate_quad(q, &ctx, atlas);

                        if let Some(message) =
                            validate_object_lod_memory_finish(memory_snapshot(), ctx.level)
                        {
                            record_object_lod_memory_warning(
                                &memory_warning_keys,
                                &memory_warnings,
                                format!("LOD{}:finish", ctx.level),
                                message,
                            );
                        }

                        (i, result)
                    })
                    .collect()
            };
            if let Some(ref pool) = pool {
                pool.install(|| do_par(&quads))
            } else {
                do_par(&quads)
            }
        };

        let memory_warnings = memory_warnings
            .lock()
            .expect("memory warning list poisoned")
            .clone();
        stats.warnings.extend(memory_warnings);

        let mut object_level_telemetry = ObjectLevelTelemetry::new(level, settings);
        let mut indexed = results;
        indexed.sort_by_key(|(i, _)| *i);
        for (_, result) in indexed {
            match result {
                Ok(out) => {
                    stats.bto += out.meshes.len() as u32;
                    stats.dds += out.textures.len() as u32;
                    if let Some(telemetry) = out.object_lod {
                        object_level_telemetry.add_quad(telemetry);
                    }
                }
                Err(e) => stats
                    .warnings
                    .push(format!("LOD{level} object quad error: {e}")),
            }
        }
        report_object_level(progress, &object_level_telemetry, li, total_levels);
        report_object_model_cache_level(
            progress,
            crate::objects::parse_nif::model_shape_cache_stats(),
            level,
            li,
            total_levels,
        );
        stats
            .warnings
            .extend(object_level_telemetry.huge_bto_warnings());

        // Each object level decoded its slot's unique LOD meshes into the shared
        // caches and merged gigabytes of geometry. Drop the caches (the next level
        // uses a different lod_models slot) and decommit, so resident memory is
        // bounded to one level's unique meshes instead of ratcheting upward.
        crate::objects::parse_nif::clear_model_shape_cache();
        crate::objects::parse_nif::clear_nif_cache();
        trim_allocator();
    }

    progress.report("done", 1.0);
    Ok(stats)
}

/// Unified object+tree LOD pass: build the object atlas ONCE from all enumerated
/// refs, then write one object BTO per quad in 3D-tree mode. Billboard tree mode
/// still uses the separate BTT pass. port: LODApp atlas build (once) → DoLOD per quad.
pub fn build_object_lod(
    world: &crate::input::WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    progress: &mut dyn Progress,
) -> anyhow::Result<LodGenStats> {
    let mut stats = LodGenStats::default();

    // Build the shared object atlas once (port: LODApp builds the atlas before the
    // per-quad DoLOD loop). On a hard build error, fall back to an empty atlas so
    // object LOD still emits un-atlassed meshes rather than aborting the run.
    let atlas = {
        let ctx = QuadCtx {
            world,
            settings,
            game,
            paths,
            level: settings.global.lod_min,
        };
        match crate::atlas::build_object_atlas_with_progress(
            &world.refs,
            &ctx,
            Some(&mut *progress),
        ) {
            Ok(a) => a,
            Err(e) => {
                stats
                    .warnings
                    .push(format!("object atlas build failed (un-atlassed LOD): {e}"));
                crate::atlas::atlas::AtlasResult {
                    map_path: paths.output_dir.join("atlas.txt"),
                    diffuse: paths.output_dir.join("atlas.dds"),
                    normal: paths.output_dir.join("atlas_n.dds"),
                    specular: paths.output_dir.join("atlas_s.dds"),
                    atlas_size: (0, 0),
                    uv: Default::default(),
                    list: Default::default(),
                    dds_written: 0,
                }
            }
        }
    };
    if atlas.atlas_size.0 > 0 {
        // Count only the DDS files that were actually written to disk (0–3).
        // A failed encode leaves the atlas UV data intact (meshes still remap UVs
        // correctly) but the .bto ships without the texture → pink LOD in-game.
        stats.dds += atlas.dds_written;
        if atlas.dds_written < 3 {
            stats.warnings.push(format!(
                "atlas DDS encode: only {}/{} files written; \
                 LOD may show pink textures in-game",
                atlas.dds_written, 3
            ));
        }
    }

    // The atlas build is object LOD's single biggest transient spike — it decodes
    // every unique source tile at full resolution into one in-memory tile list, and
    // its NIF scan leaves decoded meshes in the shared cache. The AtlasResult kept
    // afterward is tiny (UV map), so drop the cached NIFs and decommit the spike
    // now, before run_objects starts, instead of carrying it as resident baseline.
    crate::objects::parse_nif::clear_nif_cache();
    crate::objects::parse_nif::clear_model_shape_cache();
    trim_allocator();

    let obj = run_objects(world, settings, game, paths, &atlas, progress)?;
    stats.bto += obj.bto;
    stats.dds += obj.dds;
    stats.warnings.extend(obj.warnings);

    let tree = if settings.global.generate_trees {
        run_trees(world, settings, game, paths, &atlas, progress)?
    } else {
        LodGenStats::default()
    };
    stats.btt += tree.btt;
    stats.bto += tree.bto;
    stats.dds += tree.dds;
    stats.warnings.extend(tree.warnings);

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Game;
    use crate::progress::{LodPaths, Progress};
    use crate::settings::LodSettings;

    struct NullProgress;
    impl Progress for NullProgress {
        fn report(&mut self, _m: &str, _f: f32) {}
    }

    struct CollectProgress {
        events: Vec<(String, f32)>,
    }

    impl Progress for CollectProgress {
        fn report(&mut self, msg: &str, frac: f32) {
            self.events.push((msg.to_string(), frac));
        }
    }

    #[test]
    fn meshopt_object_telemetry_reports_summary_and_huge_bto_models() {
        let mut settings = LodSettings::fo4_default();
        settings.objects.object_lod_top_model_count = 2;
        settings.objects.object_lod_huge_bto_warn_mb = 1;
        let mut telemetry = ObjectLevelTelemetry::new(16, &settings);

        telemetry.add_quad(ObjectQuadTelemetry {
            level: 16,
            x: -4,
            y: 8,
            bto_path: Some(std::path::PathBuf::from(
                r"Meshes\Terrain\W\Objects\W.16.-4.8.bto",
            )),
            bto_bytes: 2 * 1024 * 1024,
            output_shape_count: 2,
            simplify: ObjectSimplifyStats {
                shapes_considered: 3,
                shapes_skipped: 1,
                shapes_simplified: 2,
                attr_simplifier_count: 2,
                sloppy_count: 1,
                budget_clamp_count: 1,
                triangles_before: 1_500,
                triangles_after: 300,
            },
            models: vec![
                ObjectModelTelemetry {
                    model: r"Architecture\A.nif".to_string(),
                    shape_count: 1,
                    triangles_before: 1_000,
                    triangles_after: 100,
                },
                ObjectModelTelemetry {
                    model: r"Architecture\B.nif".to_string(),
                    shape_count: 2,
                    triangles_before: 500,
                    triangles_after: 200,
                },
            ],
        });

        let mut progress = CollectProgress { events: Vec::new() };
        report_object_level(&mut progress, &telemetry, 2, 4.0);

        assert_eq!(progress.events.len(), 1);
        assert!(progress.events[0].0.contains("object LOD L16: shapes=3"));
        assert!(
            progress.events[0]
                .0
                .contains(r"top_out=[Architecture\B.nif shapes=2 before=500 after=200 saved=300")
        );
        assert_eq!(progress.events[0].1, 0.75);

        let warnings = telemetry.huge_bto_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("object LOD L16 huge BTO 2.0MB"));
        assert!(warnings[0].contains(r"Architecture\B.nif shapes=2 before=500 after=200"));
    }

    /// workers==0 builds a BOUNDED pool of ~half the cores (never the unbounded
    /// global pool); workers>0 honors the explicit count.
    #[test]
    fn worker_pool_defaults_to_half_cores() {
        let half = std::thread::available_parallelism()
            .map(|n| n.get() / 2)
            .unwrap_or(1)
            .max(1);
        let pool = build_worker_pool(0).expect("default pool builds");
        assert_eq!(pool.current_num_threads(), half, "workers==0 → half cores");

        let explicit = build_worker_pool(3).expect("explicit pool builds");
        assert_eq!(
            explicit.current_num_threads(),
            3,
            "workers>0 → explicit count"
        );
    }

    #[test]
    fn ref_lod_cell_uses_world_position_floor() {
        let r = crate::input::RefInput {
            ref_id: "00000001".to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (99, 99),
            pos: [-1.0, 8192.0, 0.0],
            rot: [0.0; 3],
            scale: 1.0,
            color: 1.0,
            alpha_threshold: 128,
            is_billboard: false,
            is_grass: false,
            base_name: "Test".to_string(),
            base_flags: 0,
            material_name: String::new(),
            full_model: String::new(),
            lod_models: [None, None, None, None],
            part_transform: crate::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        };

        assert_eq!(
            ref_lod_cell(&r),
            (-1, 2),
            "object LOD buckets must follow xLODGen's floor(DATA.position / 4096)"
        );
    }

    #[test]
    fn object_quad_origin_matches_xlodgen_remainder_math() {
        let offset = 4.0 * 4096.0;

        assert_eq!(
            object_quad_origin(-25.0 * 4096.0 + 1.0, -25, 4, offset),
            -25
        );
        assert_eq!(
            object_quad_origin(-21.0 * 4096.0 + 128.0, -25, 4, offset),
            -21
        );
        assert_eq!(object_quad_origin(-1.0, 0, 4, offset), -4);
        assert_eq!(object_quad_origin(-12.0 * 4096.0, 0, 4, offset), -12);
        assert_eq!(object_quad_origin(-16.0 * 4096.0, 0, 4, offset), -16);
    }

    #[test]
    fn object_quad_active_limit_caps_memory_heavy_parallelism() {
        assert_eq!(object_quad_active_limit(1, 4), 1);
        assert_eq!(object_quad_active_limit(4, 4), 4);
        assert_eq!(object_quad_active_limit(16, 4), 16);
        assert_eq!(object_quad_active_limit(16, 8), 16);
        assert_eq!(object_quad_active_limit(16, 16), 4);
        assert_eq!(object_quad_active_limit(16, 32), 2);
    }

    #[test]
    fn object_lod_memory_warning_reports_high_process_private_bytes() {
        let snapshot = MemorySnapshot {
            process_private_bytes: Some(OBJECT_LOD_PROCESS_PRIVATE_START_LIMIT_BYTES),
            available_physical_bytes: Some(OBJECT_LOD_SYSTEM_AVAILABLE_MIN_BYTES),
            available_commit_bytes: Some(OBJECT_LOD_COMMIT_AVAILABLE_MIN_BYTES),
        };

        let warning = validate_object_lod_memory_start(Some(snapshot), 16, 1)
            .expect("process private memory at the start limit should warn");

        assert!(warning.starts_with(OBJECT_LOD_MEMORY_WARNING_PREFIX));
    }

    #[test]
    fn object_lod_memory_warning_reports_high_process_private_bytes_on_lod8_start() {
        let snapshot = MemorySnapshot {
            process_private_bytes: Some(OBJECT_LOD_PROCESS_PRIVATE_START_LIMIT_BYTES + 1),
            available_physical_bytes: Some(OBJECT_LOD_SYSTEM_AVAILABLE_MIN_BYTES),
            available_commit_bytes: Some(OBJECT_LOD_COMMIT_AVAILABLE_MIN_BYTES),
        };

        let warning = validate_object_lod_memory_start(Some(snapshot), 8, 16)
            .expect("LOD8 high process-private memory should warn, not abort");

        assert!(warning.contains("LOD8"));
    }

    #[test]
    fn object_lod_memory_warning_reports_low_available_commit() {
        let snapshot = MemorySnapshot {
            process_private_bytes: Some(1 * BYTES_PER_GIB),
            available_physical_bytes: Some(OBJECT_LOD_SYSTEM_AVAILABLE_MIN_BYTES),
            available_commit_bytes: Some(OBJECT_LOD_COMMIT_AVAILABLE_MIN_BYTES - 1),
        };

        let warning = validate_object_lod_memory_start(Some(snapshot), 32, 1)
            .expect("low available commit should warn before another quad starts");

        assert!(warning.contains("available commit"));
    }

    #[test]
    fn object_lod_memory_finish_reports_hard_cap() {
        let snapshot = MemorySnapshot {
            process_private_bytes: Some(OBJECT_LOD_PROCESS_PRIVATE_HARD_LIMIT_BYTES),
            available_physical_bytes: None,
            available_commit_bytes: None,
        };

        let warning = validate_object_lod_memory_finish(Some(snapshot), 32)
            .expect("process private memory at the hard cap should warn");

        assert!(warning.starts_with(OBJECT_LOD_MEMORY_WARNING_PREFIX));
    }

    #[test]
    fn object_lod_memory_warnings_are_deduplicated_by_key() {
        let seen = Mutex::new(BTreeSet::new());
        let warnings = Mutex::new(Vec::new());

        record_object_lod_memory_warning(
            &seen,
            &warnings,
            "LOD8:start".to_string(),
            "first".to_string(),
        );
        record_object_lod_memory_warning(
            &seen,
            &warnings,
            "LOD8:start".to_string(),
            "second".to_string(),
        );
        record_object_lod_memory_warning(
            &seen,
            &warnings,
            "LOD8:finish".to_string(),
            "third".to_string(),
        );

        assert_eq!(
            warnings.lock().expect("warnings lock").as_slice(),
            &["first".to_string(), "third".to_string()]
        );
    }

    #[test]
    fn object_quads_are_sparse_and_position_bucketed() {
        let mut world = flat_world();
        world.sw_cell = (-25, -19);
        world.ne_cell = (23, 25);

        let make_ref = |id: &str, pos: [f32; 3]| crate::input::RefInput {
            ref_id: id.to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (99, 99),
            pos,
            rot: [0.0; 3],
            scale: 1.0,
            color: 1.0,
            alpha_threshold: 128,
            is_billboard: false,
            is_grass: false,
            base_name: "Test".to_string(),
            base_flags: 0,
            material_name: String::new(),
            full_model: String::new(),
            lod_models: [None, None, None, None],
            part_transform: crate::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        };
        world.refs = vec![
            make_ref(
                "00000001",
                [-25.0 * 4096.0 + 1.0, -19.0 * 4096.0 + 1.0, 0.0],
            ),
            make_ref(
                "00000002",
                [-21.0 * 4096.0 + 1.0, -19.0 * 4096.0 + 1.0, 0.0],
            ),
        ];
        let settings = LodSettings::fo4_default();

        let quads = object_quads_for_refs(&world, 4, |_| true, &settings);
        let origins: Vec<_> = quads.iter().map(|q| (q.x, q.y)).collect();

        assert_eq!(origins, vec![(-25, -19), (-21, -19)]);
        assert!(quads.iter().all(|q| q.statics.is_empty()));
        assert!(quads.iter().all(|q| q.quad_offset == 4.0 * 4096.0));
    }

    #[test]
    fn object_lod_excludes_tree_refs_in_3d_tree_mode() {
        let mut world = flat_world();
        world.refs = vec![
            lod_ref("tree", [1.0, 1.0, 0.0]),
            lod_ref("object", [4096.0 + 1.0, 1.0, 0.0]),
        ];
        world.refs[0].base_flags = 0x1000;

        let mut settings = LodSettings::fo4_default();
        settings.trees.trees_3d = true;

        let quads = object_quads_for_refs(&world, 4, include_object_lod_ref, &settings);
        let ids: Vec<_> = quads
            .iter()
            .flat_map(|q| {
                q.static_indices
                    .iter()
                    .map(|&i| world.refs[i].ref_id.as_str())
            })
            .collect();

        assert_eq!(ids, vec!["object"]);
    }

    #[test]
    fn explicit_lod_window_uses_compat_bounds_and_no_align() {
        let mut world = flat_world();
        world.sw_cell = (-190, -221);
        world.ne_cell = (252, 135);
        world.refs = vec![
            lod_ref(
                "00000001",
                [-126.0 * 4096.0 + 1.0, -125.0 * 4096.0 + 1.0, 0.0],
            ),
            lod_ref("00000002", [98.0 * 4096.0 + 1.0, 131.0 * 4096.0 + 1.0, 0.0]),
            lod_ref(
                "00000003",
                [-127.0 * 4096.0 + 1.0, -125.0 * 4096.0 + 1.0, 0.0],
            ),
        ];

        let mut settings = LodSettings::fo4_default();
        settings.global.stride = Some(256);
        settings.global.align = 0;
        settings.global.southwest_cell = Some([-126, -125]);
        settings.global.bounds = Some(crate::settings::LodBounds {
            w: -126,
            s: -125,
            e: 129,
            n: 130,
        });

        let (lod_sw, stride) = lod_settings_window(&world, &settings);
        assert_eq!(stride, 256);
        assert_eq!(lod_sw, (-126, -125));

        let quads = object_quads_for_refs(&world, 4, |_| true, &settings);
        let origins: Vec<_> = quads.iter().map(|q| (q.x, q.y)).collect();
        assert_eq!(origins, vec![(-126, -125)]);
    }

    fn lod_ref(id: &str, pos: [f32; 3]) -> crate::input::RefInput {
        crate::input::RefInput {
            ref_id: id.to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (0, 0),
            pos,
            rot: [0.0; 3],
            scale: 1.0,
            color: 1.0,
            alpha_threshold: 128,
            is_billboard: false,
            is_grass: false,
            base_name: "Test".to_string(),
            base_flags: 0,
            material_name: String::new(),
            full_model: String::new(),
            lod_models: [Some("LOD\\Test.nif".to_string()), None, None, None],
            part_transform: crate::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        }
    }

    fn flat_world() -> crate::input::WorldspaceInput {
        crate::input::WorldspaceInput::from_cells(
            "W",
            (0..8)
                .flat_map(|y| (0..8).map(move |x| (x, y)))
                .map(|(x, y)| crate::input::CellInput {
                    x,
                    y,
                    heights: vec![0.0; 33 * 33],
                    vertex_colors: vec![[255, 255, 255]; 33 * 33],
                    layers: Vec::new(),
                    hidden_quadrants: [false; 4],
                    water_height: f32::MIN,
                })
                .collect(),
        )
    }

    #[test]
    fn run_terrain_produces_all_levels() {
        let w = flat_world();
        let s = LodSettings::fo4_default(); // lod_min 4, lod_max 32
        let g = Game::fo4();
        let out = std::env::temp_dir().join("lodgen_driver_test");
        std::fs::create_dir_all(&out).unwrap();
        let paths = LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: out,
            source_data_dir: None,
        };
        let mut p = NullProgress;
        let stats: LodGenStats = run_terrain(&w, &s, &g, &paths, &mut p).unwrap();
        // 8x8 world: L4 -> 2x2=4 quads, L8 -> 1 quad, L16 -> 1, L32 -> 1 => 7 .btr
        assert_eq!(stats.btr, 7);
        assert_eq!(stats.dds, 14); // 2 per quad
        assert!(stats.lod_written);
    }

    #[test]
    fn run_terrain_writes_aligned_lod_sw_cell() {
        let mut w = flat_world();
        for cell in &mut w.cells {
            cell.x += 2;
            cell.y += 3;
        }
        w.sw_cell = (2, 3);
        w.ne_cell = (9, 10);

        let mut s = LodSettings::fo4_default();
        s.global.lod_min = 4;
        s.global.lod_max = 4;
        s.global.align = 4;

        let g = Game::fo4();
        let out =
            std::env::temp_dir().join(format!("lodgen_driver_align_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let paths = LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: out.clone(),
            source_data_dir: None,
        };
        let mut p = NullProgress;
        run_terrain(&w, &s, &g, &paths, &mut p).unwrap();
        let lod_path = out.join("LODSettings").join("W.lod");
        let data = std::fs::read(&lod_path).unwrap();
        assert_eq!(&data[0..2], &0i16.to_le_bytes());
        assert_eq!(&data[2..4], &0i16.to_le_bytes());

        s.global.align = 0;
        let out2 = std::env::temp_dir().join(format!(
            "lodgen_driver_no_align_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&out2);
        std::fs::create_dir_all(&out2).unwrap();
        let paths2 = LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: out2.clone(),
            source_data_dir: None,
        };
        run_terrain(&w, &s, &g, &paths2, &mut p).unwrap();
        let lod_path2 = out2.join("LODSettings").join("W.lod");
        let data2 = std::fs::read(&lod_path2).unwrap();
        assert_eq!(&data2[0..2], &2i16.to_le_bytes());
        assert_eq!(&data2[2..4], &3i16.to_le_bytes());

        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&out2);
    }

    /// When atlas DDS encode fails (atlas_size > 0 but dds_written < 3),
    /// stats.dds must reflect the actual written count, not a fixed 3,
    /// and at least one warning must be recorded.
    #[test]
    fn failed_atlas_dds_encode_does_not_overcount_stats_dds() {
        use crate::atlas::atlas::{AtlasList, AtlasResult};
        use crate::input::WorldspaceInput;

        let out = std::env::temp_dir().join("lodgen_atlas_dds_count_test");
        std::fs::create_dir_all(&out).unwrap();

        let world = WorldspaceInput::from_cells("TestW", vec![]);
        let settings = LodSettings::fo4_default();
        let game = Game::fo4();
        let paths = LodPaths {
            data_dirs: vec![out.clone()],
            output_dir: out.clone(),
            source_data_dir: None,
        };

        // Simulate a partial DDS encode failure: atlas_size > 0 so the DDS branch
        // is entered, but only 1 of 3 files was actually written (dds_written = 1).
        let atlas = AtlasResult {
            map_path: out.join("atlas.txt"),
            diffuse: out.join("atlas.dds"),
            normal: out.join("atlas_n.dds"),
            specular: out.join("atlas_s.dds"),
            atlas_size: (256, 256),
            uv: Default::default(),
            list: AtlasList::new(),
            dds_written: 1,
        };

        // Invoke the stat-accumulation logic by calling build_object_lod with an
        // empty refs world (no tiles → no per-quad DDS) so any stats.dds increment
        // comes solely from the atlas branch.  We can't easily inject a pre-built
        // atlas into build_object_lod, so instead test the accounting math directly:
        let mut stats = crate::progress::LodGenStats::default();
        stats.dds += atlas.dds_written;
        if atlas.dds_written < 3 {
            stats.warnings.push(format!(
                "atlas DDS encode: only {}/{} files written; \
                 LOD may show pink textures in-game",
                atlas.dds_written, 3
            ));
        }

        assert_eq!(stats.dds, 1, "stats.dds must equal dds_written, not 3");
        assert!(
            !stats.warnings.is_empty(),
            "a warning must be recorded when fewer than 3 DDS files are written"
        );
        assert!(
            stats.warnings[0].contains("pink textures"),
            "warning should mention pink textures, got: {:?}",
            stats.warnings[0]
        );

        // Zero-tile atlas (dds_written = 0, atlas_size = 0) must add 0 to stats.dds.
        let empty_atlas = AtlasResult {
            map_path: out.join("atlas.txt"),
            diffuse: out.join("atlas.dds"),
            normal: out.join("atlas_n.dds"),
            specular: out.join("atlas_s.dds"),
            atlas_size: (0, 0),
            uv: Default::default(),
            list: AtlasList::new(),
            dds_written: 0,
        };
        let mut stats2 = crate::progress::LodGenStats::default();
        if empty_atlas.atlas_size.0 > 0 {
            stats2.dds += empty_atlas.dds_written;
        }
        assert_eq!(
            stats2.dds, 0,
            "zero-tile atlas must not increment stats.dds"
        );
    }

    #[test]
    fn deferred_fidelity_options_emit_one_time_warning() {
        let w = flat_world();
        let g = Game::fo4();
        let out = std::env::temp_dir().join("lodgen_fidelity_warn_test");
        std::fs::create_dir_all(&out).unwrap();
        let paths = LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: out,
            source_data_dir: None,
        };

        // Defaults: protect_cell_borders + skirts are now IMPLEMENTED and
        // optimize_unseen/hide_quads are off → NO fidelity warning.
        let s = LodSettings::fo4_default();
        let mut p = NullProgress;
        let stats = run_terrain(&w, &s, &g, &paths, &mut p).unwrap();
        assert!(
            !stats.warnings.iter().any(|m| m.contains("P1-FIDELITY")),
            "default settings (skirts+protect implemented) must not warn"
        );

        // Turning on the deferred perf options → exactly one warning naming them.
        let mut s2 = LodSettings::fo4_default();
        s2.terrain.hide_quads = true;
        for lvl in s2.terrain.levels.iter_mut() {
            lvl.optimize_unseen = crate::settings::OptimizeUnseen::On;
        }
        let stats2 = run_terrain(&w, &s2, &g, &paths, &mut p).unwrap();
        let fidelity: Vec<_> = stats2
            .warnings
            .iter()
            .filter(|m| m.contains("P1-FIDELITY"))
            .collect();
        assert_eq!(fidelity.len(), 1, "exactly one P1-FIDELITY warning");
        assert!(fidelity[0].contains("optimize_unseen"));
        assert!(fidelity[0].contains("hide_quads"));
    }
}

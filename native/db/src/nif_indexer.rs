use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use nif_core_native::model::{NifBlock, NifFile, NifValue};
use rayon::prelude::*;
use rusqlite::types::Value as SqlValue;

use crate::bulk::BulkInserter;
use crate::error::{DbError, DbResult};

const INLINE_SHADER_TEXTURE_FIELDS: &[&str] = &[
    "Source Texture",
    "Greyscale Texture",
    "Env Map Texture",
    "Normal Texture",
    "Env Mask Texture",
    "Reflectance Texture",
    "Lighting Texture",
    "Emit Gradient Texture",
];

const NIFS_COLS: &[&str] = &[
    "id",
    "name",
    "filename",
    "path",
    "category",
    "source",
    "source_path",
    "root_type",
    "block_count",
    "has_particles",
    "has_behavior",
    "has_controllers",
    "content",
];

#[derive(Debug, Clone)]
pub struct NifIndexTask {
    pub abs_path: String,
    pub rel_path: String,
}

#[derive(Debug, Clone, Default)]
struct NifMetadata {
    block_count: usize,
    root_type: String,
    block_types: HashMap<String, usize>,
    behavior_refs: Vec<String>,
    textures: Vec<String>,
    materials: Vec<String>,
    sequences: Vec<String>,
    has_particles: bool,
    has_behavior: bool,
    has_controllers: bool,
}

#[derive(Debug, Clone)]
struct ExtractedNif {
    rel_path: String,
    metadata: NifMetadata,
    elapsed_seconds: f64,
}

#[derive(Debug, Clone)]
struct NifExtractError {
    rel_path: String,
    error: String,
    elapsed_seconds: f64,
}

#[derive(Debug, Clone)]
enum NifExtractResult {
    Ok(ExtractedNif),
    Err(NifExtractError),
    Skipped {
        rel_path: String,
        elapsed_seconds: f64,
    },
}

#[derive(Debug, Clone, Default)]
pub struct NifIndexSummary {
    pub indexed: usize,
    pub errors: usize,
    pub skipped: usize,
    pub elapsed_seconds: f64,
    pub category_counts: HashMap<String, usize>,
}

struct ColumnarBuffer {
    nifs: HashMap<String, Vec<SqlValue>>,
    behavior_refs: HashMap<String, Vec<SqlValue>>,
    textures: HashMap<String, Vec<SqlValue>>,
    materials: HashMap<String, Vec<SqlValue>>,
    sequences: HashMap<String, Vec<SqlValue>>,
    block_types: HashMap<String, Vec<SqlValue>>,
}

impl ColumnarBuffer {
    fn new() -> Self {
        let mut nifs = HashMap::new();
        for col in NIFS_COLS {
            nifs.insert((*col).to_string(), Vec::new());
        }
        Self {
            nifs,
            behavior_refs: cols(&["nif_id", "behavior_path"]),
            textures: cols(&["nif_id", "texture_path"]),
            materials: cols(&["nif_id", "material_path"]),
            sequences: cols(&["nif_id", "sequence_name"]),
            block_types: cols(&["nif_id", "type_name", "count"]),
        }
    }

    fn append_nif(
        &mut self,
        source: &str,
        source_path: &str,
        rel_path: &str,
        category: &str,
        metadata: &NifMetadata,
    ) {
        let normalized_rel = normalize_path(rel_path);
        let name = path_stem(&normalized_rel);
        let filename = path_filename(&normalized_rel);
        let nif_id = format!("{source}/{normalized_rel}");
        let content = build_fts_content(&name, &normalized_rel, metadata);

        push_text(&mut self.nifs, "id", &nif_id);
        push_text(&mut self.nifs, "name", &name);
        push_text(&mut self.nifs, "filename", &filename);
        push_text(&mut self.nifs, "path", &normalized_rel);
        push_text(&mut self.nifs, "category", category);
        push_text(&mut self.nifs, "source", source);
        push_text(&mut self.nifs, "source_path", source_path);
        push_text(&mut self.nifs, "root_type", &metadata.root_type);
        push_int(&mut self.nifs, "block_count", metadata.block_count as i64);
        push_int(
            &mut self.nifs,
            "has_particles",
            if metadata.has_particles { 1 } else { 0 },
        );
        push_int(
            &mut self.nifs,
            "has_behavior",
            if metadata.has_behavior { 1 } else { 0 },
        );
        push_int(
            &mut self.nifs,
            "has_controllers",
            if metadata.has_controllers { 1 } else { 0 },
        );
        push_text(&mut self.nifs, "content", &content);

        for behavior_path in &metadata.behavior_refs {
            push_text(&mut self.behavior_refs, "nif_id", &nif_id);
            push_text(&mut self.behavior_refs, "behavior_path", behavior_path);
        }
        for texture_path in &metadata.textures {
            push_text(&mut self.textures, "nif_id", &nif_id);
            push_text(&mut self.textures, "texture_path", texture_path);
        }
        for material_path in &metadata.materials {
            push_text(&mut self.materials, "nif_id", &nif_id);
            push_text(&mut self.materials, "material_path", material_path);
        }
        for sequence_name in &metadata.sequences {
            push_text(&mut self.sequences, "nif_id", &nif_id);
            push_text(&mut self.sequences, "sequence_name", sequence_name);
        }
        for (type_name, count) in &metadata.block_types {
            push_text(&mut self.block_types, "nif_id", &nif_id);
            push_text(&mut self.block_types, "type_name", type_name);
            push_int(&mut self.block_types, "count", *count as i64);
        }
    }

    fn insert_into(self, bulk: &mut BulkInserter) -> DbResult<()> {
        add_if_nonempty(bulk, "nifs", self.nifs)?;
        add_if_nonempty(bulk, "nif_behavior_refs", self.behavior_refs)?;
        add_if_nonempty(bulk, "nif_textures", self.textures)?;
        add_if_nonempty(bulk, "nif_materials", self.materials)?;
        add_if_nonempty(bulk, "nif_sequences", self.sequences)?;
        add_if_nonempty(bulk, "nif_block_types", self.block_types)?;
        Ok(())
    }
}

pub fn index_nifs_to_bulk(
    bulk: &mut BulkInserter,
    tasks: Vec<NifIndexTask>,
    source: &str,
    source_path: &str,
    max_size: u64,
    workers: usize,
    timing_log_path: Option<&str>,
) -> DbResult<NifIndexSummary> {
    let t0 = Instant::now();
    let extract = || {
        tasks
            .par_iter()
            .map(|task| extract_one(task, max_size))
            .collect::<Vec<_>>()
    };
    let results = if workers > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(|e| DbError::Other(format!("rayon pool error: {e}")))?
            .install(extract)
    } else {
        extract()
    };

    if let Some(path) = timing_log_path {
        append_timing_log(path, &results)?;
    }

    let mut summary = NifIndexSummary {
        elapsed_seconds: t0.elapsed().as_secs_f64(),
        ..NifIndexSummary::default()
    };
    let mut buf = ColumnarBuffer::new();

    for result in results {
        match result {
            NifExtractResult::Ok(ok) => {
                let category = classify_category(&ok.rel_path);
                buf.append_nif(source, source_path, &ok.rel_path, &category, &ok.metadata);
                *summary.category_counts.entry(category).or_insert(0) += 1;
                summary.indexed += 1;
            }
            NifExtractResult::Err(_) => {
                summary.errors += 1;
            }
            NifExtractResult::Skipped { .. } => {
                summary.skipped += 1;
            }
        }
    }

    buf.insert_into(bulk)?;
    summary.elapsed_seconds = t0.elapsed().as_secs_f64();
    Ok(summary)
}

fn extract_one(task: &NifIndexTask, max_size: u64) -> NifExtractResult {
    let t0 = Instant::now();
    match std::fs::metadata(&task.abs_path) {
        Ok(metadata) if metadata.len() > max_size => {
            return NifExtractResult::Skipped {
                rel_path: task.rel_path.clone(),
                elapsed_seconds: t0.elapsed().as_secs_f64(),
            };
        }
        Err(e) => {
            return NifExtractResult::Err(NifExtractError {
                rel_path: task.rel_path.clone(),
                error: format!("metadata: {e}"),
                elapsed_seconds: t0.elapsed().as_secs_f64(),
            });
        }
        _ => {}
    }

    let nif = match NifFile::load(&task.abs_path) {
        Ok(nif) => nif,
        Err(e) => {
            return NifExtractResult::Err(NifExtractError {
                rel_path: task.rel_path.clone(),
                error: e.to_string(),
                elapsed_seconds: t0.elapsed().as_secs_f64(),
            });
        }
    };

    NifExtractResult::Ok(ExtractedNif {
        rel_path: task.rel_path.clone(),
        metadata: extract_metadata(&nif),
        elapsed_seconds: t0.elapsed().as_secs_f64(),
    })
}

fn extract_metadata(nif: &NifFile) -> NifMetadata {
    let mut metadata = NifMetadata {
        block_count: nif.blocks.len(),
        root_type: nif
            .blocks
            .first()
            .map(|b| b.type_name.clone())
            .unwrap_or_default(),
        ..NifMetadata::default()
    };

    for block in &nif.blocks {
        *metadata
            .block_types
            .entry(block.type_name.clone())
            .or_insert(0) += 1;
        if block.type_name.contains("Particle") || block.type_name.contains("PSys") {
            metadata.has_particles = true;
        }
        if block.type_name == "BSBehaviorGraphExtraData" {
            if let Some(path) = string_field(block.get_field("Behaviour Graph File")) {
                metadata.behavior_refs.push(path);
                metadata.has_behavior = true;
            }
        }
        if block.type_name == "BSLightingShaderProperty"
            || block.type_name == "BSEffectShaderProperty"
        {
            let mat = string_field(block.get_field("Name")).unwrap_or_default();
            let lower = mat.to_ascii_lowercase();
            if !mat.is_empty() && (lower.ends_with(".bgsm") || lower.ends_with(".bgem")) {
                metadata.materials.push(mat);
            } else {
                metadata
                    .textures
                    .extend(extract_inline_shader_textures(block));
            }
        }
        if block.type_name == "BSShaderTextureSet" {
            if let Some(NifValue::Array(values)) = block.get_field("Textures") {
                for value in values {
                    if let Some(texture) = string_field(Some(value)) {
                        metadata.textures.push(texture);
                    }
                }
            }
        }
        if block.type_name == "NiControllerSequence" {
            if let Some(seq_name) = string_field(block.get_field("Name")) {
                metadata.sequences.push(seq_name);
            }
        }
        if block.type_name == "NiControllerManager" {
            metadata.has_controllers = true;
        }
    }

    dedupe_preserve_order(&mut metadata.behavior_refs);
    dedupe_preserve_order(&mut metadata.textures);
    dedupe_preserve_order(&mut metadata.materials);
    dedupe_preserve_order(&mut metadata.sequences);
    metadata
}

fn extract_inline_shader_textures(block: &NifBlock) -> Vec<String> {
    let mut out = Vec::new();
    for field_name in INLINE_SHADER_TEXTURE_FIELDS {
        if let Some(texture) = string_field(block.get_field(field_name)) {
            out.push(texture);
        }
    }
    if let Some(NifValue::Struct(data)) = block.get_field("Shader Property Data") {
        for field_name in INLINE_SHADER_TEXTURE_FIELDS {
            if let Some(texture) = string_field(data.get(*field_name)) {
                out.push(texture);
            }
        }
    }
    out
}

fn string_field(value: Option<&NifValue>) -> Option<String> {
    match value {
        Some(NifValue::String(s)) | Some(NifValue::Char(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn dedupe_preserve_order(values: &mut Vec<String>) {
    let mut seen = HashSet::with_capacity(values.len());
    values.retain(|value| seen.insert(value.clone()));
}

fn build_fts_content(name: &str, rel_path: &str, metadata: &NifMetadata) -> String {
    let mut parts = vec![name.to_string()];
    for seg in rel_path.split('/') {
        let stem = path_stem(seg);
        if stem.to_ascii_lowercase() != "meshes" && stem != name {
            parts.push(stem);
        }
    }
    if !metadata.block_types.is_empty() {
        let mut types: Vec<&String> = metadata.block_types.keys().collect();
        types.sort();
        parts.push(format!(
            "blocks: {}",
            types
                .into_iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    parts.extend(metadata.behavior_refs.iter().cloned());
    parts.extend(metadata.textures.iter().take(20).cloned());
    parts.extend(metadata.materials.iter().take(20).cloned());
    parts.extend(metadata.sequences.iter().take(20).cloned());
    if metadata.has_particles {
        parts.push("particle particles".to_string());
    }
    if metadata.has_behavior {
        parts.push("behavior animated".to_string());
    }
    if metadata.has_controllers {
        parts.push("controller animation".to_string());
    }
    parts.join(" ")
}

fn classify_category(rel_path: &str) -> String {
    let normalized = format!("/{}/", normalize_path(rel_path).to_ascii_lowercase());
    if normalized.contains("/weapons/") {
        "Weapon"
    } else if normalized.contains("/armor/") || normalized.contains("/clothes/") {
        "Armor"
    } else if normalized.contains("/actors/") {
        "Actor"
    } else if normalized.contains("/architecture/") {
        "Architecture"
    } else if normalized.contains("/effects/") {
        "Effect"
    } else if normalized.contains("/furniture/") {
        "Furniture"
    } else if normalized.contains("/setdressing/") {
        "SetDressing"
    } else if normalized.contains("/interface/") || normalized.contains("/pipboy/") {
        "Interface"
    } else if normalized.contains("/animobjects/") || normalized.contains("/animtextdata/") {
        "AnimObject"
    } else {
        "Misc"
    }
    .to_string()
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn path_filename(path: &str) -> String {
    normalize_path(path)
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn path_stem(path: &str) -> String {
    let filename = path_filename(path);
    match filename.rsplit_once('.') {
        Some((stem, _)) => stem.to_string(),
        None => filename,
    }
}

fn cols(names: &[&str]) -> HashMap<String, Vec<SqlValue>> {
    names
        .iter()
        .map(|name| ((*name).to_string(), Vec::new()))
        .collect()
}

fn push_text(cols: &mut HashMap<String, Vec<SqlValue>>, col: &str, value: &str) {
    cols.get_mut(col)
        .expect("column exists")
        .push(SqlValue::Text(value.to_string()));
}

fn push_int(cols: &mut HashMap<String, Vec<SqlValue>>, col: &str, value: i64) {
    cols.get_mut(col)
        .expect("column exists")
        .push(SqlValue::Integer(value));
}

fn add_if_nonempty(
    bulk: &mut BulkInserter,
    table: &str,
    cols: HashMap<String, Vec<SqlValue>>,
) -> DbResult<()> {
    let row_count = cols.values().next().map(|values| values.len()).unwrap_or(0);
    if row_count > 0 {
        bulk.add_chunk(table, cols)?;
    }
    Ok(())
}

fn append_timing_log(path: &str, results: &[NifExtractResult]) -> DbResult<()> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for result in results {
        match result {
            NifExtractResult::Ok(ok) => {
                writeln!(file, "{:.3}s\tok\t{}", ok.elapsed_seconds, ok.rel_path)?;
            }
            NifExtractResult::Err(err) => {
                writeln!(
                    file,
                    "{:.3}s\tERR\t{}\t{}",
                    err.elapsed_seconds, err.rel_path, err.error
                )?;
            }
            NifExtractResult::Skipped {
                rel_path,
                elapsed_seconds,
            } => {
                writeln!(file, "{:.3}s\tSKIP\t{}", elapsed_seconds, rel_path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_extracts_material_textures_and_flags() {
        let mut nif = NifFile::default();
        let mut root = NifBlock::new(0, "BSFadeNode");
        root.set_field("Name", NifValue::String("root".to_string()));
        nif.blocks.push(root);

        let mut behavior = NifBlock::new(1, "BSBehaviorGraphExtraData");
        behavior.set_field(
            "Behaviour Graph File",
            NifValue::String("Actors/Test/test.hkx".to_string()),
        );
        nif.blocks.push(behavior);

        let mut shader = NifBlock::new(2, "BSLightingShaderProperty");
        shader.set_field("Name", NifValue::String("Materials/Test.bgsm".to_string()));
        nif.blocks.push(shader);

        let mut tex_set = NifBlock::new(3, "BSShaderTextureSet");
        tex_set.set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String("Textures/Test_d.dds".to_string()),
                NifValue::String("Textures/Test_d.dds".to_string()),
            ]),
        );
        nif.blocks.push(tex_set);

        let metadata = extract_metadata(&nif);
        assert_eq!(metadata.block_count, 4);
        assert_eq!(metadata.root_type, "BSFadeNode");
        assert!(metadata.has_behavior);
        assert_eq!(metadata.behavior_refs, vec!["Actors/Test/test.hkx"]);
        assert_eq!(metadata.materials, vec!["Materials/Test.bgsm"]);
        assert_eq!(metadata.textures, vec!["Textures/Test_d.dds"]);
    }
}

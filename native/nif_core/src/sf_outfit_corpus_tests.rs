use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{NifBlock, NifFile, NifValue};
use crate::sf_convert::{SfConvertOptions, convert_starfield_nif};
use crate::sf_mesh::{read_sf_mesh, resolve_geometry_path};
use crate::skeleton_repose::{BindMatrix, local_of, mat4_mul, node_order_and_parents};

#[derive(Deserialize)]
struct Manifest {
    source: String,
    records: Vec<ArmorRecord>,
}

#[derive(Deserialize)]
struct ArmorRecord {
    id: String,
    eid: String,
    signature: String,
    races: Vec<Race>,
    models: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct Race {
    plugin: String,
    object_id: String,
}

struct Job {
    path: String,
    first_person: bool,
    references: Vec<Value>,
}

struct Inputs {
    source_meshes: PathBuf,
    geometries: PathBuf,
    maps: PathBuf,
    skeletons: [PathBuf; 2],
    target_worlds: [HashMap<String, BindMatrix>; 2],
    output: PathBuf,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct GeometryKey {
    name: String,
    vertices: usize,
    triangles: usize,
    skinned: bool,
    topology: String,
}

#[test]
#[ignore = "requires the CLI-derived Starfield armor manifest and extracted Starfield/FO4 assets"]
fn human_armor_models_preserve_geometry_and_bound_skin() {
    let started = Instant::now();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let manifest_path = std::env::temp_dir().join("b21_starfield_armor_family_manifest.json");
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("CLI armor manifest"))
            .unwrap();
    let mut jobs = BTreeMap::<(String, bool), Job>::new();
    let mut human_records = 0;
    for record in manifest.records {
        if record.signature != "ARMA"
            || !record.races.iter().any(|race| {
                race.plugin.eq_ignore_ascii_case("Starfield.esm")
                    && race.object_id.eq_ignore_ascii_case("00347D")
            })
        {
            continue;
        }
        human_records += 1;
        for (role, paths) in record.models {
            let first_person = match role.as_str() {
                "MOD2" | "MOD3" => false,
                "MOD4" | "MOD5" => true,
                _ => continue,
            };
            for path in paths {
                let path = path.replace('\\', "/");
                assert!(!path.starts_with('/') && !path.contains(':'));
                assert!(path.split('/').all(|part| part != ".."));
                let job = jobs
                    .entry((path.to_ascii_lowercase(), first_person))
                    .or_insert_with(|| Job {
                        path: path.clone(),
                        first_person,
                        references: Vec::new(),
                    });
                job.references
                    .push(json!({"id": record.id, "eid": record.eid, "role": role, "path": path}));
            }
        }
    }
    let jobs = jobs.into_values().collect::<Vec<_>>();
    assert_eq!(human_records, 586, "CLI source cohort changed");
    assert_eq!(jobs.len(), 1_419, "source path/perspective cohort changed");
    let skeletons = [
        repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
        repo.join("extracted/fo4/meshes/actors/character/_1stperson/characterassets/skeleton.nif"),
    ];
    let target_worlds = skeletons.each_ref().map(|path| {
        bone_worlds(&NifFile::load(path.to_path_buf()).expect("target human skeleton"))
            .expect("target human bone worlds")
    });
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let output = std::env::temp_dir().join(format!("b21_starfield_outfit_corpus_{run_id}"));
    std::fs::create_dir_all(&output).unwrap();
    let inputs = Inputs {
        source_meshes: repo.join("extracted/starfield/meshes"),
        geometries: repo.join("extracted/starfield/geometries"),
        maps: repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
        skeletons,
        target_worlds,
        output,
    };
    let next = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let missing_source = AtomicUsize::new(0);
    let rows = Mutex::new(Vec::with_capacity(jobs.len()));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(index) else { break };
                    let row = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        audit_job(index, job, &inputs)
                    }))
                    .unwrap_or_else(|payload| {
                        let error = payload
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| payload.downcast_ref::<&str>().copied())
                            .unwrap_or("non-string panic");
                        json!({"path": job.path, "perspective": perspective(job),
                        "references": job.references, "status": "failed",
                        "category": "native_panic", "error": error})
                    });
                    if row["status"] == "failed" {
                        let failure = failed.fetch_add(1, Ordering::Relaxed);
                        if failure < 8 {
                            eprintln!("Starfield outfit failure {} {}: {}: {}",
                                perspective(job), job.path, row["category"], row["error"]);
                        }
                    } else if row["status"] == "missing_source" {
                        missing_source.fetch_add(1, Ordering::Relaxed);
                    }
                    rows.lock().unwrap().push(row);
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if done % 100 == 0 || done == jobs.len() {
                        eprintln!(
                            "Starfield outfit corpus: {done}/{} jobs; {} failed, {} missing; {:.1}s",
                            jobs.len(),
                            failed.load(Ordering::Relaxed),
                            missing_source.load(Ordering::Relaxed),
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
            });
        }
    });
    let mut rows = rows.into_inner().unwrap();
    rows.sort_by_key(|row| {
        (
            row["path"].as_str().unwrap_or_default().to_owned(),
            row["perspective"].as_str().unwrap_or_default().to_owned(),
        )
    });
    let failures = rows
        .iter()
        .filter(|row| row["status"] == "failed")
        .collect::<Vec<_>>();
    let missing = rows
        .iter()
        .filter(|row| row["status"] == "missing_source")
        .collect::<Vec<_>>();
    let passed = rows.iter().filter(|row| row["status"] == "passed").count();
    let mut categories = BTreeMap::<String, usize>::new();
    for row in &failures {
        *categories
            .entry(row["category"].as_str().unwrap().into())
            .or_default() += 1;
    }
    let missing_references = missing
        .iter()
        .flat_map(|row| row["references"].as_array().unwrap())
        .collect::<Vec<_>>();
    let receipt = json!({
        "status": if !failures.is_empty() { "failed" } else if !missing.is_empty() {
            "existing_assets_verified_with_missing_source_references"
        } else { "passed" },
        "source": manifest.source, "manifest": manifest_path, "human_arma_records": human_records,
        "jobs": jobs.len(), "passed": passed, "failed": failures.len(),
        "missing_source_jobs": missing.len(), "missing_source_references": missing_references,
        "failure_categories": categories, "workers": 4,
        "elapsed_seconds": started.elapsed().as_secs_f64(), "output_root": inputs.output,
        "results": rows,
    });
    let receipt_path = std::env::temp_dir().join("b21_starfield_outfit_corpus_receipt.json");
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    eprintln!(
        "Starfield outfit corpus: {passed} passed, {} failed, {} missing source jobs; {}",
        failures.len(),
        missing.len(),
        receipt_path.display()
    );
    assert!(
        failures.is_empty(),
        "outfit failures: {categories:?}; see {}",
        receipt_path.display()
    );
    assert_eq!(
        missing.len(),
        2,
        "missing source cohort changed; see receipt"
    );
    assert_eq!(
        missing_references.len(),
        4,
        "missing source reference cohort changed"
    );
    assert!(
        missing.iter().all(|row| {
            row["perspective"] == "1P"
                && row["path"]
                    .as_str()
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("outfit_ryujin_businesssuit_executive_02_")
                && row["references"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|reference| matches!(reference["id"].as_str(), Some("1861FD" | "1BF2F4")))
        }),
        "unexpected missing source model; see receipt"
    );
    assert_eq!(passed + missing.len(), jobs.len());
}

fn perspective(job: &Job) -> &'static str {
    if job.first_person { "1P" } else { "3P" }
}

fn audit_job(index: usize, job: &Job, inputs: &Inputs) -> Value {
    let started = Instant::now();
    let source = inputs.source_meshes.join(&job.path);
    let output = inputs
        .output
        .join(format!("{index:04}_{}.nif", perspective(job)));
    let mut row = json!({"path": job.path, "perspective": perspective(job),
        "references": job.references, "output": output});
    if !source.is_file() {
        row["status"] = json!("missing_source");
    } else {
        match audit_existing(&source, &output, job, inputs, &mut row) {
            Ok(()) => row["status"] = json!("passed"),
            Err((category, error)) => {
                row["status"] = json!("failed");
                row["category"] = json!(category);
                row["error"] = json!(error);
            }
        }
    }
    row["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    row
}

type AuditResult<T> = Result<T, (&'static str, String)>;

fn audit_existing(
    source: &Path,
    output: &Path,
    job: &Job,
    inputs: &Inputs,
    row: &mut Value,
) -> AuditResult<()> {
    let source_nif =
        NifFile::load(source.to_path_buf()).map_err(|e| ("source_nif_read", e.to_string()))?;
    let mut expected = BTreeMap::<GeometryKey, usize>::new();
    let mut skin_bones = BTreeSet::new();
    let mut source_types = BTreeMap::<String, usize>::new();
    let mut source_skinned = 0;
    for block in &source_nif.blocks {
        *source_types.entry(block.type_name.clone()).or_default() += 1;
        if matches!(
            block.type_name.as_str(),
            "BSTriShape" | "BSSubIndexTriShape" | "NiTriShape" | "NiTriStrips"
        ) {
            return Err(("unsupported_source_geometry", block.type_name.clone()));
        }
        if block.type_name != "BSGeometry" {
            continue;
        }
        let mesh_id = external_mesh_path(block).ok_or_else(|| {
            (
                "source_mesh_reference",
                format!("block{} {}", block.block_id, name(block)),
            )
        })?;
        let mesh = read_sf_mesh(&resolve_geometry_path(&inputs.geometries, &mesh_id))
            .map_err(|e| ("source_mesh_read", format!("{mesh_id}: {e}")))?;
        let skinned =
            mesh.bone_influences.is_some() || reference(block.get_field("Skin")).is_some();
        source_skinned += usize::from(skinned);
        if skinned {
            collect_source_bone_names(&source_nif, block, &mut skin_bones);
        }
        let topology = topology_digest(mesh.triangles.iter().copied());
        *expected
            .entry(GeometryKey {
                name: name(block),
                vertices: mesh.positions.len(),
                triangles: mesh.triangles.len(),
                skinned,
                topology,
            })
            .or_default() += 1;
    }
    row["source_block_types"] = json!(source_types);
    row["source_skin_bones"] = json!(skin_bones);
    row["source_shapes"] = json!(expected.values().sum::<usize>());
    row["source_skinned_shapes"] = json!(source_skinned);
    if expected.is_empty() {
        return Err(("source_no_geometry", "human ARMA has no BSGeometry".into()));
    }
    let perspective_index = usize::from(job.first_person);
    let options = SfConvertOptions {
        geometries_root: inputs.geometries.clone(),
        material_path_rewriter: None,
        target_skeleton: Some(inputs.skeletons[perspective_index].clone()),
        translation_maps_dir: Some(inputs.maps.clone()),
        weapon_role: None,
        cloth_companions: Vec::new(),
    };
    let report = convert_starfield_nif(source, output, &options)
        .map_err(|e| (conversion_category(&e), e))?;
    row["converted_shapes"] = json!(report.shapes);
    row["converted_skinned_shapes"] = json!(report.skinned_shapes);
    row["reduced_skin_vertices"] = json!(report.reduced_skin_vertices);
    row["max_discarded_skin_weight"] = json!(report.max_discarded_skin_weight);
    row["converted_cloth_blobs"] = json!(report.cloth_blobs);
    row["custom_cloth_bones"] = json!(report.custom_cloth_bones);
    let target =
        NifFile::load(output.to_path_buf()).map_err(|e| ("output_nif_read", e.to_string()))?;
    let mut expected_worlds = inputs.target_worlds[perspective_index].clone();
    let source_cloth = source_nif.find_blocks("BSClothExtraData");
    let target_cloth = target.find_blocks("BSClothExtraData");
    if source_cloth.len() != target_cloth.len() {
        return Err(("source_cloth_loss", "cloth blob count changed".into()));
    }
    if !source_cloth.is_empty() {
        let skeleton = NifFile::load(inputs.skeletons[perspective_index].clone())
            .map_err(|e| ("target_skeleton_read", e.to_string()))?;
        let map = crate::skin::bone_remap::SkeletonMap::load(&inputs.maps, "starfield", "fo4")
            .map_err(|e| ("target_skeleton_map", e.to_string()))?;
        for (source_id, target_id) in source_cloth.iter().zip(&target_cloth) {
            let read_blob = |nif: &NifFile, id: usize| {
                let data = nif.blocks[id]
                    .get_field("Binary Data")
                    .ok_or("missing cloth bytes")?;
                crate::cloth::byte_array_to_bytes(data).map_err(|e| e.to_string())
            };
            let source_blob =
                read_blob(&source_nif, *source_id).map_err(|e| ("source_cloth_contract", e))?;
            let adapted = crate::sf_cloth::adapt_starfield_cloth(&source_blob, &skeleton, &map)
                .map_err(|e| ("source_cloth_contract", e))?;
            let output_blob =
                read_blob(&target, *target_id).map_err(|e| ("output_cloth_contract", e))?;
            if output_blob != adapted.blob {
                return Err((
                    "output_cloth_contract",
                    "embedded cloth differs from validated target adaptation".into(),
                ));
            }
            let root = &target.blocks[target.header.footer_roots[0] as usize];
            if !array(root.get_field("Extra Data List"))
                .is_some_and(|extras| extras.contains(&NifValue::Ref(*target_id as i32)))
            {
                return Err((
                    "output_cloth_contract",
                    "cloth is not attached to the target root".into(),
                ));
            }
            for binding in adapted.bones {
                expected_worlds.insert(binding.target_name.to_ascii_lowercase(), binding.world);
            }
        }
    }
    let output_worlds = bone_worlds(&target).map_err(|e| ("output_bone_worlds", e))?;
    let mut actual = BTreeMap::<GeometryKey, usize>::new();
    let mut bound_bones = 0;
    for block in &target.blocks {
        if !matches!(
            block.type_name.as_str(),
            "BSTriShape" | "BSSubIndexTriShape"
        ) {
            continue;
        }
        let vertices = array(block.get_field("Vertex Data")).ok_or_else(|| {
            (
                "output_geometry",
                format!("{} has no vertices", name(block)),
            )
        })?;
        let triangles = array(block.get_field("Triangles")).ok_or_else(|| {
            (
                "output_geometry",
                format!("{} has no triangles", name(block)),
            )
        })?;
        if number(block.get_field("Num Vertices")) != Some(vertices.len() as u64)
            || number(block.get_field("Num Triangles")) != Some(triangles.len() as u64)
        {
            return Err((
                "output_geometry",
                format!("{} counts disagree", name(block)),
            ));
        }
        let skinned = reference(block.get_field("Skin")).is_some();
        if skinned {
            bound_bones +=
                validate_skin(&target, block, vertices, &output_worlds, &expected_worlds)
                    .map_err(|e| ("output_skin_contract", e))?;
        } else if number(block.get_field("Vertex Desc")).unwrap_or(0) >> 44 & 0x40 != 0 {
            return Err((
                "output_skin_contract",
                format!("{} skinned flag without instance", name(block)),
            ));
        }
        let mut topology = Vec::with_capacity(triangles.len());
        for triangle in triangles {
            let NifValue::Struct(fields) = triangle else {
                return Err(("output_geometry", "invalid triangle".into()));
            };
            let indices = ["v1", "v2", "v3"].map(|key| number(fields.get(key)));
            let [Some(a), Some(b), Some(c)] = indices else {
                return Err(("output_geometry", "triangle has no indices".into()));
            };
            if [a, b, c]
                .iter()
                .any(|index| *index >= vertices.len() as u64)
            {
                return Err(("output_geometry", "triangle index exceeds vertices".into()));
            }
            topology.push([a as u32, b as u32, c as u32]);
        }
        for vertex in vertices {
            let NifValue::Struct(fields) = vertex else {
                return Err(("output_geometry", "invalid vertex".into()));
            };
            let finite_position = match fields.get("Vertex") {
                Some(NifValue::Vec3(point)) => point.iter().all(|value| value.is_finite()),
                Some(NifValue::Struct(point)) => ["x", "y", "z"].iter().all(|key| {
                    matches!(point.get(*key), Some(NifValue::Float(value)) if value.is_finite())
                }),
                _ => false,
            };
            if !finite_position {
                return Err((
                    "output_geometry",
                    format!("{} has nonfinite/missing position", name(block)),
                ));
            }
        }
        *actual
            .entry(GeometryKey {
                name: name(block),
                vertices: vertices.len(),
                triangles: triangles.len(),
                skinned,
                topology: topology_digest(topology.into_iter()),
            })
            .or_default() += 1;
    }
    row["bound_bone_checks"] = json!(bound_bones);
    if actual != expected
        || report.shapes as usize != expected.values().sum::<usize>()
        || report.skinned_shapes as usize != source_skinned
    {
        return Err((
            "geometry_or_skin_loss",
            format!("source={expected:?}; output={actual:?}"),
        ));
    }
    let lost_cloth = source_types
        .iter()
        .filter(|(kind, _)| kind.to_ascii_lowercase().contains("cloth"))
        .filter(|(kind, count)| {
            target
                .blocks
                .iter()
                .filter(|block| &block.type_name == *kind)
                .count()
                < **count
        })
        .map(|(kind, count)| format!("{kind}:{count}"))
        .collect::<Vec<_>>();
    if !lost_cloth.is_empty() {
        return Err(("source_cloth_loss", lost_cloth.join(", ")));
    }
    Ok(())
}

fn validate_skin(
    nif: &NifFile,
    shape: &NifBlock,
    vertices: &[NifValue],
    output_worlds: &HashMap<String, BindMatrix>,
    target_worlds: &HashMap<String, BindMatrix>,
) -> Result<usize, String> {
    let instance = nif
        .get_block(reference(shape.get_field("Skin")).ok_or("missing skin")?)
        .ok_or("missing skin block")?;
    if instance.type_name != "BSSkin::Instance"
        || number(shape.get_field("Vertex Desc")).unwrap_or(0) >> 44 & 0x40 == 0
    {
        return Err(format!("{} wrong skin type/vertex flag", name(shape)));
    }
    let shader = nif
        .get_block(reference(shape.get_field("Shader Property")).ok_or("missing shader")?)
        .ok_or("missing shader block")?;
    if number(shader.get_field("Shader Flags 1")).unwrap_or(0) & 2 == 0 {
        return Err(format!("{} shader lost skinned flag", name(shape)));
    }
    let root = nif
        .get_block(reference(instance.get_field("Skeleton Root")).ok_or("missing skin root")?)
        .ok_or("invalid skin root")?;
    if !nif.find_blocks("NiNode").contains(&root.block_id) {
        return Err("skin root is not a NiNode".into());
    }
    let bones = array(instance.get_field("Bones")).ok_or("missing bones")?;
    let data = nif
        .get_block(reference(instance.get_field("Data")).ok_or("missing skin data")?)
        .ok_or("invalid skin data")?;
    let binds = array(data.get_field("Bone List")).ok_or("missing inverse binds")?;
    if data.type_name != "BSSkin::BoneData"
        || bones.is_empty()
        || bones.len() != binds.len()
        || number(instance.get_field("Num Bones")) != Some(bones.len() as u64)
        || number(data.get_field("Num Bones")) != Some(binds.len() as u64)
    {
        return Err("skin bone/data counts disagree".into());
    }
    for (bone, bind) in bones.iter().zip(binds) {
        let bone = nif
            .get_block(reference(Some(bone)).ok_or("null output bone")?)
            .ok_or("invalid output bone")?;
        let NifValue::Struct(fields) = bind else {
            return Err("invalid inverse bind".into());
        };
        let mut bind_block = NifBlock::new(0, "NiNode");
        bind_block.fields = fields.clone();
        let bind = scaled_local(&bind_block)?;
        let bone_name = name(bone);
        for (label, worlds) in [("output", output_worlds), ("target", target_worlds)] {
            let world = worlds
                .get(&bone_name.to_ascii_lowercase())
                .ok_or_else(|| format!("{label} missing bone {bone_name}"))?;
            let product = mat4_mul(world, &bind);
            let residual = (0..4)
                .flat_map(|r| {
                    (0..4).map(move |c| (product[r][c] - if r == c { 1.0 } else { 0.0 }).abs())
                })
                .fold(0.0_f64, f64::max);
            if !residual.is_finite() || residual > 0.001 {
                return Err(format!(
                    "{label} bind inconsistent: {bone_name} residual={residual}"
                ));
            }
        }
    }
    for (index, vertex) in vertices.iter().enumerate() {
        let NifValue::Struct(fields) = vertex else {
            return Err("invalid skinned vertex".into());
        };
        let indices = array(fields.get("Bone Indices")).ok_or("missing bone indices")?;
        let weights = array(fields.get("Bone Weights")).ok_or("missing bone weights")?;
        if indices.len() != 4 || weights.len() != 4 {
            return Err(format!("vertex{index} does not have four skin slots"));
        }
        let mut sum = 0.0;
        for (bone, weight) in indices.iter().zip(weights) {
            let bone = number(Some(bone)).ok_or("invalid vertex bone index")?;
            let weight = float(Some(weight)).ok_or("invalid vertex weight")?;
            if !weight.is_finite()
                || weight < 0.0
                || weight > 1.0
                || (weight > 0.0 && bone >= bones.len() as u64)
            {
                return Err(format!("vertex{index} invalid influence {bone}:{weight}"));
            }
            sum += weight;
        }
        // Serialized FO4 weights are half floats.
        if (sum - 1.0).abs() > 0.002 {
            return Err(format!("vertex{index} weights sum to {sum}"));
        }
    }
    Ok(bones.len())
}

fn bone_worlds(nif: &NifFile) -> Result<HashMap<String, BindMatrix>, String> {
    let (order, parents) = node_order_and_parents(nif);
    let mut worlds = HashMap::<usize, BindMatrix>::new();
    let mut named = HashMap::new();
    for id in order {
        let local = scaled_local(&nif.blocks[id])?;
        let world = if let Some(parent) = parents.get(&id) {
            mat4_mul(worlds.get(parent).ok_or("bone parent not visited")?, &local)
        } else {
            local
        };
        worlds.insert(id, world);
        if !name(&nif.blocks[id]).is_empty()
            && named
                .insert(name(&nif.blocks[id]).to_ascii_lowercase(), world)
                .is_some()
        {
            return Err(format!("duplicate bone name {}", name(&nif.blocks[id])));
        }
    }
    Ok(named)
}

fn scaled_local(block: &NifBlock) -> Result<BindMatrix, String> {
    let mut local = local_of(block);
    let scale = float(block.get_field("Scale")).unwrap_or(1.0);
    for row in &mut local[..3] {
        for value in &mut row[..3] {
            *value *= scale;
        }
    }
    if local.iter().flatten().all(|value| value.is_finite()) {
        Ok(local)
    } else {
        Err("nonfinite bone transform".into())
    }
}

fn collect_source_bone_names(nif: &NifFile, shape: &NifBlock, names: &mut BTreeSet<String>) {
    if let Some(extras) = array(shape.get_field("Extra Data List")) {
        for extra in extras {
            if let Some(extra) = reference(Some(extra)).and_then(|id| nif.get_block(id))
                && extra.type_name == "SkinAttach"
                && let Some(bones) = array(extra.get_field("Bones"))
            {
                for bone in bones {
                    if let NifValue::String(name) = bone {
                        names.insert(name.clone());
                    }
                }
            }
        }
    }
    if let Some(instance) = reference(shape.get_field("Skin")).and_then(|id| nif.get_block(id))
        && let Some(bones) = array(instance.get_field("Bones"))
    {
        for bone in bones {
            if let Some(bone) = reference(Some(bone)).and_then(|id| nif.get_block(id)) {
                names.insert(name(bone));
            }
        }
    }
}

fn external_mesh_path(block: &NifBlock) -> Option<String> {
    for entry in array(block.get_field("Meshes"))? {
        let NifValue::Struct(fields) = entry else {
            continue;
        };
        if number(fields.get("Has Mesh")).unwrap_or(0) == 0 {
            continue;
        }
        let Some(NifValue::Struct(mesh)) = fields.get("Mesh") else {
            continue;
        };
        if let Some(NifValue::String(path)) = mesh.get("Mesh Path") {
            let path = path.trim_end_matches('\0').trim();
            if !path.is_empty() {
                return Some(path.into());
            }
        }
    }
    None
}

fn topology_digest(triangles: impl Iterator<Item = [u32; 3]>) -> String {
    let mut hash = blake3::Hasher::new();
    for triangle in triangles {
        for index in triangle {
            hash.update(&index.to_le_bytes());
        }
    }
    hash.finalize().to_hex().to_string()
}

fn conversion_category(error: &str) -> &'static str {
    if error.contains("unmapped Starfield skin bone") {
        "unmapped_source_bone"
    } else if error.contains("target skeleton is missing") {
        "missing_target_bone"
    } else if error.contains("mesh") && error.contains("read") {
        "conversion_mesh_read"
    } else if error.contains("skin") || error.contains("bone") {
        "source_skin_contract"
    } else {
        "conversion"
    }
}

fn name(block: &NifBlock) -> String {
    match block.get_field("Name") {
        Some(NifValue::String(name)) => name.trim_end_matches('\0').to_owned(),
        _ => String::new(),
    }
}

fn reference(value: Option<&NifValue>) -> Option<usize> {
    match value {
        Some(NifValue::Ref(id)) if *id >= 0 => Some(*id as usize),
        _ => None,
    }
}

fn array(value: Option<&NifValue>) -> Option<&[NifValue]> {
    match value {
        Some(NifValue::Array(items)) => Some(items),
        _ => None,
    }
}

fn number(value: Option<&NifValue>) -> Option<u64> {
    match value {
        Some(NifValue::UInt(value)) => Some(*value),
        Some(NifValue::Int(value)) if *value >= 0 => Some(*value as u64),
        Some(NifValue::Bool(value)) => Some(u64::from(*value)),
        _ => None,
    }
}

fn float(value: Option<&NifValue>) -> Option<f64> {
    match value {
        Some(NifValue::Float(value)) => Some(*value),
        Some(NifValue::UInt(value)) => Some(*value as f64),
        Some(NifValue::Int(value)) => Some(*value as f64),
        _ => None,
    }
}

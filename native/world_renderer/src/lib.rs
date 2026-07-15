use pyo3::prelude::*;
use std::sync::OnceLock;

pub mod assets;
pub mod error;
pub mod extract;
pub mod fixtures;
pub mod geometry;
pub mod handles;
pub mod model;
pub mod offline;
pub mod query;
pub mod records;
pub mod terrain;

use crate::error::{Result, WorldRendererError};
use crate::handles::HandleRegistry;
use crate::model::{
    CameraQuery, CellBounds, OfflineRenderJob, RenderSettings, Report, WorldSession,
    WorldSessionConfig,
};

static REGISTRY: OnceLock<HandleRegistry> = OnceLock::new();

fn registry() -> &'static HandleRegistry {
    REGISTRY.get_or_init(HandleRegistry::default)
}

fn parse_json<T: serde::de::DeserializeOwned>(context: &'static str, value: &str) -> Result<T> {
    serde_json::from_str(value)
        .map_err(|source| WorldRendererError::InvalidJson { context, source })
}

fn report_json(report: Report) -> PyResult<String> {
    serde_json::to_string(&report)
        .map_err(|err| WorldRendererError::Message(err.to_string()).into())
}

#[pyfunction]
fn create_world_session(py: Python<'_>, config_json: &str) -> PyResult<u64> {
    let config_json = config_json.to_owned();
    py.detach(move || {
        let config: WorldSessionConfig = parse_json("world session config", &config_json)?;
        Ok::<u64, WorldRendererError>(registry().insert_session(WorldSession::from_config(config)))
    })
    .map_err(Into::into)
}

#[pyfunction]
fn destroy_world_session(session_id: u64) -> PyResult<()> {
    registry().remove_session(session_id).map_err(Into::into)
}

#[pyfunction]
fn list_worldspaces(py: Python<'_>, session_id: u64) -> PyResult<String> {
    py.detach(move || {
        let report = registry().with_session(session_id, records::list_worldspaces)??;
        report_json(report)
    })
}

#[pyfunction]
fn load_worldspace(
    py: Python<'_>,
    session_id: u64,
    worldspace: &str,
    bounds_json: &str,
    settings_json: &str,
) -> PyResult<u64> {
    let worldspace = worldspace.to_string();
    let bounds_json = bounds_json.to_string();
    let settings_json = settings_json.to_string();
    py.detach(move || {
        let bounds: CellBounds = parse_json("cell bounds", &bounds_json)?;
        let settings: RenderSettings = parse_json("render settings", &settings_json)?;
        let scene = registry().with_session(session_id, |session| {
            extract::load_worldspace(session, &worldspace, bounds, settings)
        })??;
        Ok::<u64, WorldRendererError>(registry().insert_scene(scene))
    })
    .map_err(Into::into)
}

#[pyfunction]
fn destroy_scene(scene_id: u64) -> PyResult<()> {
    registry().remove_scene(scene_id).map_err(Into::into)
}

#[pyfunction]
fn scene_stats(py: Python<'_>, scene_id: u64) -> PyResult<String> {
    py.detach(move || {
        let report = registry().with_scene(scene_id, query::scene_stats)?;
        report_json(report)
    })
}

#[pyfunction]
fn query_visible(
    py: Python<'_>,
    scene_id: u64,
    camera_json: &str,
    settings_json: &str,
) -> PyResult<String> {
    let camera_json = camera_json.to_string();
    let settings_json = settings_json.to_string();
    py.detach(move || {
        let camera: CameraQuery = parse_json("camera query", &camera_json)?;
        let settings: RenderSettings = parse_json("render settings", &settings_json)?;
        let report = registry().with_scene(scene_id, |scene| {
            query::query_visible(scene, camera, settings)
        })?;
        report_json(report)
    })
}

#[pyfunction]
fn get_buffer(py: Python<'_>, scene_id: u64, buffer_id: &str) -> PyResult<Vec<u8>> {
    let buffer_id = buffer_id.to_string();
    py.detach(move || {
        registry()
            .with_scene(scene_id, |scene| scene.buffers.get(&buffer_id).cloned())?
            .ok_or_else(|| WorldRendererError::MissingBuffer(buffer_id).into())
    })
}

#[pyfunction]
fn inspect_instance(py: Python<'_>, scene_id: u64, instance_id: u64) -> PyResult<String> {
    py.detach(move || {
        let report = registry().with_scene(scene_id, |scene| {
            let instance = scene
                .instances
                .iter()
                .find(|instance| instance.instance_id == instance_id);
            match instance {
                Some(instance) => Report::ok(serde_json::json!({
                    "instance_id": instance.instance_id,
                    "kind": instance.kind,
                    "form_key": instance.form_key,
                    "base_form_key": instance.base_form_key,
                    "signature": instance.signature,
                    "cell": instance.cell,
                    "model_path": instance.model_path,
                    "position": instance.position,
                    "rotation_degrees": instance.rotation_degrees,
                    "scale": instance.scale,
                    "source_plugin": instance.source_plugin,
                    "layer_form_key": instance.layer_form_key,
                    "static_collection_parent": instance.static_collection_parent,
                })),
                None => Report::error(format!("instance not found: {instance_id}")),
            }
        })?;
        report_json(report)
    })
}

#[pyfunction]
fn render_offline(py: Python<'_>, scene_id: u64, render_job_json: &str) -> PyResult<String> {
    let render_job_json = render_job_json.to_string();
    py.detach(move || {
        let job: OfflineRenderJob = parse_json("offline render job", &render_job_json)?;
        let report =
            registry().with_scene(scene_id, |scene| offline::render_offline(scene, job))??;
        report_json(report)
    })
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(create_world_session, m)?)?;
    m.add_function(wrap_pyfunction!(destroy_world_session, m)?)?;
    m.add_function(wrap_pyfunction!(list_worldspaces, m)?)?;
    m.add_function(wrap_pyfunction!(load_worldspace, m)?)?;
    m.add_function(wrap_pyfunction!(destroy_scene, m)?)?;
    m.add_function(wrap_pyfunction!(scene_stats, m)?)?;
    m.add_function(wrap_pyfunction!(query_visible, m)?)?;
    m.add_function(wrap_pyfunction!(get_buffer, m)?)?;
    m.add_function(wrap_pyfunction!(inspect_instance, m)?)?;
    m.add_function(wrap_pyfunction!(render_offline, m)?)?;
    Ok(())
}

#[pymodule]
fn world_renderer_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}

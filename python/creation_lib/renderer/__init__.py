"""Shared rendering infrastructure — camera, lighting, grid, gizmo, PBR renderer."""
from .camera import OrbitCamera
from .lighting import (
    LightingSetup, LIGHTING_PRESETS, PRESET_NAMES,
    LIGHT_TYPE_PRESETS, LIGHT_TYPE_KEYS, LIGHT_TYPE_LABELS,
)
from .grid import Grid, compile_grid_shader
from .gizmo import (
    GizmoManager, glm_to_matrix16, matrix16_to_glm,
    glm_to_list16, list16_to_glm,
    TRANSLATE, ROTATE, SCALE, LOCAL, WORLD,
)
from .simple_renderer import SimpleRenderer, SimpleMesh, PBRMaterial, compute_tangents
from .render_toggles import RenderToggles
from .scene_renderer import SceneRenderer, Material, Mesh, SceneNode
from .render_modes import RenderMode, RenderModeManager

"""ImGuizmo integration — replaces transform_gizmo, selection_gizmo, axis_gizmo.

Uses imgui-bundle's built-in ImGuizmo for:
- manipulate(): interactive translate/rotate/scale gizmo (on hotkey)
- view_manipulate(): corner orientation cube with animated transitions
"""
from __future__ import annotations
import math
from pyglm import glm
from imgui_bundle import imgui, ImVec2
from imgui_bundle import imguizmo

# Operation and mode enums
TRANSLATE = imguizmo.im_guizmo.OPERATION.translate
ROTATE = imguizmo.im_guizmo.OPERATION.rotate
SCALE = imguizmo.im_guizmo.OPERATION.scale
LOCAL = imguizmo.im_guizmo.MODE.local
WORLD = imguizmo.im_guizmo.MODE.world

Matrix16 = imguizmo.im_guizmo.Matrix16

def glm_to_matrix16(m: glm.mat4) -> Matrix16:
    """Convert PyGLM column-major mat4 to ImGuizmo Matrix16."""
    values = []
    for col in range(4):
        for row in range(4):
            values.append(m[col][row])
    return Matrix16(values)


def matrix16_to_glm(m: Matrix16) -> glm.mat4:
    """Convert ImGuizmo Matrix16 to PyGLM column-major mat4."""
    v = m.values
    return glm.mat4(
        v[0], v[1], v[2], v[3],
        v[4], v[5], v[6], v[7],
        v[8], v[9], v[10], v[11],
        v[12], v[13], v[14], v[15],
    )


# Keep old names as aliases for any code that uses them
def glm_to_list16(m: glm.mat4) -> list[float]:
    """Convert PyGLM column-major mat4 to flat list of 16 floats."""
    result = []
    for col in range(4):
        for row in range(4):
            result.append(m[col][row])
    return result


def list16_to_glm(values: list[float]) -> glm.mat4:
    """Convert flat list of 16 floats to PyGLM column-major mat4."""
    return glm.mat4(
        values[0], values[1], values[2], values[3],
        values[4], values[5], values[6], values[7],
        values[8], values[9], values[10], values[11],
        values[12], values[13], values[14], values[15],
    )



# Z-up → Y-up: rotation -90° around X  →  (x,y,z) → (x, z, -y)
# Column-major GLM layout:
_C = glm.mat4(1, 0,  0, 0,  0, 0, -1, 0,  0, 1, 0, 0,  0, 0, 0, 1)
# Y-up → Z-up: rotation +90° around X  →  (x,y,z) → (x, -z, y)
_C_inv = glm.mat4(1, 0, 0, 0,  0, 0, 1, 0,  0, -1, 0, 0,  0, 0, 0, 1)


def _camera_to_cube_view(camera) -> Matrix16:
    """Build a Y-up view matrix for the nav cube from current camera state.

    The cube orbits around origin (ImGuizmo assumption), so we subtract
    camera.target to centre the eye. The Y-up remapping makes ImGuizmo's
    face labels (Top/Front/Right…) align with our Z-up world.
    """
    eye_rel = camera.get_eye_position() - camera.target
    eye_yup = glm.vec3(_C * glm.vec4(eye_rel, 1.0))
    mat = glm.lookAt(eye_yup, glm.vec3(0, 0, 0), glm.vec3(0, 1, 0))
    return glm_to_matrix16(mat)


class GizmoManager:

    def __init__(self):
        self.operation = TRANSLATE
        self.mode = LOCAL
        self.enabled = True
        self.manipulate_active = False  # Only show manipulate gizmo when user presses W/E/R
        self._cube_view: Matrix16 | None = None  # persistent Y-up view for nav cube

    def set_operation(self, op):
        self.operation = op
        self.manipulate_active = True  # Pressing W/E/R activates the gizmo

    def deactivate_manipulate(self):
        """Hide the manipulate gizmo (Escape or deselect)."""
        self.manipulate_active = False

    def set_mode(self, mode):
        self.mode = mode

    def draw(self, camera, viewport_pos, viewport_size, selected_node):
        """Draw ImGuizmo overlays; return the new glm.mat4 if manipulated, else None.

        ``viewport_pos`` and ``viewport_size`` are the panel's screen position and
        content size (ImVec2).
        """
        if not self.enabled:
            return None

        aspect = viewport_size.x / max(viewport_size.y, 1)
        view = glm_to_matrix16(camera.get_view_matrix())
        proj = glm_to_matrix16(camera.get_projection_matrix(aspect))

        imguizmo.im_guizmo.set_orthographic(False)
        imguizmo.im_guizmo.begin_frame()
        imguizmo.im_guizmo.set_drawlist()
        imguizmo.im_guizmo.set_rect(
            viewport_pos.x, viewport_pos.y,
            viewport_size.x, viewport_size.y,
        )

        # Manipulate gizmo on selected node — only if activated via hotkey
        result = None
        if selected_node and self.manipulate_active:
            obj = glm_to_matrix16(selected_node.world_transform)
            changed = imguizmo.im_guizmo.manipulate(
                view, proj, self.operation, self.mode, obj
            )
            if changed:
                result = matrix16_to_glm(obj)

        # Corner view cube (128x128 in top-right)
        cube_size = 128.0
        cube_pos = ImVec2(
            viewport_pos.x + viewport_size.x - cube_size,
            viewport_pos.y,
        )

        # Pass a *persistent* Y-up view to view_manipulate so ImGuizmo can run
        # its own multi-frame animation without being reset each frame.
        # When ImGuizmo changes the view we follow it (sync camera).
        # When it doesn't, we sync the cube view from the camera (orbit/pan/zoom).
        if self._cube_view is None:
            self._cube_view = _camera_to_cube_view(camera)

        cube_before = list(self._cube_view.values)
        imguizmo.im_guizmo.view_manipulate(
            self._cube_view, float(camera.distance),
            cube_pos, ImVec2(cube_size, cube_size),
            int(0x10101010),
        )

        if list(self._cube_view.values) != cube_before:
            # ImGuizmo is animating — follow it, update camera
            inv = glm.inverse(matrix16_to_glm(self._cube_view))
            eye_yup = glm.vec3(inv[3])
            diff = glm.vec3(_C_inv * glm.vec4(eye_yup, 0.0))
            dist = glm.length(diff)
            if dist > 1e-6:
                camera.elevation = math.degrees(math.asin(
                    max(-1.0, min(1.0, diff.z / dist))
                ))
                camera.azimuth = math.degrees(math.atan2(diff.y, diff.x))
                camera._clamp_elevation()
        else:
            # ImGuizmo idle — keep cube view in sync with camera (orbit/pan/zoom)
            self._cube_view = _camera_to_cube_view(camera)

        return result

    def is_using(self) -> bool:
        """Check if gizmo is currently being manipulated."""
        return imguizmo.im_guizmo.is_using()

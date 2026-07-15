"""Orbit camera with configurable navigation styles.
Input is received via imgui IO (mouse delta, scroll, modifier keys).
"""
import math
import glm

NAV_DEFAULT = "default"
NAV_BLENDER = "blender"
NAV_3DSMAX = "3dsmax"

# Sensitivity constants
ORBIT_SENSITIVITY = 0.3
PAN_SENSITIVITY = 0.003
ZOOM_SENSITIVITY = 0.08
ZOOM_SCROLL_FACTOR = 1.1


class OrbitCamera:
    def __init__(self):
        self.azimuth = 45.0       # degrees
        self.elevation = 25.0     # degrees
        self.distance = 50.0
        self.target = glm.vec3(0.0, 0.0, 0.0)
        self.fov = 45.0           # degrees
        self.near = 0.1
        self.far = 10000.0
        self._framed_radius = 0.0
        self.nav_style = NAV_3DSMAX  # default nav style

    def _clamp_elevation(self):
        self.elevation = max(-89.0, min(89.0, self.elevation))

    def get_eye_position(self) -> glm.vec3:
        az = math.radians(self.azimuth)
        el = math.radians(self.elevation)
        return self.target + glm.vec3(
            self.distance * math.cos(el) * math.cos(az),
            self.distance * math.cos(el) * math.sin(az),
            self.distance * math.sin(el),
        )

    def get_view_matrix(self) -> glm.mat4:
        eye = self.get_eye_position()
        return glm.lookAt(eye, self.target, glm.vec3(0, 0, 1))

    def get_projection_matrix(self, aspect: float) -> glm.mat4:
        return glm.perspective(
            glm.radians(self.fov), aspect, self.near, self.far
        )

    def orbit(self, dx: float, dy: float):
        """Apply orbit rotation from mouse delta (pixels)."""
        self.azimuth -= dx * ORBIT_SENSITIVITY
        self.elevation += dy * ORBIT_SENSITIVITY
        self._clamp_elevation()

    def pan(self, dx: float, dy: float):
        """Apply pan from mouse delta (pixels)."""
        view = self.get_view_matrix()
        right = glm.vec3(view[0][0], view[1][0], view[2][0])
        up = glm.vec3(view[0][1], view[1][1], view[2][1])
        scale = self.distance * PAN_SENSITIVITY
        self.target -= right * dx * scale
        self.target += up * dy * scale

    def zoom(self, delta: float):
        """Apply zoom from mouse scroll."""
        if delta > 0:
            self.distance /= ZOOM_SCROLL_FACTOR
        elif delta < 0:
            self.distance *= ZOOM_SCROLL_FACTOR
        self.distance = max(0.1, self.distance)
        self._sync_far_plane()

    def zoom_drag(self, dy: float):
        """Apply zoom from drag (RMB drag or Ctrl+MMB)."""
        self.distance *= 1.0 + dy * ZOOM_SENSITIVITY
        self.distance = max(0.1, self.distance)
        self._sync_far_plane()

    def _sync_far_plane(self):
        if self._framed_radius <= 0.0:
            return
        margin = max(1.0, self._framed_radius * 0.1)
        self.far = max(self.far, self.distance + self._framed_radius + margin)

    def frame_on_bounds(self, center: glm.vec3, radius: float):
        """Frame camera on a bounding sphere."""
        self.target = glm.vec3(center)
        self.distance = radius * 2.5
        self._framed_radius = max(0.0, radius)
        margin = max(1.0, self._framed_radius * 0.1)
        self.far = max(10000.0, self.distance + self._framed_radius + margin)

    def set_from_view_matrix(self, view: glm.mat4):
        """Decompose a view matrix into orbit camera parameters.

        Extracts eye position from the inverse view matrix, then computes
        azimuth, elevation, and distance relative to the current target.
        """
        inv = glm.inverse(view)
        eye = glm.vec3(inv[3])  # 4th column = camera position in world space
        diff = eye - self.target
        self.distance = glm.length(diff)
        if self.distance < 1e-6:
            return
        # Elevation: angle from XY plane (Z is up)
        self.elevation = math.degrees(math.asin(
            max(-1.0, min(1.0, diff.z / self.distance))
        ))
        # Azimuth: angle in XY plane
        self.azimuth = math.degrees(math.atan2(diff.y, diff.x))
        self._clamp_elevation()

    def set_nav_style(self, style: str):
        self.nav_style = style

    def set_front(self):
        self.azimuth = 90.0
        self.elevation = 0.0

    def set_back(self):
        self.azimuth = -90.0
        self.elevation = 0.0

    def set_side(self):
        self.azimuth = 0.0
        self.elevation = 0.0

    def set_right(self):
        self.set_side()

    def set_left(self):
        self.azimuth = 180.0
        self.elevation = 0.0

    def set_top(self):
        self.azimuth = 90.0
        self.elevation = 89.0

    def handle_input(self, io):
        """Process imgui IO for camera navigation.

        Call once per frame from the viewport panel, only when
        the viewport is hovered and imgui doesn't want the mouse.

        Args:
            io: imgui.get_io() result
        """
        # Scroll zoom
        if io.mouse_wheel != 0:
            self.zoom(io.mouse_wheel)

        dx = io.mouse_delta.x
        dy = io.mouse_delta.y
        if dx == 0 and dy == 0:
            return

        lmb = io.mouse_down[0]
        mmb = io.mouse_down[2]
        rmb = io.mouse_down[1]

        if self.nav_style == NAV_3DSMAX:
            # Alt+MMB = orbit, MMB = pan
            if io.key_alt and mmb:
                self.orbit(dx, dy)
            elif mmb:
                self.pan(dx, dy)
            elif io.key_alt and rmb:
                self.zoom_drag(dy)
        elif self.nav_style == NAV_BLENDER:
            # MMB = orbit, Shift+MMB = pan
            if io.key_shift and mmb:
                self.pan(dx, dy)
            elif mmb:
                self.orbit(dx, dy)
        else:  # NAV_DEFAULT
            # Ctrl+LMB = orbit, MMB = pan, Shift+Ctrl+LMB = pan
            if io.key_ctrl and io.key_shift and lmb:
                self.pan(dx, dy)
            elif io.key_ctrl and lmb:
                self.orbit(dx, dy)
            elif mmb:
                self.pan(dx, dy)

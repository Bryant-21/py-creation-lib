"""Lighting setup — uniform-based (no scene graph light nodes).

Three scene presets (studio, dramatic, outdoor) control ambient/fill/key
balance.  Light-type presets simulate different bulb color temperatures
and are applied on top of the scene preset.

- Key light: heading 30, pitch -55
- Fill light: heading -135, pitch -15
"""
import logging
import math
import glm
import moderngl

_log = logging.getLogger("nif_editor.lighting")


LIGHTING_PRESETS = {
    "studio": {
        "ambient": glm.vec3(0.15, 0.15, 0.18),
        "fill": glm.vec3(0.15, 0.15, 0.17),
        "key": glm.vec3(1.0, 1.0, 1.0),
    },
    "dramatic": {
        "ambient": glm.vec3(0.08, 0.08, 0.10),
        "fill": glm.vec3(0.10, 0.10, 0.12),
        "key": glm.vec3(1.2, 1.15, 1.1),
    },
    "outdoor": {
        "ambient": glm.vec3(0.20, 0.22, 0.28),
        "fill": glm.vec3(0.12, 0.12, 0.14),
        "key": glm.vec3(1.1, 1.05, 0.95),
    },
}

PRESET_NAMES = list(LIGHTING_PRESETS.keys())

# -- Light-type presets (bulb color temperature simulation) --
# Each entry tints the key/fill/ambient colors from the scene preset.
# "tint" is multiplied onto key+fill, "ambient_tint" onto ambient.
# "intensity" scales overall key brightness.
LIGHT_TYPE_PRESETS = {
    "standard": {
        "label": "Standard (neutral white)",
        "tint": glm.vec3(1.0, 1.0, 1.0),
        "ambient_tint": glm.vec3(1.0, 1.0, 1.0),
        "intensity": 1.0,
    },
    "soft_white": {
        "label": "Soft White (2700K)",
        "tint": glm.vec3(1.0, 0.87, 0.72),         # warm incandescent
        "ambient_tint": glm.vec3(1.0, 0.92, 0.82),
        "intensity": 0.95,
    },
    "warm_white": {
        "label": "Warm White (3000K)",
        "tint": glm.vec3(1.0, 0.90, 0.78),
        "ambient_tint": glm.vec3(1.0, 0.94, 0.85),
        "intensity": 1.0,
    },
    "cool_white": {
        "label": "Cool White (4000K)",
        "tint": glm.vec3(0.95, 0.97, 1.0),
        "ambient_tint": glm.vec3(0.94, 0.96, 1.0),
        "intensity": 1.05,
    },
    "daylight": {
        "label": "Daylight (5600K)",
        "tint": glm.vec3(0.90, 0.94, 1.0),
        "ambient_tint": glm.vec3(0.88, 0.93, 1.0),
        "intensity": 1.1,
    },
    "overcast": {
        "label": "Overcast Sky (6500K)",
        "tint": glm.vec3(0.85, 0.90, 1.0),
        "ambient_tint": glm.vec3(0.82, 0.88, 1.0),
        "intensity": 0.9,
    },
    "indoor_fluorescent": {
        "label": "Indoor Fluorescent",
        "tint": glm.vec3(0.95, 1.0, 0.92),          # slight green cast
        "ambient_tint": glm.vec3(0.92, 0.97, 0.90),
        "intensity": 1.0,
    },
    "tungsten": {
        "label": "Tungsten / Candlelight",
        "tint": glm.vec3(1.0, 0.78, 0.50),           # very warm
        "ambient_tint": glm.vec3(1.0, 0.85, 0.65),
        "intensity": 0.85,
    },
    "moonlight": {
        "label": "Moonlight",
        "tint": glm.vec3(0.70, 0.78, 1.0),
        "ambient_tint": glm.vec3(0.60, 0.68, 0.90),
        "intensity": 0.55,
    },
    "neon": {
        "label": "Neon (cool cyan)",
        "tint": glm.vec3(0.70, 1.0, 0.95),
        "ambient_tint": glm.vec3(0.60, 0.85, 0.82),
        "intensity": 1.1,
    },
    "red": {
        "label": "Red Light",
        "tint": glm.vec3(1.0, 0.25, 0.20),
        "ambient_tint": glm.vec3(0.90, 0.30, 0.25),
        "intensity": 0.95,
    },
    "green": {
        "label": "Green Light",
        "tint": glm.vec3(0.25, 1.0, 0.30),
        "ambient_tint": glm.vec3(0.30, 0.85, 0.35),
        "intensity": 0.95,
    },
}

LIGHT_TYPE_KEYS = list(LIGHT_TYPE_PRESETS.keys())
LIGHT_TYPE_LABELS = [LIGHT_TYPE_PRESETS[k]["label"] for k in LIGHT_TYPE_KEYS]


class LightingSetup:
    def __init__(self):
        self.key_heading = 110.0
        self.key_pitch = -21.0
        self.fill_heading = -135.0
        self.fill_pitch = -15.0
        self.mirror_light = False  # add a mirrored copy of key light on opposite side
        self.key_intensity = 1.0   # brightness multiplier for the key light
        self.skylight = False      # fill + ambient enabled (hemisphere sky light)
        self._preset = "studio"
        self._light_type = "standard"
        # Apply studio defaults
        p = LIGHTING_PRESETS["studio"]
        self._base_key = glm.vec3(p["key"])
        self._base_fill = glm.vec3(p["fill"])
        self._base_ambient = glm.vec3(p["ambient"])
        self.key_color = glm.vec3(self._base_key)
        self.fill_color = glm.vec3(self._base_fill)
        self.ambient_color = glm.vec3(self._base_ambient)
        # NIF point lights (populated by LightDisplay)
        self.point_lights: list[dict] = []
        self._logged_uniforms = False
        self._update_directions()

    @property
    def preset(self) -> str:
        return self._preset

    def set_preset(self, name: str):
        """Apply a named lighting preset (scene balance)."""
        if name not in LIGHTING_PRESETS:
            _log.warning("Unknown lighting preset: %s", name)
            return
        self._preset = name
        p = LIGHTING_PRESETS[name]
        self._base_key = glm.vec3(p["key"])
        self._base_fill = glm.vec3(p["fill"])
        self._base_ambient = glm.vec3(p["ambient"])
        self._apply_light_type()

    @property
    def light_type(self) -> str:
        return self._light_type

    def set_light_type(self, name: str):
        """Apply a light-type (bulb color) preset on top of the scene preset."""
        if name not in LIGHT_TYPE_PRESETS:
            _log.warning("Unknown light type: %s", name)
            return
        self._light_type = name
        self._apply_light_type()
        self._shadow_dirty = True

    def _apply_light_type(self):
        """Recompute final colors = base scene colors * light-type tint."""
        lt = LIGHT_TYPE_PRESETS.get(self._light_type)
        if lt is None:
            lt = LIGHT_TYPE_PRESETS["standard"]
        tint = lt["tint"]
        amb_tint = lt["ambient_tint"]
        intensity = lt["intensity"]
        self.key_color = glm.vec3(
            self._base_key.x * tint.x * intensity,
            self._base_key.y * tint.y * intensity,
            self._base_key.z * tint.z * intensity,
        )
        self.fill_color = glm.vec3(
            self._base_fill.x * tint.x,
            self._base_fill.y * tint.y,
            self._base_fill.z * tint.z,
        )
        self.ambient_color = glm.vec3(
            self._base_ambient.x * amb_tint.x,
            self._base_ambient.y * amb_tint.y,
            self._base_ambient.z * amb_tint.z,
        )

    def _update_directions(self):
        self.key_dir = self._heading_pitch_to_dir(self.key_heading, self.key_pitch)
        self.fill_dir = self._heading_pitch_to_dir(self.fill_heading, self.fill_pitch)
        # Mirror light: key light reflected across the vertical axis (negate heading)
        self.mirror_dir = self._heading_pitch_to_dir(-self.key_heading, self.key_pitch)

    @staticmethod
    def _heading_pitch_to_dir(heading: float, pitch: float) -> glm.vec3:
        h = math.radians(heading)
        p = math.radians(pitch)
        return glm.vec3(
            math.cos(p) * math.sin(h),
            math.cos(p) * math.cos(h),
            -math.sin(p),
        )

    def set_uniforms(self, program: moderngl.Program):
        """Set lighting uniforms on a shader program."""
        if "lightDir0" in program:
            program["lightDir0"].value = tuple(glm.normalize(self.key_dir))
        if "lightCol0" in program:
            program["lightCol0"].value = tuple(self.key_color * self.key_intensity)
        if "lightDir1" in program:
            program["lightDir1"].value = tuple(glm.normalize(self.fill_dir))
        if "lightCol1" in program:
            program["lightCol1"].value = tuple(self.fill_color if self.skylight else glm.vec3(0))
        if "ambientCol" in program:
            program["ambientCol"].value = tuple(self.ambient_color if self.skylight else glm.vec3(0.05))

        # Mirror light (key light reflected to opposite side)
        if "mirrorLightEnabled" in program:
            program["mirrorLightEnabled"].value = 1.0 if self.mirror_light else 0.0
        if self.mirror_light:
            if "lightDir2" in program:
                program["lightDir2"].value = tuple(glm.normalize(self.mirror_dir))
            if "lightCol2" in program:
                program["lightCol2"].value = tuple(self.key_color * self.key_intensity)

        # Point lights from NIF
        n = min(len(self.point_lights), 4)
        if "numPointLights" in program:
            program["numPointLights"].value = n

        if n > 0:
            # Log uniform discovery once for debugging
            if not self._logged_uniforms:
                self._logged_uniforms = True
                all_uniforms = list(program)
                pl_uniforms = [k for k in all_uniforms if "oint" in k.lower()]
                _log.info("All shader uniforms: %s", all_uniforms)
                _log.info("Point light uniforms in shader: %s", pl_uniforms)
                _log.info("Point light data [0]: %s", self.point_lights[0])

            # Try array-base-name first, fall back to indexed element access
            self._set_point_light_arrays(program, n)

    def _set_point_light_arrays(self, program: moderngl.Program, n: int):
        """Set point light uniform arrays as whole-array values.

        ModernGL exposes array uniforms under their base name (e.g.
        ``"pointLightPos"``) — NOT per-element (``"pointLightPos[0]"``).
        vec3 arrays take a list of tuples; float arrays take a flat list.
        """
        positions = []
        colors = []
        radii = []
        const_attens = []
        linear_attens = []
        quad_attens = []

        for i in range(4):
            if i < n:
                pl = self.point_lights[i]
                positions.append(tuple(float(v) for v in pl["position"]))
                colors.append(tuple(float(v) for v in pl["color"]))
                radii.append(float(pl["radius"]))
                const_attens.append(float(pl["const_atten"]))
                linear_attens.append(float(pl["linear_atten"]))
                quad_attens.append(float(pl["quad_atten"]))
            else:
                positions.append((0.0, 0.0, 0.0))
                colors.append((0.0, 0.0, 0.0))
                radii.append(0.0)
                const_attens.append(0.0)
                linear_attens.append(0.0)
                quad_attens.append(0.0)

        for name, val in [
            ("pointLightPos", positions),
            ("pointLightCol", colors),
            ("pointLightRadius", radii),
            ("pointLightConstAtten", const_attens),
            ("pointLightLinearAtten", linear_attens),
            ("pointLightQuadAtten", quad_attens),
        ]:
            if name in program:
                program[name].value = val

    def update_key_light_drag(self, dx: float, dy: float):
        """Interactive key light drag: Alt+LMB."""
        self.key_heading -= dx * 200.0
        self.key_pitch += dy * 200.0
        self.key_pitch = max(-89.0, min(89.0, self.key_pitch))
        self._update_directions()
        self._shadow_dirty = True  # signal renderer to re-render shadow map

    _shadow_dirty = True

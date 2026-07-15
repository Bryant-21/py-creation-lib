"""Base types for material readers — game-independent intermediate format."""
from __future__ import annotations
from dataclasses import dataclass, field


@dataclass
class LayerData:
    """One layer in a Starfield multi-layer material."""
    texture_paths: dict[str, str] = field(default_factory=dict)   # semantic name -> path
    tint_color: tuple[float, float, float] | None = None          # RGB tint (0-1)
    opacity: float = 1.0
    normal_intensity: float = 1.0  # floatParam from .mat TextureSet
    uv_channel: int = 0                                           # 0 = primary UV, 1 = UV2
    uv_scale: tuple[float, float] = (1.0, 1.0)                    # per-layer UV tile
    uv_offset: tuple[float, float] = (0.0, 0.0)                   # per-layer UV translation


@dataclass
class BlenderData:
    """Blending configuration between two adjacent layers."""
    mode: str = "linear"                       # linear, additive, position_contrast, multiply, screen
    mask_texture: str | None = None            # blend mask texture path
    vertex_color_channel: str | None = None    # "r", "g", "b", "a", or None
    height_blend_threshold: float = 0.5
    height_blend_factor: float = 1.0
    mask_intensity: float = 1.0
    # Per-channel blend gates. Defaults match tools/sf_render_test.py
    # _parse_summary_blender (line 460-464): only Normal is enabled by default.
    blend_albedo: bool = False
    blend_normal: bool = True
    blend_metal: bool = False
    blend_rough: bool = False
    blend_ao: bool = False


@dataclass
class MaterialData:
    """Output of a material reader — game-independent intermediate format.

    All readers (BGSM, BGEM, Starfield .mat) produce this type.
    The material pipeline then maps it to the renderer's Material dataclass.

    For Starfield multi-layer materials:
    - `layers` contains per-layer texture paths and tint colors
    - `blenders` contains blending configs between adjacent layers
    - `texture_paths` is a shortcut to layers[0].texture_paths (backward compat)
    """
    texture_paths: dict[str, str] = field(default_factory=dict)   # Layer1 textures (backward compat)
    params: dict[str, object] = field(default_factory=dict)       # named material params
    material_model: str = "spec-gloss"                            # "spec-gloss" | "metallic-roughness"
    layers: list[LayerData] = field(default_factory=list)
    blenders: list[BlenderData] = field(default_factory=list)
    # Starfield Objects section
    alpha_settings: AlphaSettings | None = None
    decal_settings: DecalSettings | None = None
    emissive_settings: EmissiveSettings | None = None


@dataclass
class RenderFlags:
    """Per-draw render state derived from material properties."""
    depth_write: bool = True
    depth_test: bool = True
    polygon_offset: tuple[float, float] | None = None
    render_layer: int = 0          # sort key within transparent pass
    blend_enabled: bool = False
    blend_src: int = 0             # NiAlphaProperty blend factor index
    blend_dst: int = 0
    alpha_flags: int = 0
    alpha_threshold: float = 0.0
    double_sided: bool = False


@dataclass
class AlphaSettings:
    """Starfield AlphaSettingsComponent from .mat Objects section."""
    has_opacity: bool = False
    opacity_source_layer: int = 0
    is_decal: bool = False
    alpha_test_threshold: float = 0.5


@dataclass
class DecalSettings:
    """Starfield DecalSettingsComponent from .mat Objects section."""
    is_decal: bool = False
    material_overall_alpha: float = 1.0
    write_mask: int = 0xFFFFFFFF
    render_layer: int = 0
    blend_mode: int = 0            # 0=Lerp, 1=Additive, 2=Subtractive, 3=Multiplicative
    is_projected: bool = False
    use_parallax_occlusion: bool = False
    parallax_scale: float = 0.0
    max_parallax_steps: int = 200


@dataclass
class EmissiveSettings:
    """Starfield EmissiveSettingsComponent from .mat Objects section."""
    is_enabled: bool = False
    emissive_source_layer: int = 0
    luminous_emittance: float = 0.0
    adaptive_emittance: bool = False

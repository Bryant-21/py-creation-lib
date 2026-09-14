"""SceneBackend protocol for per-game rendering paths.

``SceneRenderer`` owns FBOs, post-processing (SSAO, composite), shadow
infrastructure, the camera, the grid, and the viewport; per-game draw logic lives
behind this protocol. ``NodeHandle`` is opaque (a ``SceneNode`` for FO4, an index
or ``RenderMesh`` for Starfield, ...). ``SceneRenderer`` never inspects it and only
passes it back to the owning backend.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Protocol, runtime_checkable

# Opaque per-backend identifier. Renderer code must treat this as
# pass-through and never branch on its concrete type.
NodeHandle = object


@dataclass
class RenderState:
    """Per-frame, game-agnostic render state built from the App's ``_toggle_*`` /
    ``_dbg_*`` attributes, so backends never read ``self._app``.

    Defaults match the renderer's fallbacks, so an empty RenderState renders the
    same as none.
    """

    # Material / lighting feature toggles (Scene menu checkboxes)
    diffuse: bool = True
    normal: bool = True
    spec: bool = True
    lighting: bool = True
    vertex_color: bool = True
    env_map: bool = True

    # Debug PBR sliders (toolbar tuning controls)
    env_boost: float = 1.0
    metal_f0: float = 0.9
    diffuse_bleed: float = 0.0
    exposure: float = 4.23
    spec_boost: float = 1.0
    ambient_boost: float = 1.0

    # Post-process toggles
    ssao_enabled: bool = False
    shadows_enabled: bool = False

    # Backend-specific state, mostly diagnostic flags. Promote anything
    # shareable to a real field.
    extra: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_app(cls, app: Any) -> "RenderState":
        """Build a RenderState from an App instance; ``app=None`` gives all defaults."""
        if app is None:
            return cls()
        return cls(
            diffuse=getattr(app, "_toggle_diffuse", True),
            normal=getattr(app, "_toggle_normal", True),
            spec=getattr(app, "_toggle_spec", True),
            lighting=getattr(app, "_toggle_lighting", True),
            vertex_color=getattr(app, "_toggle_vertexColor", True),
            env_map=getattr(app, "_toggle_envMap", True),
            env_boost=getattr(app, "_dbg_envBoost", 1.0),
            metal_f0=getattr(app, "_dbg_metalF0", 0.9),
            diffuse_bleed=getattr(app, "_dbg_diffuseBleed", 0.0),
            exposure=getattr(app, "_dbg_exposure", 4.23),
            spec_boost=getattr(app, "_dbg_specBoost", 1.0),
            ambient_boost=getattr(app, "_dbg_ambientBoost", 1.0),
            ssao_enabled=getattr(app, "_toggle_ssao", False),
            shadows_enabled=getattr(app, "_toggle_shadows", False),
        )


@runtime_checkable
class SceneBackend(Protocol):
    """Per-game scene + draw protocol.

    Backends in ``creation_lib/renderer/backends/`` implement the per-game draw
    loop; the host ``SceneRenderer`` owns FBOs, the camera, post-processing,
    shadow FBO setup, and the grid. Backends hold a back-reference to the host
    renderer to read shared FBO / GL state.
    """

    # ----- Lifecycle -----------------------------------------------------

    def load_nif(self, path: Path) -> None:
        """Load a NIF as the primary scene, replacing any existing one."""

    def attach_nif(self, path: Path, parent: NodeHandle | None) -> NodeHandle:
        """Attach a NIF as a child of ``parent`` (or scene root if None).

        Returns an opaque handle the caller can later pass to
        ``set_visible`` / ``node_world_transform`` / etc.
        """

    def unload(self) -> None:
        """Release all GPU + scene resources owned by this backend."""

    # ----- Per-frame draw ------------------------------------------------

    def render(self, camera: Any, lighting: Any, state: RenderState) -> None:
        """Draw the main scene into the currently bound FBO.

        The host ``SceneRenderer`` is responsible for binding ``self.fbo``,
        clearing, computing the view-projection matrix, and running any
        post-process passes (SSAO, composite). Backends draw geometry and
        nothing else.
        """

    def render_shadow_casters(self, light_space_matrix: Any) -> None:
        """Draw shadow-casting geometry depth-only into the shadow FBO."""

    # ----- Scene introspection ------------------------------------------

    def iter_nodes(self) -> Iterable[NodeHandle]:
        """Yield every scene node the backend currently owns."""

    def node_visible(self, h: NodeHandle) -> bool: ...
    def set_visible(self, h: NodeHandle, v: bool) -> None: ...
    def node_world_transform(self, h: NodeHandle) -> Any: ...
    def node_aabb(self, h: NodeHandle) -> Any: ...
    def node_label(self, h: NodeHandle) -> str: ...

    # ----- Overlays (each backend draws into the active FBO) ------------

    def draw_vertex_points(self, vp: Any) -> None: ...
    def draw_collision(self, vp: Any) -> None: ...
    def draw_selection_outline(self, h: NodeHandle, vp: Any, color: Any) -> None: ...

    # ----- Capability flags ---------------------------------------------

    @property
    def supports_ssao(self) -> bool: ...

    @property
    def supports_shadows(self) -> bool: ...

    @property
    def supports_collision_overlay(self) -> bool: ...

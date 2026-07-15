"""Render mode switching: textured, wireframe, normals, UV checker, unlit."""
from __future__ import annotations
from enum import IntEnum
import moderngl


class RenderMode(IntEnum):
    TEXTURED = 1
    WIREFRAME = 2
    NORMALS = 3
    UV_CHECKER = 4
    UNLIT = 5


LABELS = {
    RenderMode.TEXTURED: "Textured",
    RenderMode.WIREFRAME: "Wireframe",
    RenderMode.NORMALS: "Normals",
    RenderMode.UV_CHECKER: "UV Checker",
    RenderMode.UNLIT: "Unlit",
}


class RenderModeManager:
    def __init__(self, renderer):
        self.renderer = renderer
        self.enabled_modes = {RenderMode.TEXTURED}
        self.mode = RenderMode.TEXTURED

    def set_mode(self, mode: RenderMode):
        mode = RenderMode(mode)
        self.enabled_modes = {mode}
        self.mode = mode

    def set_enabled(self, mode: RenderMode, enabled: bool):
        mode = RenderMode(mode)
        if enabled:
            self.enabled_modes.add(mode)
            self.mode = mode
        else:
            self.enabled_modes.discard(mode)
            if self.mode == mode:
                self.mode = self.active_modes()[0] if self.enabled_modes else RenderMode.TEXTURED

    def toggle_mode(self, mode: RenderMode):
        mode = RenderMode(mode)
        self.set_enabled(mode, mode not in self.enabled_modes)

    def is_enabled(self, mode: RenderMode) -> bool:
        return RenderMode(mode) in self.enabled_modes

    def active_modes(self) -> tuple[RenderMode, ...]:
        return tuple(mode for mode in RenderMode if mode in self.enabled_modes)

    def should_draw_textured_base(self) -> bool:
        return (
            not self.is_enabled(RenderMode.UV_CHECKER)
            and self.is_enabled(RenderMode.TEXTURED)
        )

    def get_shader_program(self) -> str:
        """Return which shader program key to use for the current mode."""
        if self.is_enabled(RenderMode.WIREFRAME):
            return "wireframe"
        elif self.is_enabled(RenderMode.NORMALS):
            return "normals"
        elif self.is_enabled(RenderMode.UV_CHECKER):
            return "uv_checker"
        else:
            return "default"  # textured and unlit both use game-specific default shader

    def apply_pre_render(self):
        """Configure renderer state before drawing meshes."""
        ctx = self.renderer.ctx
        if self.is_enabled(RenderMode.WIREFRAME):
            ctx.wireframe = True
        else:
            ctx.wireframe = False

    def apply_shader_uniforms(self, program):
        """Set debug toggle uniforms based on current mode.

        Only forces toggle_lighting OFF for UNLIT mode. Does NOT force it ON
        for other modes — that would override the user's Textures > Lighting toggle.
        """
        if self.is_enabled(RenderMode.UNLIT):
            if "toggle_lighting" in program:
                program["toggle_lighting"].value = 0.0

        if self.is_enabled(RenderMode.NORMALS):
            if "u_normal_length" in program:
                program["u_normal_length"].value = 0.5

    def apply_post_render(self):
        """Reset state after drawing meshes."""
        self.renderer.ctx.wireframe = False

    def get_label(self) -> str:
        labels = [LABELS[mode] for mode in self.active_modes()]
        return ", ".join(labels) if labels else "None"

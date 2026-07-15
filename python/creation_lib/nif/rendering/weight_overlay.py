"""Weight heatmap visualization state manager.

Tracks which bone is selected for weight display and applies the
corresponding shader uniforms. Used by any editor that wants to show
bone weight influence as a blue-to-red heatmap overlay.
"""
from __future__ import annotations

import logging

_log = logging.getLogger("nif.rendering.weight_overlay")


class WeightOverlay:
    """Manages weight heatmap visualization state."""

    def __init__(self):
        self.enabled: bool = False
        self.selected_bone_index: int = -1
        self.selected_bone_name: str = ""

    def select_bone(self, bone_index: int, bone_name: str = ""):
        """Enable weight visualization for the given bone.

        Args:
            bone_index: Index into the mesh's bone palette.
            bone_name: Optional human-readable bone name for UI display.
        """
        self.selected_bone_index = bone_index
        self.selected_bone_name = bone_name
        self.enabled = True
        _log.debug("Weight overlay: bone %d (%s)", bone_index, bone_name)

    def clear(self):
        """Disable weight visualization."""
        self.enabled = False
        self.selected_bone_index = -1
        self.selected_bone_name = ""

    def apply_uniforms(self, program):
        """Set weight visualization uniforms on a shader program.

        Args:
            program: moderngl.Program with u_weight_mode and u_selected_bone
                uniforms.
        """
        program["u_weight_mode"].value = self.enabled
        if self.enabled:
            program["u_selected_bone"].value = self.selected_bone_index

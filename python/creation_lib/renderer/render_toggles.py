"""RenderToggles — per-renderer visual debug flags.

UIs write to renderer.toggles directly.
"""
from __future__ import annotations
from dataclasses import dataclass, field


@dataclass
class RenderToggles:
    """Per-renderer visual override flags."""
    diffuse:       bool  = True
    normal:        bool  = True
    specular:      bool  = True
    env_map:       bool  = True
    vertex_colors: bool  = True
    lighting:      bool  = True
    show_vertices: bool  = False
    ssao:          bool  = False
    shadows:       bool  = False
    mesh_alpha:    float = 1.0   # 0.0-1.0 global mesh opacity (cloth maker transparency)

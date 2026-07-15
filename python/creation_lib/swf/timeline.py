"""SWF timeline model: frames, display list, sprites.

The display list is cumulative -- each frame modifies the previous
frame's display list via PlaceObject2 (add/update) and RemoveObject2
(delete). This module provides both the raw frame list and a method
to compute the effective display list at any frame index.
"""
from __future__ import annotations

from dataclasses import dataclass, field

from creation_lib.swf.types import MATRIX, RGBA


@dataclass
class DisplayEntry:
    """One entry on the display list (a placed shape instance)."""
    character_id: int
    matrix: MATRIX = field(default_factory=MATRIX)
    color_transform: bytes | None = None
    name: str | None = None
    ratio: int | None = None
    clip_depth: int | None = None


@dataclass
class Frame:
    """Single frame in a timeline. Stores delta operations."""
    label: str | None = None
    placements: dict[int, DisplayEntry] = field(default_factory=dict)   # depth -> entry
    removals: set[int] = field(default_factory=set)                     # depths to remove

    def place(self, depth: int, character_id: int, matrix: MATRIX,
              name: str | None = None, color_transform: bytes | None = None,
              ratio: int | None = None, clip_depth: int | None = None) -> None:
        self.placements[depth] = DisplayEntry(
            character_id=character_id, matrix=matrix, name=name,
            color_transform=color_transform, ratio=ratio, clip_depth=clip_depth,
        )

    def remove(self, depth: int) -> None:
        self.removals.add(depth)

    def update_transform(self, depth: int, matrix: MATRIX) -> None:
        """Update only the transform at a depth (move flag in PlaceObject2)."""
        if depth in self.placements:
            self.placements[depth].matrix = matrix


@dataclass
class Timeline:
    """Ordered list of frames with cumulative display list semantics."""
    frames: list[Frame] = field(default_factory=list)

    @property
    def frame_count(self) -> int:
        return len(self.frames)

    def display_list_at(self, frame_index: int) -> dict[int, DisplayEntry]:
        """Compute the effective display list at a given frame.

        Iterates frames 0..frame_index, applying placements and removals.
        Returns depth -> DisplayEntry mapping.
        """
        dl: dict[int, DisplayEntry] = {}
        for i in range(min(frame_index + 1, len(self.frames))):
            f = self.frames[i]
            for depth in f.removals:
                dl.pop(depth, None)
            for depth, entry in f.placements.items():
                dl[depth] = entry
        return dl


@dataclass
class SpriteDef:
    """DefineSprite -- a nested movieclip with its own timeline."""
    sprite_id: int
    timeline: Timeline

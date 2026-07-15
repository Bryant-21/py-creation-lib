"""Core skinning data interchange format."""
from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np


@dataclass
class SubSegmentInfo:
    """A sub-segment within an FO4 BSSubIndexTriShape segment."""
    start_index: int        # Triangle start index within the segment
    num_primitives: int     # Number of triangles in this sub-segment
    user_index: int = 0     # User slot ID / biped object type
    bone_id: int = 0xFFFFFFFF  # Bone hash (0xFFFFFFFF = use parent segment)
    cut_offsets: list[float] = field(default_factory=list)


@dataclass
class SegmentInfo:
    """A top-level segment in an FO4 BSSubIndexTriShape."""
    start_index: int        # Triangle start index (into shape's triangle list)
    num_primitives: int     # Total triangles in this segment
    sub_segments: list[SubSegmentInfo] = field(default_factory=list)
    user_index: int = 0     # User slot ID from Per Segment Data (used when no sub-segments)


@dataclass
class SkinData:
    """Interchange format for skinning data between engine, UI, and MCP.

    All arrays use numpy for efficient vectorized operations.
    The weights and bone_indices arrays have shape (N, max_bones_per_vertex),
    where N is the number of vertices.  Unused slots have weight 0.0 and
    bone index 0.
    """

    vertices: np.ndarray          # (N, 3) float32 positions
    triangles: np.ndarray         # (M, 3) uint32 indices
    normals: np.ndarray           # (N, 3) float32
    uvs: np.ndarray               # (N, 2) float32
    bone_names: list[str]         # skeleton bone list
    weights: np.ndarray           # (N, max_bones) float32 weight values
    bone_indices: np.ndarray      # (N, max_bones) int32 bone indices per vertex
    segment_ids: np.ndarray       # (M,) int32 per-triangle segment index, -1 = unassigned
    max_bones_per_vertex: int = 4
    inv_bind_transforms: list[np.ndarray] = field(default_factory=list)  # Per-bone 4x4 inverse bind pose
    segments: list[SegmentInfo] = field(default_factory=list)  # FO4 segment hierarchy
    ssf_file: str = ""  # SSF filename from BSSubIndexTriShape Segment Data
    vertex_colors: np.ndarray | None = None  # (N, 4) float32 RGBA vertex colors, None if absent
    bone_parents: list[int] = field(default_factory=list)  # Per-bone parent index (-1 = root)

    # ------------------------------------------------------------------
    # Properties
    # ------------------------------------------------------------------

    @property
    def num_vertices(self) -> int:
        """Number of vertices in the mesh."""
        return len(self.vertices)

    @property
    def num_triangles(self) -> int:
        """Number of triangles in the mesh."""
        return len(self.triangles)

    # ------------------------------------------------------------------
    # Accessors
    # ------------------------------------------------------------------

    def get_vertex_weights(self, vertex_idx: int) -> list[tuple[str, float]]:
        """Return [(bone_name, weight), ...] for a vertex, sorted by weight descending."""
        if vertex_idx < 0 or vertex_idx >= self.num_vertices:
            return []
        result: list[tuple[str, float]] = []
        for j in range(self.weights.shape[1]):
            w = float(self.weights[vertex_idx, j])
            bi = int(self.bone_indices[vertex_idx, j])
            if w > 0 and 0 <= bi < len(self.bone_names):
                result.append((self.bone_names[bi], w))
        return sorted(result, key=lambda x: -x[1])

    # ------------------------------------------------------------------
    # Factory helpers
    # ------------------------------------------------------------------

    @classmethod
    def empty(cls, max_bones_per_vertex: int = 4) -> SkinData:
        """Create an empty SkinData with zero vertices/triangles."""
        return cls(
            vertices=np.empty((0, 3), dtype=np.float32),
            triangles=np.empty((0, 3), dtype=np.uint32),
            normals=np.empty((0, 3), dtype=np.float32),
            uvs=np.empty((0, 2), dtype=np.float32),
            bone_names=[],
            weights=np.empty((0, max_bones_per_vertex), dtype=np.float32),
            bone_indices=np.empty((0, max_bones_per_vertex), dtype=np.int32),
            segment_ids=np.empty((0,), dtype=np.int32),
            max_bones_per_vertex=max_bones_per_vertex,
        )

    @classmethod
    def from_geometry(
        cls,
        vertices: np.ndarray,
        triangles: np.ndarray,
        normals: np.ndarray | None = None,
        uvs: np.ndarray | None = None,
        max_bones_per_vertex: int = 4,
    ) -> SkinData:
        """Create SkinData from geometry arrays with empty weights."""
        n = len(vertices)
        m = len(triangles)
        return cls(
            vertices=np.asarray(vertices, dtype=np.float32).reshape(-1, 3),
            triangles=np.asarray(triangles, dtype=np.uint32).reshape(-1, 3),
            normals=(
                np.asarray(normals, dtype=np.float32).reshape(-1, 3)
                if normals is not None
                else np.zeros((n, 3), dtype=np.float32)
            ),
            uvs=(
                np.asarray(uvs, dtype=np.float32).reshape(-1, 2)
                if uvs is not None
                else np.zeros((n, 2), dtype=np.float32)
            ),
            bone_names=[],
            weights=np.zeros((n, max_bones_per_vertex), dtype=np.float32),
            bone_indices=np.zeros((n, max_bones_per_vertex), dtype=np.int32),
            segment_ids=np.full(m, -1, dtype=np.int32),
            max_bones_per_vertex=max_bones_per_vertex,
        )

    def copy(self) -> SkinData:
        """Return a deep copy."""
        import copy as _copy
        return SkinData(
            vertices=self.vertices.copy(),
            triangles=self.triangles.copy(),
            normals=self.normals.copy(),
            uvs=self.uvs.copy(),
            bone_names=list(self.bone_names),
            weights=self.weights.copy(),
            bone_indices=self.bone_indices.copy(),
            segment_ids=self.segment_ids.copy(),
            max_bones_per_vertex=self.max_bones_per_vertex,
            segments=_copy.deepcopy(self.segments),
            ssf_file=self.ssf_file,
            vertex_colors=self.vertex_colors.copy() if self.vertex_colors is not None else None,
            bone_parents=list(self.bone_parents),
        )

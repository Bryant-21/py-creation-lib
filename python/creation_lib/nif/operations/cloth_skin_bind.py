"""Bind a BSTriShape to cloth bones generated from BSClothExtraData.

Clusters sim particles down to at most MAX_CLOTH_BONES bones, creates a
NiNode per cluster centroid, promotes the target shape to skinned, and
assigns each render vertex a weight of 1.0 to the nearest cluster bone.

BSVertexData.Bone Indices is a `byte[4]` field (see py_creation_lib/python/creation_lib/nif/nif_xml/nif.xml,
member `Bone Indices`), so shape-local bone index values above 255 saturate
on serialization. We stay under that with a safety margin by capping at
MAX_CLOTH_BONES=240.

The per-bone inverse bind transform counteracts the bone's world placement
(`translation = -bone_world_translation`, rotation identity) so the skinned
mesh renders at its original world position — matching the vanilla hair
cloth bone pattern from `FemaleHair04.nif`.
"""
from __future__ import annotations

import json
import numpy as np

from creation_lib._native import havok_native, nif_core_native
from creation_lib.nif.nif_file import NifFile
from creation_lib.nif.operations.skinning import (
    add_bone_node,
    convert_to_sub_index_tri_shape,
    make_shape_skinned,
    set_vertex_weights,
)

MAX_CLOTH_BONES = 240


def cloth_skin_bind(
    nif: NifFile,
    shape_id: int,
    *,
    nif_path: str | None = None,
    sim_positions: list[tuple[float, float, float]] | None = None,
    bone_prefix: str = "Cloth",
    root_node_id: int = 0,
) -> list[str]:
    """Skin a BSTriShape to cloth-particle bones.

    Args:
        nif:            Loaded NifFile. Mutated in place.
        shape_id:       Block id of the unskinned BSTriShape to promote.
        nif_path:       Path to a NIF on disk with a BSClothExtraData
                        blob. Used to load sim particle positions when
                        ``sim_positions`` is not provided.
        sim_positions:  Explicit (x, y, z) sim-particle positions. When
                        supplied, takes precedence over ``nif_path`` —
                        callers that already have the cloth scene in
                        memory (e.g. cloth_maker after a fresh authoring
                        pass) can skip the disk round-trip.
        bone_prefix:    Prefix for generated bone names (`{prefix}_{i:03d}`).
        root_node_id:   NiNode block id to parent the new bones under.

    Returns:
        Ordered list of bone names. Index position matches the shape-local
        bone index used in the vertex weight buffers.
    """
    if sim_positions is None:
        if nif_path is None:
            raise ValueError(
                "cloth_skin_bind requires either sim_positions or nif_path"
            )
        sim_positions = _load_sim_positions(nif_path)

    n_sim = len(sim_positions)
    if n_sim == 0:
        raise ValueError(f"{nif_path}: BSClothExtraData has no sim particles")

    shape = nif.get_block(shape_id)
    if shape is None:
        raise ValueError(f"shape block {shape_id} not found")
    vertex_data = list(shape.get_field("Vertex Data") or [])
    if not vertex_data:
        raise ValueError(f"shape {shape_id} has no vertex data")

    sim_points = np.asarray(sim_positions, dtype=np.float32)  # (S, 3)

    # Cluster sim particles down to <=MAX_CLOTH_BONES. Each cluster bone
    # lives at the centroid of its assigned sim particles. Render verts
    # are weighted to the nearest cluster bone (not the nearest raw sim
    # particle) so the uint8 bone-index field in BSVertexData cannot
    # overflow.
    sim_to_cluster = _cluster_sim_particles(sim_points, MAX_CLOTH_BONES)
    n_clusters = int(sim_to_cluster.max()) + 1 if n_sim else 0
    if n_clusters > 255:
        raise ValueError(
            f"cluster count {n_clusters} exceeds uint8 bone index limit 255"
        )
    cluster_centroids = np.zeros((n_clusters, 3), dtype=np.float32)
    for c in range(n_clusters):
        members = sim_points[sim_to_cluster == c]
        cluster_centroids[c] = members.mean(axis=0)

    render_positions = np.asarray(
        [(v["Vertex"]["x"], v["Vertex"]["y"], v["Vertex"]["z"]) for v in vertex_data],
        dtype=np.float32,
    )  # (N, 3)

    bone_ids: list[int] = []
    bone_names: list[str] = []
    for i in range(n_clusters):
        name = f"{bone_prefix}_{i:03d}"
        bid = add_bone_node(
            nif,
            name,
            translation=(
                float(cluster_centroids[i, 0]),
                float(cluster_centroids[i, 1]),
                float(cluster_centroids[i, 2]),
            ),
            parent_id=root_node_id,
        )
        bone_ids.append(bid)
        bone_names.append(name)

    inv_bind_transforms = []
    for i in range(n_clusters):
        m = np.eye(4, dtype=np.float32)
        m[:3, 3] = -cluster_centroids[i]
        inv_bind_transforms.append(m)

    make_shape_skinned(
        nif,
        shape_id,
        bone_ids=bone_ids,
        skeleton_root_id=root_node_id,
        inv_bind_transforms=inv_bind_transforms,
    )

    nearest = _nearest_bone(render_positions, cluster_centroids)
    n_verts = render_positions.shape[0]
    weights = np.zeros((n_verts, 4), dtype=np.float32)
    bone_indices = np.zeros((n_verts, 4), dtype=np.int32)
    weights[:, 0] = 1.0
    bone_indices[:, 0] = nearest
    set_vertex_weights(nif, shape_id, weights, bone_indices)

    # FO4 cloth sim requires BSSubIndexTriShape (hair, bathrobe, etc. —
    # see extracted/fo4/Meshes/Actors/Character/CharacterAssets/Hair/
    # Female/FemaleHair04.nif and Clothes/Bathrobe/OutfitM.nif). Promote
    # last so the skin writes land on a plain BSTriShape first.
    convert_to_sub_index_tri_shape(nif, shape_id)

    return bone_names


def _cluster_sim_particles(points: np.ndarray, max_clusters: int) -> np.ndarray:
    """Assign each sim particle to a cluster index in [0, n_clusters).

    Uses a deterministic stride-then-nearest-seed approach: every k-th
    particle becomes a seed (k chosen so that `ceil(n_sim / k) <= max_clusters`),
    and every particle is then assigned to its nearest seed. The returned
    cluster indices are compacted to a dense range starting at 0.
    """
    n = points.shape[0]
    if n <= max_clusters:
        return np.arange(n, dtype=np.int32)

    stride = (n + max_clusters - 1) // max_clusters
    seed_indices = np.arange(0, n, stride, dtype=np.int32)
    seeds = points[seed_indices]

    diff = points[:, None, :] - seeds[None, :, :]
    d2 = np.einsum("nsi,nsi->ns", diff, diff)
    raw = np.argmin(d2, axis=1).astype(np.int32)

    # Compact cluster ids into a dense [0, k) range so unused seeds (none
    # in this scheme, but guard anyway) do not create gaps.
    _, inverse = np.unique(raw, return_inverse=True)
    return inverse.astype(np.int32)


def _load_sim_positions(nif_path: str) -> list[tuple[float, float, float]]:
    nif_bytes = open(nif_path, "rb").read()
    blob = nif_core_native.cloth_extract_blob(nif_bytes)
    cloth_json = json.loads(havok_native.cloth_inspect_full_json(blob))
    sim_cloths = cloth_json.get("sim_cloths", [])
    if not sim_cloths:
        raise ValueError(f"{nif_path}: hclClothData has no sim_cloths")
    particles = sim_cloths[0].get("particles", [])
    if not particles:
        raise ValueError(f"{nif_path}: sim cloth has no particles")
    return [
        (float(p["position"][0]), float(p["position"][1]), float(p["position"][2]))
        for p in particles
        if p.get("position") and len(p["position"]) >= 3
    ]


def _nearest_bone(verts: np.ndarray, bones: np.ndarray) -> np.ndarray:
    """Return an (N,) array of bone indices: the closest bone for each vert."""
    # (N, B) squared distances; np.argmin along B gives the nearest.
    diff = verts[:, None, :] - bones[None, :, :]
    d2 = np.einsum("nbi,nbi->nb", diff, diff)
    return np.argmin(d2, axis=1).astype(np.int32)

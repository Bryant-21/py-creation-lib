"""Triangle strip / triangle list conversion operations."""
from ..actions import OperationResult


def triangulate(nif, block_id: int) -> OperationResult:
    """Convert triangle strips to triangle list.

    Reads the 'Points' field (strip indices) and writes back as 'Triangles'.
    Each strip of N indices produces N-2 triangles with alternating winding.
    """
    block = nif.get_block(block_id)
    if not block:
        return OperationResult(False, f"Block {block_id} not found")

    strips = block.get_field("Points") or block.get_field("Strips")
    if not strips:
        return OperationResult(False, f"Block {block_id} has no strip data")

    triangles = []
    for strip in strips:
        if not isinstance(strip, list) or len(strip) < 3:
            continue
        for i in range(len(strip) - 2):
            v0, v1, v2 = int(strip[i]), int(strip[i + 1]), int(strip[i + 2])
            # Skip degenerate triangles (used as strip restarts)
            if v0 == v1 or v1 == v2 or v0 == v2:
                continue
            # Alternate winding for even/odd triangles
            if i % 2 == 0:
                triangles.append({"v1": v0, "v2": v1, "v3": v2})
            else:
                triangles.append({"v1": v0, "v2": v2, "v3": v1})

    block.set_field("Triangles", triangles)
    block.set_field("Num Triangles", len(triangles))

    return OperationResult(True, f"Triangulated {len(triangles)} triangle(s) from strips", [block_id])


def strippify(nif, block_id: int) -> OperationResult:
    """Convert triangle list to a single triangle strip.

    Uses a simple greedy algorithm: builds strips by following shared edges.
    Not optimal but produces valid strips for NIF serialization.
    """
    block = nif.get_block(block_id)
    if not block:
        return OperationResult(False, f"Block {block_id} not found")

    triangles = block.get_field("Triangles") or []
    if not triangles:
        return OperationResult(False, f"Block {block_id} has no triangle data")

    # Build adjacency: edge -> list of (tri_index, third_vertex)
    edge_adj: dict[tuple[int, int], list[tuple[int, int]]] = {}
    tri_verts = []
    for ti, t in enumerate(triangles):
        v0, v1, v2 = int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0))
        tri_verts.append((v0, v1, v2))
        for e, opp in [((v0, v1), v2), ((v1, v2), v0), ((v0, v2), v1)]:
            key = (min(e), max(e))
            edge_adj.setdefault(key, []).append((ti, opp))

    used = [False] * len(tri_verts)
    strips = []

    for start_ti in range(len(tri_verts)):
        if used[start_ti]:
            continue
        used[start_ti] = True
        v0, v1, v2 = tri_verts[start_ti]
        strip = [v0, v1, v2]

        # Greedily extend the strip
        last_edge = (v1, v2)
        while True:
            key = (min(last_edge), max(last_edge))
            found = False
            for adj_ti, opp in edge_adj.get(key, []):
                if used[adj_ti]:
                    continue
                used[adj_ti] = True
                strip.append(opp)
                # The new trailing edge is (last_edge[1], opp) for correct winding
                last_edge = (last_edge[1], opp)
                found = True
                break
            if not found:
                break

        strips.append(strip)

    block.set_field("Points", strips)
    block.set_field("Strips", strips)
    block.set_field("Num Strips", len(strips))

    total_indices = sum(len(s) for s in strips)
    return OperationResult(
        True,
        f"Created {len(strips)} strip(s) with {total_indices} total indices",
        [block_id],
    )

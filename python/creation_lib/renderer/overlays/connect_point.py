"""Connection point visualization for the NIF editor viewport.

BSConnectPoint::Parents draw as small axis crosses at each point's world position,
with lines back to the parent node's origin. BSConnectPoint::Children have names
but no local offsets, so they draw at the owner node's position. Labels are imgui
text projected to screen space.

Selecting a Parents or Children block shows all connect points; selecting one
point in the tree shows only that one.
"""
from __future__ import annotations
import logging

import glm
import numpy as np
import moderngl

_log = logging.getLogger("renderer.overlays.connect_point")


class ConnectPointDisplay:
    """Renders BSConnectPoint markers and labels in the 3D viewport."""

    MARKER_SIZE = 1.0
    LINK_COLOR = (0.9, 0.6, 0.2, 1.0)       # Orange for link lines
    MARKER_COLOR = (1.0, 0.8, 0.2, 1.0)      # Yellow for markers
    SELECTED_COLOR = (0.2, 1.0, 0.6, 1.0)    # Green for selected point
    LABEL_COLOR = (1.0, 0.9, 0.5, 1.0)       # Warm yellow for labels
    LABEL_SELECTED_COLOR = (0.3, 1.0, 0.7, 1.0)
    CHILD_MARKER_COLOR = (0.4, 0.7, 1.0, 1.0)   # Light blue for child CPs
    CHILD_LABEL_COLOR = (0.5, 0.8, 1.0, 1.0)     # Light blue for child labels

    def __init__(self, app=None):
        self.app = app
        self._vbo: moderngl.Buffer | None = None
        self._color_vbo: moderngl.Buffer | None = None
        self._vao: moderngl.VertexArray | None = None
        self._num_vertices = 0
        self._visible = False
        self._needs_rebuild = False
        self._marker_size = self.MARKER_SIZE
        # Which BSConnectPoint::Parents block is selected (or None)
        self._selected_cp_block_id: int | None = None
        # Which individual connect point index within that block (or None = all)
        self._selected_cp_index: int | None = None
        # Label data for imgui overlay: list of (name, world_x, world_y, world_z, is_selected, is_child)
        self._labels: list[tuple[str, float, float, float, bool, bool]] = []
        # Lightweight SceneNodes for gizmo + picking (one per visible parent CP)
        self.cp_nodes: list = []  # list[SceneNode]

        # Listen for selection changes
        if app and hasattr(app, 'selection_mgr'):
            app.selection_mgr.on_selection_changed(self._on_selection_changed)

    @property
    def visible(self) -> bool:
        return self._visible

    @visible.setter
    def visible(self, val: bool):
        self._visible = val
        if not val:
            self._release()
            self._labels = []
        elif val:
            self._needs_rebuild = True

    def select_connect_point(self, block_id: int, cp_index: int | None):
        """Select a specific connect point (or all if cp_index is None).

        Called by the scene tree when clicking individual CP entries.
        """
        self._selected_cp_block_id = block_id
        self._selected_cp_index = cp_index
        self._visible = True
        self._needs_rebuild = True

    def _on_selection_changed(self, nif_id, block_id):
        """Auto-show connect points when a BSConnectPoint::Parents block is selected."""
        registry = getattr(self.app, 'registry', None)
        if not registry or nif_id is None or block_id is None:
            self._selected_cp_block_id = None
            self._selected_cp_index = None
            if self._visible:
                self._needs_rebuild = True
            return

        try:
            session = registry.get_session(nif_id)
            nif = session.nif
        except KeyError:
            return
        block = nif.get_block(block_id)
        if block and block.type_name in ("BSConnectPoint::Parents", "BSConnectPoint::Children"):
            self._selected_cp_block_id = block_id
            self._selected_cp_index = None  # Show all when selecting the parent block
            self._visible = True
            self._needs_rebuild = True
        else:
            self._selected_cp_block_id = None
            self._selected_cp_index = None
            self._visible = False  # Hide markers when clicking non-CP block
            self._needs_rebuild = True

    def rebuild(self, nif_or_registry, ctx: moderngl.Context, program: moderngl.Program,
                scene_radius: float = 0.0):
        """Rebuild the connect point line geometry from NIF data.

        Accepts either a single NifFile (legacy) or a NifRegistry (multi-NIF).
        When given a registry, iterates all sessions.
        scene_radius scales marker size proportionally to the scene.
        """
        self._release()
        self._labels = []
        self.cp_nodes = []

        # Scale marker size to ~2% of scene radius (clamped to sensible range)
        if scene_radius > 0:
            self._marker_size = max(0.005, scene_radius * 0.02)
        else:
            self._marker_size = self.MARKER_SIZE

        if not nif_or_registry or not self._visible:
            return

        # Detect whether we got a registry or a single NIF
        if hasattr(nif_or_registry, "all_sessions"):
            registry = nif_or_registry
            self._rebuild_multi(registry, ctx, program)
            return

        # Legacy single-NIF path
        nif = nif_or_registry
        self._rebuild_single(nif, ctx, program, nif_id="main")

    def _rebuild_multi(self, registry, ctx, program):
        """Rebuild CP geometry from all NIF sessions."""
        all_vertices = []
        all_colors = []

        for session in registry.all_sessions():
            if not session.nif or not session.nif.blocks:
                continue
            verts, cols = self._collect_cp_geometry(
                session.nif, nif_id=session.nif_id, registry=registry)
            all_vertices.extend(verts)
            all_colors.extend(cols)

        if not all_vertices:
            return

        self._upload_geometry(all_vertices, all_colors, ctx, program)

    def _rebuild_single(self, nif, ctx, program, nif_id="main"):
        """Rebuild CP geometry from a single NIF."""
        verts, cols = self._collect_cp_geometry(nif, nif_id=nif_id)
        if not verts:
            return
        self._upload_geometry(verts, cols, ctx, program)

    def _upload_geometry(self, vertices, colors, ctx, program):
        """Upload vertex/color data to GPU."""
        pos_data = np.array(vertices, dtype=np.float32)
        col_data = np.array(colors, dtype=np.float32)
        self._vbo = ctx.buffer(pos_data.tobytes())
        self._color_vbo = ctx.buffer(col_data.tobytes())
        self._num_vertices = len(vertices) // 3
        self._vao = ctx.vertex_array(program, [
            (self._vbo, "3f", "in_position"),
            (self._color_vbo, "4f", "in_color"),
        ])

    def _collect_cp_geometry(self, nif, nif_id="main", registry=None):
        """Collect CP line vertices and colors for a single NIF."""
        from pathlib import Path as _Path

        vertices = []
        colors = []

        # Build parent map and extra data ownership
        parent_map = {}
        extra_data_owner = {}
        name_to_block = {}  # for Starfield per-CP parent resolution

        # Starfield (bs_version >= 170) stores CP translations relative to
        # the named Parent node, not the CPA block owner.
        is_starfield = getattr(nif.header, 'bs_version', 0) >= 170

        for block in nif.blocks:
            block_name = block.get_field("Name")
            if block_name and isinstance(block_name, str):
                name_to_block[block_name] = block.block_id

            if not nif.schema.is_subtype_of(block.type_name, "NiNode"):
                continue

            children = block.get_field("Children") or []
            for ref in children:
                ref_id = _extract_ref(ref)
                if ref_id >= 0:
                    parent_map[ref_id] = block.block_id

            extra_list = block.get_field("Extra Data List") or []
            for ref in extra_list:
                ref_id = _extract_ref(ref)
                if ref_id >= 0:
                    extra_data_owner[ref_id] = block.block_id

        cp_parent_blocks = [b for b in nif.blocks if b.type_name == "BSConnectPoint::Parents"]
        cp_child_blocks = [b for b in nif.blocks if b.type_name == "BSConnectPoint::Children"]
        if not cp_parent_blocks and not cp_child_blocks:
            return vertices, colors

        # Determine which type to show based on selected block
        selected_block = nif.get_block(self._selected_cp_block_id) if self._selected_cp_block_id is not None else None
        selected_type = selected_block.type_name if selected_block else None
        show_parents = selected_type != "BSConnectPoint::Children"
        show_children = selected_type != "BSConnectPoint::Parents"

        for cp_block in (cp_parent_blocks if show_parents else []):
            fallback_owner_id = extra_data_owner.get(cp_block.block_id)
            if fallback_owner_id is None:
                fallback_owner_id = self._find_owner(nif, cp_block.block_id)
            if fallback_owner_id is None:
                fallback_owner_id = 0

            connect_points = cp_block.get_field("Connect Points") or []
            is_this_block_selected = (cp_block.block_id == self._selected_cp_block_id)

            # Pre-compute owner transform for non-Starfield (shared by all CPs)
            if not is_starfield:
                owner_world_pos = _compute_world_position(nif, fallback_owner_id, parent_map)
                owner_world_rot = _compute_world_rotation(nif, fallback_owner_id, parent_map)

            for i, cp in enumerate(connect_points):
                if not isinstance(cp, dict):
                    continue

                # If a specific CP index is selected, only show that one
                if (is_this_block_selected
                        and self._selected_cp_index is not None
                        and i != self._selected_cp_index):
                    continue

                cp_name = cp.get("Name", f"CP_{i}")
                if isinstance(cp_name, list):
                    cp_name = "".join(str(c) for c in cp_name)

                # Starfield: resolve the named Parent node per-CP
                if is_starfield:
                    cp_parent_name = cp.get("Parent", "")
                    if isinstance(cp_parent_name, list):
                        cp_parent_name = "".join(str(c) for c in cp_parent_name)
                    owner_id = name_to_block.get(cp_parent_name, fallback_owner_id)
                    owner_world_pos = _compute_world_position(nif, owner_id, parent_map)
                    owner_world_rot = _compute_world_rotation(nif, owner_id, parent_map)

                cp_trans = cp.get("Translation", {})
                tx = float(cp_trans.get("x", 0))
                ty = float(cp_trans.get("y", 0))
                tz = float(cp_trans.get("z", 0))

                local_pos = np.array([tx, ty, tz])
                world_pos = owner_world_rot @ local_pos + np.array(owner_world_pos)
                wx, wy, wz = world_pos

                # Is this specific point the selected one?
                is_selected = (is_this_block_selected
                               and (self._selected_cp_index is None
                                    or self._selected_cp_index == i))

                # Cross marker color
                marker_color = self.SELECTED_COLOR if is_selected else self.MARKER_COLOR

                # Cross marker (6 line segments = 12 vertices)
                sz = self._marker_size
                for dx, dy, dz in [(sz, 0, 0), (-sz, 0, 0), (0, sz, 0),
                                    (0, -sz, 0), (0, 0, sz), (0, 0, -sz)]:
                    vertices.extend([wx, wy, wz])
                    colors.extend(marker_color)
                    vertices.extend([wx + dx, wy + dy, wz + dz])
                    colors.extend(marker_color)

                # Line to owner origin (2 vertices)
                ox, oy, oz = owner_world_pos
                vertices.extend([wx, wy, wz])
                colors.extend(self.LINK_COLOR)
                vertices.extend([ox, oy, oz])
                colors.extend(self.LINK_COLOR)

                # Check if a child NIF is attached to this CP
                attached_name = ""
                if registry:
                    for child_s in registry.get_children(nif_id):
                        if child_s.attachment_point == cp_name:
                            attached_name = _Path(child_s.file_path).name
                            break

                # Store label data for imgui overlay
                display_label = f"{cp_name} \u2192 {attached_name}" if attached_name else cp_name
                self._labels.append((display_label, wx, wy, wz, is_selected, False))

                # SceneNode for gizmo + picking
                from creation_lib.renderer.scene_renderer import SceneNode
                cp_node = SceneNode(name=cp_name, block_id=cp_block.block_id, nif_id=nif_id)
                cp_node.world_transform = glm.translate(glm.mat4(1.0), glm.vec3(wx, wy, wz))
                cp_node.bound_center = glm.vec3(wx, wy, wz)
                cp_node.bound_radius = self._marker_size * 3.0
                # Store routing metadata so app.py knows how to write back
                cp_node._cp_block_id = cp_block.block_id
                cp_node._cp_index = i
                cp_node._cp_owner_id = owner_id if is_starfield else fallback_owner_id
                # Compose CP's own quaternion rotation with owner rotation
                cp_rot_q = cp.get("Rotation")
                if cp_rot_q and isinstance(cp_rot_q, dict) and "w" in cp_rot_q:
                    cp_node._cp_owner_world_rot = (owner_world_rot @ _quat_to_matrix(cp_rot_q)).copy()
                else:
                    cp_node._cp_owner_world_rot = owner_world_rot.copy()
                self.cp_nodes.append(cp_node)

        # Process BSConnectPoint::Children blocks
        for cp_block in (cp_child_blocks if show_children else []):
            owner_id = extra_data_owner.get(cp_block.block_id)
            if owner_id is None:
                owner_id = self._find_owner(nif, cp_block.block_id)
            if owner_id is None:
                owner_id = 0

            owner_world_pos = _compute_world_position(nif, owner_id, parent_map)
            is_this_block_selected = (cp_block.block_id == self._selected_cp_block_id)

            # Children have "Point Name" field — an array of string names
            point_names = cp_block.get_field("Point Name") or []
            if isinstance(point_names, str):
                point_names = [point_names]

            for i, name in enumerate(point_names):
                if not isinstance(name, str):
                    name = str(name)

                # If a specific CP index is selected, only show that one
                if (is_this_block_selected
                        and self._selected_cp_index is not None
                        and i != self._selected_cp_index):
                    continue

                # Children don't have their own transforms — place at owner position
                # Offset slightly per index so multiple names don't overlap
                ox, oy, oz = owner_world_pos
                wx = ox + i * self._marker_size * 0.5
                wy = oy
                wz = oz

                is_selected = (is_this_block_selected
                               and (self._selected_cp_index is None
                                    or self._selected_cp_index == i))

                marker_color = self.SELECTED_COLOR if is_selected else self.CHILD_MARKER_COLOR

                # Cross marker
                sz = self._marker_size
                for dx, dy, dz in [(sz, 0, 0), (-sz, 0, 0), (0, sz, 0),
                                    (0, -sz, 0), (0, 0, sz), (0, 0, -sz)]:
                    vertices.extend([wx, wy, wz])
                    colors.extend(marker_color)
                    vertices.extend([wx + dx, wy + dy, wz + dz])
                    colors.extend(marker_color)

                # Label — mark as child with "C:" prefix for clarity
                label = f"C:{name}" if not name.startswith("C-") else name
                self._labels.append((label, wx, wy, wz, is_selected, True))

        return vertices, colors

    def render(self, program: moderngl.Program, mvp_tuple):
        """Draw connect point lines."""
        if not self._visible or not self._vao:
            return
        program["u_mvp"].value = mvp_tuple
        self._vao.render(moderngl.LINES)

    def draw_labels(self, vp_matrix, viewport_pos, viewport_size):
        """Draw connect point names as imgui overlay text, after the FBO image.

        ``vp_matrix`` is the glm view*projection matrix; ``viewport_pos`` is the
        viewport's top-left imgui screen position.
        """
        if not self._visible or not self._labels:
            return

        from imgui_bundle import imgui
        import glm

        draw_list = imgui.get_window_draw_list()
        vp_x = viewport_pos.x
        vp_y = viewport_pos.y
        vp_w = viewport_size.x
        vp_h = viewport_size.y

        for name, wx, wy, wz, is_selected, is_child in self._labels:
            # Project world position to clip space
            clip = vp_matrix * glm.vec4(wx, wy, wz, 1.0)
            if clip.w <= 0.001:
                continue  # Behind camera

            ndc_x = clip.x / clip.w
            ndc_y = clip.y / clip.w
            ndc_z = clip.z / clip.w

            # Skip if outside NDC range
            if ndc_z < -1.0 or ndc_z > 1.0:
                continue

            # NDC to screen (imgui coordinates)
            screen_x = vp_x + (ndc_x * 0.5 + 0.5) * vp_w
            screen_y = vp_y + (1.0 - (ndc_y * 0.5 + 0.5)) * vp_h

            # Offset label above the marker
            screen_y -= 16.0

            # Draw text with shadow
            if is_selected:
                color = imgui.color_convert_float4_to_u32(
                    imgui.ImVec4(*self.LABEL_SELECTED_COLOR))
            elif is_child:
                color = imgui.color_convert_float4_to_u32(
                    imgui.ImVec4(*self.CHILD_LABEL_COLOR))
            else:
                color = imgui.color_convert_float4_to_u32(
                    imgui.ImVec4(*self.LABEL_COLOR))

            shadow_color = imgui.color_convert_float4_to_u32(
                imgui.ImVec4(0.0, 0.0, 0.0, 0.8))

            # Center text
            text_size = imgui.calc_text_size(name)
            tx = screen_x - text_size.x * 0.5
            ty = screen_y

            draw_list.add_text(imgui.ImVec2(tx + 1, ty + 1), shadow_color, name)
            draw_list.add_text(imgui.ImVec2(tx, ty), color, name)

    def _find_owner(self, nif, extra_data_id):
        """Find the NiNode that references this extra data block."""
        for block in nif.blocks:
            if not nif.schema.is_subtype_of(block.type_name, "NiNode"):
                continue
            extra_list = block.get_field("Extra Data List") or []
            for ref in extra_list:
                if _extract_ref(ref) == extra_data_id:
                    return block.block_id
        return None

    def _release(self):
        if self._vbo:
            self._vbo.release()
            self._vbo = None
        if self._color_vbo:
            self._color_vbo.release()
            self._color_vbo = None
        if self._vao:
            self._vao.release()
            self._vao = None
        self._num_vertices = 0

    def destroy(self):
        self._release()


def compute_cp_world_transform(nif, cp_name: str):
    """Compute the world-space position and rotation for a named connect point.

    Returns (world_pos, world_rot) as ((x,y,z) tuple, 3x3 numpy array),
    or (None, None) if the CP is not found.

    Exported for use by app.py to sync AttachmentNode transforms.
    """
    # Build parent map
    parent_map = {}
    extra_data_owner = {}
    name_to_block = {}
    is_starfield = getattr(nif.header, 'bs_version', 0) >= 170

    for block in nif.blocks:
        block_name = block.get_field("Name")
        if block_name and isinstance(block_name, str):
            name_to_block[block_name] = block.block_id
        if not nif.schema.is_subtype_of(block.type_name, "NiNode"):
            continue
        children = block.get_field("Children") or []
        for ref in children:
            ref_id = _extract_ref(ref)
            if ref_id >= 0:
                parent_map[ref_id] = block.block_id
        extra_list = block.get_field("Extra Data List") or []
        for ref in extra_list:
            ref_id = _extract_ref(ref)
            if ref_id >= 0:
                extra_data_owner[ref_id] = block.block_id

    for block in nif.blocks:
        if block.type_name != "BSConnectPoint::Parents":
            continue
        connect_points = block.get_field("Connect Points") or []
        fallback_owner_id = extra_data_owner.get(block.block_id, 0)

        for cp in connect_points:
            if not isinstance(cp, dict):
                continue
            name = cp.get("Name", "")
            if isinstance(name, list):
                name = "".join(str(c) for c in name)
            if name != cp_name:
                continue

            # Starfield: resolve per-CP named parent node
            if is_starfield:
                cp_parent_name = cp.get("Parent", "")
                if isinstance(cp_parent_name, list):
                    cp_parent_name = "".join(str(c) for c in cp_parent_name)
                owner_id = name_to_block.get(cp_parent_name, fallback_owner_id)
            else:
                owner_id = fallback_owner_id

            owner_world_pos = _compute_world_position(nif, owner_id, parent_map)
            owner_world_rot = _compute_world_rotation(nif, owner_id, parent_map)

            cp_trans = cp.get("Translation", {})
            local_pos = np.array([
                float(cp_trans.get("x", 0)),
                float(cp_trans.get("y", 0)),
                float(cp_trans.get("z", 0)),
            ])
            world_pos = owner_world_rot @ local_pos + np.array(owner_world_pos)

            # Apply the CP's own quaternion rotation
            cp_rot_q = cp.get("Rotation")
            if cp_rot_q and isinstance(cp_rot_q, dict) and "w" in cp_rot_q:
                cp_rot = _quat_to_matrix(cp_rot_q)
                world_rot = owner_world_rot @ cp_rot
            else:
                world_rot = owner_world_rot
            return tuple(world_pos), world_rot

    return None, None


def _quat_to_matrix(q: dict) -> np.ndarray:
    """Convert a quaternion dict {w, x, y, z} to a 3x3 rotation matrix."""
    w = float(q.get("w", 1))
    x = float(q.get("x", 0))
    y = float(q.get("y", 0))
    z = float(q.get("z", 0))
    # Normalize
    n = (w * w + x * x + y * y + z * z) ** 0.5
    if n > 1e-12:
        w, x, y, z = w / n, x / n, y / n, z / n
    return np.array([
        [1 - 2*(y*y + z*z),     2*(x*y - z*w),     2*(x*z + y*w)],
        [    2*(x*y + z*w), 1 - 2*(x*x + z*z),     2*(y*z - x*w)],
        [    2*(x*z - y*w),     2*(y*z + x*w), 1 - 2*(x*x + y*y)],
    ], dtype=np.float64)


def _extract_ref(ref) -> int:
    """Extract block index from a reference value."""
    if isinstance(ref, (int, float)):
        return int(ref)
    if isinstance(ref, dict):
        return int(ref.get("value", ref.get("Value", -1)))
    return -1


def _compute_world_position(nif, block_id, parent_map):
    """Compute world-space position by accumulating transforms up the chain."""
    chain = []
    bid = block_id
    while bid is not None:
        chain.append(bid)
        bid = parent_map.get(bid)
    chain.reverse()

    wx, wy, wz = 0.0, 0.0, 0.0
    cum_rot = np.eye(3, dtype=np.float64)
    cum_scale = 1.0

    for bid in chain:
        block = nif.get_block(bid)
        if not block:
            continue

        trans = block.get_field("Translation") or {}
        tx = float(trans.get("x", 0))
        ty = float(trans.get("y", 0))
        tz = float(trans.get("z", 0))

        scale = float(block.get_field("Scale") or 1.0)

        local = np.array([tx, ty, tz])
        world_t = cum_rot @ (local * cum_scale) + np.array([wx, wy, wz])
        wx, wy, wz = world_t

        rot = block.get_field("Rotation") or {}
        # nif.xml Matrix33 names use m[col][row]; transpose to [row][col]
        local_rot = np.array([
            [float(rot.get("m11", 1)), float(rot.get("m21", 0)), float(rot.get("m31", 0))],
            [float(rot.get("m12", 0)), float(rot.get("m22", 1)), float(rot.get("m32", 0))],
            [float(rot.get("m13", 0)), float(rot.get("m23", 0)), float(rot.get("m33", 1))],
        ])
        cum_rot = cum_rot @ local_rot
        cum_scale *= scale

    return (wx, wy, wz)


def _compute_world_rotation(nif, block_id, parent_map):
    """Compute cumulative world rotation matrix for a block."""
    chain = []
    bid = block_id
    while bid is not None:
        chain.append(bid)
        bid = parent_map.get(bid)
    chain.reverse()

    cum_rot = np.eye(3, dtype=np.float64)

    for bid in chain:
        block = nif.get_block(bid)
        if not block:
            continue

        rot = block.get_field("Rotation") or {}
        # nif.xml Matrix33 names use m[col][row]; transpose to [row][col]
        local_rot = np.array([
            [float(rot.get("m11", 1)), float(rot.get("m21", 0)), float(rot.get("m31", 0))],
            [float(rot.get("m12", 0)), float(rot.get("m22", 1)), float(rot.get("m32", 0))],
            [float(rot.get("m13", 0)), float(rot.get("m23", 0)), float(rot.get("m33", 1))],
        ])
        cum_rot = cum_rot @ local_rot

    return cum_rot

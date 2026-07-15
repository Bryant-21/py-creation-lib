"""Worldspace placement extraction and export planning."""

from __future__ import annotations

import struct
import json
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Callable, Iterable

from creation_lib.esp.model import Group, Record


PLACED_REFERENCE_SIGNATURES = frozenset({
    "REFR",
    "ACHR",
    "PGRE",
    "PMIS",
    "PHZD",
    "PARW",
    "PBAR",
    "PBEA",
    "PCON",
    "PFLA",
})

WORLD_CHILDREN_GROUP_TYPE = 1
CELL_CHILDREN_GROUP_TYPES = frozenset({6, 8, 9, 10})


@dataclass(frozen=True)
class WorldspaceInfo:
    form_id: int
    editor_id: str
    name: str
    plugin_name: str = ""


@dataclass(frozen=True)
class CellInfo:
    form_id: int
    editor_id: str
    worldspace_form_id: int
    plugin_name: str = ""


@dataclass(frozen=True)
class PlacementTransform:
    position: tuple[float, float, float]
    rotation: tuple[float, float, float]
    scale: float = 1.0


@dataclass(frozen=True)
class PlacementEntry:
    source_form_id: int
    base_form_id: int
    plugin_name: str
    worldspace_form_id: int
    cell_form_id: int
    model_path: str
    transform: PlacementTransform


@dataclass(frozen=True)
class ResolvedPlacement:
    entry: PlacementEntry
    resolved_mesh_path: Path
    transform: PlacementTransform


@dataclass(frozen=True)
class SkippedPlacement:
    entry: PlacementEntry
    reason: str


@dataclass(frozen=True)
class ExportManifest:
    origin: tuple[float, float, float]
    placements: list[ResolvedPlacement]
    skipped: list[SkippedPlacement]


@dataclass
class LoadedPluginBundle:
    active_plugin: object
    plugins: list[object]

    def __post_init__(self) -> None:
        self._record_index: dict[tuple[str, int], Record] = {}
        for plugin in self.plugins:
            plugin_name = str(getattr(plugin, "plugin_name", "")).lower()
            for record in getattr(plugin, "records", []):
                self._record_index[(plugin_name, int(record.object_id))] = record

    def list_worldspaces(self) -> list[WorldspaceInfo]:
        return list_worldspaces(
            getattr(self.active_plugin, "root_items", []),
            plugin_name=getattr(self.active_plugin, "plugin_name", ""),
        )

    def list_cells(self, worldspace_form_id: int) -> list[CellInfo]:
        return list_cells(
            getattr(self.active_plugin, "root_items", []),
            worldspace_form_id,
            plugin_name=getattr(self.active_plugin, "plugin_name", ""),
        )

    def resolve_base_record(self, raw_form_id: int) -> Record | None:
        normalized = self.active_plugin.normalize_form_id(raw_form_id)
        plugin_name = normalized.plugin_name or getattr(self.active_plugin, "plugin_name", "")
        return self._record_index.get((plugin_name.lower(), normalized.object_id))

    def extract_placements(
        self,
        worldspace_form_id: int,
        cell_form_ids: set[int] | None = None,
    ) -> list[PlacementEntry]:
        return extract_placements(
            getattr(self.active_plugin, "root_items", []),
            plugin_name=getattr(self.active_plugin, "plugin_name", ""),
            resolve_base_record=self.resolve_base_record,
            worldspace_form_id=worldspace_form_id,
            cell_form_ids=cell_form_ids,
        )


@dataclass(frozen=True)
class WorldspaceExportResult:
    fbx_path: Path
    manifest_path: Path
    placement_count: int
    skipped_count: int


def load_plugin_bundle(
    plugin_path: str | Path,
    *,
    game: str | None = None,
    master_search_paths: Iterable[str | Path] = (),
) -> LoadedPluginBundle:
    from creation_lib.esp import Plugin

    plugin_file = Path(plugin_path)
    search_paths = [plugin_file.parent] + [Path(path) for path in master_search_paths]
    loaded: dict[str, object] = {}

    def load_one(path: Path) -> object:
        plugin = Plugin.load(path, game=game)
        _materialize_plugin(plugin)
        loaded[plugin.plugin_name.lower()] = plugin
        for master_name in plugin.header.masters:
            key = master_name.lower()
            if key in loaded:
                continue
            master_path = _find_named_file(master_name, search_paths)
            if master_path is not None:
                load_one(master_path)
        return plugin

    active = load_one(plugin_file)
    plugins = list(loaded.values())
    plugins.sort(key=lambda plugin: 0 if plugin is not active else 1)
    return LoadedPluginBundle(active_plugin=active, plugins=plugins)


def export_manifest(
    manifest: ExportManifest,
    output_path: str | Path,
    *,
    fbx_exporter: Callable[[ExportManifest, Path], Path | str | None] | None = None,
) -> WorldspaceExportResult:
    fbx_path = Path(output_path)
    fbx_path.parent.mkdir(parents=True, exist_ok=True)
    if fbx_exporter is None:
        fbx_exporter = _default_fbx_exporter

    exported = fbx_exporter(manifest, fbx_path)
    if exported is None:
        raise RuntimeError("FBX export failed")

    manifest_path = fbx_path.with_suffix(".worldspace.json")
    manifest_path.write_text(
        json.dumps(_manifest_to_dict(manifest), indent=2),
        encoding="utf-8",
    )
    return WorldspaceExportResult(
        fbx_path=Path(exported),
        manifest_path=manifest_path,
        placement_count=len(manifest.placements),
        skipped_count=len(manifest.skipped),
    )


def parse_reference_transform(record) -> PlacementTransform:
    """Read placed-reference position, rotation, and scale subrecords."""
    data = record.get_subrecord("DATA")
    position = (0.0, 0.0, 0.0)
    rotation = (0.0, 0.0, 0.0)
    if data is not None and len(data.data) >= 24:
        values = struct.unpack_from("<6f", data.data, 0)
        position = (float(values[0]), float(values[1]), float(values[2]))
        rotation = (float(values[3]), float(values[4]), float(values[5]))

    scale = 1.0
    xscl = record.get_subrecord("XSCL")
    if xscl is not None and len(xscl.data) >= 4:
        scale = float(struct.unpack_from("<f", xscl.data, 0)[0])

    return PlacementTransform(position, rotation, scale)


def extract_model_path(record) -> str | None:
    signatures = ("MOD2", "MODL", "MOD3", "MOD4") if record.signature == "ARMO" else (
        "MODL",
        "MOD2",
        "MOD3",
        "MOD4",
    )
    for signature in signatures:
        subrecord = record.get_subrecord(signature)
        if subrecord is None:
            continue
        value = subrecord.get_string().strip()
        if value:
            return canonical_model_path(value)
    return None


def canonical_model_path(path: str) -> str:
    normalized = path.replace("\\", "/").strip().lower()
    while normalized.startswith("/"):
        normalized = normalized[1:]
    if normalized.startswith("meshes/"):
        normalized = normalized[len("meshes/"):]
    return normalized


def resolve_mesh_path(model_path: str, mesh_roots: Iterable[str | Path]) -> Path | None:
    rel_path = Path(canonical_model_path(model_path))
    for root in mesh_roots:
        root_path = Path(root)
        candidates = [
            root_path / rel_path,
            root_path / "Meshes" / rel_path,
            root_path / "meshes" / rel_path,
        ]
        for candidate in candidates:
            if candidate.is_file():
                return candidate
            resolved = _case_insensitive_path(candidate)
            if resolved is not None and resolved.is_file():
                return resolved
    return None


def build_export_manifest(
    placements: Iterable[PlacementEntry],
    mesh_roots: Iterable[str | Path],
    *,
    normalize_origin: bool = True,
) -> ExportManifest:
    placement_list = list(placements)
    origin = _average_position(placement_list) if normalize_origin else (0.0, 0.0, 0.0)

    resolved: list[ResolvedPlacement] = []
    skipped: list[SkippedPlacement] = []
    for entry in placement_list:
        mesh_path = resolve_mesh_path(entry.model_path, mesh_roots)
        if mesh_path is None:
            skipped.append(SkippedPlacement(entry, "missing_mesh"))
            continue
        transform = _offset_transform(entry.transform, origin)
        resolved.append(ResolvedPlacement(entry, mesh_path, transform))

    return ExportManifest(origin, resolved, skipped)


def list_worldspaces(
    root_items: Iterable[Group | Record],
    *,
    plugin_name: str = "",
) -> list[WorldspaceInfo]:
    worldspaces: list[WorldspaceInfo] = []
    for record, _worldspace_id, _cell_id in _walk_records(root_items):
        if record.signature != "WRLD":
            continue
        worldspaces.append(
            WorldspaceInfo(
                form_id=int(record.form_id),
                editor_id=_record_text(record, "EDID"),
                name=_record_text(record, "FULL"),
                plugin_name=plugin_name,
            )
        )
    return worldspaces


def list_cells(
    root_items: Iterable[Group | Record],
    worldspace_form_id: int,
    *,
    plugin_name: str = "",
) -> list[CellInfo]:
    cells: list[CellInfo] = []
    for record, active_worldspace_id, _cell_id in _walk_records(root_items):
        if record.signature != "CELL" or active_worldspace_id != worldspace_form_id:
            continue
        cells.append(
            CellInfo(
                form_id=int(record.form_id),
                editor_id=_record_text(record, "EDID"),
                worldspace_form_id=worldspace_form_id,
                plugin_name=plugin_name,
            )
        )
    return cells


def extract_placements(
    root_items: Iterable[Group | Record],
    *,
    plugin_name: str,
    resolve_base_record: Callable[[int], Record | None],
    worldspace_form_id: int,
    cell_form_ids: set[int] | None = None,
) -> list[PlacementEntry]:
    selected_cells = set(cell_form_ids or set())
    placements: list[PlacementEntry] = []
    for record, active_worldspace_id, active_cell_id in _walk_records(root_items):
        if record.signature not in PLACED_REFERENCE_SIGNATURES:
            continue
        if active_worldspace_id != worldspace_form_id:
            continue
        if selected_cells and active_cell_id not in selected_cells:
            continue

        name = record.get_subrecord("NAME")
        if name is None or len(name.data) < 4:
            continue
        base_form_id = int(struct.unpack_from("<I", name.data, 0)[0])
        base_record = resolve_base_record(base_form_id)
        if base_record is None:
            continue
        model_path = extract_model_path(base_record)
        if not model_path:
            continue

        placements.append(
            PlacementEntry(
                source_form_id=int(record.form_id),
                base_form_id=base_form_id,
                plugin_name=plugin_name,
                worldspace_form_id=worldspace_form_id,
                cell_form_id=int(active_cell_id or 0),
                model_path=model_path,
                transform=parse_reference_transform(record),
            )
        )
    return placements


def _average_position(
    placements: list[PlacementEntry],
) -> tuple[float, float, float]:
    if not placements:
        return (0.0, 0.0, 0.0)
    total_x = total_y = total_z = 0.0
    for entry in placements:
        x, y, z = entry.transform.position
        total_x += x
        total_y += y
        total_z += z
    count = float(len(placements))
    return (total_x / count, total_y / count, total_z / count)


def _offset_transform(
    transform: PlacementTransform,
    origin: tuple[float, float, float],
) -> PlacementTransform:
    x, y, z = transform.position
    ox, oy, oz = origin
    return replace(transform, position=(x - ox, y - oy, z - oz))


def _walk_records(
    items: Iterable[Group | Record],
    *,
    worldspace_id: int | None = None,
    cell_id: int | None = None,
):
    for item in items:
        if isinstance(item, Record):
            yield item, worldspace_id, cell_id
            continue

        next_worldspace_id = worldspace_id
        next_cell_id = cell_id
        if item.group_type == WORLD_CHILDREN_GROUP_TYPE:
            next_worldspace_id = _group_label_form_id(item)
            next_cell_id = None
        elif item.group_type in CELL_CHILDREN_GROUP_TYPES:
            next_cell_id = _group_label_form_id(item)
        yield from _walk_records(
            item.children,
            worldspace_id=next_worldspace_id,
            cell_id=next_cell_id,
        )


def _group_label_form_id(group: Group) -> int:
    if len(group.label) < 4:
        return 0
    return int(struct.unpack_from("<I", group.label, 0)[0])


def _record_text(record: Record, signature: str) -> str:
    subrecord = record.get_subrecord(signature)
    if subrecord is None:
        return ""
    return subrecord.get_string().strip()


def _materialize_plugin(plugin: object) -> None:
    _ = getattr(plugin, "root_items", [])
    _ = getattr(getattr(plugin, "header", None), "masters", [])


def _find_named_file(name: str, roots: Iterable[Path]) -> Path | None:
    for root in roots:
        candidates = [root / name, root / "Data" / name]
        for candidate in candidates:
            if candidate.is_file():
                return candidate
            resolved = _case_insensitive_path(candidate)
            if resolved is not None and resolved.is_file():
                return resolved
    return None


def _default_fbx_exporter(manifest: ExportManifest, output_path: Path) -> Path | None:
    from creation_lib.fbx.nif_to_fbx import export_worldspace_manifest_to_fbx

    result = export_worldspace_manifest_to_fbx(manifest, str(output_path))
    return Path(result) if result else None


def _manifest_to_dict(manifest: ExportManifest) -> dict:
    return {
        "origin": list(manifest.origin),
        "placements": [
            {
                "source_form_id": _format_form_id(item.entry.source_form_id),
                "base_form_id": _format_form_id(item.entry.base_form_id),
                "plugin_name": item.entry.plugin_name,
                "worldspace_form_id": _format_form_id(item.entry.worldspace_form_id),
                "cell_form_id": _format_form_id(item.entry.cell_form_id),
                "model_path": item.entry.model_path,
                "resolved_mesh_path": str(item.resolved_mesh_path),
                "position": list(item.transform.position),
                "rotation": list(item.transform.rotation),
                "scale": item.transform.scale,
            }
            for item in manifest.placements
        ],
        "skipped": [
            {
                "source_form_id": _format_form_id(item.entry.source_form_id),
                "base_form_id": _format_form_id(item.entry.base_form_id),
                "plugin_name": item.entry.plugin_name,
                "worldspace_form_id": _format_form_id(item.entry.worldspace_form_id),
                "cell_form_id": _format_form_id(item.entry.cell_form_id),
                "model_path": item.entry.model_path,
                "reason": item.reason,
            }
            for item in manifest.skipped
        ],
    }


def _format_form_id(form_id: int) -> str:
    value = int(form_id) & 0xFFFFFFFF
    width = 6 if value <= 0x00FF_FFFF else 8
    return f"{value:0{width}X}"


def _case_insensitive_path(path: Path) -> Path | None:
    parts = path.parts
    if not parts:
        return None
    current = Path(parts[0])
    for part in parts[1:]:
        if not current.exists() or not current.is_dir():
            return None
        try:
            matches = {child.name.lower(): child for child in current.iterdir()}
        except OSError:
            return None
        match = matches.get(part.lower())
        if match is None:
            return None
        current = match
    return current

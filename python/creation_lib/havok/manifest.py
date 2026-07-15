"""Havok manifest assembly binding for the native Havok backend."""
from __future__ import annotations

import dataclasses
from dataclasses import dataclass, field

from creation_lib.havok.discovery import FileEntry


@dataclass
class ManifestFileEntry:
    """A file in a manifest."""

    file_path: str
    file_type: str
    role: str
    ref_type: str = "owned"
    file_size: int = 0


@dataclass
class ManifestDep:
    """A dependency on another manifest."""

    depends_on: str
    dep_type: str


@dataclass
class ManifestData:
    """A complete asset manifest."""

    id: str
    name: str
    manifest_type: str
    source: str
    project_id: str = ""
    files: list[ManifestFileEntry] = field(default_factory=list)
    dependencies: list[ManifestDep] = field(default_factory=list)
    file_count: int = 0
    total_size: int = 0


def _entry_payload(entry: FileEntry) -> dict:
    return {
        "rel_path": entry.rel_path,
        "role": entry.role,
        "category": entry.category,
        "file_type": entry.file_type,
        "is_xml": entry.is_xml,
    }


def _character_payload(value: object) -> dict:
    if dataclasses.is_dataclass(value):
        return dataclasses.asdict(value)
    if isinstance(value, dict):
        return value
    return {
        "rig_name": getattr(value, "rig_name", ""),
        "behavior_filename": getattr(value, "behavior_filename", ""),
        "model_up": getattr(value, "model_up", ""),
        "model_forward": getattr(value, "model_forward", ""),
        "model_right": getattr(value, "model_right", ""),
    }


def build_manifests(
    entries: list[FileEntry],
    character_data: dict[str, object],
    source: str,
) -> list[ManifestData]:
    """Build manifests via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import build_manifests_native

    size_by_rel_path = {
        entry.rel_path: entry.abs_path.stat().st_size if entry.abs_path.exists() else 0
        for entry in entries
    }
    raw_manifests = build_manifests_native(
        [_entry_payload(entry) for entry in entries],
        {key: _character_payload(value) for key, value in character_data.items()},
        source,
    )

    manifests: list[ManifestData] = []
    for raw in raw_manifests:
        files = [
            ManifestFileEntry(
                file_path=str(file_entry.get("file_path", "")),
                file_type=str(file_entry.get("file_type", "")),
                role=str(file_entry.get("role", "")),
                ref_type=str(file_entry.get("ref_type", "owned")),
                file_size=int(size_by_rel_path.get(str(file_entry.get("file_path", "")), file_entry.get("file_size", 0))),
            )
            for file_entry in raw.get("files", [])
        ]
        dependencies = [
            ManifestDep(
                depends_on=str(dep.get("depends_on", "")),
                dep_type=str(dep.get("dep_type", "")),
            )
            for dep in raw.get("dependencies", [])
        ]
        manifests.append(
            ManifestData(
                id=str(raw.get("id", "")),
                name=str(raw.get("name", "")),
                manifest_type=str(raw.get("manifest_type", "")),
                source=str(raw.get("source", source)),
                project_id=str(raw.get("project_id", "")),
                files=files,
                dependencies=dependencies,
                file_count=int(raw.get("file_count", len(files))),
                total_size=sum(file.file_size for file in files),
            )
        )
    return manifests

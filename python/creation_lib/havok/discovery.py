"""Havok asset discovery bindings for the native Havok backend."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass
class FileEntry:
    """A discovered file with its classification."""

    abs_path: Path
    rel_path: str
    role: str
    category: str
    file_type: str
    is_xml: bool


def classify_category(rel_path: str) -> str:
    """Classify a file category via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import classify_category_native

    return classify_category_native(rel_path)


def classify_role(rel_path: str, meshes_dir: Path | None = None) -> str:
    """Classify a file role via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import classify_role_native, walk_meshes_dir_native

    if meshes_dir is not None:
        normalized = rel_path.replace("\\", "/")
        for entry in walk_meshes_dir_native(str(meshes_dir), ""):
            if entry.get("rel_path") == normalized:
                return str(entry.get("role", "unknown"))
        return "unknown"
    return classify_role_native(rel_path)


def discover_havok_files(meshes_dir: Path, source: str = "fo4") -> list[FileEntry]:
    """Walk a meshes directory via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import walk_meshes_dir_native

    root = Path(meshes_dir)
    entries = []
    for entry in walk_meshes_dir_native(str(root), source):
        rel_path = str(entry.get("rel_path", ""))
        entries.append(
            FileEntry(
                abs_path=root / rel_path.replace("/", "\\"),
                rel_path=rel_path,
                role=str(entry.get("role", "")),
                category=str(entry.get("category", "")),
                file_type=str(entry.get("file_type", "")),
                is_xml=bool(entry.get("is_xml", False)),
            )
        )
    return entries

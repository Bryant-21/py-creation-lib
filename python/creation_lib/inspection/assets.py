from __future__ import annotations

import io
from pathlib import Path, PurePosixPath

from creation_lib.ba2 import native_runtime as archives


_ROOTS = {".nif": "meshes", ".hkx": "meshes", ".kf": "meshes", ".dds": "textures",
          ".bgsm": "materials", ".bgem": "materials", ".mat": "materials", ".pex": "scripts",
          ".wav": "sound", ".xwm": "sound", ".fuz": "sound", ".swf": "interface"}


def asset_key(path):
    text = str(path).replace("\\", "/")
    pure = PurePosixPath(text)
    if pure.is_absolute() or ":" in text or ".." in pure.parts or "\0" in text:
        raise ValueError(f"Asset must be a Data-relative path: {path}")
    root = _ROOTS.get(pure.suffix.casefold())
    if root and pure.parts and pure.parts[0].casefold() != root:
        text = root + "/" + text
    return text.casefold()


class AssetResolver:
    def __init__(self, roots=(), archive_paths=()):
        self.roots = [Path(p).resolve() for p in roots]
        self.archives = [(Path(p).resolve(), {str(member).replace("\\", "/").casefold(): member
                                            for member in archives.list_archive(str(p))}) for p in archive_paths]
        self._directories = {}
        self._resolved = {}

    def _loose_path(self, root, key):
        parts = PurePosixPath(key).parts
        if parts and root.name.casefold() == parts[0]:
            parts = parts[1:]
        current = root
        for part in parts:
            if current not in self._directories:
                self._directories[current] = {p.name.casefold(): p for p in current.iterdir()} if current.is_dir() else {}
            current = self._directories[current].get(part)
            if current is None:
                return None
        return current if current.is_file() else None

    def _locations(self, key):
        locations = []
        for root in reversed(self.roots):
            path = self._loose_path(root, key)
            if path:
                locations.append({"kind": "loose", "path": str(path), "root": str(root)})
        for archive, members in reversed(self.archives):
            if key in members:
                locations.append({"kind": "archive", "path": str(archive), "member": members[key]})
        return locations

    def resolve(self, path):
        if path in self._resolved:
            return self._resolved[path]
        try:
            key = asset_key(path)
        except ValueError as error:
            return {"path": path, "status": "invalid_path", "locations": [], "winner": None, "issue": str(error)}
        locations = self._locations(key)
        result = {"path": path, "asset_key": key, "status": "available" if locations else "missing",
                  "locations": locations, "winner": locations[0] if locations else None}
        if not locations:
            trimmed = "/".join(part.strip() for part in key.split("/"))
            alternatives = self._locations(trimmed) if trimmed != key else []
            if alternatives:
                result.update(status="malformed_path", repair_candidate=trimmed, repair_locations=alternatives)
        self._resolved[path] = result
        return result

    def read(self, result):
        winner = result.get("winner")
        if not winner:
            raise FileNotFoundError(result["path"])
        if winner["kind"] == "loose":
            return Path(winner["path"]).read_bytes()
        data = archives.extract_one(winner["path"], winner["member"])
        if data is None:
            raise FileNotFoundError(f"{winner['path']}:{winner['member']}")
        return data

    def describe(self):
        return {"roots": [str(p) for p in self.roots], "archives": [str(p) for p, _ in self.archives],
                "precedence": "Loose files before archives; later supplied roots/archives win. This describes the supplied configuration."}


def asset_dependencies(path, data):
    suffix = PurePosixPath(path.replace("\\", "/")).suffix.casefold()
    if suffix == ".nif":
        from creation_lib.nif.native_runtime import nif_from_bytes_raw
        nif = nif_from_bytes_raw(data)
        found = []

        def walk(value, field):
            if isinstance(value, str) and PurePosixPath(value.replace("\\", "/")).suffix.casefold() in _ROOTS:
                found.append({"path": value, "field": field})
            elif isinstance(value, dict):
                for key, item in value.items():
                    walk(item, f"{field}.{key}")
            elif isinstance(value, list):
                for index, item in enumerate(value):
                    walk(item, f"{field}.{index}")

        for index, block in enumerate(nif.get("blocks", [])):
            walk(block.get("fields", {}), f"blocks.{index}.fields")
        return found
    if suffix in {".bgsm", ".bgem"}:
        from creation_lib.material_tools.extract_textures import BGSM_TEXTURE_SLOTS, BGEM_TEXTURE_SLOTS
        from creation_lib.material_tools.bgsm_bin import read_bgsm
        from creation_lib.material_tools.bgem_bin import read_bgem
        material = read_bgsm(io.BytesIO(data)) if suffix == ".bgsm" else read_bgem(io.BytesIO(data))
        slots = BGSM_TEXTURE_SLOTS if suffix == ".bgsm" else BGEM_TEXTURE_SLOTS
        dependencies = []
        for slot in [*slots, "RootMaterialPath"]:
            path = (getattr(material, slot, None) or "").rstrip("\0")
            if path:
                dependencies.append({"path": path, "field": slot})
        return dependencies
    return []

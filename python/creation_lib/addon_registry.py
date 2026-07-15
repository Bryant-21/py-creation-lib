"""Global AddonNode index registry.

Tracks allocated AddonNode NodeIndex values across all mods to prevent
collisions.  Persisted as ``mods/.addon_registry.json``.

NodeIndex is the integer stored in both the AddonNode record's ``NodeIndex``
field and the matching ``BSValueNode.Value`` inside a NIF.  Each value must
be unique across ALL plugins loaded by the engine.
"""
from __future__ import annotations

import glob as glob_mod
import json
import logging
import os
import re

from creation_lib import yaml_util

_log = logging.getLogger("conversion.addon_registry")

_REGISTRY_FILENAME = ".addon_registry.json"
_DEFAULT_START_INDEX = 20000


class AddonNodeRegistry:
    """JSON-backed registry of allocated AddonNode indices."""

    def __init__(
        self,
        mods_dir: str,
        start_index: int | None = None,
        *,
        registry_filename: str = _REGISTRY_FILENAME,
        seed_from_mods: bool = True,
    ):
        self._mods_dir = mods_dir
        self._path = os.path.join(mods_dir, registry_filename)
        self._start_index = start_index or _DEFAULT_START_INDEX
        self._seed_from_mods = seed_from_mods
        self._allocations: dict[int, dict] = {}  # index -> {mod, editor_id, game}
        self._next_index: int = self._start_index
        self._loaded = False

    # ------------------------------------------------------------------
    # Persistence
    # ------------------------------------------------------------------

    def load(self) -> None:
        """Load registry from disk, seeding from existing mods if first use."""
        if os.path.isfile(self._path):
            try:
                with open(self._path, encoding="utf-8") as f:
                    data = json.load(f)
                for idx_str, info in data.get("allocations", {}).items():
                    self._allocations[int(idx_str)] = info
                self._next_index = data.get("next_index", self._start_index)
                _log.info(
                    "Loaded addon registry: %d allocations, next=%d",
                    len(self._allocations), self._next_index,
                )
            except (json.JSONDecodeError, ValueError) as e:
                _log.warning("Failed to load addon registry: %s", e)
        elif self._seed_from_mods:
            # First use — seed from existing mods
            self.seed_from_mods()
        self._loaded = True

    def save(self) -> None:
        """Write registry to disk."""
        data = {
            "version": 1,
            "allocations": {str(k): v for k, v in sorted(self._allocations.items())},
            "next_index": self._next_index,
        }
        os.makedirs(os.path.dirname(self._path), exist_ok=True)
        with open(self._path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
        _log.info("Saved addon registry: %d allocations", len(self._allocations))

    def _ensure_loaded(self) -> None:
        if not self._loaded:
            self.load()

    # ------------------------------------------------------------------
    # Seeding
    # ------------------------------------------------------------------

    def seed_from_mods(self) -> None:
        """Scan all mods/*/yaml/AddonNodes/ to populate the registry."""
        pattern = os.path.join(self._mods_dir, "*/yaml/AddonNodes/*.yaml")
        count = 0
        max_index = self._start_index - 1

        for path in glob_mod.glob(pattern):
            try:
                with open(path, encoding="utf-8") as f:
                    record = yaml_util.safe_load_hex(f)
                if not isinstance(record, dict):
                    continue
                node_index = record.get("NodeIndex")
                if node_index is None:
                    continue
                node_index = int(node_index)

                # Extract mod name from path: mods/{ModName}/yaml/...
                parts = os.path.normpath(path).split(os.sep)
                mods_idx = None
                for pi, part in enumerate(parts):
                    if part.lower() == "mods" and pi + 1 < len(parts):
                        mods_idx = pi + 1
                        break
                mod_name = parts[mods_idx] if mods_idx is not None else "unknown"

                editor_id = record.get("EditorID", "")

                # Determine game from .game file
                mod_dir = os.path.join(self._mods_dir, mod_name)
                game = self._read_game_file(mod_dir)

                self._allocations[node_index] = {
                    "mod": mod_name,
                    "editor_id": editor_id,
                    "game": game,
                }
                if node_index > max_index:
                    max_index = node_index
                count += 1
            except Exception as e:
                _log.warning("Failed to read AddonNode YAML %s: %s", path, e)

        # Advance next_index past any discovered allocations
        if max_index >= self._start_index:
            self._next_index = max_index + 1

        if count:
            _log.info("Seeded addon registry with %d allocations from existing mods", count)
            self.save()

    @staticmethod
    def _read_game_file(mod_dir: str) -> str:
        game_file = os.path.join(mod_dir, ".game")
        if os.path.isfile(game_file):
            try:
                return open(game_file, encoding="utf-8").read().strip()
            except OSError:
                pass
        return ""

    # ------------------------------------------------------------------
    # Queries
    # ------------------------------------------------------------------

    def is_allocated(self, index: int) -> bool:
        self._ensure_loaded()
        return index in self._allocations

    def get_allocation(self, index: int) -> dict | None:
        self._ensure_loaded()
        return self._allocations.get(index)

    def get_mod_allocations(self, mod_name: str) -> dict[int, dict]:
        """Return all allocations belonging to a mod."""
        self._ensure_loaded()
        return {
            idx: info
            for idx, info in self._allocations.items()
            if info.get("mod") == mod_name
        }

    def items(self) -> list[tuple[int, dict]]:
        """Return all allocations as sorted (index, info) pairs."""
        self._ensure_loaded()
        return sorted(self._allocations.items())

    def next_index(self) -> int:
        """Return the next usable index based on the current allocations."""
        self._ensure_loaded()
        return self._next_index

    def get_stale_allocations(self) -> dict[int, dict]:
        """Return allocations whose mod folder no longer exists."""
        self._ensure_loaded()
        stale: dict[int, dict] = {}
        for idx, info in self._allocations.items():
            mod_name = str(info.get("mod", "")).strip()
            if not mod_name:
                stale[idx] = info
                continue
            mod_dir = os.path.join(self._mods_dir, mod_name)
            if not os.path.isdir(mod_dir):
                stale[idx] = info
        return stale

    # ------------------------------------------------------------------
    # Allocation
    # ------------------------------------------------------------------

    def allocate(self, mod_name: str, editor_id: str, game: str) -> int:
        """Assign the next available index and register it."""
        self._ensure_loaded()
        idx = self._next_index
        # Skip any indices that are somehow already taken
        while idx in self._allocations:
            idx += 1
        self._allocations[idx] = {
            "mod": mod_name,
            "editor_id": editor_id,
            "game": game,
        }
        self._next_index = idx + 1
        return idx

    def register(self, index: int, mod_name: str, editor_id: str, game: str) -> None:
        """Register a known index (e.g. from a vanilla remap that we want to track)."""
        self._ensure_loaded()
        self._allocations[index] = {
            "mod": mod_name,
            "editor_id": editor_id,
            "game": game,
        }
        # Keep next_index ahead
        if index >= self._next_index:
            self._next_index = index + 1

    # ------------------------------------------------------------------
    # Cleanup
    # ------------------------------------------------------------------

    def release_mod(self, mod_name: str) -> int:
        """Remove all allocations for a mod. Returns count removed."""
        self._ensure_loaded()
        to_remove = [
            idx for idx, info in self._allocations.items()
            if info.get("mod") == mod_name
        ]
        for idx in to_remove:
            del self._allocations[idx]
        if to_remove:
            self._sync_next_index()
            self.save()
            _log.info("Released %d addon node allocations for %s", len(to_remove), mod_name)
        return len(to_remove)

    def remove(self, index: int) -> bool:
        """Remove a single allocation by index."""
        self._ensure_loaded()
        if index not in self._allocations:
            return False
        del self._allocations[index]
        self._sync_next_index()
        self.save()
        _log.info("Removed addon node allocation %d", index)
        return True

    def release_stale(self) -> int:
        """Remove allocations whose mod folder is missing."""
        self._ensure_loaded()
        stale = self.get_stale_allocations()
        for idx in stale:
            del self._allocations[idx]
        if stale:
            self._sync_next_index()
            self.save()
            _log.info("Released %d stale addon node allocations", len(stale))
        return len(stale)

    def _sync_next_index(self) -> None:
        """Recompute the next index from the remaining live allocations."""
        if self._allocations:
            self._next_index = max(self._allocations) + 1
        else:
            self._next_index = self._start_index

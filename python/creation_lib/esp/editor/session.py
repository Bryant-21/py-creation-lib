"""Multi-plugin editor session coordinator.

Wraps `plugin_handle_*` calls into a load-order-aware view that the UI
consumes. The native crate is single-plugin; this is where cross-plugin
resolution lives.
"""

from __future__ import annotations

import logging
import struct
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Iterable

from creation_lib.esp.native_runtime import (
    RecordSummary,
    plugin_handle_close,
    plugin_handle_get,
    plugin_handle_record_payload_hash,
    plugin_handle_record_summary,
    plugin_handle_load,
    plugin_handle_load_index,
)

_log = logging.getLogger("creation_lib.esp.editor.session")

# Master filenames that uniquely identify each game.
_MASTER_FINGERPRINTS: dict[str, str] = {
    "fallout4.esm": "fo4",
    "dlcrobot.esm": "fo4",
    "dlcworkshop01.esm": "fo4",
    "dlccoast.esm": "fo4",
    "dlcnukaworld.esm": "fo4",
    "seventysix.esm": "fo76",
    "skyrim.esm": "skyrimse",
    "update.esm": "skyrimse",
    "dawnguard.esm": "skyrimse",
    "starfield.esm": "starfield",
    "constellation.esm": "starfield",
    "fallout3.esm": "fo3",
    "falloutnv.esm": "fnv",
    "oblivion.esm": "oblivion",
}

# HEDR.version → game(s). Where ambiguous (1.0 = both fo4 and starfield),
# subsequent steps disambiguate via masters or path.
_HEADER_VERSION_HINTS: dict[float, tuple[str, ...]] = {
    0.94: ("fo3",),
    1.34: ("fnv",),
    1.7: ("skyrimse",),
    0.95: ("fo4",),
    0.96: ("starfield",),
    68.0: ("fo76",),
    257.0: ("fo76",),
    1.0: ("fo4", "starfield", "oblivion"),
}


class ConflictStatus(Enum):
    """Mirrors xEdit's caXxx for record conflicts across the load order."""

    ONLY_ONE = "only_one"
    OVERRIDE = "override"
    NO_CONFLICT = "no_conflict"
    CONFLICT = "conflict"


@dataclass
class LoadedPlugin:
    """One plugin loaded into the session."""

    handle: int
    path: str
    game: str
    is_master: bool
    load_order_index: int
    plugin_name: str

    @property
    def display_name(self) -> str:
        return self.plugin_name or Path(self.path).name


@dataclass
class _GameDetection:
    """Result of game detection."""

    game: str
    confidence: str  # "sidecar", "path", "masters", "header", "fallback"


def _read_hedr_version(path: Path) -> float | None:
    """Read the float at offset 24 of a plugin file (TES4.HEDR.version).

    Plugin layout: TES4 (24-byte record header) → HEDR subrecord (4 ascii sig
    + 2 byte size + payload). HEDR payload starts at offset 24 + 6 = 30.
    But many readers prefer the simpler "scan for HEDR" approach. We do that.
    """
    try:
        with open(path, "rb") as fh:
            head = fh.read(64)
    except OSError:
        return None
    idx = head.find(b"HEDR")
    if idx < 0 or idx + 8 >= len(head):
        return None
    # idx + 4 = subrecord size (u16); idx + 6 = payload start.
    payload_off = idx + 6
    if payload_off + 4 > len(head):
        return None
    try:
        return struct.unpack_from("<f", head, payload_off)[0]
    except struct.error:
        return None


def _read_hedr_masters(path: Path) -> list[str]:
    """Scan the first ~64 KiB for MAST subrecords (TES4 master filenames)."""
    try:
        with open(path, "rb") as fh:
            head = fh.read(64 * 1024)
    except OSError:
        return []
    masters: list[str] = []
    cursor = 0
    while True:
        idx = head.find(b"MAST", cursor)
        if idx < 0 or idx + 6 >= len(head):
            break
        size = struct.unpack_from("<H", head, idx + 4)[0]
        start = idx + 6
        end = start + size
        if end > len(head):
            break
        name = head[start:end].rstrip(b"\x00").decode("cp1252", errors="replace")
        if name:
            masters.append(name)
        cursor = end
    return masters


def detect_game(
    plugin_path: str | Path,
    *,
    toolkit_settings=None,
    fallback: str = "fo4",
) -> str:
    """Best-effort game detection without loading the plugin via the native crate.

    Strategy, in order:
        1. Sidecar `<plugin>.game` or `<plugin_dir>/.game` file
        2. Path heuristic — under a known game's `root_dir`/`extracted_dir`
        3. Masters list — `Fallout4.esm` ⇒ fo4, etc.
        4. HEDR.version float
        5. `fallback` (caller default, typically the toolkit's active_game)
    """
    path = Path(plugin_path)

    # 1. Sidecar
    for sidecar in (path.with_suffix(path.suffix + ".game"), path.parent / ".game"):
        if sidecar.is_file():
            content = sidecar.read_text(encoding="utf-8", errors="replace").strip()
            if content:
                return content.lower()

    # 2. Path heuristic
    if toolkit_settings is not None:
        try:
            normalized = path.resolve()
        except OSError:
            normalized = path
        for game in ("fo4", "fo76", "skyrimse", "starfield", "fo3", "fnv"):
            paths = toolkit_settings.get_game_paths(game)
            for key in ("root_dir", "extracted_dir"):
                root = paths.get(key, "")
                if not root:
                    continue
                try:
                    root_resolved = Path(root).resolve()
                    normalized.relative_to(root_resolved)
                    return game
                except (OSError, ValueError):
                    continue
            for extra in paths.get("additional_paths", []) or []:
                try:
                    extra_resolved = Path(extra).resolve()
                    normalized.relative_to(extra_resolved)
                    return game
                except (OSError, ValueError):
                    continue

    # 3. Masters fingerprint
    masters = _read_hedr_masters(path)
    for master in masters:
        guess = _MASTER_FINGERPRINTS.get(master.lower())
        if guess:
            return guess

    # 4. HEDR.version
    version = _read_hedr_version(path)
    if version is not None:
        candidates = _HEADER_VERSION_HINTS.get(round(version, 2))
        if candidates and len(candidates) == 1:
            return candidates[0]
        if candidates and fallback in candidates:
            return fallback

    return fallback


class EditorSession:
    """Holds an ordered list of loaded plugins and resolves cross-plugin refs."""

    def __init__(
        self,
        *,
        toolkit_settings=None,
        default_game: str = "fo4",
        auto_scan_conflicts: bool = True,
        master_search_paths: Iterable[str | Path] | None = None,
        lazy_masters: bool = False,
    ):
        self._plugins: list[LoadedPlugin] = []
        self._toolkit_settings = toolkit_settings
        self._master_search_paths = [Path(path) for path in (master_search_paths or [])]
        self._lazy_masters = lazy_masters
        self._default_game = default_game
        self._active_handle: int | None = None
        self._resolve_cache: dict[int, tuple[int, RecordSummary] | None] = {}
        self._conflict_cache: dict[int, ConflictStatus] = {}
        # Patch-target plugin (created via creation_lib.esp.editor.patch); marked
        # in the UI so users can tell it apart from the active plugin.
        self._patch_handle: int | None = None
        # Cache of last conflict scan, populated by ConflictScanner consumers.
        self._last_conflict_scan = None
        # When True, every public load operation triggers an inline native
        # conflict scan and stores the result in `_last_conflict_scan`.
        # Off by default for non-UI consumers (CLI commands, validators).
        self.auto_scan_conflicts = auto_scan_conflicts
        # Re-entrancy counter so recursive load() calls (master resolution)
        # and load_folder()'s loop don't trigger N scans — only the outermost
        # public call fires `_maybe_run_auto_scan`.
        self._auto_scan_suppress_depth = 0

    # -- load / close ------------------------------------------------------

    def load(
        self,
        plugin_path: str | Path,
        *,
        game: str | None = None,
        as_master: bool | None = None,
        _seen: set[str] | None = None,
    ) -> LoadedPlugin:
        """Load a plugin and recursively load its required masters.

        `game` is auto-detected if not supplied. `as_master` is auto-derived
        from the plugin's TES4 master flag if not supplied. `_seen` is a
        cycle guard for recursive master loading.
        """
        self._auto_scan_suppress_depth += 1
        try:
            path = Path(plugin_path).resolve()
            seen = _seen if _seen is not None else set()
            key = str(path).lower()
            if key in seen:
                raise RuntimeError(f"Cyclic master reference: {path.name}")
            seen.add(key)

            existing = self.get_by_path(path)
            if existing is not None:
                return existing

            if game is None:
                game = detect_game(
                    path,
                    toolkit_settings=self._toolkit_settings,
                    fallback=self._default_game,
                )

            handle = (
                plugin_handle_load_index(str(path), game=game)
                if as_master and self._lazy_masters
                else None
            )
            if handle is None:
                handle = plugin_handle_load(str(path), game=game)
            handle_id = int(handle)

            plugin_name = plugin_handle_get(handle_id, "plugin_name") or path.name
            header_flags = int(plugin_handle_get(handle_id, "header_flags") or 0)
            is_master = bool(header_flags & 0x01) if as_master is None else as_master

            master_filenames: list[str] = list(plugin_handle_get(handle_id, "masters") or [])

            # Insert masters BEFORE this plugin so load_order_index ordering is correct.
            # We append the new plugin first, then resolve masters (which will be
            # inserted with lower load_order indices via _load_master).
            loaded = LoadedPlugin(
                handle=handle_id,
                path=str(path),
                game=game,
                is_master=is_master,
                load_order_index=len(self._plugins),
                plugin_name=plugin_name,
            )
            self._plugins.append(loaded)
            if self._active_handle is None and not is_master:
                self._active_handle = handle_id

            for master_name in master_filenames:
                self._load_master(master_name, game=game, _seen=seen)

            self._invalidate_cache()
            return loaded
        finally:
            self._auto_scan_suppress_depth -= 1
            self._maybe_run_auto_scan()

    def load_folder(
        self,
        folder: str | Path,
        *,
        load_order_file: str | Path | None = None,
        game: str | None = None,
        extensions: tuple[str, ...] = (".esp", ".esm", ".esl"),
    ) -> list[LoadedPlugin]:
        """Load every plugin under `folder` (non-recursive).

        Default ordering is ESM-first then alpha. If `load_order_file` is
        given, it is parsed as a textual load-order list (one plugin name
        per line, MO2 `*` prefix tolerated, blank/`#` lines skipped) and
        used as the ordering — any plugin in the folder not mentioned in
        the file is appended afterwards in alpha order.
        """
        folder_path = Path(folder)
        if not folder_path.is_dir():
            raise FileNotFoundError(f"Not a directory: {folder_path}")

        candidates = [
            p for p in folder_path.iterdir()
            if p.is_file() and p.suffix.lower() in extensions
        ]
        ordering = (
            _parse_load_order_file(Path(load_order_file))
            if load_order_file
            else None
        )
        ordered = _order_plugins(candidates, ordering)

        self._auto_scan_suppress_depth += 1
        try:
            loaded: list[LoadedPlugin] = []
            for path in ordered:
                try:
                    loaded.append(self.load(path, game=game))
                except Exception:
                    _log.exception("load_folder: failed to load %s", path)
            return loaded
        finally:
            self._auto_scan_suppress_depth -= 1
            self._maybe_run_auto_scan()

    def import_load_order(self, load_order_file: str | Path) -> None:
        """Re-order the already-loaded plugins by a `loadorder.txt` file.

        Plugins not mentioned in the file keep their relative order at the
        end. Recomputes `load_order_index` on every plugin.
        """
        ordering = _parse_load_order_file(Path(load_order_file))
        if not ordering:
            return
        order_index = {name.lower(): i for i, name in enumerate(ordering)}
        # Stable sort: known names come first in file order, unknown last.
        unknown_offset = len(ordering)
        self._plugins.sort(
            key=lambda p: order_index.get(p.plugin_name.lower(), unknown_offset)
        )
        for i, plugin in enumerate(self._plugins):
            plugin.load_order_index = i
        self._invalidate_cache()
        self._maybe_run_auto_scan()

    def _load_master(self, master_name: str, *, game: str, _seen: set[str]) -> None:
        """Resolve `master_name` (filename only) against the game's data dir."""
        if any(p.plugin_name.lower() == master_name.lower() for p in self._plugins):
            return
        candidate = self._resolve_master_path(master_name, game)
        if candidate is None:
            _log.warning("Master not found in game data dir: %s (game=%s)", master_name, game)
            return
        try:
            self.load(candidate, game=game, as_master=True, _seen=_seen)
        except Exception:
            _log.exception("Failed to load master %s", master_name)

    def _resolve_master_path(self, master_name: str, game: str) -> Path | None:
        candidates: list[Path] = []
        for search_path in self._master_search_paths:
            candidates.append(search_path / master_name)
            candidates.append(search_path / "Data" / master_name)
        if self._toolkit_settings is not None:
            paths = self._toolkit_settings.get_game_paths(game)
            root = paths.get("root_dir", "")
            if root:
                candidates.append(Path(root) / "Data" / master_name)
                candidates.append(Path(root) / master_name)
            extracted = paths.get("extracted_dir", "")
            if extracted:
                candidates.append(Path(extracted) / master_name)
            for extra in paths.get("additional_paths", []) or []:
                candidates.append(Path(extra) / master_name)
        for candidate in candidates:
            if candidate.is_file():
                return candidate
        return None

    def close(self, handle: int, *, with_dependents: bool = False) -> None:
        target = self.get_by_handle(handle)
        if target is None:
            return
        dependents = [p for p in self._plugins if target.plugin_name in (
            plugin_handle_get(p.handle, "masters") or []
        )]
        if dependents and not with_dependents:
            raise RuntimeError(
                f"Cannot close {target.plugin_name}: still required by "
                f"{', '.join(p.plugin_name for p in dependents)}"
            )
        for dependent in dependents:
            self.close(dependent.handle, with_dependents=True)
        plugin_handle_close(target.handle)
        self._plugins = [p for p in self._plugins if p.handle != target.handle]
        if self._active_handle == target.handle:
            self._active_handle = next(
                (p.handle for p in self._plugins if not p.is_master), None
            )
        self._invalidate_cache()

    def close_all(self) -> None:
        for plugin in list(self._plugins):
            try:
                plugin_handle_close(plugin.handle)
            except Exception:
                _log.exception("Failed to close handle %s", plugin.handle)
        self._plugins.clear()
        self._active_handle = None
        self._invalidate_cache()

    # -- accessors ---------------------------------------------------------

    @property
    def plugins(self) -> list[LoadedPlugin]:
        return list(self._plugins)

    @property
    def active(self) -> LoadedPlugin | None:
        if self._active_handle is None:
            return None
        return self.get_by_handle(self._active_handle)

    def set_active(self, handle: int) -> None:
        if any(p.handle == handle for p in self._plugins):
            self._active_handle = handle

    def get_by_handle(self, handle: int) -> LoadedPlugin | None:
        for plugin in self._plugins:
            if plugin.handle == handle:
                return plugin
        return None

    def get_by_path(self, path: Path) -> LoadedPlugin | None:
        target = str(Path(path).resolve()).lower()
        for plugin in self._plugins:
            if str(Path(plugin.path).resolve()).lower() == target:
                return plugin
        return None

    def get_by_name(self, name: str) -> LoadedPlugin | None:
        target = name.lower()
        for plugin in self._plugins:
            if plugin.plugin_name.lower() == target:
                return plugin
        return None

    # -- cross-plugin resolution ------------------------------------------

    def resolve_form_id(self, form_id: int) -> tuple[int, RecordSummary] | None:
        """Walk load order high-to-low and return (handle, record summary) or None."""
        cached = self._resolve_cache.get(form_id, _SENTINEL)
        if cached is not _SENTINEL:
            return cached
        result: tuple[int, RecordSummary] | None = None
        for plugin in reversed(self._plugins):
            try:
                plugin_handle_get(plugin.handle, "header")  # warm metadata
                record = plugin_handle_record_summary(plugin.handle, form_id)
            except Exception:
                continue
            if record is not None:
                result = (plugin.handle, record)
                break
        self._resolve_cache[form_id] = result
        return result

    def referencing(self, form_id: int) -> list[tuple[int, int]]:
        """All (plugin_handle, ref_form_id) pairs that reference `form_id`."""
        out: list[tuple[int, int]] = []
        for plugin in self._plugins:
            try:
                ids = self._call(plugin.handle, "get_referencing_form_ids", form_id) or []
            except Exception:
                ids = []
            for fid in ids:
                out.append((plugin.handle, int(fid)))
        return out

    def referenced(self, form_id: int) -> list[int]:
        result = self.resolve_form_id(form_id)
        if result is None:
            return []
        handle, _ = result
        try:
            ids = self._call(handle, "get_referenced_form_ids", form_id) or []
        except Exception:
            return []
        return [int(fid) for fid in ids]

    def conflict_status(self, form_id: int) -> ConflictStatus:
        """Compare record bytes across plugins that contain `form_id`."""
        cached = self._conflict_cache.get(form_id)
        if cached is not None:
            return cached
        winners: list[str] = []
        for plugin in self._plugins:
            try:
                payload_hash = plugin_handle_record_payload_hash(plugin.handle, form_id)
            except Exception:
                continue
            if payload_hash is None:
                continue
            winners.append(payload_hash)
        if len(winners) <= 1:
            result = ConflictStatus.ONLY_ONE
        else:
            first = winners[0]
            if all(payload == first for payload in winners[1:]):
                result = ConflictStatus.OVERRIDE
            else:
                result = ConflictStatus.CONFLICT
        self._conflict_cache[form_id] = result
        return result

    def conflict_role(self, handle: int, form_id: int) -> str:
        """Per-plugin role for ``form_id`` viewed from ``handle``.

        Returns one of:
          - ``"only"``   — only this plugin contains the record.
          - ``"override"`` — multiple plugins contain it but bytes match.
          - ``"winner"`` — this plugin is the latest in load-order *and* the
            payload differs from at least one earlier copy.
          - ``"loser"``  — this plugin is overridden by a later one with
            different bytes.
          - ``"missing"`` — handle has no entry for this form_id.
        """
        # Build (handle, payload) chain in load-order.
        chain: list[tuple[int, str]] = []
        for plugin in self._plugins:
            try:
                payload_hash = plugin_handle_record_payload_hash(plugin.handle, form_id)
            except Exception:
                continue
            if payload_hash is None:
                continue
            chain.append((plugin.handle, payload_hash))

        if not any(h == handle for h, _ in chain):
            return "missing"
        if len(chain) <= 1:
            return "only"
        first = chain[0][1]
        if all(payload == first for _, payload in chain[1:]):
            return "override"
        last_handle = chain[-1][0]
        return "winner" if handle == last_handle else "loser"

    # -- internal ---------------------------------------------------------

    @staticmethod
    def _call(handle: int, name: str, *args):
        from creation_lib.esp.native_runtime import plugin_handle_call

        return plugin_handle_call(handle, name, *args)

    def invalidate_form_id(self, form_id: int) -> None:
        """Invalidate cached data for a single form_id after an edit."""
        self._resolve_cache.pop(form_id, None)
        self._conflict_cache.pop(form_id, None)

    def _invalidate_cache(self) -> None:
        self._resolve_cache.clear()
        self._conflict_cache.clear()

    # -- conflict scanning ------------------------------------------------

    def run_conflict_scan(self):
        """Run the native cross-plugin conflict scanner inline.

        Stores the result in `_last_conflict_scan` and returns it.
        """
        from creation_lib.esp.editor.conflicts import ConflictScanner

        scan = ConflictScanner().scan(self)
        self._last_conflict_scan = scan
        return scan

    def _maybe_run_auto_scan(self) -> None:
        """Trigger an inline scan only at the outermost public load call."""
        if not self.auto_scan_conflicts:
            return
        if self._auto_scan_suppress_depth > 0:
            return
        if not self._plugins:
            self._last_conflict_scan = None
            return
        try:
            self.run_conflict_scan()
        except Exception:
            _log.exception("auto-scan: native conflict scan failed")


_SENTINEL = object()


def _parse_load_order_file(path: Path) -> list[str]:
    """Parse a MO2-style loadorder.txt — one plugin per line, '*' prefix ok."""
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        _log.warning("load order file not readable: %s", path)
        return []
    out: list[str] = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("*"):
            line = line[1:].strip()
        if line:
            out.append(line)
    return out


def _order_plugins(
    candidates: list[Path],
    ordering: list[str] | None,
) -> list[Path]:
    """Order `candidates` ESM-first then alpha, or by `ordering` if supplied."""
    if ordering:
        order_index = {name.lower(): i for i, name in enumerate(ordering)}
        unknown = len(ordering)
        return sorted(
            candidates,
            key=lambda p: (order_index.get(p.name.lower(), unknown), p.name.lower()),
        )
    return sorted(
        candidates,
        key=lambda p: (0 if p.suffix.lower() == ".esm" else 1, p.name.lower()),
    )

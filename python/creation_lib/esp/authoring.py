"""ESP↔YAML serialization wrapper.

Thin Python wrapper around the in-process Rust ESP pipeline (``creation_lib.esp.api``
backed by ``creation_lib._native``). The public function names are kept stable
for the existing call sites in ``cli/`` and ``py_creation_lib/python/creation_lib/mod/``.

Authoring directories use the standard layout: ``plugin.yaml`` at the root,
per-record YAML under ``records/<SIG>/``, and ``Strings/`` sidecars next to
``plugin.yaml``. Old-format sentinels such as ``spriggit-meta.json`` or
top-level ``RecordData.yaml`` are rejected with a clear error pointing at
``modkit mod import``.
"""
from __future__ import annotations

import logging
import re
from pathlib import Path
from typing import Callable

from creation_lib.core.game_profiles import get_profile

_log = logging.getLogger(__name__)

# Bit set on the TES4 record header for Light/ESL plugins.
# Mirrors HEADER_FLAG_DEFINITIONS in py_creation_lib/native/esp/src/plugin_runtime.rs.
_HEADER_FLAG_LIGHT = 0x0000_0200
_HEADER_FLAG_MASTER = 0x0000_0001


# ---------------------------------------------------------------------------
# Legacy-format detection
# ---------------------------------------------------------------------------

def _is_legacy_authoring_dir(path: Path) -> bool:
    """Detect old YAML layouts by their legacy sentinel files."""
    if not path.is_dir():
        return False
    return (path / "spriggit-meta.json").is_file() or (path / "RecordData.yaml").is_file()


def _legacy_authoring_format_error(yaml_dir: Path) -> RuntimeError:
    return RuntimeError(
        f"Legacy authoring YAML format detected at {yaml_dir}.\n"
        f"Re-import the mod from its built .esp:\n"
        f"    modkit mod import <path-to-built-esp> --game <game>\n"
        f"The old yaml/ directory will need to be removed or renamed first."
    )


def _resolve_master_esm_paths(yaml_dir: Path, data_folder: Path | None) -> list[str] | None:
    """Resolve master ESM names listed in plugin.yaml/json against data_folder.

    The first readable master is what the Rust codec scans to derive the
    canonical top-level GRUP order at build time. We hand it the full list so
    it can pick whichever exists; missing entries are filtered out.

    Returns None when data_folder is unset, plugin.yaml is unreadable, or
    no master is listed — the codec then falls back to its bundled baseline.
    """
    if data_folder is None:
        return None
    manifest = yaml_dir / "plugin.yaml"
    if not manifest.is_file():
        manifest = yaml_dir / "plugin.json"
    if not manifest.is_file():
        return None
    try:
        if manifest.suffix == ".yaml":
            import yaml as _yaml
            doc = _yaml.safe_load(manifest.read_text(encoding="utf-8"))
        else:
            import json as _json
            doc = _json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    masters = ((doc or {}).get("header") or {}).get("masters") or []
    if not isinstance(masters, list):
        return None
    resolved: list[str] = []
    for name in masters:
        if not isinstance(name, str) or not name:
            continue
        candidate = Path(data_folder) / name
        if candidate.is_file():
            resolved.append(str(candidate))
    return resolved or None


# ---------------------------------------------------------------------------
# Plugin extension helper
# ---------------------------------------------------------------------------

_PLUGIN_LINE_RE = re.compile(r"^plugin:\s*(\S+)", re.MULTILINE)


def get_plugin_ext(mod_dir: Path, yaml_dir: Path | None = None) -> str:
    """Read plugin extension from ``plugin.yaml`` (preferred) or by scanning
    sibling plugin files. Falls back to ``"esp"``.
    """
    yaml_root = yaml_dir or mod_dir / "yaml"
    plugin_yaml = yaml_root / "plugin.yaml"
    if plugin_yaml.is_file():
        try:
            text = plugin_yaml.read_text(encoding="utf-8")
        except OSError:
            text = ""
        m = _PLUGIN_LINE_RE.search(text[:2048])
        if m:
            ext_match = re.search(r"\.(es[plm])$", m.group(1), re.IGNORECASE)
            if ext_match:
                return ext_match.group(1).lower()

    if mod_dir.is_dir():
        candidates = sorted(
            p for ext in ("esl", "esp", "esm")
            for p in mod_dir.glob(f"*.{ext}")
        )
        if candidates:
            return candidates[0].suffix.lstrip(".").lower()

    return "esp"


# ---------------------------------------------------------------------------
# Serialize (.esp → YAML)
# ---------------------------------------------------------------------------

def serialize(
    esp_path: Path,
    output_dir: Path,
    *,
    game: str,
    data_folder: Path | None = None,
    error_on_unknown: bool = True,
    on_progress: Callable[[str], None] | None = None,
) -> Path:
    """Serialize an ``.esp/.esm/.esl`` to YAML.

    Args:
        esp_path: Path to the plugin file.
        output_dir: Directory where ``yaml/`` will be created.
        game: Game ID (fo4, skyrimse, starfield, fo76, fo3, fnv).
        data_folder: Game ``Data/`` folder for master/strings resolution.
            Currently unused by the in-process exporter (it locates strings
            relative to the source plugin) — accepted for API compatibility.
        error_on_unknown: Accepted for API compatibility.
        on_progress: Optional callback for status messages.

    Returns:
        Path to the created ``yaml/`` directory.
    """
    del data_folder, error_on_unknown  # accepted for back-compat

    from creation_lib.esp.native_runtime import export_authoring_dir_native

    if not Path(esp_path).is_file():
        raise FileNotFoundError(f"Plugin not found: {esp_path}")

    get_profile(game)

    yaml_dir = Path(output_dir) / "yaml"
    yaml_dir.mkdir(parents=True, exist_ok=True)

    msg = f"Serializing {Path(esp_path).name} to YAML ({game})..."
    _log.info(msg)
    if on_progress:
        on_progress(msg)

    export_authoring_dir_native(
        str(esp_path),
        str(yaml_dir),
        game=game,
        format="yaml",
    )

    return yaml_dir


# ---------------------------------------------------------------------------
# Deserialize (YAML → .esp)
# ---------------------------------------------------------------------------

def deserialize(
    yaml_dir: Path,
    output_path: Path,
    *,
    game: str,
    data_folder: Path | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> Path:
    """Build a plugin file from a YAML authoring dir.

    Args:
        yaml_dir: Path to the ``yaml/`` directory containing ``plugin.yaml``
            and ``records/<SIG>/`` subdirs.
        output_path: Path for the output ``.esp/.esm/.esl`` file.
        game: Game ID.
        data_folder: Game data directory. When provided, the build resolves
            master ESM names from ``plugin.yaml`` against this folder so the
            codec can scan the live master for the canonical top-level GRUP
            order — keeps KYWD before COBJ etc. across Bethesda content
            updates without code changes. Falls back to a hardcoded baseline
            when ``data_folder`` is ``None`` or a master file isn't present.
        on_progress: Optional callback for status messages.

    Returns:
        Path to the built plugin file.

    Raises:
        RuntimeError: if ``yaml_dir`` uses a legacy authoring format.
    """
    from creation_lib.esp.api import build_authoring_dir

    yaml_dir = Path(yaml_dir)
    output_path = Path(output_path)

    if _is_legacy_authoring_dir(yaml_dir):
        raise _legacy_authoring_format_error(yaml_dir)

    if not (yaml_dir / "plugin.yaml").is_file() and not (yaml_dir / "plugin.json").is_file():
        raise RuntimeError(
            f"No plugin.yaml or plugin.json found in {yaml_dir}. "
            f"This does not look like an ESP authoring directory."
        )

    get_profile(game)

    if output_path.is_file():
        output_path.unlink()
    output_path.parent.mkdir(parents=True, exist_ok=True)

    msg = f"Building {output_path.name} from YAML ({game})..."
    _log.info(msg)
    if on_progress:
        on_progress(msg)

    master_esm_paths = _resolve_master_esm_paths(yaml_dir, data_folder)
    build_authoring_dir(yaml_dir, output_path, game=game, master_esm_paths=master_esm_paths)

    if not output_path.is_file():
        raise RuntimeError(
            f"Build completed but output file was not created: {output_path}"
        )

    return output_path


# ---------------------------------------------------------------------------
# New mod YAML scaffold
# ---------------------------------------------------------------------------

def new_mod_yaml(
    mod_name: str,
    mod_dir: Path,
    *,
    game: str,
    plugin_ext: str = "esl",
    mod_prefix: str = "",
) -> Path:
    """Create a YAML scaffold for a new mod.

    Args:
        mod_name: Mod name (e.g. ``"B21_MyMod"``).
        mod_dir: Root mod directory (``mods/<ModName>/``).
        game: Game ID.
        plugin_ext: Plugin extension — ``"esl"``, ``"esp"``, or ``"esm"``.
        mod_prefix: Author prefix string for the plugin header.

    Returns:
        Path to the created ``yaml/`` directory.
    """
    from creation_lib.esp.api import export_authoring_dir
    from creation_lib.esp.plugin import Plugin

    profile = get_profile(game)

    if plugin_ext not in {"esl", "esp", "esm"}:
        raise ValueError(f"Invalid plugin_ext: {plugin_ext!r} (expected esl/esp/esm)")

    yaml_dir = mod_dir / "yaml"
    if yaml_dir.is_dir():
        raise FileExistsError(f"{yaml_dir} already exists")

    plugin_name = f"{mod_name}.{plugin_ext}"
    masters = [profile.master_esm] if profile.master_esm else []

    plugin = Plugin.new(plugin_name, game=game, masters=masters)

    # Header flags. ESL plugins set the Light bit; ESM plugins set the Master
    # bit. ESPs leave both clear.
    flag_value = 0
    if plugin_ext == "esl":
        flag_value |= _HEADER_FLAG_LIGHT
    elif plugin_ext == "esm":
        flag_value |= _HEADER_FLAG_MASTER
    if flag_value:
        plugin.header.flags = flag_value

    if mod_prefix:
        plugin.header.author = mod_prefix

    yaml_dir.mkdir(parents=True)
    export_authoring_dir(plugin, yaml_dir, format="yaml")

    return yaml_dir


_FLAG_LABELS = (
    ("FLAG_MASTER", "Master"),
    ("FLAG_LOCALIZED", "Localized"),
    ("FLAG_LIGHT", "Light"),
    ("FLAG_MEDIUM", "Medium"),
    ("FLAG_UPDATE", "Update"),
)


def _dedup_masters(masters: list[str]) -> list[str]:
    """Drop case-insensitive duplicate master names, preserving first-seen order."""
    seen: set[str] = set()
    result: list[str] = []
    for name in masters:
        key = name.lower()
        if key in seen:
            continue
        seen.add(key)
        result.append(name)
    return result


def new_plugin_file(
    output_path: Path,
    *,
    game: str,
    extension: str,
    masters: list[str] | None = None,
    include_base_master: bool = True,
    set_master: bool | None = None,
    set_light: bool | None = None,
    set_medium: bool = False,
    set_update: bool = False,
    set_localized: bool = False,
    force: bool = False,
    backend: str = "auto",
) -> dict:
    """Create an empty plugin binary at ``output_path`` and return a summary dict.

    ``extension`` (``"esp"``/``"esm"``/``"esl"``) is auto-coupled to its canonical
    header bit (esm→Master, esl→Light). The ``set_*`` overrides are applied after
    the auto-couple — ``set_master``/``set_light`` of ``None`` leave the
    auto-coupled value untouched, while ``True``/``False`` force the bit on/off.
    The game profile's base ESM is seeded as the first master unless
    ``include_base_master`` is False; ``masters`` are appended and the whole list
    is de-duplicated case-insensitively.
    """
    from creation_lib.esp.editor import header_flags
    from creation_lib.esp.plugin import Plugin

    if extension not in {"esp", "esm", "esl"}:
        raise ValueError(f"Invalid extension: {extension!r} (expected esp/esm/esl)")
    if output_path.exists() and not force:
        raise FileExistsError(f"{output_path} already exists (pass force=True to overwrite)")

    profile = get_profile(game)
    seeded: list[str] = []
    if include_base_master and profile.master_esm:
        seeded.append(profile.master_esm)
    seeded.extend(masters or [])
    resolved_masters = _dedup_masters(seeded)

    plugin = Plugin.new(output_path.name, game=game, masters=resolved_masters)
    try:
        handle = plugin._rust_handle
        if extension == "esm":
            header_flags.set_master(handle, True)
        elif extension == "esl":
            header_flags.set_light(handle, True)
        if set_master is not None:
            header_flags.set_master(handle, set_master)
        if set_light is not None:
            header_flags.set_light(handle, set_light)
        if set_medium:
            header_flags.set_medium(handle, True)
        if set_update:
            header_flags.set_update(handle, True)
        if set_localized:
            header_flags.set_localized(handle, True)

        output_path.parent.mkdir(parents=True, exist_ok=True)
        plugin.save(output_path, backend=backend)

        flags_value = header_flags.get_flags(handle)
        flag_names = [
            label for const, label in _FLAG_LABELS
            if flags_value & getattr(header_flags, const)
        ]
        header_version = float(plugin.header.version)
    finally:
        plugin.close()

    return {
        "plugin": output_path.name,
        "game": game,
        "extension": extension,
        "output": str(output_path),
        "flags": flag_names,
        "flags_hex": f"0x{flags_value:08X}",
        "masters": resolved_masters,
        "header_version": header_version,
    }

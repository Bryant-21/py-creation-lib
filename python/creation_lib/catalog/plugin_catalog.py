"""Per-plugin JSON catalog contract (mods/B21_TalesFromAppalachia/docs/2026-10-12-plugin-catalogs-plan.md).

The reader half mirrors src/PluginCatalogRules.h in Tales; both run
resources/catalog_schemas/plugin_catalog_cases.json.
"""

from __future__ import annotations

import json
import re
from collections.abc import Callable, Iterable
from pathlib import Path
from typing import Any

SCHEMAS_DIR = Path(__file__).resolve().parents[1] / "resources" / "catalog_schemas"
ROOT = Path("F4SE/Plugins/B21_TalesFromAppalachia")
ENVELOPE_VERSION = 1

_HEX = re.compile(r"[0-9A-Fa-f]{1,8}")


def catalog_path(feature: str, plugin: str) -> Path:
    return ROOT / feature / f"{plugin}.json"


def form_key(plugin: str, object_id: int) -> str:
    width = 6 if object_id <= 0xFFFFFF else 8
    return f"{plugin}:{object_id:0{width}X}"


def parse_form_key(text: str) -> tuple[str, int] | None:
    """(lowercased plugin, id) for "Plugin.ext:HEX", else None."""
    if not isinstance(text, str) or text.count(":") != 1:
        return None
    plugin, hex_part = text.split(":")
    if not plugin or not _HEX.fullmatch(hex_part):
        return None
    dot = plugin.rfind(".")
    if dot <= 0 or dot == len(plugin) - 1 or any(c.isspace() for c in plugin):
        return None
    object_id = int(hex_part, 16)
    if object_id == 0:
        return None
    return _lower(plugin), object_id


def _lower(text: str) -> str:
    return "".join(c.lower() if c < "\x80" else c for c in text)


def order_files(file_names: Iterable[str], active_plugins_in_load_order: Iterable[str]) -> list[str]:
    load_index: dict[str, int] = {}
    for index, name in enumerate(active_plugins_in_load_order):
        load_index.setdefault(_lower(name), index)
    kept: list[tuple[int, str]] = []
    for name in file_names:
        if len(name) <= 5 or _lower(name[-5:]) != ".json":
            continue
        index = load_index.get(_lower(name[:-5]))
        if index is not None:
            kept.append((index, name))
    kept.sort(key=lambda pair: pair[0])
    return [name for _, name in kept]


def merge_section(
    into: dict[Any, Any],
    section: Any,
    key_of: Callable[[dict[str, Any], str], Any | None],
) -> None:
    """key_of(entry, map_key) -> key or None; map_key is "" for array sections.

    Array items must be objects; map-section values may be any JSON type.
    """

    def apply(entry: Any, map_key: str) -> None:
        key = key_of(entry, map_key)
        if key is None:
            return
        if isinstance(entry, dict) and entry.get("remove") is True:
            into.pop(key, None)
        else:
            into[key] = entry

    if isinstance(section, list):
        for entry in section:
            if isinstance(entry, dict):
                apply(entry, "")
    elif isinstance(section, dict):
        for map_key, entry in section.items():
            apply(entry, map_key)


def merge_settings(into: dict[str, Any], source: Any) -> None:
    if isinstance(source, dict):
        into.update(source)


def write_document(feature: str, plugin: str, doc: dict[str, Any], out_root: Path | str) -> Path:
    """Write the envelope: version, settings (if any), then the remaining sections in given order."""
    envelope: dict[str, Any] = {"version": doc.get("version", ENVELOPE_VERSION)}
    if "settings" in doc:
        envelope["settings"] = doc["settings"]
    for name, value in doc.items():
        if name not in envelope:
            envelope[name] = value
    path = Path(out_root) / catalog_path(feature, plugin)
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(envelope, indent=2, ensure_ascii=False) + "\n"
    path.write_bytes(text.encode("utf-8"))
    return path

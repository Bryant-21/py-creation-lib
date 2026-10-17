"""Compatibility entry points for native PEX type-header generation."""
from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any, Iterable

from creation_lib.pex.native_runtime import load_native_module

DATA_DIR = Path(__file__).with_name("data")
_DATA_PATH = DATA_DIR / "fo4_papyrus_defaults.json"
FLAGS_FILE_NAME = "Institute_Papyrus_Flags.flg"


@lru_cache(maxsize=1)
def load_manifest(path: str | None = None) -> dict[str, Any]:
    source = Path(path) if path else _DATA_PATH
    if not source.is_file():
        return {"defaults": {}, "events": {}}
    with source.open(encoding="utf-8") as handle:
        payload = json.load(handle)
    return {"defaults": payload.get("defaults", {}), "events": payload.get("events", {})}


def emit_header(obj: Any, manifest: dict[str, Any] | None = None) -> str:
    native = load_native_module()
    if native is None:
        raise RuntimeError("papyrus_core native PEX header emitter is not available")
    return native.emit_pex_header(
        json.dumps(obj, default=vars),
        json.dumps(manifest if manifest is not None else load_manifest()),
    )


def emit_headers_for_file(
    pex_path: str | Path,
    manifest: dict[str, Any] | None = None,
) -> Iterable[tuple[str, str]]:
    native = load_native_module()
    if native is None:
        raise RuntimeError("papyrus_core native PEX header emitter is not available")
    yield from native.emit_pex_file_headers(
        str(pex_path), json.dumps(manifest if manifest is not None else load_manifest()),
    )

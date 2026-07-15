"""TES4 header flag toggles — ESL / Light / Medium / Localized.

Bit values per game (from xEdit `wbDefinitionsXxx.pas`):

    0x00000001  Master (ESM)
    0x00000020  Deleted (record-level only; not used here)
    0x00000080  Localized (FO4/SF/SSE)
    0x00000200  Light plugin (ESL flag) — FO4, SF, SSE
    0x00000400  Medium plugin — Starfield only
    0x00001000  Update — Starfield only

The ESL flag and the .esl extension are independent: flagging an .esp/.esm
ESL makes the game treat it as light without renaming the file.
"""

from __future__ import annotations

from creation_lib.esp.native_runtime import plugin_handle_call, plugin_handle_get

FLAG_MASTER = 0x00000001
FLAG_LOCALIZED = 0x00000080
FLAG_LIGHT = 0x00000200
FLAG_MEDIUM = 0x00000400
FLAG_UPDATE = 0x00001000


def get_flags(handle: int) -> int:
    return int(plugin_handle_get(handle, "header_flags") or 0)


def _set_bit(handle: int, bit: int, on: bool) -> None:
    flags = get_flags(handle)
    if on:
        new = flags | bit
    else:
        new = flags & ~bit
    if new != flags:
        plugin_handle_call(handle, "set_header_flags", new)


def set_master(handle: int, on: bool) -> None:
    _set_bit(handle, FLAG_MASTER, on)


def set_light(handle: int, on: bool) -> None:
    """Toggle the ESL/light flag (0x200). Game treats the plugin as light."""
    _set_bit(handle, FLAG_LIGHT, on)


# Alias — modders call it "ESL flag", xEdit calls it "Light".
set_esl = set_light


def set_medium(handle: int, on: bool) -> None:
    """Toggle the medium-plugin flag (0x400, Starfield only)."""
    _set_bit(handle, FLAG_MEDIUM, on)


def set_update(handle: int, on: bool) -> None:
    """Toggle the update-plugin flag (0x1000, Starfield only)."""
    _set_bit(handle, FLAG_UPDATE, on)


def set_localized(handle: int, on: bool) -> None:
    _set_bit(handle, FLAG_LOCALIZED, on)


def is_light(handle: int) -> bool:
    return bool(get_flags(handle) & FLAG_LIGHT)


def is_medium(handle: int) -> bool:
    return bool(get_flags(handle) & FLAG_MEDIUM)


def is_master(handle: int) -> bool:
    return bool(get_flags(handle) & FLAG_MASTER)


def is_localized(handle: int) -> bool:
    return bool(get_flags(handle) & FLAG_LOCALIZED)

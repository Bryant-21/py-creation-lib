"""Read the file version of a Fallout 4 install from its Fallout4.exe.

Windows-only (uses the version.dll resource API via ctypes); returns ``None``
on any other platform or when the version resource can't be read. Used only to
seed a default label when the user adds a deploy-target install.
"""

from __future__ import annotations

import sys
from pathlib import Path


def detect_exe_version(exe_path) -> str | None:
    """Return the ``a.b.c.d`` file version of a Windows PE, or ``None``."""
    path = Path(exe_path)
    if sys.platform != "win32" or not path.is_file():
        return None
    try:
        import ctypes
        from ctypes import wintypes

        version = ctypes.windll.version
        size = version.GetFileVersionInfoSizeW(str(path), None)
        if not size:
            return None
        buf = ctypes.create_string_buffer(size)
        if not version.GetFileVersionInfoW(str(path), 0, size, buf):
            return None

        block = ctypes.c_void_p()
        length = wintypes.UINT()
        if not version.VerQueryValueW(
            buf, "\\", ctypes.byref(block), ctypes.byref(length)
        ):
            return None

        class VS_FIXEDFILEINFO(ctypes.Structure):
            _fields_ = [
                ("dwSignature", wintypes.DWORD),
                ("dwStrucVersion", wintypes.DWORD),
                ("dwFileVersionMS", wintypes.DWORD),
                ("dwFileVersionLS", wintypes.DWORD),
                ("dwProductVersionMS", wintypes.DWORD),
                ("dwProductVersionLS", wintypes.DWORD),
                ("dwFileFlagsMask", wintypes.DWORD),
                ("dwFileFlags", wintypes.DWORD),
                ("dwFileOS", wintypes.DWORD),
                ("dwFileType", wintypes.DWORD),
                ("dwFileSubtype", wintypes.DWORD),
                ("dwFileDateMS", wintypes.DWORD),
                ("dwFileDateLS", wintypes.DWORD),
            ]

        info = ctypes.cast(block, ctypes.POINTER(VS_FIXEDFILEINFO)).contents
        ms, ls = info.dwFileVersionMS, info.dwFileVersionLS
        return f"{(ms >> 16) & 0xFFFF}.{ms & 0xFFFF}.{(ls >> 16) & 0xFFFF}.{ls & 0xFFFF}"
    except Exception:
        return None


def detect_fo4_version(root_dir) -> str | None:
    """Return the file version of Fallout4.exe under root_dir or its parent.

    root_dir may point at the FO4 install root or at its Data folder (with the
    exe one directory up) — check both locations.
    """
    if not root_dir:
        return None
    root = Path(root_dir)
    candidates = dict.fromkeys([root / "Fallout4.exe", root.parent / "Fallout4.exe"])
    for candidate in candidates:
        version = detect_exe_version(candidate)
        if version is not None:
            return version
    return None


_NEXTGEN_MIN = (1, 10, 980)


def _parse_version(version: str | None) -> tuple[int, ...] | None:
    if not version:
        return None
    try:
        return tuple(int(part) for part in version.split("."))
    except ValueError:
        return None


def classify_ba2_target(version: str | None) -> str:
    """Return "og" or "nextgen" for a dotted file-version string."""
    parsed = _parse_version(version)
    if parsed is None:
        return "nextgen"
    return "nextgen" if tuple(parsed[:3]) >= _NEXTGEN_MIN else "og"


def detect_ba2_target(root_dir, *, reader=detect_fo4_version) -> tuple[str, str | None]:
    """Detect the FO4 version under root_dir and classify its BA2 target.

    Returns (target, version_string_or_None).
    """
    version = reader(root_dir)
    return classify_ba2_target(version), version

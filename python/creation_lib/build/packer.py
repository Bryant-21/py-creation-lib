"""Pack archives (BA2/BSA) for PC, Xbox, and/or PlayStation platforms.

Packs with native `bsarchive_native`, or Archive2.exe when explicitly
requested. Entry point: `pack_mod`.
"""
from __future__ import annotations

import json
import logging
import os
import shutil
import struct
import subprocess
import time
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from creation_lib.core.game_profiles import get_profile
from creation_lib.ba2 import native_runtime
from creation_lib.audio import native_runtime as audio_native_runtime
from creation_lib.build.archive_plan import (
    DEFAULT_ARCHIVE_MAX_BYTES,
    ArchiveEntry,
    PlannedArchive,
    discover_mod_archives,
    plan_archive_outputs,
)

_log = logging.getLogger("modkit.packer")
_ATRAC9_SUBFORMAT_GUID = bytes.fromhex("d242e147ba368d4d88fc61654f8c836c")


def _find_archive2(game: str, game_dir: str = "") -> str:
    """Locate Archive2.exe from the game's Tools directory."""
    if not game_dir:
        raise RuntimeError(
            f"{game.upper()}_DIR not provided (needed for --use-archive2)"
        )
    path = os.path.join(game_dir, "Tools", "Archive2", "Archive2.exe")
    if not os.path.isfile(path):
        raise FileNotFoundError(f"Archive2.exe not found at {path}")
    return path


def _find_xtexconv(resource_dir: Path | str) -> str:
    """Locate xtexconv.exe in app or packaged creation_lib resources."""
    candidates = [Path(resource_dir) / "xtexconv.exe"]
    try:
        from creation_lib.paths import get_resource_dir

        candidates.append(get_resource_dir() / "xtexconv.exe")
    except Exception:
        pass
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    raise FileNotFoundError(f"xtexconv.exe not found at {candidates[0]}")


def _native_archive_type(
    game: str,
    texture_archive: bool,
    *,
    xbox: bool = False,
    ps: bool = False,
    og: bool = False,
) -> str | None:
    if xbox and game == "fo4":
        return "fo4xboxdds" if texture_archive else "fo4xbox"
    if ps and game == "fo4":
        return "fo4psdds" if texture_archive else "fo4ps"
    suffix = "dds" if texture_archive else ""
    if og and game == "fo4":
        return f"fo4og{suffix}"
    return {
        "fo4": f"fo4{suffix}",
        "fo76": f"fo76{suffix}",
        "starfield": "starfielddds" if texture_archive else "starfield",
        "skyrimse": "sse",
        "skyrim": "tes5",
        "oblivion": "tes4",
        "fo3": "fo3",
        "fonv": "fonv",
    }.get(game)


def _run_native_pack(
    source_dir: str,
    output_path: str,
    game: str,
    *,
    texture_archive: bool = False,
    xbox: bool = False,
    ps: bool = False,
    og: bool = False,
    compress: bool = True,
    compression_level: int | None = None,
    manifest_path: str | None = None,
    include_prefixes: list[str] | None = None,
    exclude_prefixes: list[str] | None = None,
    jobs: int | None = None,
):
    """Pack via the native Rust backend when the binding is available."""
    archive_type = _native_archive_type(game, texture_archive, xbox=xbox, ps=ps, og=og)
    if archive_type is None:
        raise RuntimeError(f"native packer does not support game: {game}")
    if not native_runtime.native_function_available("pack_archive"):
        raise RuntimeError("bsarchive_native.pack_archive() is not available")
    # FO4/FO76/Starfield texture archives must be compressed.
    if texture_archive and game in {"fo4", "fo76", "starfield"}:
        compress = True
    job_count = max(1, int(jobs)) if jobs is not None else None
    job_label = f" jobs={job_count}" if job_count is not None else ""
    _log.info(
        "bsarchive_native %s%s -> %s",
        archive_type,
        job_label,
        os.path.basename(output_path),
    )
    pack_kwargs = {
        "compress": compress,
        "compression_level": compression_level,
        "share_data": False,
        "manifest_path": manifest_path,
    }
    if include_prefixes is not None:
        pack_kwargs["include_prefixes"] = include_prefixes
    if exclude_prefixes is not None:
        pack_kwargs["exclude_prefixes"] = exclude_prefixes
    if job_count is not None:
        pack_kwargs["jobs"] = job_count
    native_runtime.pack_archive(
        source_dir,
        output_path,
        archive_type,
        **pack_kwargs,
    )
    size = os.path.getsize(output_path)
    _log.info("Created: %s (%.1f MB)", os.path.basename(output_path), size / (1024 * 1024))


def _run_native_pack_entries(
    entries: tuple[ArchiveEntry, ...],
    output_path: str,
    game: str,
    *,
    texture_archive: bool = False,
    xbox: bool = False,
    ps: bool = False,
    og: bool = False,
    compress: bool = True,
    compression_level: int | None = None,
    manifest_path: str | None = None,
    jobs: int | None = None,
):
    """Pack a planned archive directly from source files without staging."""
    archive_type = _native_archive_type(game, texture_archive, xbox=xbox, ps=ps, og=og)
    if archive_type is None:
        raise RuntimeError(f"native packer does not support game: {game}")
    if texture_archive and game in {"fo4", "fo76", "starfield"}:
        compress = True
    job_count = max(1, int(jobs)) if jobs is not None else None
    job_label = f" jobs={job_count}" if job_count is not None else ""
    native_entries = [(str(entry.source_path), entry.relative_path) for entry in entries]
    _log.info(
        "bsarchive_native %s entries=%d%s -> %s",
        archive_type,
        len(native_entries),
        job_label,
        os.path.basename(output_path),
    )
    native_runtime.pack_archive_entries(
        native_entries,
        output_path,
        archive_type,
        texture_archive=texture_archive,
        compress=compress,
        compression_level=compression_level,
        share_data=False,
        manifest_path=manifest_path,
        **({"jobs": job_count} if job_count is not None else {}),
    )
    size = os.path.getsize(output_path)
    _log.info("Created: %s (%.1f MB)", os.path.basename(output_path), size / (1024 * 1024))


def _run_native_pack_plans(
    plans: list[tuple[PlannedArchive, Path]],
    game: str,
    *,
    og: bool = False,
    total_workers: int = 0,
    progress: Callable[[dict], bool | None] | None = None,
) -> int:
    """Pack already-planned archives with the native batch scheduler."""
    native_plans: list[tuple[str, str, bool, list[tuple[str, str, int]]]] = []
    for planned, output_path in plans:
        archive_type = _native_archive_type(
            game,
            planned.texture_archive,
            og=og,
        )
        if archive_type is None:
            raise RuntimeError(f"native packer does not support game: {game}")
        entries = [
            (str(entry.source_path), entry.relative_path, int(entry.size))
            for entry in planned.entries
        ]
        native_plans.append(
            (str(output_path), archive_type, planned.texture_archive, entries)
        )

    def log_progress(event: dict) -> bool:
        message = event.get("message")
        if message:
            _log.info("%s", message)
        if progress is None:
            return True
        keep_going = progress(event)
        return True if keep_going is None else bool(keep_going)

    return native_runtime.pack_archive_plans(
        native_plans,
        total_workers=max(0, int(total_workers)),
        progress=log_progress,
    )


# Archive2 format mappings
_ARCHIVE2_FORMATS = {
    "main": ("General", "None"),
    "textures_pc": ("DDS", "Default"),
    "textures_xbox": ("XBoxDDS", "XBox"),
}


def _run_archive2(archive2: str, source_dir: str, output_path: str,
                   fmt: str, compression: str):
    """Run Archive2.exe to create a BA2 (official Bethesda tool)."""
    cmd = [
        archive2, source_dir,
        f"-create={output_path}",
        f"-root={source_dir}",
        f"-format={fmt}",
        f"-compression={compression}",
    ]
    _log.info("Archive2 %s/%s -> %s", fmt, compression, os.path.basename(output_path))
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        _log.error("Archive2 stderr: %s", result.stderr.strip())
        raise RuntimeError(f"Archive2 failed (exit {result.returncode})")
    size = os.path.getsize(output_path)
    _log.info("Created: %s (%.1f MB)", os.path.basename(output_path), size / (1024*1024))


def _write_ba2_reference_manifest(source_archive: str, manifest_path: str) -> str | None:
    """Write the ordered BA2 string table to manifest_path as a JSON array."""
    if not os.path.isfile(source_archive):
        return None

    with open(source_archive, "rb") as stream:
        header = stream.read(24)
        if len(header) < 24:
            return None
        magic, _version, _format, file_count, strings_offset = struct.unpack("<4sIIIQ", header)
        if magic != b"BTDX" or file_count == 0:
            return None

        stream.seek(strings_offset)
        names: list[str] = []
        for _ in range(file_count):
            raw_len = stream.read(2)
            if len(raw_len) < 2:
                return None
            (name_len,) = struct.unpack("<H", raw_len)
            raw_name = stream.read(name_len)
            if len(raw_name) < name_len:
                return None
            names.append(raw_name.decode("utf-8", errors="surrogateescape"))

    os.makedirs(os.path.dirname(manifest_path), exist_ok=True)
    with open(manifest_path, "w", encoding="utf-8", newline="\n") as stream:
        json.dump(names, stream, ensure_ascii=False)
    return manifest_path


def _tile_textures_for_xbox(src_dir: str, dest_dir: str, *, xtexconv_path: str):
    """Convert PC DDS textures to Xbox tiled format using xtexconv -xbox."""
    count = 0
    errors = 0

    for root, _dirs, files in os.walk(src_dir):
        dds_files = [f for f in files if f.lower().endswith(".dds")]
        if not dds_files:
            continue

        rel = os.path.relpath(root, src_dir)
        out_dir = os.path.join(dest_dir, rel) if rel != "." else dest_dir
        os.makedirs(out_dir, exist_ok=True)

        for f in dds_files:
            src_path = os.path.join(root, f)
            # xtexconv uses a different CLI surface than texconv and does not
            # accept the PC-style overwrite flag.
            cmd = [xtexconv_path, "-xbox", "-o", out_dir, src_path]
            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode != 0:
                output = "\n".join(
                    part for part in (result.stdout.strip(), result.stderr.strip()) if part
                )
                _log.warning(
                    "xtexconv failed for %s (exit %s): %s",
                    os.path.join(rel, f),
                    result.returncode,
                    output or "no output",
                )
                errors += 1
            else:
                count += 1

    _log.info("Tiled %d textures for Xbox (%d failed)", count, errors)
    if errors > 0 and count == 0:
        raise RuntimeError("All Xbox texture tiling failed")


def _has_files(directory: str) -> bool:
    """Check if directory contains any files recursively."""
    for _, _, files in os.walk(directory):
        if files:
            return True
    return False


def _non_texture_dir_prefixes(data_dir: str) -> list[str]:
    prefixes: list[str] = []
    for item in sorted(os.listdir(data_dir), key=lambda value: (value.lower(), value)):
        item_path = os.path.join(data_dir, item)
        if not os.path.isdir(item_path):
            continue
        if item.lower() == "textures":
            continue
        if _has_files(item_path):
            prefixes.append(f"{item}/")
    return prefixes


def _copy_non_texture_dirs(data_dir: str, dest: str) -> bool:
    """Copy all data/ subdirs except Textures/ to dest. Returns True if anything copied."""
    has_content = False
    for item in os.listdir(data_dir):
        item_path = os.path.join(data_dir, item)
        if not os.path.isdir(item_path):
            continue
        if item.lower() == "textures":
            continue
        if _has_files(item_path):
            shutil.copytree(item_path, os.path.join(dest, item))
            has_content = True
    return has_content


def _copy_root_strings_dir(strings_dir: str, dest: str) -> bool:
    if not os.path.isdir(strings_dir) or not _has_files(strings_dir):
        return False
    shutil.copytree(strings_dir, os.path.join(dest, "Strings"), dirs_exist_ok=True)
    return True


def _default_native_archive_workers() -> int:
    return max(1, (os.cpu_count() or 2) // 2)


def _inventory_tree_entries(root: Path, *, relative_prefix: str = "") -> list[ArchiveEntry]:
    entries: list[ArchiveEntry] = []
    if not root.is_dir():
        return entries
    for source_path in sorted(root.rglob("*"), key=lambda path: path.as_posix().lower()):
        if not source_path.is_file():
            continue
        rel_path = source_path.relative_to(root)
        if relative_prefix:
            rel_path = Path(relative_prefix) / rel_path
        entries.append(
            ArchiveEntry(
                rel_path.as_posix(),
                source_path,
                source_path.stat().st_size,
            )
        )
    return entries


def _inventory_data_entries(data_dir: Path, *, include_textures: bool = True) -> list[ArchiveEntry]:
    entries: list[ArchiveEntry] = []
    if not data_dir.is_dir():
        return entries

    for child in sorted(data_dir.iterdir(), key=lambda path: path.as_posix().lower()):
        if child.is_file():
            entries.append(ArchiveEntry(child.name, child, child.stat().st_size))
            continue
        if not child.is_dir():
            continue
        if not include_textures and child.name.lower() == "textures":
            continue
        entries.extend(_inventory_tree_entries(child, relative_prefix=child.name))

    return sorted(entries, key=lambda entry: entry.relative_path.lower())


def _inventory_root_strings_entries(strings_dir: Path) -> list[ArchiveEntry]:
    entries: list[ArchiveEntry] = []
    if not strings_dir.is_dir():
        return entries
    for source_path in sorted(strings_dir.rglob("*"), key=lambda path: path.as_posix().lower()):
        if not source_path.is_file():
            continue
        # Dot-prefixed files are orphaned temp-save string tables, not
        # shippable localization — never pack them.
        if source_path.name.startswith("."):
            continue
        rel_path = Path("Strings") / source_path.relative_to(strings_dir)
        entries.append(
            ArchiveEntry(
                rel_path.as_posix(),
                source_path,
                source_path.stat().st_size,
            )
        )
    return entries


def _stage_archive_entries(entries: tuple[ArchiveEntry, ...], dest_root: Path) -> None:
    if dest_root.is_dir():
        shutil.rmtree(dest_root)
    for entry in entries:
        dest_path = dest_root / Path(entry.relative_path)
        dest_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(entry.source_path, dest_path)


def _validate_archive_size(output_path: Path, max_bytes: int) -> None:
    actual_size = output_path.stat().st_size
    if actual_size > max_bytes:
        raise RuntimeError(
            f"{output_path.name} packed to {actual_size} bytes, "
            f"exceeding archive max size {max_bytes} bytes. "
            "For texture archives, lower the texture resolution; otherwise increase "
            "the archive size cap only if the target platform can tolerate it."
        )


def _is_platform_archive(path: Path, platform_suffix: str) -> bool:
    try:
        label = path.stem.split(" - ", 1)[1]
    except IndexError:
        return False
    if platform_suffix == "_xbox":
        return label.endswith("_xbox")
    if platform_suffix == "_ps":
        return label.endswith("_ps")
    return not label.endswith(("_xbox", "_ps"))


def _repack_playstation_fuz(
    source_fuz: Path,
    source_audio: Path,
    output_fuz: Path,
    *,
    require_at9: bool = False,
) -> None:
    fuz_bytes = source_fuz.read_bytes()
    if len(fuz_bytes) < 12 or fuz_bytes[:4] != b"FUZE":
        raise ValueError(f"invalid FUZ file for PlayStation: {source_fuz}")
    lip_size = struct.unpack_from("<I", fuz_bytes, 8)[0]
    audio_offset = 12 + lip_size
    if audio_offset > len(fuz_bytes):
        raise ValueError(f"invalid FUZ lip size for PlayStation: {source_fuz}")

    audio_bytes = source_audio.read_bytes()
    wave_format = _riff_wave_format_tag(audio_bytes)
    if wave_format is None or 0x0160 <= wave_format <= 0x0166:
        raise ValueError(f"PlayStation FUZ companion is not supported audio: {source_audio}")
    if require_at9 and not _is_atrac9_wave(audio_bytes):
        raise ValueError(f"PlayStation FUZ companion is not ATRAC9: {source_audio}")

    output_fuz.parent.mkdir(parents=True, exist_ok=True)
    output_fuz.write_bytes(fuz_bytes[:audio_offset] + audio_bytes)


def _riff_wave_format_tag(payload: bytes) -> int | None:
    if len(payload) < 12 or payload[:4] != b"RIFF" or payload[8:12] != b"WAVE":
        return None
    offset = 12
    while offset + 8 <= len(payload):
        chunk_size = struct.unpack_from("<I", payload, offset + 4)[0]
        chunk_data = offset + 8
        chunk_end = chunk_data + chunk_size
        if chunk_end > len(payload):
            return None
        if payload[offset : offset + 4] == b"fmt " and chunk_size >= 2:
            return struct.unpack_from("<H", payload, chunk_data)[0]
        offset = chunk_end + (chunk_size & 1)
    return None


def _is_atrac9_wave(payload: bytes) -> bool:
    if _riff_wave_format_tag(payload) != 0xFFFE:
        return False
    offset = 12
    while offset + 8 <= len(payload):
        chunk_size = struct.unpack_from("<I", payload, offset + 4)[0]
        chunk_data = offset + 8
        chunk_end = chunk_data + chunk_size
        if chunk_end > len(payload):
            return False
        if payload[offset : offset + 4] == b"fmt ":
            return (
                chunk_size >= 40
                and payload[chunk_data + 24 : chunk_data + 40]
                == _ATRAC9_SUBFORMAT_GUID
            )
        offset = chunk_end + (chunk_size & 1)
    return False


def _fuz_embeds_atrac9(path: Path) -> bool:
    payload = path.read_bytes()
    if len(payload) < 12 or payload[:4] != b"FUZE":
        return False
    audio_offset = 12 + struct.unpack_from("<I", payload, 8)[0]
    return audio_offset <= len(payload) and _is_atrac9_wave(payload[audio_offset:])


def _prepare_playstation_audio_entries(
    entries: list[ArchiveEntry],
    temp_dir: Path,
    *,
    convert_to_at9: bool = False,
) -> tuple[list[ArchiveEntry], bool]:
    use_at9 = convert_to_at9
    prepared: list[ArchiveEntry] = []
    adjusted = False
    missing_wavs: list[str] = []
    emitted_paths: set[str] = set()
    existing_at9 = {
        entry.relative_path.lower(): entry.source_path
        for entry in entries
        if Path(entry.relative_path).suffix.lower() == ".at9"
    }
    encoded_at9: dict[str, Path] = {}

    def at9_for(source_wav: Path, relative_at9: str) -> Path:
        key = relative_at9.lower()
        if key in existing_at9:
            return existing_at9[key]
        if key not in encoded_at9:
            output_at9 = temp_dir / "ps_audio" / "at9" / Path(relative_at9)
            audio_native_runtime.encode_at9(os.fspath(source_wav), os.fspath(output_at9))
            encoded_at9[key] = output_at9
        return encoded_at9[key]

    for entry in entries:
        suffix = Path(entry.relative_path).suffix.lower()
        if not use_at9 and suffix not in {".xwm", ".fuz"}:
            prepared.append(entry)
            continue
        if use_at9 and suffix not in {".wav", ".xwm", ".fuz"}:
            key = entry.relative_path.lower()
            if key not in emitted_paths:
                prepared.append(entry)
                emitted_paths.add(key)
            continue

        wav_path = entry.source_path if suffix == ".wav" else entry.source_path.with_suffix(".wav")
        if use_at9 and suffix == ".fuz" and not wav_path.is_file():
            relative_at9 = Path(entry.relative_path).with_suffix(".at9").as_posix()
            source_at9 = existing_at9.get(relative_at9.lower())
            if source_at9 is not None:
                output_fuz = temp_dir / "ps_audio" / Path(entry.relative_path)
                _repack_playstation_fuz(
                    entry.source_path, source_at9, output_fuz, require_at9=True
                )
                prepared.append(
                    ArchiveEntry(entry.relative_path, output_fuz, output_fuz.stat().st_size)
                )
                adjusted = True
                continue
            if _fuz_embeds_atrac9(entry.source_path):
                prepared.append(entry)
                continue
        if not wav_path.is_file():
            missing_wavs.append(entry.relative_path)
            continue

        adjusted = True
        if use_at9 and suffix in {".wav", ".xwm"}:
            relative_at9 = Path(entry.relative_path).with_suffix(".at9").as_posix()
            key = relative_at9.lower()
            if key not in emitted_paths and key not in existing_at9:
                source_at9 = at9_for(wav_path, relative_at9)
                prepared.append(
                    ArchiveEntry(relative_at9, source_at9, source_at9.stat().st_size)
                )
                emitted_paths.add(key)
            continue
        if suffix == ".xwm":
            continue

        output_fuz = temp_dir / "ps_audio" / Path(entry.relative_path)
        source_audio = wav_path
        if use_at9:
            relative_at9 = Path(entry.relative_path).with_suffix(".at9").as_posix()
            source_audio = at9_for(wav_path, relative_at9)
        _repack_playstation_fuz(
            entry.source_path, source_audio, output_fuz, require_at9=use_at9
        )
        prepared.append(
            ArchiveEntry(
                relative_path=entry.relative_path,
                source_path=output_fuz,
                size=output_fuz.stat().st_size,
            )
        )

    if missing_wavs:
        examples = ", ".join(missing_wavs[:3])
        extra = "" if len(missing_wavs) <= 3 else f" (+{len(missing_wavs) - 3} more)"
        raise ValueError(
            "PlayStation audio requires a companion WAV for every XWM/FUZ file: "
            f"{examples}{extra}"
        )

    return prepared, adjusted


def _cleanup_obsolete_platform_archives(
    mod_dir: Path,
    mod_name: str,
    platform_suffix: str,
    expected_names: set[str],
) -> None:
    for archive in discover_mod_archives(mod_dir, mod_name):
        if archive.name in expected_names or not _is_platform_archive(archive, platform_suffix):
            continue
        archive.unlink()
        _log.info("Removed old archive: %s", archive.name)


def _can_use_direct_compact_ba2_path(
    plans: list[PlannedArchive],
    *,
    mod_name: str,
    archive_ext: str,
    platform_suffix: str,
) -> bool:
    if not plans:
        return True
    expected = {
        "Main": f"{mod_name} - Main{platform_suffix}.{archive_ext}",
        "Textures": f"{mod_name} - Textures{platform_suffix}.{archive_ext}",
    }
    for plan in plans:
        if plan.label not in expected or plan.output_name != expected[plan.label]:
            return False
        if any(len(Path(entry.relative_path).parts) == 1 for entry in plan.entries):
            return False
    return True


def _resize_textures(src_dir: str, dest_dir: str, max_res: int):
    """Resize textures in src_dir to max_res, output to dest_dir."""
    from creation_lib.dds import batch_resize

    _log.info("Resizing textures to %dpx max...", max_res)
    result = batch_resize(
        input_dir=src_dir,
        output_dir=dest_dir,
        sizes=[max_res],
        generate_mips=True,
        no_upscale=True,
        per_size_subfolders=False,
    )
    _log.info("Resized: %d files, %d failed", result["processed"], result["failed"])
    if result["failed"] > 0:
        for err in result["errors"][:5]:
            _log.error("  %s", err)


def _find_effects_dir(texture_root: str) -> str | None:
    """Return the first child directory named Effects/effects, if present."""
    if not os.path.isdir(texture_root):
        return None
    for entry in os.listdir(texture_root):
        if entry.lower() == "effects":
            path = os.path.join(texture_root, entry)
            if os.path.isdir(path):
                return path
    return None


def _prepare_texture_root(
    texture_src_dir: str,
    dest_root: str,
    max_res: int,
    effects_max_res: int,
):
    """Build a staged texture tree under dest_root/Textures."""
    dest_textures_dir = os.path.join(dest_root, "Textures")
    if os.path.isdir(dest_root):
        shutil.rmtree(dest_root)
    os.makedirs(dest_root, exist_ok=True)
    shutil.copytree(texture_src_dir, dest_textures_dir)

    if max_res > 0:
        # Resize everything except Textures/Effects first. Effects gets its own pass below.
        _log.info("Preparing %s at max %dpx", os.path.basename(dest_root), max_res)
        from creation_lib.dds import batch_resize
        batch_resize(
            input_dir=texture_src_dir,
            output_dir=dest_textures_dir,
            sizes=[max_res],
            generate_mips=True,
            no_upscale=True,
            per_size_subfolders=False,
            ignore_patterns=["Effects", "effects"],
        )

    if effects_max_res > 0:
        effects_src_dir = _find_effects_dir(texture_src_dir)
        effects_dest_dir = _find_effects_dir(dest_textures_dir)
        if effects_src_dir and effects_dest_dir:
            _log.info("Preparing %s/Effects at max %dpx", os.path.basename(dest_root), effects_max_res)
            from creation_lib.dds import batch_resize
            batch_resize(
                input_dir=effects_src_dir,
                output_dir=effects_dest_dir,
                sizes=[effects_max_res],
                generate_mips=True,
                no_upscale=True,
                per_size_subfolders=False,
            )


def pack_mod(mod_name: str, *, pc: bool = True, xbox: bool = False, ps: bool = False,
             pc_max_res: int = 0, pc_effects_max_res: int | None = None,
             xbox_max_res: int = 0, xbox_effects_max_res: int | None = None,
             ps_max_res: int = 0, ps_effects_max_res: int | None = None,
             game: str = "fo4", use_archive2: bool = False, game_dir: str = "",
             project_root: Path | str | None = None,
             resource_dir: Path | str | None = None,
             archive_max_bytes: int | None = None,
             expanded_archives: bool = False,
             archive_workers: int = 0,
             archive_output_dir: Path | str | None = None,
             plugin_base_name: str | None = None,
             ps_at9: bool = False,
             fo4_ba2_target: str = "auto"):
    """Pack archives for a mod with the native packer, or Archive2 when
    ``use_archive2`` is set.

    Args:
        game_dir: Game install directory (value of {GAME}_DIR). Required when
                  use_archive2=True so Archive2.exe can be located.
        archive_workers: Native worker count; 0 uses regen conversion's
                  default. When >1 on the direct-entries path (PC BA2, no
                  resize/tile/staging), that many archives pack concurrently.
                  Each in-flight archive stages up to archive_max_bytes of
                  temp payloads, so raise with disk in mind.
        archive_max_bytes: Per-archive size cap used only when
                  expanded_archives is enabled.
        archive_output_dir: Directory for completed BA2/BSA files. Defaults to
                  the mod directory.
        fo4_ba2_target: ``"auto"`` detects game_dir; ``"og"`` emits FO4 v1 BA2s;
                  ``"nextgen"`` emits v8. Unknown installs default to nextgen.
        ps_at9: Encode PlayStation WAV/XWM audio as ATRAC9 and embed ATRAC9
                  audio in FUZ files using the built-in native encoder.
    """
    fo4_ba2_target = str(fo4_ba2_target).strip().lower()
    if fo4_ba2_target not in {"auto", "og", "nextgen"}:
        raise ValueError("fo4_ba2_target must be 'auto', 'og' or 'nextgen'")
    if game == "fo4" and fo4_ba2_target == "auto":
        from creation_lib.core.fo4_version import detect_ba2_target
        fo4_ba2_target, version = detect_ba2_target(game_dir)
        if version is None:
            _log.warning("Could not detect Fallout4.exe version; using nextgen BA2s. Set --ba2-target og for an older install.")
        else:
            _log.info("Fallout 4 %s: using %s BA2s", version, fo4_ba2_target)
    og_target = game == "fo4" and fo4_ba2_target == "og"
    og_pack_kwargs = {"og": True} if og_target else {}
    profile = get_profile(game)
    archive_format = profile.archive_format
    archive_ext = "ba2" if archive_format == "ba2" else "bsa"
    if expanded_archives:
        archive_cap = (
            DEFAULT_ARCHIVE_MAX_BYTES
            if archive_max_bytes is None
            else int(archive_max_bytes)
        )
        if archive_cap <= 0:
            raise ValueError("archive_max_bytes must be greater than 0")
    else:
        archive_cap = DEFAULT_ARCHIVE_MAX_BYTES

    def _validate_packed_archive_size(output_path: Path) -> None:
        if expanded_archives:
            _validate_archive_size(output_path, archive_cap)
    if pc_effects_max_res is None:
        pc_effects_max_res = pc_max_res
    if xbox_effects_max_res is None:
        xbox_effects_max_res = xbox_max_res
    if ps_effects_max_res is None:
        ps_effects_max_res = ps_max_res

    # Archive2 only supports BA2 -- use the native packer for BSA games.
    if use_archive2 and archive_format != "ba2":
        _log.info("Archive2 does not support %s format, using native packer", archive_ext.upper())
        use_archive2 = False

    archive2_path = ""
    if use_archive2 and (pc or xbox):
        archive2_path = _find_archive2(game, game_dir=game_dir)
        _log.info("Using Archive2: %s", archive2_path)
    elif not native_runtime.native_function_available("pack_archive"):
        raise RuntimeError("bsarchive_native.pack_archive() is required for archive packing")
    if ps and not native_runtime.native_function_available("pack_archive"):
        raise RuntimeError("bsarchive_native.pack_archive() is required for PlayStation archives")

    if project_root is None:
        raise ValueError("project_root is required")
    if xbox and resource_dir is None:
        raise ValueError("resource_dir is required for Xbox texture tiling")
    project_root = Path(project_root)
    xtexconv_path = _find_xtexconv(resource_dir) if xbox else ""
    mod_dir_path = project_root / "mods" / mod_name
    archive_output_dir_path = (
        Path(archive_output_dir) if archive_output_dir is not None else mod_dir_path
    )
    data_dir_path = mod_dir_path / "data"
    strings_dir_path = mod_dir_path / "Strings"
    mod_dir = str(mod_dir_path)
    data_dir = str(data_dir_path)
    strings_dir = str(strings_dir_path)
    manifest_path = os.path.join(mod_dir, "archive.achlist")
    if not os.path.isfile(manifest_path):
        manifest_path = None

    if not mod_dir_path.is_dir():
        raise RuntimeError(f"{mod_dir} not found")
    if not data_dir_path.is_dir():
        raise RuntimeError(f"{data_dir} not found -- nothing to pack")
    archive_output_dir_path.mkdir(parents=True, exist_ok=True)

    archive_base = plugin_base_name or mod_name

    can_use_native_mod_pack = (
        not use_archive2
        and not og_target
        and pc
        and not xbox
        and not ps
        and pc_max_res <= 0
        and pc_effects_max_res <= 0
        and native_runtime.native_function_available("pack_mod_archives")
    )
    if can_use_native_mod_pack:
        _log.info("Using native mod archive planner/packer")
        native_archive_workers = (
            int(archive_workers)
            if archive_workers and archive_workers > 0
            else _default_native_archive_workers()
        )

        def _on_native_progress(event: dict) -> bool:
            message = event.get("message")
            if message:
                _log.info("%s", message)
            return True

        result = native_runtime.pack_mod_archives(
            {
                "mod_name": archive_base,
                "mod_dir": str(archive_output_dir_path),
                "data_dir": str(data_dir_path),
                "strings_dir": str(strings_dir_path),
                "game": game,
                "archive_ext": archive_ext,
                "archive_max_bytes": archive_cap,
                "expanded_archives": expanded_archives,
                "pc": pc,
                "xbox": xbox,
                "archive_workers": native_archive_workers,
                "manifest_path": manifest_path,
            },
            progress=_on_native_progress,
        )
        for archive in result.get("archives", []):
            archive_name = archive.get("name")
            if archive_name:
                _validate_packed_archive_size(archive_output_dir_path / archive_name)
        return None

    temp_dir = os.path.join(mod_dir, "_deploy_tmp")

    tex_src_dir = os.path.join(data_dir, "Textures")
    has_textures = os.path.isdir(tex_src_dir) and _has_files(tex_src_dir)
    has_root_strings = os.path.isdir(strings_dir) and _has_files(strings_dir)

    def _build_texture_stage(stage_name: str, max_res: int, effects_max_res: int, tile_for_xbox: bool = False) -> str:
        source_root = os.path.join(temp_dir, f"{stage_name}_src")
        _prepare_texture_root(tex_src_dir, source_root, max_res, effects_max_res)
        if not tile_for_xbox:
            return source_root

        tiled_root = os.path.join(temp_dir, stage_name)
        if os.path.isdir(tiled_root):
            shutil.rmtree(tiled_root)
        shutil.copytree(source_root, tiled_root)
        _tile_textures_for_xbox(
            os.path.join(source_root, "Textures"),
            os.path.join(tiled_root, "Textures"),
            xtexconv_path=xtexconv_path,
        )
        return tiled_root

    platforms = []
    if pc:
        platforms.append(("pc", "", pc_max_res, pc_effects_max_res, False))
    if xbox:
        platforms.append(("xbox", "_xbox", xbox_max_res, xbox_effects_max_res, True))
    if ps:
        platforms.append(("ps", "_ps", ps_max_res, ps_effects_max_res, False))

    try:
        for platform, suffix, max_res, effects_max_res, is_xbox in platforms:
            is_ps = platform == "ps"
            ps_pack_kwargs = {"ps": True} if is_ps else {}
            _log.info("=== Packing %s %ss (%s) ===", platform.upper(), archive_ext.upper(), game)

            # Clean temp dir for this platform so each archive build starts from a fresh staging tree.
            if os.path.isdir(temp_dir):
                _log.info("Cleaning archive temp dir: %s", temp_dir)
                shutil.rmtree(temp_dir)

            inventory_started = time.perf_counter()
            _log.info("Archive inventory: scanning non-texture data for platform=%s", platform)
            main_entries = _inventory_data_entries(data_dir_path, include_textures=False)
            main_entries.extend(_inventory_root_strings_entries(strings_dir_path))
            ps_audio_adjusted = False
            if is_ps:
                main_entries, ps_audio_adjusted = _prepare_playstation_audio_entries(
                    main_entries,
                    Path(temp_dir),
                    convert_to_at9=ps_at9,
                )
            texture_entries: list[ArchiveEntry] = []
            texture_needs_stage = has_textures and (max_res > 0 or effects_max_res > 0 or is_xbox)
            if has_textures:
                _log.info("Archive inventory: scanning textures for platform=%s", platform)
                if texture_needs_stage:
                    _log.info(
                        "Archive inventory: staging textures for platform=%s max_res=%s effects_max_res=%s xbox=%s",
                        platform,
                        max_res,
                        effects_max_res,
                        is_xbox,
                    )
                    texture_stage_root = Path(
                        _build_texture_stage(
                            f"textures_{platform}",
                            max_res,
                            effects_max_res,
                            tile_for_xbox=is_xbox,
                        )
                    )
                    _log.info("Archive inventory: scanning staged textures for platform=%s", platform)
                    texture_entries = _inventory_tree_entries(
                        texture_stage_root / "Textures",
                        relative_prefix="Textures",
                    )
                else:
                    texture_entries = _inventory_tree_entries(
                        data_dir_path / "Textures",
                        relative_prefix="Textures",
                    )
            all_entries = [*main_entries, *texture_entries]
            _log.info(
                "Archive inventory: platform=%s files=%d bytes=%.1f MB elapsed=%.3fs",
                platform,
                len(all_entries),
                sum(entry.size for entry in all_entries) / (1024 * 1024),
                time.perf_counter() - inventory_started,
            )

            planning_started = time.perf_counter()
            plans = plan_archive_outputs(
                archive_base,
                all_entries,
                archive_ext,
                suffix,
                archive_cap,
                game=game,
                expanded_archives=expanded_archives,
            )
            _log.info(
                "Archive planning: platform=%s plans=%d elapsed=%.3fs",
                platform,
                len(plans),
                time.perf_counter() - planning_started,
            )
            expected_names = {plan.output_name for plan in plans}

            can_pack_compact_ba2_direct = (
                archive_format == "ba2"
                and not use_archive2
                and not is_xbox
                and max_res <= 0
                and effects_max_res <= 0
                and not has_root_strings
                and not ps_audio_adjusted
                and _can_use_direct_compact_ba2_path(
                    plans,
                    mod_name=archive_base,
                    archive_ext=archive_ext,
                    platform_suffix=suffix,
                )
            )
            if can_pack_compact_ba2_direct:
                planned_by_label = {plan.label: plan for plan in plans}
                if "Main" in planned_by_label:
                    main_archive = str(
                        archive_output_dir_path / planned_by_label["Main"].output_name
                    )
                    main_manifest_path = manifest_path
                    reference_main_manifest = _write_ba2_reference_manifest(
                        main_archive,
                        os.path.join(temp_dir, f"main{suffix}_reference_manifest.json"),
                    )
                    if reference_main_manifest:
                        main_manifest_path = reference_main_manifest
                    main_include_prefixes = _non_texture_dir_prefixes(data_dir)
                    _run_native_pack(
                        data_dir,
                        main_archive,
                        game,
                        manifest_path=main_manifest_path,
                        include_prefixes=main_include_prefixes,
                        **og_pack_kwargs,
                        **ps_pack_kwargs,
                    )
                    _validate_packed_archive_size(Path(main_archive))
                else:
                    _log.info("No non-texture assets -- skipping Main archive")

                if "Textures" in planned_by_label:
                    tex_archive = str(
                        archive_output_dir_path / planned_by_label["Textures"].output_name
                    )
                    tex_manifest_path = manifest_path
                    reference_tex_manifest = _write_ba2_reference_manifest(
                        tex_archive,
                        os.path.join(temp_dir, f"textures{suffix}_reference_manifest.json"),
                    )
                    if reference_tex_manifest:
                        tex_manifest_path = reference_tex_manifest
                    _run_native_pack(
                        data_dir,
                        tex_archive,
                        game,
                        texture_archive=True,
                        manifest_path=tex_manifest_path,
                        include_prefixes=["Textures/"],
                        **og_pack_kwargs,
                        **ps_pack_kwargs,
                    )
                    _validate_packed_archive_size(Path(tex_archive))
                else:
                    _log.info("No textures -- skipping Textures archive")
                _cleanup_obsolete_platform_archives(
                    archive_output_dir_path, archive_base, suffix, expected_names
                )
                continue

            if not plans:
                _log.info("No assets -- skipping archive packing")
                _cleanup_obsolete_platform_archives(
                    archive_output_dir_path, archive_base, suffix, expected_names
                )
                continue

            # `can_pack_plan_entries_direct` is plan-independent — it depends only
            # on per-platform constants — so either every plan packs directly from
            # source entries or every plan is staged. Only the direct path is
            # thread-safe to fan out: each archive reads distinct sources, writes a
            # distinct output, and the native packer's temp payload dir is uniquely
            # prefixed per output. The staged path shares temp_dir and stays serial.
            can_pack_plan_entries_direct = (
                not use_archive2
                and not is_xbox
                and archive_format == "ba2"
                and not texture_needs_stage
            )

            def _pack_plan(plan):
                output_path = archive_output_dir_path / plan.output_name
                plan_manifest_path = manifest_path
                reference_manifest = _write_ba2_reference_manifest(
                    str(output_path),
                    os.path.join(temp_dir, f"{plan.label.lower()}{suffix}_reference_manifest.json"),
                )
                if reference_manifest:
                    plan_manifest_path = reference_manifest
                plan_bytes = sum(entry.size for entry in plan.entries) / (1024 * 1024)

                if can_pack_plan_entries_direct:
                    pack_started = time.perf_counter()
                    _run_native_pack_entries(
                        plan.entries,
                        str(output_path),
                        game,
                        texture_archive=plan.texture_archive,
                        manifest_path=plan_manifest_path,
                        **og_pack_kwargs,
                        **ps_pack_kwargs,
                    )
                    _log.info(
                        "Archive packed direct: name=%s files=%d bytes=%.1f MB elapsed=%.3fs",
                        plan.output_name,
                        len(plan.entries),
                        plan_bytes,
                        time.perf_counter() - pack_started,
                    )
                    _validate_packed_archive_size(output_path)
                    return

                source_root = Path(temp_dir) / f"planned_{plan.label.lower()}"
                stage_started = time.perf_counter()
                _stage_archive_entries(plan.entries, source_root)
                _log.info(
                    "Archive staged: name=%s files=%d bytes=%.1f MB elapsed=%.3fs",
                    plan.output_name,
                    len(plan.entries),
                    plan_bytes,
                    time.perf_counter() - stage_started,
                )

                if use_archive2 and not is_ps:
                    if plan.texture_archive:
                        tex_fmt = "XBoxDDS" if is_xbox else "DDS"
                        tex_comp = "XBox" if is_xbox else "Default"
                        _run_archive2(archive2_path, str(source_root), str(output_path), tex_fmt, tex_comp)
                    else:
                        _run_archive2(archive2_path, str(source_root), str(output_path), "General", "None")
                elif (is_xbox or is_ps) and archive_format == "ba2":
                    _run_native_pack(
                        str(source_root),
                        str(output_path),
                        game,
                        texture_archive=plan.texture_archive,
                        xbox=is_xbox,
                        compress=plan.texture_archive if is_xbox else True,
                        manifest_path=plan_manifest_path,
                        **og_pack_kwargs,
                        **ps_pack_kwargs,
                    )
                else:
                    _run_native_pack(
                        str(source_root),
                        str(output_path),
                        game,
                        texture_archive=plan.texture_archive,
                        manifest_path=plan_manifest_path,
                        **og_pack_kwargs,
                    )
                _validate_packed_archive_size(output_path)

            pack_workers = (
                min(archive_workers, len(plans))
                if archive_workers > 1 and can_pack_plan_entries_direct
                else 1
            )
            if pack_workers > 1:
                _log.info(
                    "Packing %d archives concurrently (%d workers)", len(plans), pack_workers
                )
                first_error: Exception | None = None
                with ThreadPoolExecutor(
                    max_workers=pack_workers, thread_name_prefix="ba2-pack"
                ) as executor:
                    futures = [executor.submit(_pack_plan, plan) for plan in plans]
                    for future in futures:
                        try:
                            future.result()
                        except Exception as exc:  # first error re-raised after join
                            if first_error is None:
                                first_error = exc
                if first_error is not None:
                    raise first_error
            else:
                for plan in plans:
                    _pack_plan(plan)

            _cleanup_obsolete_platform_archives(
                archive_output_dir_path, archive_base, suffix, expected_names
            )
    finally:
        if os.path.isdir(temp_dir):
            shutil.rmtree(temp_dir)

    _log.info("Done.")

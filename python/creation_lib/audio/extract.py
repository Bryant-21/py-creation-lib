"""Decode game voice containers: FUZ -> XWM (+LIP) -> WAV.

The decode counterpart of creation_lib.audio.release, using the same bundled
tools. xWMAEncode decodes when called without -b; BmlFuzDecode always writes
its output next to its input, so callers get a staged copy instead.
"""

from __future__ import annotations

import logging
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from creation_lib.audio import native_runtime as audio_native_runtime
from creation_lib.audio.release import _CREATE_NO_WINDOW, _tool_paths

_log = logging.getLogger("creation_lib.audio.extract")


def _tools(resource_dir: str | os.PathLike[str] | None) -> dict[str, str]:
    if resource_dir is None:
        from creation_lib.paths import get_resource_dir

        resource_dir = get_resource_dir()
    return _tool_paths(resource_dir)


def _require_tool(resource_dir: str | os.PathLike[str] | None, key: str) -> str:
    path = _tools(resource_dir)[key]
    if not os.path.isfile(path):
        raise FileNotFoundError(f"{os.path.basename(path)} not found: {path}")
    return path


def _run(cmd: list, what: str) -> None:
    """Run a tool, raising RuntimeError with its own output on failure.

    xWMAEncode reports errors on stdout, not stderr, so both are inspected.
    """
    proc = subprocess.run(
        [str(part) for part in cmd],
        capture_output=True,
        text=True,
        creationflags=_CREATE_NO_WINDOW,
    )
    if proc.returncode != 0:
        detail = (proc.stdout or "").strip() or (proc.stderr or "").strip() or "no output"
        raise RuntimeError(f"{what} failed: {detail}")


def decode_xwm(
    xwm_path: str | os.PathLike[str],
    wav_path: str | os.PathLike[str],
    *,
    resource_dir: str | os.PathLike[str] | None = None,
) -> Path:
    """Decode an XWM file to WAV. Returns the WAV path."""
    source = Path(xwm_path)
    target = Path(wav_path)
    if not source.is_file():
        raise FileNotFoundError(f"XWM not found: {source}")
    tool = _require_tool(resource_dir, "xwma")
    target.parent.mkdir(parents=True, exist_ok=True)
    _run([tool, source, target], "XWM decode")
    if not target.is_file():
        raise RuntimeError(f"XWM decode produced no WAV for {source.name}")
    _log.debug("XWM decoded: %s", target)
    return target


def decode_fuz(
    fuz_path: str | os.PathLike[str],
    output_dir: str | os.PathLike[str],
    *,
    keep_lip: bool = False,
    resource_dir: str | os.PathLike[str] | None = None,
) -> Path:
    """Decode a FUZ into <output_dir>/<stem>.xwm, optionally keeping the LIP.

    Returns the XWM path. Chain decode_xwm for audio.
    """
    source = Path(fuz_path)
    if not source.is_file():
        raise FileNotFoundError(f"FUZ not found: {source}")
    tool = _require_tool(resource_dir, "bmlfuz_decode")
    out_dir = Path(output_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="creation_lib_fuz_") as tmp:
        tmp_dir = Path(tmp)
        staged = tmp_dir / source.name
        shutil.copy2(source, staged)
        _run([tool, staged], "FUZ decode")

        produced = tmp_dir / f"{source.stem}.xwm"
        if not produced.is_file():
            raise RuntimeError(f"FUZ decode produced no XWM for {source.name}")
        target = out_dir / produced.name
        shutil.copy2(produced, target)

        if keep_lip:
            lip = tmp_dir / f"{source.stem}.lip"
            if lip.is_file():
                shutil.copy2(lip, out_dir / lip.name)

    _log.debug("FUZ decoded: %s", target)
    return target


def decode_to_wav(
    path: str | os.PathLike[str],
    output_dir: str | os.PathLike[str],
    *,
    keep_lip: bool = False,
    resource_dir: str | os.PathLike[str] | None = None,
    ffmpeg_path: str | None = None,
    wwise_codebooks: str | os.PathLike[str] | None = None,
) -> Path:
    """Decode any supported voice container to WAV in output_dir.

    .wav is returned untouched. .xwm and .fuz use the bundled tools; .ogg goes
    through ffmpeg. .wem is Wwise Vorbis, which ffmpeg cannot read, so it is
    first rebuilt as Ogg Vorbis from the codebook table inside
    `wwise_codebooks`, the game executable (Starfield.exe).
    """
    source = Path(path)
    out_dir = Path(output_dir)
    suffix = source.suffix.lower()

    if suffix == ".wav":
        return source
    if suffix == ".xwm":
        return decode_xwm(source, out_dir / f"{source.stem}.wav", resource_dir=resource_dir)
    if suffix == ".fuz":
        with tempfile.TemporaryDirectory(prefix="creation_lib_fuz_wav_") as tmp:
            tmp_dir = Path(tmp)
            xwm = decode_fuz(source, tmp_dir, keep_lip=keep_lip, resource_dir=resource_dir)
            if keep_lip:
                lip = tmp_dir / f"{source.stem}.lip"
                if lip.is_file():
                    out_dir.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(lip, out_dir / lip.name)
            return decode_xwm(xwm, out_dir / f"{source.stem}.wav", resource_dir=resource_dir)
    if suffix == ".ogg":
        return _ffmpeg_to_wav(source, out_dir / f"{source.stem}.wav", ffmpeg_path)
    if suffix == ".wem":
        if wwise_codebooks is None:
            raise ValueError(f"decoding {source.name} needs wwise_codebooks, the game executable holding them")
        with tempfile.TemporaryDirectory(prefix="creation_lib_wem_") as tmp:
            ogg = Path(tmp) / f"{source.stem}.ogg"
            audio_native_runtime.wem_to_ogg(os.fspath(source), os.fspath(ogg), os.fspath(wwise_codebooks))
            return _ffmpeg_to_wav(ogg, out_dir / f"{source.stem}.wav", ffmpeg_path)

    raise ValueError(f"Unsupported voice audio format: {source.suffix}")


def _ffmpeg_to_wav(source: Path, target: Path, ffmpeg_path: str | None) -> Path:
    tool = ffmpeg_path or shutil.which("ffmpeg")
    if not tool:
        raise FileNotFoundError("ffmpeg not found on PATH")
    target.parent.mkdir(parents=True, exist_ok=True)
    _run([tool, "-y", "-i", source, "-c:a", "pcm_s16le", target], "Audio decode")
    if not target.is_file():
        raise RuntimeError(f"Audio decode produced no WAV for {source.name}")
    return target

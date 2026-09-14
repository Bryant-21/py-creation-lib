"""Deploy/undeploy pipeline for mods.

Replaces utils/deploy_mod.sh and utils/undeploy_mod.sh.
All functions accept explicit Path arguments.
"""
from __future__ import annotations

import errno
import hashlib
import logging
import os
import shutil
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from pathlib import Path

from creation_lib.core.game_profiles import get_profile
from creation_lib.esp.validate import validate_authoring
from creation_lib.mod.patches import list_patches, get_patch_yaml_dir, get_patch_plugin_name
from creation_lib.build.archive_plan import discover_mod_archives
from creation_lib.build.packer import pack_mod
from creation_lib.esp.authoring import deserialize
from creation_lib.build.plugin_source import resolve_plugin_source

_log = logging.getLogger(__name__)

_WINDOWS_FAST_COPY_MIN_BYTES = 64 * 1024 * 1024
_COPY_FILE_NO_BUFFERING = 0x00001000


# Map game id → script-extender directory name. Same shape as the on-disk
# `Data/<NAME>/Plugins/` layout each extender uses, and the same stem the
# co-save uses (.f4se / .skse / etc.).
XSE_PLUGIN_DIR: dict[str, str] = {
    "fo4":       "F4SE",
    "skyrimse":  "SKSE",
    "starfield": "SFSE",
    "fnv":       "NVSE",
    "fo3":       "FOSE",
}


def xse_plugin_dir_for(game: str) -> str:
    """Return the script-extender directory name for `game` (e.g. fo4 → "F4SE").

    Raises KeyError for unsupported games.
    """
    return XSE_PLUGIN_DIR[game]


# Auxiliary loose-asset root dirs an XSE/UI mod may ship alongside its <XSE>/ tree,
# mirrored straight into Data/ on deploy.
_AUX_LOOSE_DIRS: tuple[str, ...] = ("PrismaUI_F4", "FO4CS")

# Standard loose Data asset trees that ship alongside an XSE-plugin-only (no-esp)
# mod. The esp deploy path already handles Meshes/ (and packs Textures/ into BA2s),
# so these are mirrored only on the no-esp path. Materials/ covers renderer mods
# that ship .bgsm/.bgem next to their DLL.
_XSE_LOOSE_DATA_DIRS: tuple[str, ...] = ("Meshes", "Textures", "Materials", "MCM")


def _file_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _use_windows_fast_copy(src: Path, file_size: int) -> bool:
    return os.name == "nt" and file_size >= _WINDOWS_FAST_COPY_MIN_BYTES


def _windows_copyfileex(src: Path, dest: Path, flags: int) -> None:
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    copy_file_ex = kernel32.CopyFileExW
    copy_file_ex.argtypes = [
        wintypes.LPCWSTR,
        wintypes.LPCWSTR,
        wintypes.LPVOID,
        wintypes.LPVOID,
        ctypes.POINTER(wintypes.BOOL),
        wintypes.DWORD,
    ]
    copy_file_ex.restype = wintypes.BOOL

    cancel = wintypes.BOOL(False)
    ok = copy_file_ex(
        str(src),
        str(dest),
        None,
        None,
        ctypes.byref(cancel),
        flags,
    )
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())


def _copy2_fast(src: Path, dest: Path) -> None:
    src = Path(src)
    dest = Path(dest)
    try:
        file_size = src.stat().st_size
    except OSError:
        shutil.copy2(src, dest)
        return

    if _use_windows_fast_copy(src, file_size):
        try:
            _windows_copyfileex(src, dest, _COPY_FILE_NO_BUFFERING)
            shutil.copystat(src, dest)
            return
        except OSError:
            pass

    shutil.copy2(src, dest)


def _verified_copy(src: Path, dest: Path, emit: Callable[[str], None]) -> None:
    """Copy `src` to `dest` and verify the destination's hash matches `src`.

    Catches stale-deploy ghosts:
    the source is rebuilt, deploy claims success, but the destination still
    has the prior contents (CK holding the file, antivirus quarantine
    intercept, network filesystem write coalesce, etc.). When the
    post-copy hash differs from the source, raise a RuntimeError with both
    sizes and SHA-256 prefixes so the cause is named, not chased.
    """
    _copy2_fast(src, dest)
    src_sha = _file_sha256(src)
    dest_sha = _file_sha256(dest)
    if src_sha != dest_sha:
        src_size = src.stat().st_size
        dest_size = dest.stat().st_size
        raise RuntimeError(
            f"deploy verify failed for {dest.name}: "
            f"src ({src_size}b sha={src_sha[:16]}) != "
            f"dest ({dest_size}b sha={dest_sha[:16]}). "
            f"Common causes: CK has the file open (close CK, retry); "
            f"another process wrote to {dest} mid-copy; "
            f"the destination filesystem is being mirrored/synced."
        )


def _move_archive(src: Path, dest: Path) -> None:
    """Replace dest with src, using the fast copier across volumes."""
    src = Path(src)
    dest = Path(dest)
    backup = dest.with_name(f"{dest.name}.old")
    if backup.exists():
        backup.unlink()
    had_dest = dest.exists()
    if had_dest:
        dest.replace(backup)

    def _restore_previous_destination() -> None:
        if dest.exists():
            dest.unlink()
        if had_dest and backup.exists():
            backup.replace(dest)

    try:
        try:
            src.replace(dest)
        except OSError as exc:
            if exc.errno != errno.EXDEV and getattr(exc, "winerror", None) != 17:
                raise
            _copy2_fast(src, dest)
            src_size = src.stat().st_size
            dest_size = dest.stat().st_size
            if src_size != dest_size:
                raise OSError(
                    f"cross-volume archive move size mismatch: "
                    f"{src_size} != {dest_size}"
                )
            src.unlink()
    except Exception:
        _restore_previous_destination()
        raise
    else:
        if backup.exists():
            backup.unlink()


def _transfer_archive(src: Path, dest: Path, mode: str) -> None:
    if mode == "copy":
        _copy2_fast(src, dest)
        return
    if mode == "move":
        _move_archive(src, dest)
        return
    raise ValueError(f"unsupported archive_transfer_mode: {mode}")


def _remove_loose_string_sidecars(
    game_data_dir: Path,
    mod_name: str,
    emit: Callable[[str], None],
    *,
    stale: bool = False,
    dry_run: bool = False,
) -> list[str]:
    strings_dir = game_data_dir / "Strings"
    if not strings_dir.is_dir():
        return []

    removed: list[str] = []
    mod_key = mod_name.lower()
    for strfile in sorted(strings_dir.iterdir(), key=lambda path: path.name.lower()):
        name_key = strfile.name.lower()
        if (
            not strfile.is_file()
            or not name_key.endswith("strings")
            or not (
                name_key.startswith(f"{mod_key}_")
                or name_key.startswith(f"{mod_key}.")
            )
        ):
            continue
        if not dry_run:
            strfile.unlink()
        removed.append(f"Strings/{strfile.name}")
        if stale:
            emit(f"  Removed stale loose string: Strings/{strfile.name}")
        else:
            emit(f"Removed: Strings/{strfile.name}")
    return removed


@dataclass
class DeployResult:
    """Summary of a deploy operation."""
    plugin_deployed: str = ""
    archives_deployed: list[str] = field(default_factory=list)
    strings_deployed: int = 0
    loose_files_deployed: int = 0
    patches_deployed: list[str] = field(default_factory=list)
    preserved_xse_inis: list[str] = field(default_factory=list)


# ---------------------------------------------------------------------------
# Papyrus compile
# ---------------------------------------------------------------------------

def _native_papyrus_diagnostics_message(diagnostics: Iterable[object]) -> str:
    messages: list[str] = []
    for diagnostic in diagnostics:
        if isinstance(diagnostic, Mapping):
            message = str(diagnostic.get("message", diagnostic))
            line = diagnostic.get("line")
            col = diagnostic.get("col")
            if line is not None and col is not None:
                message = f"{line}:{col}: {message}"
            messages.append(message)
        else:
            messages.append(str(diagnostic))
    return "; ".join(messages[:3]) if messages else "native compiler returned no output"


def compile_papyrus(
    mod_dir: Path,
    game: str,
    game_data_dir: Path,
    on_progress: Callable[[str], None] | None = None,
    *, verify_stock: bool = False,
) -> int:
    """Compile .psc sources in Scripts/Source/User/ -> data/Scripts/."""
    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    source_dir = mod_dir / "Scripts" / "Source" / "User"
    psc_files = sorted(source_dir.rglob("*.psc")) if source_dir.is_dir() else []
    if not psc_files:
        return 0

    profile = get_profile(game)
    from creation_lib.pex.native_runtime import compile_psc

    output_dir = mod_dir / "data" / "Scripts"

    # Import search path (first match wins):
    #   1. This mod's own Source/User     — the scripts being compiled + their siblings
    #   2. Game's Source/User             — any deployed scripts from other mods
    #   3. Game's Source/Base             — vanilla base game scripts
    game_user_dir = game_data_dir / "Scripts" / "Source" / "User"
    # Fallout 4 keeps vanilla sources under Scripts/Source/Base; Skyrim reverses
    # the two path components and drops the Base level entirely.
    base_candidates = (
        game_data_dir / "Scripts" / "Source" / "Base",
        game_data_dir / "Source" / "Scripts",
        game_data_dir / "Scripts" / "Source",
    )
    scripts_base = next(
        (path for path in base_candidates if next(path.glob("*.psc"), None)),
        base_candidates[0],
    )
    import_parts = [str(source_dir)]
    if game_user_dir.is_dir():
        import_parts.append(str(game_user_dir))
    if scripts_base.is_dir():
        import_parts.append(str(scripts_base))

    # An empty Source/Base satisfies is_dir() and then resolves nothing, so every
    # base-game call types as None and the errors land on the mod's own lines
    # ("cannot assign None to Bool") as though the mod were at fault. A game
    # install without Creation Kit sources looks exactly like broken source.
    have_base_sources = scripts_base.is_dir() and next(
        scripts_base.glob("*.psc"), None
    ) is not None
    if not have_base_sources:
        # No game install, or one the Creation Kit never wrote sources into. The
        # bundled corpus carries the same type surface, synthesized from the
        # game's own .pex, so compiling does not require the game at all.
        from creation_lib.pex.corpus import bundled_corpus_root

        bundled = bundled_corpus_root(game)
        if bundled is not None:
            import_parts.append(str(bundled))
            have_base_sources = True
            _emit(f"  [compile] using the bundled {game} type universe ({bundled})")
        else:
            _emit(
                f"  [compile] WARNING: no vanilla Papyrus sources under "
                f"{scripts_base}, and no bundled {game} corpus — base-game types "
                "will not resolve"
            )

    flags_arg = None
    if profile.papyrus_flags:
        flags_arg = profile.papyrus_flags
        for candidate in (
            scripts_base / profile.papyrus_flags,
            *(Path(part) / profile.papyrus_flags for part in import_parts),
        ):
            if candidate.is_file():
                flags_arg = str(candidate)
                break

    if verify_stock:
        from creation_lib.build.papyrus_verification import verify_stock_sources
        if not profile.papyrus_compiler_dir:
            raise ValueError(f"No stock Papyrus compiler is configured for {game}")
        verify_stock_sources(psc_files, compiler=game_data_dir.parent / profile.papyrus_compiler_dir / "PapyrusCompiler.exe",
                             game_root=game_data_dir.parent, imports=import_parts, flags=flags_arg, on_progress=_emit)
    output_dir.mkdir(parents=True, exist_ok=True)
    _emit(f"  [compile] native-compiling {len(psc_files)} script(s)")
    failures: list[str] = []
    for psc_path in psc_files:
        rel_path = psc_path.relative_to(source_dir)
        output_pex = (output_dir / rel_path).with_suffix(".pex")
        if output_pex.is_file():
            output_pex.unlink()
        try:
            result = compile_psc(
                psc_path.read_text(encoding="utf-8"),
                imports=import_parts,
                game=game.lower(),
                flags=flags_arg,
                source_path=str(psc_path),
            )
        except Exception as exc:
            failures.append(f"{rel_path}: {exc}")
            continue
        if not result.ok or result.pex_bytes is None:
            failures.append(
                f"{rel_path}: {_native_papyrus_diagnostics_message(result.diagnostics)}"
            )
            continue
        output_pex.parent.mkdir(parents=True, exist_ok=True)
        output_pex.write_bytes(result.pex_bytes)

    if failures:
        for failure in failures[:20]:
            _emit(f"  [compile] FAILED {failure}")
        if len(failures) > 20:
            _emit(f"  [compile] ... {len(failures) - 20} more failure(s)")
        message = "Papyrus compile failed — see output above"
        if not have_base_sources:
            message = (
                f"{message}. No vanilla Papyrus sources under {scripts_base}; "
                f"the selected {game} install has no Creation Kit sources, so "
                "base-game types cannot resolve."
            )
        raise RuntimeError(message)

    return len(psc_files)


# ---------------------------------------------------------------------------
# Deploy
# ---------------------------------------------------------------------------

def deploy_mod(
    mod_name: str,
    *,
    game: str,
    game_data_dir: Path,
    plugin_base_name: str | None = None,
    deploy_data_dir: Path | None = None,
    skip_build: bool = False,
    skip_pack: bool = False,
    esp_only: bool = False,
    no_esp: bool = False,
    preserve_xse_inis: bool = False,
    source: Path | str | None = None,
    xbox: bool = False,
    ps: bool = False,
    skip_papyrus_compile: bool = False,
    skip_validation: bool = False,
    pc_max_res: int = 0,
    pc_effects_max_res: int | None = None,
    xbox_max_res: int = 0,
    xbox_effects_max_res: int | None = None,
    ps_max_res: int = 0,
    ps_effects_max_res: int | None = None,
    patches: list[str] | None = None,
    project_root: Path | str | None = None,
    resource_dir: Path | str | None = None,
    archive_max_bytes: int | None = None,
    expanded_archives: bool = False,
    archive_workers: int = 0,
    fo4_ba2_target: str = "auto",
    archive_transfer_mode: str = "copy",
    pack_archives_to_deploy_target: bool = False,
    deploy_archives: bool = True,
    on_progress: Callable[[str], None] | None = None,
) -> DeployResult:
    """Full deploy pipeline: build esp, pack behaviors, pack BA2, copy to game.

    Args:
        game: Game ID (fo4, skyrimse, starfield).
        plugin_base_name: Base name the game keys BA2s and loose string tables
            by — it mounts "<PluginStem> - Main.ba2", not "<ModName> - Main.ba2".
            Defaults to the selected plugin stem.
        source: Plugin binary, whole-plugin YAML/JSON, or authoring directory
            inside the mod folder. Relative paths resolve from that folder.
        preserve_xse_inis: Keep existing destination XSE .ini files; install missing ones.
        esp_only: Deploy only the .esp (no archives or loose files).
        no_esp: Mod has no .esp (e.g. XSE-plugin-only). Skips the build/pack
            pipeline and just copies whatever is under ``mods/<name>/<XSE>/``,
            where ``<XSE>`` is one of F4SE/SKSE/SFSE/NVSE/FOSE per the mod's game.
        pc_max_res: Max texture resolution for PC archives (0 = unlimited).
        xbox_max_res: Max texture resolution for Xbox archives.
        ps_max_res: Max texture resolution for PlayStation archives (0 = unlimited).
        archive_transfer_mode: "copy" keeps BA2/BSA files in the mod folder;
            "move" cuts them into the deploy target after packing.
        pack_archives_to_deploy_target: Pack archives directly into the resolved
            deploy target and skip the later archive transfer.
        deploy_archives: False skips the archive deploy/stale-cleanup loop. Use
            only when archives were packed directly into the deploy target.
    """
    if project_root is None:
        raise ValueError("project_root is required")
    if archive_transfer_mode not in {"copy", "move"}:
        raise ValueError("archive_transfer_mode must be 'copy' or 'move'")
    project_root = Path(project_root)
    game_data_dir = Path(game_data_dir)
    target_data_dir = Path(deploy_data_dir) if deploy_data_dir is not None else game_data_dir
    mod_dir = project_root / "mods" / mod_name
    if not mod_dir.is_dir():
        raise FileNotFoundError(f"Mod directory not found: {mod_dir}")

    result = DeployResult()

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    def _deploy_xse_tree(strict: bool = False) -> int:
        """Copy mods/<name>/<XSE>/ tree to Data/<XSE>/. Returns file count copied.

        ``strict=True`` raises if the XSE dir is missing (used for --no-esp deploy
        where the XSE tree is the only thing to ship). ``strict=False`` is a no-op
        when the dir is absent (used for combined mods that may or may not have a DLL).
        """
        ext_dir = xse_plugin_dir_for(game)
        xse_dir = mod_dir / ext_dir
        if not xse_dir.is_dir():
            if strict:
                raise FileNotFoundError(
                    f"--no-esp expected {xse_dir} but it does not exist. "
                    f"Build/install your {ext_dir} plugin first (e.g. xmake install -y)."
                )
            return 0
        copied = 0
        for srcfile in xse_dir.rglob("*"):
            if not srcfile.is_file():
                continue
            relpath = srcfile.relative_to(xse_dir)
            destdir = target_data_dir / ext_dir / relpath.parent
            destination = destdir / srcfile.name
            if preserve_xse_inis and srcfile.suffix.casefold() == ".ini" and destination.is_file():
                result.preserved_xse_inis.append(f"{ext_dir}/{relpath.as_posix()}")
                _emit(f"  Preserved existing INI: {ext_dir}/{relpath}")
                continue
            destdir.mkdir(parents=True, exist_ok=True)
            _copy2_fast(srcfile, destination)
            result.loose_files_deployed += 1
            copied += 1
            _emit(f"  Copied: {ext_dir}/{relpath}")
        return copied

    def _deploy_loose_dir(subdir: str) -> int:
        """Mirror mods/<name>/<subdir>/ → Data/<subdir>/. Returns file count copied."""
        src_root = mod_dir / subdir
        if not src_root.is_dir():
            return 0
        copied = 0
        for srcfile in src_root.rglob("*"):
            if not srcfile.is_file():
                continue
            relpath = srcfile.relative_to(src_root)
            destdir = target_data_dir / subdir / relpath.parent
            destdir.mkdir(parents=True, exist_ok=True)
            _copy2_fast(srcfile, destdir / srcfile.name)
            result.loose_files_deployed += 1
            copied += 1
            _emit(f"  Copied: {subdir}/{relpath}")
        return copied

    # ── No-esp path: XSE-plugin-only mod ────────────────────────────
    if no_esp:
        _emit(f"Deploying {xse_plugin_dir_for(game)}-only mod {mod_name} to {target_data_dir}...")
        copied = _deploy_xse_tree(strict=True)
        for aux in _AUX_LOOSE_DIRS:
            copied += _deploy_loose_dir(aux)
        for data_tree in _XSE_LOOSE_DATA_DIRS:
            copied += _deploy_loose_dir(data_tree)
        if copied == 0 and not result.preserved_xse_inis:
            _emit(f"  WARNING: No files found under mods/{mod_name}/{xse_plugin_dir_for(game)}/")
        _emit(f"=== Deploy complete === ({copied} file(s))")
        return result

    authoring_source, esp = resolve_plugin_source(mod_dir, source)
    archive_base = plugin_base_name or esp.stem
    data_dir = mod_dir / "data"
    meshes_dir = mod_dir / "Meshes"

    # ── Step 1: Build .esp ──────────────────────────────────────────
    if not skip_build and authoring_source is not None:
        if not skip_validation and authoring_source.is_dir():
            _emit("[1/5] Validating authoring dir...")
            errors, _ = validate_authoring(authoring_source)
            if errors:
                _emit(f"WARNING: Validation found {len(errors)} error(s)")
                for err in errors:
                    _emit(f"VALIDATION_ERROR: {err['file']}:{err['line']} [{err['formkey']}] — {err['reason']}")
        _emit("[1/5] Building .esp...")

        deserialize(
            authoring_source,
            esp,
            game=game,
            data_folder=game_data_dir,
            on_progress=on_progress,
        )
    else:
        _emit("[1/5] Skipping .esp build")

    if not esp.is_file():
        raise FileNotFoundError(f"{esp} not found. Build the mod first.")

    # ── Step 2: Pack behavior XMLs → HKX ─────────────────────────
    if esp_only:
        _emit("[2/5] Skipping behavior packing")
    elif not meshes_dir.is_dir():
        _emit("[2/5] No Meshes/ directory — skipping behavior packing")
    else:
        xml_files = list(meshes_dir.rglob("*.xml"))
        if not xml_files:
            _emit("[2/5] No behavior XMLs found — skipping")
        else:
            _emit(f"[2/5] Packing {len(xml_files)} behavior XML(s) → HKX...")
            from creation_lib._native.havok_native import pack_xml_to_hkx

            pack_failed = False
            for xmlfile in xml_files:
                hkxfile = xmlfile.with_suffix(".hkx")
                relpath = xmlfile.relative_to(meshes_dir)
                try:
                    pack_xml_to_hkx(str(xmlfile), str(hkxfile))
                    _emit(f"  Packed {relpath} OK")
                except Exception as e:
                    _emit(f"  FAILED {relpath}: {e}")
                    pack_failed = True

            if pack_failed:
                raise RuntimeError(
                    "One or more behavior XMLs failed to pack. "
                    "Fix the errors, then re-run deploy."
                )

    # ── Step 3: Compile Papyrus scripts (.psc → .pex) ─────────────
    if esp_only or skip_papyrus_compile:
        _emit("[3/5] Skipping Papyrus compile")
    else:
        _emit("[3/5] Compiling Papyrus scripts...")
        compiled = compile_papyrus(mod_dir, game, game_data_dir, on_progress=on_progress)
        if compiled == 0:
            _emit("[3/5] No .psc files found — skipping")

    # ── Step 4: Pack BA2 archives ───────────────────────────────────
    archives_packed_in_target = False
    if esp_only or skip_pack:
        _emit("[4/5] Skipping BA2 packing")
    elif not data_dir.is_dir():
        _emit("[4/5] No data/ directory — skipping BA2 packing")
    else:
        _emit("[4/5] Packing BA2 archives...")
        archive_output_dir = None
        if pack_archives_to_deploy_target:
            target_data_dir.mkdir(parents=True, exist_ok=True)
            for existing_archive in discover_mod_archives(target_data_dir, archive_base):
                existing_archive.unlink()
                _emit(f"  Removed existing archive: {existing_archive.name}")
            archive_output_dir = target_data_dir
        pack_mod(
            mod_name,
            pc=True,
            xbox=xbox,
            ps=ps,
            pc_max_res=pc_max_res,
            pc_effects_max_res=pc_effects_max_res,
            xbox_max_res=xbox_max_res,
            xbox_effects_max_res=xbox_effects_max_res,
            ps_max_res=ps_max_res,
            ps_effects_max_res=ps_effects_max_res,
            game=game,
            game_dir=str(target_data_dir.parent),
            project_root=project_root,
            resource_dir=resource_dir,
            archive_max_bytes=archive_max_bytes,
            expanded_archives=expanded_archives,
            archive_workers=archive_workers,
            fo4_ba2_target=fo4_ba2_target,
            archive_output_dir=archive_output_dir,
            plugin_base_name=archive_base,
        )
        archives_packed_in_target = archive_output_dir is not None

    # ── Step 5: Deploy to game Data ─────────────────────────────────
    _emit(f"[5/5] Deploying to {target_data_dir}...")
    target_data_dir.mkdir(parents=True, exist_ok=True)

    # Deploy plugin (verified copy — surfaces stale-deploy ghosts where
    # CK holds the file, an antivirus quarantine swaps content, or some
    # other mid-copy interference produces a deployed file that differs
    # from the source. Without this check, a corrupted deploy is invisible
    # until you load in CK and see "Unable to find keyword" errors.)
    dest_esp = target_data_dir / esp.name
    _verified_copy(esp, dest_esp, _emit)
    result.plugin_deployed = esp.name
    _emit(f"  Copied: {esp.name}")

    if not esp_only:
        if archives_packed_in_target:
            packed_archives = discover_mod_archives(target_data_dir, archive_base)
            result.archives_deployed.extend(archive.name for archive in packed_archives)
            for archive in packed_archives:
                _emit(f"  Packed directly: {archive.name}")
            if mod_dir.resolve() != target_data_dir.resolve():
                for local_archive in discover_mod_archives(mod_dir, archive_base):
                    local_archive.unlink()
                    _emit(f"  Removed local archive: {local_archive.name}")
        elif deploy_archives:
            # Deploy BA2s
            current_archives = discover_mod_archives(mod_dir, archive_base)
            current_archive_names = {archive.name for archive in current_archives}
            for stale_archive in discover_mod_archives(target_data_dir, archive_base):
                if stale_archive.name in current_archive_names:
                    continue
                stale_archive.unlink()
                _emit(f"  Removed stale archive: {stale_archive.name}")
            for archive in current_archives:
                _transfer_archive(archive, target_data_dir / archive.name, archive_transfer_mode)
                result.archives_deployed.append(archive.name)
                verb = "Moved" if archive_transfer_mode == "move" else "Copied"
                _emit(f"  {verb}: {archive.name}")
        else:
            _emit("  Skipping BA2 deploy; archives already in deploy target")

        _remove_loose_string_sidecars(target_data_dir, archive_base, _emit, stale=True)

        # Deploy loose Meshes/ files (everything except .xml source files)
        if meshes_dir.is_dir():
            for srcfile in meshes_dir.rglob("*"):
                if srcfile.is_file() and srcfile.suffix.lower() != ".xml":
                    relpath = srcfile.relative_to(meshes_dir)
                    destdir = target_data_dir / "Meshes" / relpath.parent
                    destdir.mkdir(parents=True, exist_ok=True)
                    _copy2_fast(srcfile, destdir / srcfile.name)
                    result.loose_files_deployed += 1
            if result.loose_files_deployed > 0:
                _emit(f"  Deployed {result.loose_files_deployed} loose file(s) from Meshes/")

        # Deploy MCM menu definitions/settings as loose files.
        mcm_dir = mod_dir / "MCM"
        mcm_copied = 0
        if mcm_dir.is_dir():
            for srcfile in mcm_dir.rglob("*"):
                if srcfile.is_file():
                    relpath = srcfile.relative_to(mcm_dir)
                    destdir = target_data_dir / "MCM" / relpath.parent
                    destdir.mkdir(parents=True, exist_ok=True)
                    _copy2_fast(srcfile, destdir / srcfile.name)
                    result.loose_files_deployed += 1
                    mcm_copied += 1
            if mcm_copied > 0:
                _emit(f"  Deployed {mcm_copied} file(s) from MCM/")

        # Deploy loose Terrain/ sidecars (e.g. converted-worldspace .btd4 files the
        # B21_BTD plugin's SidecarStore discovers under Data/Terrain/ at load).
        terrain_copied = _deploy_loose_dir("Terrain")
        if terrain_copied > 0:
            _emit(f"  Deployed {terrain_copied} file(s) from Terrain/")

        # Deploy XSE plugin tree (DLL/INI/etc.) when present — combined mods
        xse_copied = _deploy_xse_tree(strict=False)
        if xse_copied > 0:
            _emit(f"  Deployed {xse_copied} file(s) from {xse_plugin_dir_for(game)}/")

        # Deploy auxiliary UI asset trees (e.g. PrismaUI_F4 HTML views) when present
        for aux in _AUX_LOOSE_DIRS:
            aux_copied = _deploy_loose_dir(aux)
            if aux_copied > 0:
                _emit(f"  Deployed {aux_copied} file(s) from {aux}/")

    # ── Step 5: Build & deploy patches ──────────────────────────────
    if patches is not None:
        patch_names = list_patches(mod_dir) if "all" in patches else patches
        if patch_names:
            _emit(f"[5] Building & deploying {len(patch_names)} patch(es)...")
            profile = get_profile(game)
            for pname in patch_names:
                patch_yaml = get_patch_yaml_dir(mod_dir, pname)
                plugin_file = get_patch_plugin_name(mod_dir, pname)
                patch_esp = mod_dir / plugin_file

                if not patch_yaml.is_dir():
                    _emit(f"  WARNING: No yaml/ for patch {pname} — skipping")
                    continue

                # Build patch ESP
                if not skip_build:
                    _emit(f"  Building: {plugin_file}")
                    deserialize(
                        patch_yaml,
                        patch_esp,
                        game=game,
                        data_folder=game_data_dir,
                        on_progress=on_progress,
                    )

                if not patch_esp.is_file():
                    _emit(f"  WARNING: {patch_esp} not found — skipping deploy")
                    continue

                # Deploy patch ESP
                _copy2_fast(patch_esp, target_data_dir / plugin_file)
                result.patches_deployed.append(plugin_file)
                _emit(f"  Deployed: {plugin_file}")

    _emit("=== Deploy complete ===")
    return result


# ---------------------------------------------------------------------------
# Undeploy
# ---------------------------------------------------------------------------

def undeploy_mod(
    mod_name: str,
    *,
    game: str,
    game_data_dir: Path,
    no_esp: bool = False,
    dry_run: bool = False,
    source: Path | str | None = None,
    patches: list[str] | None = None,
    project_root: Path | str | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> list[str]:
    """Remove deployed mod files from game Data/ folder.

    Returns list of removed filenames.
    """
    removed: list[str] = []

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    if project_root is None:
        raise ValueError("project_root is required")
    mod_dir = Path(project_root) / "mods" / mod_name

    def _undeploy_xse_tree() -> None:
        """Walk mods/<name>/<XSE>/ and unlink the matching files in Data/<XSE>/."""
        ext_dir = xse_plugin_dir_for(game)
        xse_dir = mod_dir / ext_dir
        if not xse_dir.is_dir():
            return
        for srcfile in xse_dir.rglob("*"):
            if not srcfile.is_file():
                continue
            relpath = srcfile.relative_to(xse_dir)
            deployed = game_data_dir / ext_dir / relpath
            if deployed.is_file():
                if not dry_run:
                    deployed.unlink()
                removed.append(f"{ext_dir}/{relpath}")
                _emit(f"Removed: {ext_dir}/{relpath}")

    def _undeploy_loose_dir(subdir: str) -> None:
        """Unlink only the files THIS mod shipped under Data/<subdir>/, then prune the
        empty dirs it owned. Walks the mod's own source tree, so another mod sharing the
        same root (e.g. a second PrismaUI mod under Data/PrismaUI_F4/views/) is untouched:
        its files aren't in our tree, and Path.rmdir refuses to remove a non-empty shared
        dir, so the shared root survives as long as anyone else still lives there.
        """
        src_root = mod_dir / subdir
        if not src_root.is_dir():
            return
        for srcfile in src_root.rglob("*"):
            if not srcfile.is_file():
                continue
            relpath = srcfile.relative_to(src_root)
            deployed = game_data_dir / subdir / relpath
            if deployed.is_file():
                if not dry_run:
                    deployed.unlink()
                removed.append(f"{subdir}/{relpath}")
                _emit(f"Removed: {subdir}/{relpath}")
        if dry_run:
            return
        # Prune the dirs we own, deepest first, plus the aux root. rmdir only deletes
        # empty dirs, so a shared parent still holding another mod's views is left intact.
        src_dirs = sorted(
            (p for p in src_root.rglob("*") if p.is_dir()),
            key=lambda p: len(p.parts), reverse=True,
        )
        for srcdir in src_dirs:
            try:
                (game_data_dir / subdir / srcdir.relative_to(src_root)).rmdir()
            except OSError:
                pass  # non-empty (another mod's files) or already gone
        try:
            (game_data_dir / subdir).rmdir()
        except OSError:
            pass

    # ── No-esp path: walk source <XSE>/ to compute deployed paths ───
    if no_esp:
        ext_dir = xse_plugin_dir_for(game)
        if not (mod_dir / ext_dir).is_dir():
            _emit(f"No {ext_dir}/ directory under {mod_dir} — nothing to undeploy")
            return removed
        _undeploy_xse_tree()
        for aux in _AUX_LOOSE_DIRS:
            _undeploy_loose_dir(aux)
        for data_tree in _XSE_LOOSE_DATA_DIRS:
            _undeploy_loose_dir(data_tree)
        if removed:
            _emit(f"=== Undeploy complete === ({len(removed)} file(s) removed)")
        else:
            _emit(f"No deployed {ext_dir} files found for {mod_name} in {game_data_dir}")
        return removed

    plugin_base = mod_name
    if mod_dir.is_dir():
        try:
            _, plugin = resolve_plugin_source(mod_dir, source)
            plugin_base = plugin.stem
        except FileNotFoundError:
            if source is not None:
                raise
    # Plugin files
    for ext in ["esp", "esl", "esm"]:
        f = game_data_dir / f"{plugin_base}.{ext}"
        if f.is_file():
            if not dry_run:
                f.unlink()
            removed.append(f.name)
            _emit(f"Removed: {f.name}")

    # Archive files
    for archive in discover_mod_archives(game_data_dir, plugin_base):
        if not dry_run:
            archive.unlink()
        removed.append(archive.name)
        _emit(f"Removed: {archive.name}")

    # String files
    removed.extend(_remove_loose_string_sidecars(game_data_dir, plugin_base, _emit, dry_run=dry_run))

    # XSE plugin tree (DLL/INI/etc.) — combined mods only ship these
    _undeploy_xse_tree()

    # Auxiliary UI asset trees (e.g. PrismaUI_F4 HTML views)
    for aux in _AUX_LOOSE_DIRS:
        _undeploy_loose_dir(aux)

    # Loose Terrain/ sidecars (e.g. converted-worldspace .btd4 files)
    _undeploy_loose_dir("Terrain")

    # Patch plugins
    if patches is not None:
        patch_names = list_patches(mod_dir) if "all" in patches else patches
        for pname in patch_names:
            plugin_file = get_patch_plugin_name(mod_dir, pname)
            f = game_data_dir / plugin_file
            if f.is_file():
                if not dry_run:
                    f.unlink()
                removed.append(plugin_file)
                _emit(f"Removed patch: {plugin_file}")

    if not removed:
        _emit(f"No deployed files found for {mod_name} in {game_data_dir}")
    else:
        _emit(f"=== Undeploy complete === ({len(removed)} file(s) removed)")

    return removed

from pathlib import Path

from creation_lib.build.archive_plan import discover_mod_archives
from creation_lib.build.deployer import _AUX_LOOSE_DIRS, _XSE_LOOSE_DATA_DIRS, _remove_loose_string_sidecars, xse_plugin_dir_for
from creation_lib.build.plugin_source import resolve_plugin_source
from creation_lib.mod.patches import get_patch_plugin_name, get_patch_yaml_dir, list_patches


def plan_deploy(mod_dir, target, *, game, source=None, no_esp=False, esp_only=False, loose=False,
                skip_build=False, skip_pack=False, skip_papyrus_compile=False, preserve_xse_inis=False,
                patches=None, fo4_ba2_target="auto"):
    mod_dir, target = Path(mod_dir).resolve(), Path(target).resolve()
    if not mod_dir.is_dir():
        raise FileNotFoundError(f"Mod directory not found: {mod_dir}")
    operations, pending, copies = [], [], {}
    xse = xse_plugin_dir_for(game)

    def copy(path, relative):
        relative = Path(relative)
        destination = target / relative
        keep = preserve_xse_inis and relative.parts[0].casefold() == xse.casefold() and path.suffix.casefold() == ".ini" and destination.is_file()
        key = str(destination).casefold()
        previous = copies.get(key)
        if loose and previous and previous["source"].casefold() != str(path).casefold():
            raise ValueError(f"duplicate loose deployment path {relative}: {previous['source']} and {path}")
        copies[key] = {"action": "preserve" if keep else "copy", "source": str(path),
            "destination": str(destination), "exists": destination.exists()}

    def tree(root, prefix, exclude_xml=False):
        if root.is_dir():
            for path in sorted(root.rglob("*")):
                if path.is_file() and not (exclude_xml and path.suffix.casefold() == ".xml"):
                    copy(path, Path(prefix) / path.relative_to(root))

    if no_esp:
        if not (mod_dir / xse).is_dir():
            raise FileNotFoundError(f"--no-esp expected {mod_dir / xse}")
        for root in (xse, *_AUX_LOOSE_DIRS, *_XSE_LOOSE_DATA_DIRS):
            tree(mod_dir / root, root)
    else:
        authoring, plugin = resolve_plugin_source(mod_dir, source)
        if authoring is not None and not skip_build:
            pending.append({"action": "build_plugin", "source": str(authoring), "output": str(plugin)})
        elif not plugin.is_file():
            raise FileNotFoundError(f"{plugin} not found. Build the mod first.")
        copy(plugin, plugin.name)
        if not esp_only:
            for xml in sorted((mod_dir / "Meshes").rglob("*.xml")):
                generated = xml.with_suffix(".hkx")
                pending.append({"action": "pack_behavior", "source": str(xml), "output": str(generated)})
                copy(generated, generated.relative_to(mod_dir))
            if not skip_papyrus_compile:
                scripts = list((mod_dir / "Scripts/Source/User").rglob("*.psc"))
                if scripts:
                    pending.append({"action": "compile_papyrus", "sources": len(scripts), "output_directory": str(mod_dir / "data/Scripts")})
            if loose:
                from creation_lib.build.loose_deploy import _iter_source_files
                for _, _, path, relative in _iter_source_files(mod_dir):
                    copy(path, relative)
                tree(mod_dir / "data/Textures", "Textures")
                for path in sorted((mod_dir / "Strings").rglob("*")):
                    if path.is_file():
                        copy(path, Path("Strings") / path.name)
                operations.append({"action": "write_manifest", "destination": str(mod_dir / ".loose_manifest.json")})
            else:
                archives = discover_mod_archives(mod_dir, plugin.stem)
                if not skip_pack and (mod_dir / "data").is_dir():
                    pending.append({"action": "pack_archives", "source": str(mod_dir / "data"),
                                    "output_directory": str(mod_dir), "ba2_target": fo4_ba2_target})
                else:
                    names = {path.name for path in archives}
                    operations.extend({"action": "remove", "destination": str(path)} for path in discover_mod_archives(target, plugin.stem) if path.name not in names)
                for archive in archives:
                    copy(archive, archive.name)
                for relative in _remove_loose_string_sidecars(target, plugin.stem, lambda message: None, dry_run=True):
                    operations.append({"action": "remove", "destination": str(target / relative)})
                for root in ("Meshes", "MCM", "Terrain", xse, *_AUX_LOOSE_DIRS):
                    tree(mod_dir / root, root, exclude_xml=root == "Meshes")
        for patch in list_patches(mod_dir) if patches and "all" in patches else patches or []:
            yaml_dir = get_patch_yaml_dir(mod_dir, patch)
            path = mod_dir / get_patch_plugin_name(mod_dir, patch)
            if yaml_dir.is_dir():
                if not skip_build:
                    pending.append({"action": "build_patch", "source": str(yaml_dir), "output": str(path)})
                if path.is_file() or not skip_build:
                    copy(path, path.name)
    operations.extend(copies.values())
    return {"dry_run": True, "mod": mod_dir.name, "game": game, "target": str(target),
            "operations": operations, "pending_build_steps": pending, "file_inventory_complete": not pending,
            "note": "No files written. Build steps are planned only; generated files and archive cleanup may change after building."}

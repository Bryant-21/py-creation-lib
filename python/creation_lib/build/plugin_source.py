from __future__ import annotations

import json
from pathlib import Path

from creation_lib.esp.authoring import get_plugin_ext


def _read_document(path: Path):
    text = path.read_text(encoding="utf-8-sig")
    if path.suffix.lower() == ".json":
        return json.loads(text)
    from ruamel.yaml import YAML
    return YAML(typ="safe").load(text)


def resolve_plugin_source(mod_dir: Path, source: Path | str | None = None) -> tuple[Path | None, Path]:
    if source is not None:
        source = Path(source)
        if not source.is_absolute():
            source = mod_dir / source
        source = source.resolve()
        if not source.is_relative_to(mod_dir.resolve()):
            raise ValueError("Deployment sources must be inside the mod folder")
        if not source.exists():
            raise FileNotFoundError(f"Plugin source not found: {source}")
    else:
        yaml_dir = mod_dir / "yaml"
        if any((yaml_dir / f"plugin.{suffix}").is_file() for suffix in ("yaml", "json")):
            source = yaml_dir
        else:
            candidates = [mod_dir / f"{stem}.{suffix}"
                          for stem in (mod_dir.name, "plugin")
                          for suffix in ("yaml", "yml", "json")]
            candidates = [path for path in candidates if path.is_file()]
            if len(candidates) > 1:
                raise ValueError("Multiple whole-plugin sources found; select one with --source")
            source = candidates[0] if candidates else None

    if source is not None:
        if source.is_dir():
            metadata = next((source / f"plugin.{suffix}" for suffix in ("yaml", "json")
                             if (source / f"plugin.{suffix}").is_file()), None)
            document = _read_document(metadata) if metadata else None
            if isinstance(document, dict) and "items" in document:
                source = metadata
            else:
                return source, mod_dir / f"{mod_dir.name}.{get_plugin_ext(mod_dir, source)}"
        if source.suffix.lower() in {".esp", ".esm", ".esl"}:
            return None, source
        if source.suffix.lower() not in {".yaml", ".yml", ".json"}:
            raise ValueError("--source must select a plugin binary, whole-plugin YAML/JSON, or authoring directory")
        document = _read_document(source)
        name = document.get("plugin") if isinstance(document, dict) else None
        if not isinstance(name, str) or Path(name).name != name or Path(name).suffix.lower() not in {".esp", ".esm", ".esl"}:
            raise ValueError(f"Whole-plugin source needs a plain .esp/.esm/.esl filename in 'plugin': {source}")
        return source, mod_dir / name

    candidates = sorted(path for path in mod_dir.iterdir()
                        if path.is_file() and path.suffix.lower() in {".esp", ".esm", ".esl"})
    matching = [path for path in candidates if path.stem.casefold() == mod_dir.name.casefold()]
    candidates = matching or candidates
    if len(candidates) > 1:
        raise ValueError("Multiple plugin binaries found; select one with --source")
    if candidates:
        return None, candidates[0]
    if (mod_dir / "yaml").is_dir():
        return mod_dir / "yaml", mod_dir / f"{mod_dir.name}.{get_plugin_ext(mod_dir)}"
    raise FileNotFoundError(f"No plugin binary or authoring source found in {mod_dir}; use --source for a whole-plugin file")

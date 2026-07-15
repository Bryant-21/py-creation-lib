"""Game execution context for py_creation_lib/python/creation_lib/ consumers.

This module is the single place py_creation_lib/python/creation_lib/ looks for per-game paths and knobs.
It is pure data — no .env parsing, no I/O, no defaults beyond explicit
dataclass field defaults. Callers (CLI workflow commands, UI, tests,
tools/reconvert scripts) are responsible for constructing a GameContext
from whatever source makes sense (env, ToolkitSettings, pytest fixture).

py_creation_lib/python/creation_lib/ code that needs game context should accept a ``GameContext`` param
(or the individual fields) explicitly. It must NEVER read os.environ
directly to construct one itself.
"""
from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class GameContext:
    """Per-game execution context.

    Fields are all optional because different lib entry points need
    different subsets. Code that requires a specific field should
    validate it explicitly and raise with a clear error referencing
    the missing field name.
    """

    game: str
    """Game identifier — 'fo4', 'fo76', 'skyrimse', 'starfield', etc."""

    root_dir: Path | None = None
    """Path to the game install root. Callers derive this from their settings
    or env boundary; lib never discovers it."""

    data_dir: Path | None = None
    """Path to the installed game's Data folder (e.g., ``.../Fallout 4/Data``).
    Used when the pipeline needs to reference already-installed game assets."""

    extracted_dir: Path | None = None
    """Path to a directory containing extracted (unpacked) game assets.
    Used by conversion/havok code that needs raw NIF/HKX reference files."""

    compiler_path: Path | None = None
    """Path to the Papyrus compiler executable for this game."""

    archive2_path: Path | None = None
    """Path to the Archive2 packing executable for this game."""

    strings_dir: Path | None = None
    """Path to a directory containing localized STRINGS/DLSTRINGS/ILSTRINGS.
    Populated by the config builder; esp does not probe for it at call time."""

    strings_dirs: tuple[Path, ...] = ()
    """Ordered localized string directories to probe."""

    script_source_dirs: tuple[Path, ...] = ()
    """Ordered Papyrus source roots for script indexing and LSP services."""

    content_resources_dir: Path | None = None
    """Starfield ContentResources source root, when configured."""

    fbx_sdk_dir: Path | None = None
    """Optional Autodesk FBX SDK root supplied by boundary code."""

    addon_index_start: int = 20000
    """Starting NodeIndex for Addon Node generation during conversion.
    Matches the legacy default when no explicit override is passed."""


@dataclass(frozen=True)
class ResourceConfig:
    """Bundled read-only resources shipped with the package."""

    nif_xml: Path
    grammar_lark: Path
    classxml_dir: Path
    shader_dirs: Mapping[str, Path]
    """Keyed shader directories: 'renderer', 'skinned', 'simple', 'grid', 'shader_pipeline'."""
    hdri_dir: Path
    spellcheck_dict: Path
    havok_templates_dir: Path
    novablast_bin: Path
    convex_type_bin: Path
    conversion_yaml_dir: Path
    semantic_overlay: Path
    fbx_sdk_dir: Path


@dataclass(frozen=True)
class DataConfig:
    """User-writable databases, logs, caches."""

    db_dir: Path
    logs_dir: Path
    cache_dir: Path | None = None


@dataclass(frozen=True)
class ProjectConfig:
    """Workspace roots for operations on the user's project."""

    project_root: Path
    mods_dir: Path
    templates_dir: Path
    extracted_dir: Path | None = None
    external_mods_dir: Path | None = None
    wiki_dir: Path | None = None


@dataclass(frozen=True)
class LibConfig:
    """Convenience bag for CLI/UI — lib functions take sub-configs, not this."""

    resources: ResourceConfig
    data: DataConfig
    project: ProjectConfig
    games: Mapping[str, GameContext] = field(default_factory=dict)

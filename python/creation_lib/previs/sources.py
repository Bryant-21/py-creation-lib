"""Load orders for the previsbine checker: the active plugins in load order
plus every data source the game would read, lowest priority first.

Fallout 4 lets loose files beat every archive, and archives load in plugin
order, so archives come first (by plugin), then loose roots by priority.
"""

from __future__ import annotations

import configparser
import struct
from dataclasses import dataclass, field
from pathlib import Path

from creation_lib.core.game_profiles import get_profile

# Full previsbine bases rebuilt over the game, its DLC and a fix patch: a load
# order built on one only needs patches for what later mods break, so their
# cells count as the game's own.
PREVIS_BASE_PLUGINS = ("PPF.esm",)

# Previsbines never live in these archives; skipping them keeps indexing fast.
_IGNORED_ARCHIVE_KINDS = ("textures", "voices", "sounds", "animations", "interface", "shaders", "nvflex", "startup")


@dataclass
class LoadOrderSpec:
    label: str
    game_root: Path
    plugins: list[Path]
    data_sources: list[dict] = field(default_factory=list)
    base_plugins: list[str] = field(default_factory=list)

    def request(self, **options) -> dict:
        return {
            "plugins": [str(p) for p in self.plugins],
            "data_sources": self.data_sources,
            "base_plugins": self.base_plugins,
            "options": {k: v for k, v in options.items() if v is not None},
        }


def official_plugins(game_root: Path) -> list[str]:
    """Game, DLC and installed Creation Club plugins, in the order the game loads them."""
    names = list(get_profile("fo4").voice_official_masters)
    ccc = Path(game_root) / "Fallout4.ccc"
    if ccc.is_file():
        names += [line.strip() for line in ccc.read_text(encoding="utf-8", errors="replace").splitlines() if line.strip()]
    return names


def from_plugin_list(game_root: Path, active: list[str], loose_roots: list[Path], label: str) -> LoadOrderSpec:
    """`active` are plugin names after the implicit official ones; `loose_roots`
    are lowest priority first and include the game's Data folder."""
    roots = [Path(r) for r in loose_roots]
    official = official_plugins(game_root)
    names, seen = [], set()
    for name in [n for n in official if _find(roots, n)] + list(active):
        if name.lower() not in seen:
            seen.add(name.lower())
            names.append(name)
    plugins, missing = [], []
    for name in names:
        path = _find(roots, name)
        (plugins if path else missing).append(path or name)
    if missing:
        raise FileNotFoundError(f"active plugins not found in any data folder: {', '.join(missing)}")
    archives = []
    for plugin in plugins:
        stem = plugin.stem.lower()
        for archive in _archives_named(roots, stem):
            archives.append({"kind": "archive", "path": str(archive)})
    loose = [{"kind": "loose", "path": str(r)} for r in roots if r.is_dir()]
    base_names = {o.lower() for o in official} | {b.lower() for b in PREVIS_BASE_PLUGINS}
    base = [n for n in names if n.lower() in base_names]
    return LoadOrderSpec(label, Path(game_root), plugins, archives + loose, base)


def from_mo2_profile(profile_dir: Path, game_root: Path | None = None) -> LoadOrderSpec:
    """An MO2 profile: `plugins.txt` order, `modlist.txt` priority over the
    instance's `mods` folder, then `overwrite`. `game_root` defaults to a
    portable instance's `ModOrganizer.ini` gamePath."""
    profile_dir = Path(profile_dir)
    instance = profile_dir.parent.parent
    game_root = Path(game_root) if game_root else _mo2_game_path(instance)
    if game_root is None:
        raise FileNotFoundError(f"no game folder given and {instance / 'ModOrganizer.ini'} has no gamePath")
    enabled = []
    for line in _lines(profile_dir / "modlist.txt"):
        if line.startswith("+"):
            enabled.append(line[1:])
    mods_dir = instance / "mods"
    roots = [game_root / "Data"]
    roots += [mods_dir / name for name in reversed(enabled) if (mods_dir / name).is_dir()]
    roots.append(instance / "overwrite")
    active = [line[1:] for line in _lines(profile_dir / "plugins.txt") if line.startswith("*")]
    return from_plugin_list(game_root, active, roots, f"MO2 {instance.name} / {profile_dir.name}")


def modbox_profiles(library_root: Path) -> list[tuple[str, str]]:
    """`(id, name)` of each profile in a ModBox21 mod-manager library."""
    from creation_lib.mod_manager.library import Library

    return [(p.id, p.name) for p in Library(library_root).profiles()]


def from_modbox_profile(library_root: Path, profile_id: str | None = None) -> LoadOrderSpec:
    """A ModBox21 mod-manager profile: its plugin order and enabled mods by
    priority, over the instance's game folder, then the profile's overwrite."""
    from creation_lib.mod_manager import plugin_state, transactions
    from creation_lib.mod_manager.library import Library

    library = Library(library_root)
    profiles = library.profiles()
    if not profiles:
        raise FileNotFoundError(f"{library_root} has no profiles")
    profile_id = profile_id or profiles[0].id
    profile, rows, configuration = library.profile_snapshot(profile_id)[:3]
    game_root = Path(configuration.get("game", {}).get("gamepath", ""))
    if not (game_root / "Data").is_dir():
        raise FileNotFoundError(f"profile {profile.name} has no valid game folder: {game_root}")
    roots = [game_root / "Data"]
    roots += [Path(row.source) for row in rows if row.enabled and row.kind != "separator" and row.source]
    roots.append(Path(library_root) / "runtime" / "profiles" / transactions.key(profile_id) / "overwrite")
    catalog = plugin_state.inventory(library, profile_id)
    active = [p.name for p in sorted(catalog.plugins, key=lambda p: p.priority) if p.enabled]
    return from_plugin_list(game_root, active, roots, f"ModBox21 / {profile.name}")


def from_plugin(plugin: Path, game_root: Path, against: list[Path] = ()) -> LoadOrderSpec:
    """One plugin plus its masters, found next to it or in the game's Data folder.

    ``against`` are plugins it is checked against without mastering them (a
    previsbine base such as PPF.esm, or another mod): each loads, with its
    masters and archives, before the plugin.
    """
    plugin = Path(plugin)
    against = [Path(p) for p in against]
    data = Path(game_root) / "Data"
    roots = [data]
    for folder in [p.parent for p in against] + [plugin.parent]:
        if all(folder.resolve() != root.resolve() for root in roots):
            roots.append(folder)
    order: list[str] = []

    def visit(name: str, chain: tuple[str, ...]) -> None:
        if name.lower() in (n.lower() for n in order):
            return
        if name.lower() in chain:
            raise ValueError(f"master cycle: {' -> '.join(chain + (name.lower(),))}")
        path = _find(roots, name)
        if path is None:
            raise FileNotFoundError(f"master not found: {name}")
        for master in read_masters(path):
            visit(master, chain + (name.lower(),))
        order.append(path.name)

    for other in against:
        visit(other.name, ())
    visit(plugin.name, ())
    official = {o.lower() for o in official_plugins(game_root)}
    label = plugin.name + (f" against {', '.join(p.name for p in against)}" if against else "")
    spec = from_plugin_list(game_root, [n for n in order if n.lower() not in official], roots, label)
    # Only the officials this plugin needs, not every installed one.
    keep = {n.lower() for n in order}
    spec.plugins = [p for p in spec.plugins if p.name.lower() in keep]
    spec.base_plugins = [n for n in spec.base_plugins if n.lower() in keep]
    stems = {p.stem.lower() for p in spec.plugins}
    spec.data_sources = [s for s in spec.data_sources if s["kind"] == "loose" or _archive_stem(Path(s["path"])) in stems]
    return spec


def read_masters(path: Path) -> list[str]:
    with open(path, "rb") as handle:
        header = handle.read(24)
        if len(header) < 24 or header[:4] != b"TES4":
            raise ValueError(f"{path} is not a plugin")
        payload = handle.read(struct.unpack_from("<I", header, 4)[0])
    masters, offset = [], 0
    while offset + 6 <= len(payload):
        signature = payload[offset:offset + 4]
        size = struct.unpack_from("<H", payload, offset + 4)[0]
        if signature == b"MAST":
            masters.append(payload[offset + 6:offset + 6 + size].rstrip(b"\0").decode("utf-8", "replace"))
        offset += 6 + size
    return masters


def _find(roots: list[Path], name: str) -> Path | None:
    for root in reversed(roots):
        candidate = root / name
        if candidate.is_file():
            return candidate
    return None


def _archive_stem(path: Path) -> str:
    return path.stem.lower().rsplit(" - ", 1)[0]


def _archives_named(roots: list[Path], stem: str) -> list[Path]:
    """`<stem> - *.ba2`, the highest-priority copy of each, previsbine-capable kinds only."""
    winners: dict[str, Path] = {}
    for root in roots:
        if not root.is_dir():
            continue
        for archive in root.glob("*.ba2"):
            name = archive.stem.lower()
            if " - " not in name:
                continue
            archive_stem, kind = name.rsplit(" - ", 1)
            if archive_stem == stem and not kind.startswith(_IGNORED_ARCHIVE_KINDS):
                winners[archive.name.lower()] = archive
    return [winners[k] for k in sorted(winners)]


def _lines(path: Path) -> list[str]:
    if not path.is_file():
        return []
    return [line.strip() for line in path.read_text(encoding="utf-8-sig", errors="replace").splitlines() if line.strip() and not line.startswith("#")]


def _mo2_game_path(instance: Path) -> Path | None:
    ini = instance / "ModOrganizer.ini"
    if not ini.is_file():
        return None
    parser = configparser.ConfigParser(interpolation=None, strict=False)
    parser.read(ini, encoding="utf-8")
    raw = parser.get("General", "gamePath", fallback="")
    if raw.startswith("@ByteArray(") and raw.endswith(")"):
        raw = raw[len("@ByteArray("):-1]
    raw = raw.replace("\\\\", "\\").strip()
    return Path(raw) if raw else None

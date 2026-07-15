"""Bethesda Havok collision material helpers."""

from __future__ import annotations

from functools import lru_cache
from pathlib import Path
from xml.etree import ElementTree

_DEFAULT_ENUM = "Fallout4HavokMaterial"
_DEFAULT_MATERIAL = "Material__GenericMaster"
_NONE_ALIASES = {"", "none", "<none>", "null", "nullmaterial", "invalidmaterial"}


def _enum_name(profile=None) -> str:
    if profile is not None:
        enum_name = getattr(profile, "physics_material_enum", None)
        if enum_name:
            return str(enum_name)
    return _DEFAULT_ENUM


def _nif_xml_path() -> Path:
    return Path(__file__).resolve().parents[1] / "nif_xml" / "nif.xml"


def _short_label(name: str, text: str) -> str:
    if name == "<None>":
        return "NullMaterial"
    if text and text != "Invalid Material" and text != name:
        return text
    if name.startswith("Material__"):
        return name[len("Material__") :]
    if name.startswith("Material_"):
        return name[len("Material_") :]
    if name.startswith("Material"):
        return name[len("Material") :]
    return name


@lru_cache(maxsize=None)
def _options_for_enum(enum_name: str) -> tuple[dict[str, object], ...]:
    root = ElementTree.parse(_nif_xml_path()).getroot()
    enum = root.find(f".//enum[@name='{enum_name}']")
    if enum is None:
        enum = root.find(f".//enum[@name='{_DEFAULT_ENUM}']")
    if enum is None:
        return ()

    options: list[dict[str, object]] = []
    for option in enum.findall("option"):
        raw_value = option.get("value")
        name = option.get("name") or ""
        if raw_value is None or not name:
            continue
        value = int(raw_value)
        text = (option.text or "").strip()
        label = _short_label(name, text)
        options.append({"name": name, "label": label, "value": value})
    return tuple(options)


def get_collision_material_options(profile=None) -> list[dict[str, object]]:
    """Return material options for the profile's Bethesda Havok material enum."""
    def sort_key(option: dict[str, object]) -> tuple[int, str, str]:
        label = str(option["label"])
        pinned = {"NullMaterial": 0, "Generic": 1}.get(label, 2)
        return (pinned, label.casefold(), str(option["name"]).casefold())

    return [dict(option) for option in sorted(_options_for_enum(_enum_name(profile)), key=sort_key)]


def _maxscript_string(value: object) -> str:
    return '"' + str(value).replace("\\", "\\\\").replace('"', '\\"') + '"'


def build_maxscript_collision_material_defs(profile=None) -> str:
    options = get_collision_material_options(profile)
    labels = ", ".join(_maxscript_string(option["label"]) for option in options)
    names = ", ".join(_maxscript_string(option["name"]) for option in options)
    return "\n".join(
        [
            "global MB21_NIF_COLLISION_MATERIAL_LABELS",
            "global MB21_NIF_COLLISION_MATERIAL_NAMES",
            f"MB21_NIF_COLLISION_MATERIAL_LABELS = #({labels})",
            f"MB21_NIF_COLLISION_MATERIAL_NAMES = #({names})",
            "",
        ]
    )


def write_maxscript_collision_material_defs(path: str | Path, profile=None) -> Path:
    output_path = Path(path)
    output_path.write_text(build_maxscript_collision_material_defs(profile), encoding="utf-8")
    return output_path


def _alias_key(value: object) -> str:
    return "".join(ch for ch in str(value).lower() if ch.isalnum())


@lru_cache(maxsize=None)
def _lookup_for_enum(enum_name: str) -> dict[str, int]:
    lookup: dict[str, int] = {}
    for option in _options_for_enum(enum_name):
        name = str(option["name"])
        label = str(option["label"])
        value = int(option["value"])
        for alias in {name, label, name.removeprefix("Material")}:
            lookup[_alias_key(alias)] = value
        if value == 0:
            for alias in _NONE_ALIASES:
                lookup[_alias_key(alias)] = value
    return lookup


def resolve_collision_material(material: object | None, profile=None) -> int | None:
    """Resolve a material name, label, or numeric CRC to an integer value."""
    if material is None:
        return None
    if isinstance(material, int):
        return material
    if isinstance(material, float):
        return int(material)

    text = str(material).strip()
    if text.lower().startswith("0x"):
        return int(text, 16)
    if text.lstrip("-").isdigit():
        return int(text, 10)

    lookup = _lookup_for_enum(_enum_name(profile))
    key = _alias_key(text)
    if key in lookup:
        return lookup[key]
    raise ValueError(f"Unknown collision material: {material!r}")


def default_collision_material(profile=None) -> int:
    """Return the profile default collision material CRC."""
    return resolve_collision_material(_DEFAULT_MATERIAL, profile) or 0


def format_collision_material(material: object | None, profile=None) -> str | None:
    """Return a display label such as ``WeaponPistol (4146539321)``."""
    value = resolve_collision_material(material, profile)
    if value is None:
        return None
    for option in _options_for_enum(_enum_name(profile)):
        if int(option["value"]) == value:
            return f"{option['label']} ({value})"
    return f"0x{value:08X} ({value})"


def collision_material_type_name(material: object | None, profile=None) -> str | None:
    """Return the enum/type token such as ``MaterialWeaponPistol``."""
    value = resolve_collision_material(material, profile)
    if value is None:
        return None
    for option in _options_for_enum(_enum_name(profile)):
        if int(option["value"]) == value:
            name = str(option["name"])
            return "NullMaterial" if name == "<None>" else name
    return f"0x{value:08X}"

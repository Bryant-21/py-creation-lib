from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class BsxFlagDef:
    bit: int
    label: str
    description: str = ""

    @property
    def mask(self) -> int:
        return 1 << self.bit


# Labels mirror NifSkope's picker names where it exposes them, with the
# remaining known Bethesda bits filled from nif.xml so both tools share one set.
BSX_FLAG_DEFS: tuple[BsxFlagDef, ...] = (
    BsxFlagDef(0, "Animated", "Enable havok / bAnimated"),
    BsxFlagDef(1, "Havok", "Enable collision / bHavok"),
    BsxFlagDef(2, "Ragdoll", "Skeleton nif / bRagdoll"),
    BsxFlagDef(3, "Complex", "Enable animation / bComplex"),
    BsxFlagDef(4, "Addon", "FlameNodes present / bAddon"),
    BsxFlagDef(5, "Editor Marker", "EditorMarkers present / bEditorMarker"),
    BsxFlagDef(6, "Dynamic", "bDynamic"),
    BsxFlagDef(7, "Articulated", "bArticulated"),
    BsxFlagDef(8, "Needs Transform Updates", "bIKTarget / needsTransformUpdates"),
    BsxFlagDef(9, "External Emit", "bExternalEmit"),
    BsxFlagDef(10, "Magic Shader Particles", "bMagicShaderParticles"),
    BsxFlagDef(11, "Lights", "bLights"),
    BsxFlagDef(12, "Breakable", "bBreakable"),
    BsxFlagDef(
        13, "Searched Breakable", "bSearchedBreakable (runtime-only in some games)"
    ),
)


def bsx_flag_mask(bit: int) -> int:
    return 1 << bit


def normalize_bsx_flags(value: int | str | None) -> int:
    if value in (None, ""):
        return 0
    return int(value)


def bsx_flags_to_bits(value: int | str | None) -> list[int]:
    flags = normalize_bsx_flags(value)
    return [flag.bit for flag in BSX_FLAG_DEFS if flags & flag.mask]


def build_maxscript_bsx_flag_defs() -> str:
    lines = [
        "global MB21_NIF_BSX_FLAG_DEFS",
        "MB21_NIF_BSX_FLAG_DEFS = #(",
    ]
    for index, flag in enumerate(BSX_FLAG_DEFS):
        description = flag.description.replace('"', '\\"')
        suffix = "," if index < len(BSX_FLAG_DEFS) - 1 else ""
        lines.append(
            f'    #({flag.bit}, "{flag.label}", {flag.mask}, "{description}"){suffix}'
        )
    lines.append(")")
    lines.append("")
    return "\n".join(lines)


def write_maxscript_bsx_flag_defs(path: str | Path) -> Path:
    output_path = Path(path)
    output_path.write_text(build_maxscript_bsx_flag_defs(), encoding="utf-8")
    return output_path

"""Shared NIF validation checks used by CLI and editor UI."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .nif_file import NifBlock, NifFile


ERROR = "ERROR"
WARNING = "WARNING"
INFO = "INFO"

_TEXTURE_FIELD_NAMES = {
    "Source Texture",
    "Normal Texture",
    "Greyscale Texture",
    "Env Map Texture",
    "Env Mask Texture",
    "Reflectance Texture",
    "Lighting Texture",
    "Diffuse Texture",
    "Specular Texture",
}


@dataclass(frozen=True)
class ValidationIssue:
    severity: str
    block_id: int
    message: str
    field: str = ""

    def as_tuple(self) -> tuple[str, int, str]:
        return (self.severity, self.block_id, self.message)

    def as_dict(self) -> dict[str, Any]:
        result = {
            "severity": self.severity.lower(),
            "block": self.block_id,
            "message": self.message,
        }
        if self.field:
            result["field"] = self.field
        return result


def validate_nif(nif: NifFile) -> dict[str, Any]:
    """Validate generic NIF structure, references, geometry, and materials."""
    issues: list[ValidationIssue] = []
    block_count = len(nif.blocks)

    if block_count == 0:
        issues.append(ValidationIssue(ERROR, -1, "NIF has no blocks"))

    header_count = getattr(nif.header, "num_blocks", 0)
    if header_count and header_count != block_count:
        issues.append(ValidationIssue(
            WARNING,
            -1,
            f"header num_blocks is {header_count}, loaded block count is {block_count}",
            "num_blocks",
        ))

    roots = _root_blocks(nif)
    for root in roots:
        if root < 0 or root >= block_count:
            issues.append(ValidationIssue(
                ERROR,
                root,
                f"footer root {root} is out of range",
                "footer_roots",
            ))

    _check_block_ids(nif, issues)
    _check_unknown_block_types(nif, issues)
    _check_broken_refs(nif, issues)
    _check_orphaned_blocks(nif, roots, issues)
    _check_missing_shaders(nif, issues)
    _check_degenerate_tris(nif, issues)
    _check_unnormalized_normals(nif, issues)
    _check_duplicate_names(nif, issues)
    _check_external_geometry(nif, issues)
    _check_material_blocks(nif, issues)

    errors = [issue.as_dict() for issue in issues if issue.severity == ERROR]
    warnings = [issue.as_dict() for issue in issues if issue.severity == WARNING]
    infos = [issue.as_dict() for issue in issues if issue.severity == INFO]
    return {
        "valid": not errors,
        "block_count": block_count,
        "root_blocks": roots,
        "error_count": len(errors),
        "warning_count": len(warnings),
        "info_count": len(infos),
        "errors": errors,
        "warnings": warnings,
        "infos": infos,
        "issues": [issue.as_dict() for issue in issues],
    }


def validate_external_geometry(nif: NifFile) -> list[ValidationIssue]:
    issues: list[ValidationIssue] = []
    _check_external_geometry(nif, issues)
    return issues


def fix_validation_issues(nif: NifFile) -> int:
    """Fix generic issues that do not require guessing asset intent."""
    fixed = 0
    fixed += _fix_broken_refs(nif)
    fixed += _fix_degenerate_tris(nif)
    fixed += _fix_unnormalized_normals(nif)
    fixed += _fix_duplicate_names(nif)
    return fixed


def _root_blocks(nif: NifFile) -> list[int]:
    roots = list(getattr(nif, "_footer_roots", []) or [])
    if not roots and nif.blocks:
        roots = [0]
    return roots


def _has_text(value: Any) -> bool:
    return isinstance(value, str) and bool(value.replace("\x00", "").strip())


def _is_absolute_asset_path(value: str) -> bool:
    normalized = value.replace("/", "\\")
    return (
        len(normalized) >= 3
        and normalized[1] == ":"
        and normalized[2] == "\\"
    ) or normalized.startswith("\\")


def _is_bstrishape(nif: NifFile, block: NifBlock) -> bool:
    try:
        return nif.schema.is_subtype_of(block.type_name, "BSTriShape")
    except AttributeError:
        return block.type_name == "BSTriShape"


def _check_block_ids(nif: NifFile, issues: list[ValidationIssue]) -> None:
    for idx, block in enumerate(nif.blocks):
        if block.block_id != idx:
            issues.append(ValidationIssue(
                WARNING,
                block.block_id,
                f"block id {block.block_id} does not match loaded index {idx}",
            ))


def _check_unknown_block_types(nif: NifFile, issues: list[ValidationIssue]) -> None:
    schema = nif.schema
    niobjects = getattr(schema, "niobjects", {})
    structs = getattr(schema, "structs", {})
    if not niobjects and not structs:
        return
    for block in nif.blocks:
        if block.type_name not in niobjects and block.type_name not in structs:
            issues.append(ValidationIssue(
                WARNING,
                block.block_id,
                f"unknown block type {block.type_name}",
            ))


def _check_broken_refs(nif: NifFile, issues: list[ValidationIssue]) -> None:
    block_count = len(nif.blocks)
    schema = nif.schema
    for block in nif.blocks:
        for field_name, refs in _get_ref_fields(block, schema):
            for ref in refs:
                if ref < 0 or ref >= block_count:
                    issues.append(ValidationIssue(
                        ERROR,
                        block.block_id,
                        f"references missing block {ref}",
                        field_name,
                    ))


def _get_ref_fields(block: NifBlock, schema: Any) -> list[tuple[str, list[int]]]:
    try:
        return block.get_all_ref_fields(schema)
    except AttributeError:
        pass

    ref_fields: list[tuple[str, list[int]]] = []
    fdef_map = _field_def_map(schema, block.type_name)
    for name, value in block.fields:
        fdef = fdef_map.get(name)
        if not fdef:
            continue
        if getattr(fdef, "type", "") not in ("Ref", "Ptr"):
            continue
        if isinstance(value, int):
            ref_fields.append((name, [value]))
        elif isinstance(value, list):
            ref_fields.append((name, [v for v in value if isinstance(v, int)]))
    return ref_fields


def _field_def_map(schema: Any, type_name: str) -> dict[str, Any]:
    try:
        all_fields = schema.get_all_fields(type_name)
    except AttributeError:
        return {}
    result = {}
    for fdef in all_fields:
        result[fdef.name] = fdef
        suffix = getattr(fdef, "suffix", None)
        if suffix:
            result[f"{fdef.name}:{suffix}"] = fdef
    return result


def _check_orphaned_blocks(
    nif: NifFile,
    roots: list[int],
    issues: list[ValidationIssue],
) -> None:
    referenced = {root for root in roots if 0 <= root < len(nif.blocks)}
    schema = nif.schema
    for block in nif.blocks:
        try:
            refs = block.get_refs(schema)
        except AttributeError:
            refs = []
        referenced.update(r for r in refs if 0 <= r < len(nif.blocks))

    for block in nif.blocks:
        if block.block_id not in referenced:
            issues.append(ValidationIssue(
                WARNING,
                block.block_id,
                f"Orphaned block: {block.type_name} (not referenced by any other block)",
            ))


def _check_missing_shaders(nif: NifFile, issues: list[ValidationIssue]) -> None:
    for block in nif.blocks:
        if not _is_bstrishape(nif, block):
            continue
        shader_ref = block.get_field("Shader Property")
        if not isinstance(shader_ref, int) or shader_ref < 0:
            issues.append(ValidationIssue(
                WARNING,
                block.block_id,
                "Shape has no shader property",
                "Shader Property",
            ))


def _check_degenerate_tris(nif: NifFile, issues: list[ValidationIssue]) -> None:
    for block in nif.blocks:
        if not _is_bstrishape(nif, block):
            continue
        triangles = block.get_field("Triangles") or []
        degen = 0
        for tri in triangles:
            if not isinstance(tri, dict):
                continue
            v1 = int(tri.get("v1", 0))
            v2 = int(tri.get("v2", 0))
            v3 = int(tri.get("v3", 0))
            if v1 == v2 or v2 == v3 or v1 == v3:
                degen += 1
        if degen:
            issues.append(ValidationIssue(
                WARNING,
                block.block_id,
                f"{degen} degenerate triangle(s)",
                "Triangles",
            ))


def _check_unnormalized_normals(nif: NifFile, issues: list[ValidationIssue]) -> None:
    for block in nif.blocks:
        if not _is_bstrishape(nif, block):
            continue
        vertex_data = block.get_field("Vertex Data") or []
        bad = 0
        for vd in vertex_data:
            if not isinstance(vd, dict):
                continue
            normal = vd.get("Normal")
            if not normal:
                continue
            nx = float(normal.get("x", 0))
            ny = float(normal.get("y", 0))
            nz = float(normal.get("z", 0))
            length = (nx * nx + ny * ny + nz * nz) ** 0.5
            if abs(length - 1.0) > 0.01:
                bad += 1
        if bad:
            issues.append(ValidationIssue(
                INFO,
                block.block_id,
                f"{bad} unnormalized normal(s)",
                "Vertex Data",
            ))


def _check_duplicate_names(nif: NifFile, issues: list[ValidationIssue]) -> None:
    names = {}
    for block in nif.blocks:
        name = block.get_field("Name")
        if isinstance(name, list):
            name = "".join(str(c) for c in name)
        if not _has_text(name):
            continue
        name = str(name)
        if name in names:
            issues.append(ValidationIssue(
                INFO,
                block.block_id,
                f'Duplicate name "{name}" (also block {names[name]})',
                "Name",
            ))
        else:
            names[name] = block.block_id


def _check_external_geometry(nif: NifFile, issues: list[ValidationIssue]) -> None:
    for block in nif.blocks:
        if block.type_name != "BSGeometry":
            continue
        meshes = block.get_field("Meshes") or []
        for mesh_entry in meshes:
            if not isinstance(mesh_entry, dict):
                continue
            mesh = mesh_entry.get("Mesh", {})
            if not isinstance(mesh, dict):
                continue
            mesh_path = mesh.get("Mesh Path", "")
            if mesh_path:
                issues.append(ValidationIssue(
                    WARNING,
                    block.block_id,
                    f"Starfield external geometry: {mesh_path} "
                    f"(placeholder rendered -- full parsing not yet supported)",
                    "Meshes",
                ))


def _check_material_blocks(nif: NifFile, issues: list[ValidationIssue]) -> None:
    for block in nif.blocks:
        _check_absolute_texture_paths(block, issues)
        if block.type_name == "BSEffectShaderProperty":
            _check_effect_shader(block, issues)
        elif block.type_name == "BSLightingShaderProperty":
            _check_lighting_shader(nif, block, issues)


def _check_absolute_texture_paths(
    block: NifBlock,
    issues: list[ValidationIssue],
) -> None:
    for name, value in block.fields:
        bare_name = name.split(":", 1)[0]
        if bare_name in _TEXTURE_FIELD_NAMES and isinstance(value, str):
            if _is_absolute_asset_path(value):
                issues.append(ValidationIssue(
                    WARNING,
                    block.block_id,
                    f"absolute asset path should be Data-relative: {value}",
                    bare_name,
                ))
        elif bare_name == "Textures" and isinstance(value, list):
            for idx, texture in enumerate(value):
                if isinstance(texture, str) and _is_absolute_asset_path(texture):
                    issues.append(ValidationIssue(
                        WARNING,
                        block.block_id,
                        f"absolute asset path should be Data-relative: {texture}",
                        f"{bare_name}[{idx}]",
                    ))


def _check_effect_shader(block: NifBlock, issues: list[ValidationIssue]) -> None:
    name = block.get_field("Name")
    source_texture = block.get_field("Source Texture")
    if not _has_text(name) and not _has_text(source_texture):
        issues.append(ValidationIssue(
            WARNING,
            block.block_id,
            "inline BSEffectShaderProperty has no material name and no source texture",
            "Source Texture",
        ))

    env_texture = block.get_field("Env Map Texture")
    flags = block.get_field("Shader Flags 1") or 0
    has_env_flag = (
        isinstance(flags, int)
        and bool(flags & (1 << 7))
    ) or (
        isinstance(flags, list)
        and "Environment_Mapping" in flags
    )
    if has_env_flag and not _has_text(env_texture):
        issues.append(ValidationIssue(
            WARNING,
            block.block_id,
            "Environment_Mapping flag is set but Env Map Texture is empty",
            "Env Map Texture",
        ))
    elif _has_text(env_texture) and isinstance(flags, int) and not has_env_flag:
        issues.append(ValidationIssue(
            WARNING,
            block.block_id,
            "Env Map Texture is set but Environment_Mapping flag is not set",
            "Shader Flags 1",
        ))


def _check_lighting_shader(
    nif: NifFile,
    block: NifBlock,
    issues: list[ValidationIssue],
) -> None:
    texset_ref = block.get_field("Texture Set")
    if not isinstance(texset_ref, int) or texset_ref < 0:
        return
    target = nif.get_block(texset_ref)
    if target is None:
        issues.append(ValidationIssue(
            WARNING,
            block.block_id,
            f"Texture Set references missing block {texset_ref}",
            "Texture Set",
        ))
    elif target.type_name != "BSShaderTextureSet":
        issues.append(ValidationIssue(
            WARNING,
            block.block_id,
            f"Texture Set references block {texset_ref} ({target.type_name}), expected BSShaderTextureSet",
            "Texture Set",
        ))


def _fix_broken_refs(nif: NifFile) -> int:
    block_count = len(nif.blocks)
    fixed = 0
    schema = nif.schema
    for block in nif.blocks:
        fdef_map = _field_def_map(schema, block.type_name)
        for name, value in list(block.fields):
            fdef = fdef_map.get(name)
            if not fdef or getattr(fdef, "type", "") not in ("Ref", "Ptr"):
                continue
            if isinstance(value, int) and value >= block_count:
                block.set_field(name, -1)
                fixed += 1
    return fixed


def _fix_degenerate_tris(nif: NifFile) -> int:
    fixed = 0
    for block in nif.blocks:
        if not _is_bstrishape(nif, block):
            continue
        triangles = block.get_field("Triangles") or []
        if not triangles:
            continue
        clean = []
        for tri in triangles:
            if not isinstance(tri, dict):
                clean.append(tri)
                continue
            v1 = int(tri.get("v1", 0))
            v2 = int(tri.get("v2", 0))
            v3 = int(tri.get("v3", 0))
            if v1 != v2 and v2 != v3 and v1 != v3:
                clean.append(tri)
        removed = len(triangles) - len(clean)
        if removed:
            block.set_field("Triangles", clean)
            block.set_field("Num Triangles", len(clean))
            fixed += removed
    return fixed


def _fix_unnormalized_normals(nif: NifFile) -> int:
    fixed = 0
    for block in nif.blocks:
        if not _is_bstrishape(nif, block):
            continue
        vertex_data = block.get_field("Vertex Data") or []
        changed = False
        for vd in vertex_data:
            if not isinstance(vd, dict):
                continue
            normal = vd.get("Normal")
            if not normal:
                continue
            nx = float(normal.get("x", 0))
            ny = float(normal.get("y", 0))
            nz = float(normal.get("z", 0))
            length = (nx * nx + ny * ny + nz * nz) ** 0.5
            if length <= 1e-8 or abs(length - 1.0) <= 0.01:
                continue
            normal["x"] = nx / length
            normal["y"] = ny / length
            normal["z"] = nz / length
            fixed += 1
            changed = True
        if changed:
            block.set_field("Vertex Data", vertex_data)
    return fixed


def _fix_duplicate_names(nif: NifFile) -> int:
    used: set[str] = set()
    fixed = 0
    for block in nif.blocks:
        name = block.get_field("Name")
        if isinstance(name, list):
            name = "".join(str(c) for c in name)
        if not _has_text(name):
            continue
        name = str(name)
        if name not in used:
            used.add(name)
            continue

        suffix = 2
        candidate = f"{name}_{suffix}"
        while candidate in used:
            suffix += 1
            candidate = f"{name}_{suffix}"
        block.set_field("Name", candidate)
        used.add(candidate)
        fixed += 1
    return fixed

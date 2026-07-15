from __future__ import annotations

import json
import struct
from pathlib import Path
from typing import Any, BinaryIO


TRI_SIGNATURE = b"FRTRI003"
MORPH_DOCUMENT_KIND = "max_morph_document"
SUPPORTED_EXTENSIONS = {".tri", ".trip", ".osd", ".json"}


def build_morph_document(
    path: str | Path,
    *,
    known_shape_names: list[str] | tuple[str, ...] = (),
) -> dict[str, Any]:
    source = Path(path)
    suffix = source.suffix.lower()
    if suffix not in SUPPORTED_EXTENSIONS:
        raise ValueError(f"unsupported morph sidecar extension: {source.suffix}")
    if suffix == ".json":
        payload = json.loads(source.read_text(encoding="utf-8"))
        if not isinstance(payload, dict):
            raise ValueError(f"morph document must be a JSON object: {source}")
        return _normalize_document(payload, source_path=str(source))
    if suffix == ".tri":
        return _read_tri_document(source)
    if suffix == ".trip":
        return _read_trip_document(source)
    return _read_osd_document(source, known_shape_names=known_shape_names)


def write_morph_document(document: dict[str, Any], output: str | Path) -> dict[str, Any]:
    target = Path(output)
    suffix = target.suffix.lower()
    if suffix not in SUPPORTED_EXTENSIONS:
        raise ValueError(f"unsupported morph sidecar extension: {target.suffix}")

    normalized = _normalize_document(
        document,
        sidecar_kind=suffix.lstrip("."),
        output_path=str(target),
    )
    target.parent.mkdir(parents=True, exist_ok=True)
    if suffix == ".json":
        target.write_text(json.dumps(normalized, indent=2), encoding="utf-8")
    elif suffix == ".tri":
        _write_tri_document(normalized, target)
    elif suffix == ".trip":
        _write_trip_document(normalized, target)
    else:
        _write_osd_document(normalized, target)
    return normalized


def _normalize_document(
    document: dict[str, Any],
    *,
    sidecar_kind: str | None = None,
    source_path: str = "",
    output_path: str = "",
) -> dict[str, Any]:
    kind = str(sidecar_kind or document.get("sidecar_kind") or "tri").lower()
    normalized: dict[str, Any] = {
        "kind": MORPH_DOCUMENT_KIND,
        "game": str(document.get("game") or "fo4"),
        "sidecar_kind": kind,
        "targets": [_normalize_target(target) for target in document.get("targets") or []],
    }
    base_mesh = document.get("base_mesh")
    if isinstance(base_mesh, dict):
        normalized["base_mesh"] = _normalize_base_mesh(base_mesh)
    if source_path or document.get("source_path"):
        normalized["source_path"] = source_path or str(document.get("source_path"))
    if output_path or document.get("output_path"):
        normalized["output_path"] = output_path or str(document.get("output_path"))
    return normalized


def _normalize_target(target: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": str(target.get("name") or ""),
        "shape_name": str(target.get("shape_name") or ""),
        "offsets": [_normalize_offset(offset) for offset in target.get("offsets") or []],
    }


def _normalize_offset(offset: dict[str, Any]) -> dict[str, Any]:
    return {
        "vertex": int(offset.get("vertex") or 0),
        "x": float(offset.get("x") or 0.0),
        "y": float(offset.get("y") or 0.0),
        "z": float(offset.get("z") or 0.0),
    }


def _normalize_base_mesh(base_mesh: dict[str, Any]) -> dict[str, Any]:
    return {
        "vertices": [_normalize_vertex(vertex) for vertex in base_mesh.get("vertices") or []],
        "faces": [_normalize_index_triplet(face) for face in base_mesh.get("faces") or []],
        "uvs": [_normalize_uv(uv) for uv in base_mesh.get("uvs") or []],
        "uv_faces": [
            _normalize_index_triplet(face) for face in base_mesh.get("uv_faces") or []
        ],
    }


def _normalize_vertex(vertex: dict[str, Any] | list[Any] | tuple[Any, ...]) -> dict[str, float]:
    if isinstance(vertex, dict):
        return {
            "x": float(vertex.get("x") or 0.0),
            "y": float(vertex.get("y") or 0.0),
            "z": float(vertex.get("z") or 0.0),
        }
    return {
        "x": float(vertex[0]) if len(vertex) > 0 else 0.0,
        "y": float(vertex[1]) if len(vertex) > 1 else 0.0,
        "z": float(vertex[2]) if len(vertex) > 2 else 0.0,
    }


def _normalize_uv(uv: dict[str, Any] | list[Any] | tuple[Any, ...]) -> dict[str, float]:
    if isinstance(uv, dict):
        return {"u": float(uv.get("u") or 0.0), "v": float(uv.get("v") or 0.0)}
    return {
        "u": float(uv[0]) if len(uv) > 0 else 0.0,
        "v": float(uv[1]) if len(uv) > 1 else 0.0,
    }


def _normalize_index_triplet(value: list[Any] | tuple[Any, ...]) -> list[int]:
    return [int(value[0]), int(value[1]), int(value[2])]


def _read_tri_document(source: Path) -> dict[str, Any]:
    with source.open("rb") as file:
        header = file.read(64)
        if len(header) != 64:
            raise ValueError(f"invalid TRI header: {source}")
        signature, vertex_count, face_count, _, _, _, uv_count, _, morph_count, mod_morph_count, mod_vertex_count = struct.unpack(
            "<8s10I16x",
            header,
        )
        if signature != TRI_SIGNATURE:
            raise ValueError(f"invalid TRI signature: {source}")
        vertices = _read_structs(file, f"<{vertex_count * 3}f", vertex_count, 3)
        file.seek(mod_vertex_count * 12, 1)
        faces = _read_structs(file, f"<{face_count * 3}I", face_count, 3)
        uvs = _read_structs(file, f"<{uv_count * 2}f", uv_count, 2)
        uv_faces = _read_structs(file, f"<{face_count * 3}I", face_count, 3)
        targets = []
        for _ in range(morph_count):
            name = _read_tri_string(file)
            scale = struct.unpack("<f", file.read(4))[0]
            raw_offsets = _read_structs(file, f"<{vertex_count * 3}h", vertex_count, 3)
            offsets = []
            for index, raw in enumerate(raw_offsets):
                x = raw[0] * scale
                y = raw[1] * scale
                z = raw[2] * scale
                if _has_offset(x, y, z):
                    offsets.append(_offset(index, x, y, z))
            shape_name, morph_name = _split_compound_name(name)
            targets.append({"name": morph_name, "shape_name": shape_name, "offsets": offsets})
        for _ in range(mod_morph_count):
            _skip_tri_mod_morph(file)
    return {
        "kind": MORPH_DOCUMENT_KIND,
        "game": "fo4",
        "sidecar_kind": "tri",
        "source_path": str(source),
        "base_mesh": {
            "vertices": [_vertex_dict(vertex) for vertex in vertices],
            "faces": [list(face) for face in faces],
            "uvs": [_uv_dict(uv) for uv in uvs],
            "uv_faces": [list(face) for face in uv_faces],
        },
        "targets": targets,
    }


def _write_tri_document(document: dict[str, Any], target: Path) -> None:
    base_mesh = _base_mesh_for_tri(document)
    vertices = base_mesh["vertices"]
    faces = base_mesh["faces"]
    uvs = base_mesh["uvs"]
    uv_faces = base_mesh["uv_faces"]
    vertex_count = len(vertices)
    header = struct.pack(
        "<8s14I",
        TRI_SIGNATURE,
        vertex_count,
        len(faces),
        0,
        0,
        0,
        len(uvs),
        1,
        len(document["targets"]),
        0,
        0,
        0,
        0,
        0,
        0,
    )
    with target.open("wb") as file:
        file.write(header)
        for vertex in vertices:
            file.write(struct.pack("<3f", vertex["x"], vertex["y"], vertex["z"]))
        for face in faces:
            file.write(struct.pack("<3I", *face))
        for uv in uvs:
            file.write(struct.pack("<2f", uv["u"], uv["v"]))
        for face in uv_faces:
            file.write(struct.pack("<3I", *face))
        for target_doc in document["targets"]:
            name = _compound_name(target_doc)
            encoded = name.encode("utf-8")
            dense_offsets = [(0.0, 0.0, 0.0)] * vertex_count
            for offset in target_doc["offsets"]:
                dense_offsets[offset["vertex"]] = (offset["x"], offset["y"], offset["z"])
            scale = _tri_scale(dense_offsets)
            file.write(struct.pack(f"<I{len(encoded)}sx", len(encoded) + 1, encoded))
            file.write(struct.pack("<f", scale))
            for x, y, z in dense_offsets:
                file.write(
                    struct.pack(
                        "<3h",
                        _quantize_short(x, scale),
                        _quantize_short(y, scale),
                        _quantize_short(z, scale),
                    )
                )


def _read_trip_document(source: Path) -> dict[str, Any]:
    with source.open("rb") as file:
        if file.read(4) not in (b"PIRT", b"\0IRT"):
            raise ValueError(f"invalid TRIP signature: {source}")
        shape_count = struct.unpack("<H", file.read(2))[0]
        targets = []
        for _ in range(shape_count):
            shape_name = _read_counted_string(file, "iso-8859-15")
            morph_count = struct.unpack("<H", file.read(2))[0]
            for _ in range(morph_count):
                morph_name = _read_counted_string(file, "iso-8859-15")
                multiplier = struct.unpack("<f", file.read(4))[0]
                vertex_count = struct.unpack("<H", file.read(2))[0]
                offsets = []
                for _ in range(vertex_count):
                    vertex, x, y, z = struct.unpack("<H3h", file.read(8))
                    x *= multiplier
                    y *= multiplier
                    z *= multiplier
                    if _has_offset(x, y, z):
                        offsets.append(_offset(vertex, x, y, z))
                targets.append({"name": morph_name, "shape_name": shape_name, "offsets": offsets})
    return {
        "kind": MORPH_DOCUMENT_KIND,
        "game": "fo4",
        "sidecar_kind": "trip",
        "source_path": str(source),
        "targets": targets,
    }


def _write_trip_document(document: dict[str, Any], target: Path) -> None:
    shapes: dict[str, list[dict[str, Any]]] = {}
    for target_doc in document["targets"]:
        shapes.setdefault(target_doc["shape_name"], []).append(target_doc)
    with target.open("wb") as file:
        file.write(b"PIRT")
        file.write(struct.pack("<H", len(shapes)))
        for shape_name, targets in shapes.items():
            _write_counted_string(file, shape_name, "iso-8859-15")
            file.write(struct.pack("<H", len(targets)))
            for target_doc in targets:
                _write_counted_string(file, target_doc["name"], "iso-8859-15")
                multiplier = _trip_multiplier(target_doc["offsets"])
                file.write(struct.pack("<f", multiplier))
                file.write(struct.pack("<H", len(target_doc["offsets"])))
                for offset in target_doc["offsets"]:
                    file.write(struct.pack("<H", offset["vertex"]))
                    file.write(
                        struct.pack(
                            "<3h",
                            _quantize_short(offset["x"], multiplier),
                            _quantize_short(offset["y"], multiplier),
                            _quantize_short(offset["z"], multiplier),
                        )
                    )


def _read_osd_document(
    source: Path,
    *,
    known_shape_names: list[str] | tuple[str, ...] = (),
) -> dict[str, Any]:
    with source.open("rb") as file:
        if file.read(4) not in (b"\0DSO", b"OSD\0"):
            raise ValueError(f"invalid OSD signature: {source}")
        file.read(4)
        entry_count = struct.unpack("<I", file.read(4))[0]
        targets = []
        for _ in range(entry_count):
            compound_name = _read_counted_string(file, "utf-8")
            offset_count = struct.unpack("<H", file.read(2))[0]
            offsets = []
            for _ in range(offset_count):
                vertex, x, y, z = struct.unpack("<H3f", file.read(14))
                if _has_offset(x, y, z):
                    offsets.append(_offset(vertex, x, y, z))
            shape_name, morph_name = _split_compound_name(
                compound_name,
                known_shape_names=known_shape_names,
            )
            targets.append({"name": morph_name, "shape_name": shape_name, "offsets": offsets})
    return {
        "kind": MORPH_DOCUMENT_KIND,
        "game": "fo4",
        "sidecar_kind": "osd",
        "source_path": str(source),
        "targets": targets,
    }


def _write_osd_document(document: dict[str, Any], target: Path) -> None:
    with target.open("wb") as file:
        file.write(b"\0DSO")
        file.write(struct.pack("<I", 1))
        file.write(struct.pack("<I", len(document["targets"])))
        for target_doc in document["targets"]:
            _write_counted_string(file, _osd_compound_name(target_doc), "utf-8")
            file.write(struct.pack("<H", len(target_doc["offsets"])))
            for offset in target_doc["offsets"]:
                file.write(
                    struct.pack(
                        "<H3f",
                        offset["vertex"],
                        offset["x"],
                        offset["y"],
                        offset["z"],
                    )
                )


def _read_structs(
    file: BinaryIO,
    fmt: str,
    count: int,
    width: int,
) -> list[tuple[float | int, ...]]:
    if count == 0:
        return []
    data = struct.unpack(fmt, file.read(struct.calcsize(fmt)))
    return [tuple(data[index : index + width]) for index in range(0, len(data), width)]


def _read_tri_string(file: BinaryIO) -> str:
    length = struct.unpack("<I", file.read(4))[0]
    if length == 0:
        return ""
    return struct.unpack(f"<{length - 1}sx", file.read(length))[0].decode("utf-8")


def _skip_tri_mod_morph(file: BinaryIO) -> None:
    name_length = struct.unpack("<I", file.read(4))[0]
    file.seek(name_length, 1)
    block_length = struct.unpack("<I", file.read(4))[0]
    file.seek(block_length * 4, 1)


def _read_counted_string(file: BinaryIO, encoding: str) -> str:
    length = struct.unpack("<B", file.read(1))[0]
    if length == 0:
        return ""
    return file.read(length).decode(encoding, errors="replace")


def _write_counted_string(file: BinaryIO, value: str, encoding: str) -> None:
    encoded = value.encode(encoding)
    if len(encoded) > 255:
        raise ValueError(f"morph name is too long for counted string: {value}")
    file.write(struct.pack("<B", len(encoded)))
    file.write(encoded)


def _required_vertex_count(targets: list[dict[str, Any]]) -> int:
    highest = -1
    for target in targets:
        for offset in target["offsets"]:
            highest = max(highest, offset["vertex"])
    return highest + 1


def _base_mesh_for_tri(document: dict[str, Any]) -> dict[str, Any]:
    base_mesh = document.get("base_mesh")
    if isinstance(base_mesh, dict):
        normalized = _normalize_base_mesh(base_mesh)
        if normalized["vertices"]:
            if not normalized["uv_faces"]:
                normalized["uv_faces"] = list(normalized["faces"])
            return normalized

    vertex_count = max(3, _required_vertex_count(document["targets"]))
    vertices = [{"x": float(index), "y": 0.0, "z": 0.0} for index in range(vertex_count)]
    faces = [[0, 1, 2]]
    uvs = [{"u": 0.0, "v": 0.0} for _ in range(vertex_count)]
    return {"vertices": vertices, "faces": faces, "uvs": uvs, "uv_faces": faces}


def _tri_scale(offsets: list[tuple[float, float, float]]) -> float:
    max_diff = max((abs(value) for offset in offsets for value in offset), default=0.0)
    if max_diff == 0.0:
        return 1.0
    return max_diff / 0x7FFF


def _trip_multiplier(offsets: list[dict[str, Any]]) -> float:
    max_diff = max(
        (abs(offset[axis]) for offset in offsets for axis in ("x", "y", "z")),
        default=0.0,
    )
    if max_diff == 0.0:
        return 1.0
    return max_diff / 0x7FFF


def _quantize_short(value: float, scale: float) -> int:
    if scale == 0.0:
        return 0
    return max(-0x8000, min(0x7FFF, int(value / scale)))


def _has_offset(x: float, y: float, z: float) -> bool:
    return abs(x) > 0.0001 or abs(y) > 0.0001 or abs(z) > 0.0001


def _offset(vertex: int, x: float, y: float, z: float) -> dict[str, Any]:
    return {
        "vertex": int(vertex),
        "x": round(float(x), 6),
        "y": round(float(y), 6),
        "z": round(float(z), 6),
    }


def _compound_name(target: dict[str, Any]) -> str:
    shape_name = str(target.get("shape_name") or "")
    name = str(target.get("name") or "")
    return f"{shape_name}:{name}" if shape_name else name


def _osd_compound_name(target: dict[str, Any]) -> str:
    return f"{target.get('shape_name') or ''}{target.get('name') or ''}"


def _split_compound_name(
    name: str,
    *,
    known_shape_names: list[str] | tuple[str, ...] = (),
) -> tuple[str, str]:
    if ":" not in name:
        for shape_name in sorted(known_shape_names, key=len, reverse=True):
            if name.startswith(shape_name) and len(name) > len(shape_name):
                return shape_name, name[len(shape_name) :]
        return "", name
    shape_name, morph_name = name.split(":", 1)
    return shape_name, morph_name


def _vertex_dict(vertex: tuple[float | int, ...]) -> dict[str, float]:
    return {"x": round(float(vertex[0]), 6), "y": round(float(vertex[1]), 6), "z": round(float(vertex[2]), 6)}


def _uv_dict(uv: tuple[float | int, ...]) -> dict[str, float]:
    return {"u": round(float(uv[0]), 6), "v": round(float(uv[1]), 6)}

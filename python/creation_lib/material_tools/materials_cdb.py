"""Thin Python dataclass/API layer over the native MaterialsDB.cdb reader."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

from creation_lib.material_tools import native_runtime
from creation_lib.material_tools._bsrefl_stringtable import STRING_TABLE


# ---------------------------------------------------------------------------
# CRC-32 / BSResourceID
# ---------------------------------------------------------------------------

def bethesda_crc32(data: bytes) -> int:
    """Raw CRC-32 with poly 0xEDB88320, init=0, no post-xor."""
    return native_runtime.bethesda_crc32(data)


@dataclass(frozen=True)
class BSResourceID:
    """Content-addressed file id for .mat files.

    Three u32 values:
      * ``dir``  - CRC32 of the directory path, lowercased and with '/'
                   normalized to '\\' before hashing.
      * ``file`` - CRC32 of the base file name (no extension), lowercased.
      * ``ext``  - Little-endian packed ASCII of the extension characters
                   after the dot, lowercased via
                   ``ext | ((ext >> 1) & 0x20202020)``. For "mat" this
                   yields 0x0074616D.
    """

    dir: int
    file: int
    ext: int

    @classmethod
    def from_path(cls, path: str) -> "BSResourceID":
        payload = native_runtime.resource_id_from_path(path)
        return cls(
            dir=int(payload["dir"]),
            file=int(payload["file"]),
            ext=int(payload["ext"]),
        )


# ---------------------------------------------------------------------------
# Class definitions (from TYPE/CLAS chunks)
# ---------------------------------------------------------------------------

@dataclass
class FieldDef:
    """A single field inside a ``ClassDef``.

    ``name_index`` and ``type_index`` are master STRING_TABLE indices.
    """

    name_index: int
    type_index: int
    data_offset: int
    data_size: int

    @property
    def name(self) -> str:
        if 0 <= self.name_index < len(STRING_TABLE):
            return STRING_TABLE[self.name_index]
        return "<unknown>"

    @property
    def type_name(self) -> str:
        if 0 <= self.type_index < len(STRING_TABLE):
            return STRING_TABLE[self.type_index]
        return "<unknown>"


@dataclass
class ClassDef:
    """A class schema registered from a CLAS chunk inside a TYPE block.

    Mirrors ``BSMaterialsCDB::CDBClassDef``.
    """

    class_name: str
    class_name_index: int
    class_version: int
    class_flags: int
    field_count: int
    fields: list[FieldDef] = field(default_factory=list)

    @property
    def is_user(self) -> bool:
        # cpp: classDef.isUser = bool(classFlags & 4)
        return bool(self.class_flags & 4)


# ---------------------------------------------------------------------------
# Component blob + MaterialObject
# ---------------------------------------------------------------------------

@dataclass
class ComponentBlob:
    """A raw component body (OBJT or DIFF chunk) awaiting interpretation.

    Stored alongside its resolved class name and whether it's a DIFF
    (sparse, field-number prefixed) or OBJT (dense, iterated in order)
    chunk. Interpretation is delegated to the native component walker.
    """

    class_name: str
    is_diff: bool
    key: int            # (class_name_index << 16) | (component_key & 0xFFFF)
    body: bytes         # the raw chunk body after the initial className u32

    @property
    def component_type(self) -> int:
        # cpp: key = (class_name_index << 16) | (key & 0xFFFF)
        return (self.key >> 16) & 0xFFFF

    @property
    def component_index(self) -> int:
        return self.key & 0xFFFF


@dataclass
class MaterialObject:
    """A single entry from the DBFileIndex::ObjectInfo list.

    Mirrors ``BSMaterialsCDB::MaterialObject`` at the level of detail
    needed for FO4 downgrade. Components are accumulated as raw blobs
    and tagged with the class name resolved from the matching
    ComponentInfo list entry.
    """

    persistent_id: BSResourceID
    db_id: int
    base_object_db_id: int
    has_data: bool
    path: str = ""  # optional human-readable path, set by fixture/test
    parent: Optional["MaterialObject"] = None
    components: list[ComponentBlob] = field(default_factory=list)


# ---------------------------------------------------------------------------
# CE2 layer/blender/texture-set dataclasses
# ---------------------------------------------------------------------------
#
# These mirror the four CE2Material child-object types in
# ``material.hpp`` at the level of detail needed by ``cdb_to_bgsm``:
#
#   CE2Material.Layer (type 3) -> references a Material (type 4)
#                                 and a UVStream (type 6)
#   CE2Material.Material       -> holds shader params + a TextureSet
#   CE2Material.TextureSet (5) -> diffuse / normal / opacity slot strings
#   CE2Material.UVStream (6)   -> texture coord transform
#   CE2Material.Blender (2)    -> layer blend mode (dropped on FO4 flatten)
#
# Python keeps these dataclasses as the stable public API; native code
# populates them from CDB payloads.

@dataclass
class CE2TextureSet:
    """A CE2Material::TextureSet.

    Slot names follow the FO76 convention in ``material.hpp``:
    ``diffuse`` = color, ``normal`` = tangent-space normal,
    ``opacity`` = alpha mask. Only diffuse/normal are consumed today;
    the rest default to empty.
    """

    diffuse: str = ""
    normal: str = ""
    opacity: str = ""
    rough: str = ""
    metal: str = ""
    ao: str = ""
    emissive: str = ""


@dataclass
class CE2UVStream:
    """A CE2Material::UVStream (UV transform). Defaults to identity."""

    scale_u: float = 1.0
    scale_v: float = 1.0
    offset_u: float = 0.0
    offset_v: float = 0.0
    channel: int = 0


@dataclass
class CE2MaterialProps:
    """CE2Material::Material shader props (metal-rough PBR).

    Mirrors the subset of ``BSMaterial::Material`` fields that
    ``cdb_to_bgsm`` consumes: smoothness is ``1 - roughness``, metalness
    is the standard metal-rough factor. The rest of the shader
    parameters default to neutral values.
    """

    smoothness: float = 0.5
    metalness: float = 0.0
    emissive_color: tuple[float, float, float] = (0.0, 0.0, 0.0)
    emissive_multiplier: float = 0.0
    alpha: float = 1.0


@dataclass
class CE2Layer:
    """One layer in a CE2Material (a (Material, TextureSet, UVStream) tuple)."""

    material: CE2MaterialProps = field(default_factory=CE2MaterialProps)
    texture_set: CE2TextureSet = field(default_factory=CE2TextureSet)
    uv_stream: CE2UVStream = field(default_factory=CE2UVStream)
    name: str = ""


@dataclass
class CE2Blender:
    """A CE2Material::Blender (layer-blend node).

    FO4 has no concept of multi-layer blending; ``cdb_to_bgsm`` logs and
    drops these. We keep the dataclass so fixtures can exercise the
    drop path.
    """

    mode: int = 0
    mask_texture: str = ""


# ---------------------------------------------------------------------------
# CE2Material (thin view)
# ---------------------------------------------------------------------------

@dataclass
class CE2Material:
    """A material object surfaced for downstream consumers.

    The underlying ``MaterialObject`` is the raw CDB record; the
    ``layers`` / ``blenders`` / ``lod_materials`` fields are the
    flattened, consumer-friendly view that ``cdb_to_bgsm`` reads.

    Native projection populates these lists from ``MaterialsCDB`` payloads.
    """

    name: str
    material_object: Optional[MaterialObject] = None
    layers: list[CE2Layer] = field(default_factory=list)
    blenders: list[CE2Blender] = field(default_factory=list)
    lod_materials: list["CE2Material"] = field(default_factory=list)

    @property
    def components(self) -> list[ComponentBlob]:
        if self.material_object is None:
            return []
        return self.material_object.components


def _apply_native_ce2_payload(mat: CE2Material, payload: dict) -> None:
    object_path = payload.get("object_path")
    if (
        isinstance(object_path, str)
        and object_path
        and mat.material_object is not None
        and not mat.material_object.path
    ):
        mat.material_object.path = object_path
    mat.layers = [
        _native_layer_from_payload(layer_payload)
        for layer_payload in payload.get("layers", [])
        if isinstance(layer_payload, dict)
    ]
    mat.blenders = [
        _native_blender_from_payload(blender_payload)
        for blender_payload in payload.get("blenders", [])
        if isinstance(blender_payload, dict)
    ]
    mat.lod_materials = [
        _native_material_from_payload(lod_payload)
        for lod_payload in payload.get("lod_materials", [])
        if isinstance(lod_payload, dict)
    ]


def _native_material_from_payload(payload: dict) -> CE2Material:
    mat = CE2Material(name=str(payload.get("name") or ""))
    _apply_native_ce2_payload(mat, payload)
    return mat


def _native_layer_from_payload(payload: dict) -> CE2Layer:
    return CE2Layer(
        material=_native_material_props_from_payload(
            _payload_dict(payload.get("material"))
        ),
        texture_set=_native_texture_set_from_payload(
            _payload_dict(payload.get("texture_set"))
        ),
        uv_stream=_native_uv_stream_from_payload(
            _payload_dict(payload.get("uv_stream"))
        ),
        name=str(payload.get("name") or ""),
    )


def _native_texture_set_from_payload(payload: dict) -> CE2TextureSet:
    return CE2TextureSet(
        diffuse=str(payload.get("diffuse") or ""),
        normal=str(payload.get("normal") or ""),
        opacity=str(payload.get("opacity") or ""),
        rough=str(payload.get("rough") or ""),
        metal=str(payload.get("metal") or ""),
        ao=str(payload.get("ao") or ""),
        emissive=str(payload.get("emissive") or ""),
    )


def _native_uv_stream_from_payload(payload: dict) -> CE2UVStream:
    return CE2UVStream(
        scale_u=float(payload.get("scale_u", 1.0)),
        scale_v=float(payload.get("scale_v", 1.0)),
        offset_u=float(payload.get("offset_u", 0.0)),
        offset_v=float(payload.get("offset_v", 0.0)),
        channel=int(payload.get("channel", 0)),
    )


def _native_material_props_from_payload(payload: dict) -> CE2MaterialProps:
    color = payload.get("emissive_color", (0.0, 0.0, 0.0))
    if not isinstance(color, (list, tuple)) or len(color) < 3:
        color = (0.0, 0.0, 0.0)
    return CE2MaterialProps(
        smoothness=float(payload.get("smoothness", 0.5)),
        metalness=float(payload.get("metalness", 0.0)),
        emissive_color=(float(color[0]), float(color[1]), float(color[2])),
        emissive_multiplier=float(payload.get("emissive_multiplier", 0.0)),
        alpha=float(payload.get("alpha", 1.0)),
    )


def _native_blender_from_payload(payload: dict) -> CE2Blender:
    return CE2Blender(
        mode=int(payload.get("mode", 0)),
        mask_texture=str(payload.get("mask_texture") or ""),
    )


def _payload_dict(value: object) -> dict:
    return value if isinstance(value, dict) else {}


# ---------------------------------------------------------------------------
# MaterialsCDB
# ---------------------------------------------------------------------------

class MaterialsCDB:
    """Parsed in-memory view of a MaterialsDB.cdb file.

    Construct via ``MaterialsCDB.from_file(path)`` or
    ``MaterialsCDB.from_bytes(data)``. After construction:

    * ``class_defs`` maps class name -> ``ClassDef``.
    * ``objects_by_db_id`` maps dbID -> ``MaterialObject``.
    * ``objects_by_persistent_id`` maps ``BSResourceID`` ->
      ``MaterialObject``.
    * ``lookup_by_path(path)`` resolves a .mat file path to the
      corresponding ``MaterialObject``.
    * ``list_materials()`` returns a sorted list of all known material
      paths (best-effort: only objects with a recorded path surface).
    """

    def __init__(self) -> None:
        self.class_defs: dict[str, ClassDef] = {}
        self.objects_by_db_id: dict[int, MaterialObject] = {}
        self.objects_by_persistent_id: dict[BSResourceID, MaterialObject] = {}
        self._component_info: list[tuple[int, int]] = []  # (db_id, key)

    # -- construction ------------------------------------------------------

    @classmethod
    def from_bytes(cls, data: bytes) -> "MaterialsCDB":
        cdb = cls()
        cdb._load_native_payload(native_runtime.parse_cdb(data))
        return cdb

    @classmethod
    def from_file(cls, path: Path | str) -> "MaterialsCDB":
        return cls.from_bytes(Path(path).read_bytes())

    def _load_native_payload(self, payload: dict) -> None:
        self.class_defs = {}
        self.objects_by_db_id = {}
        self.objects_by_persistent_id = {}
        self._component_info = [
            (int(item["db_id"]), int(item["key"]))
            for item in payload.get("component_info", [])
        ]

        for item in payload.get("class_defs", []):
            cdef = ClassDef(
                class_name=str(item["class_name"]),
                class_name_index=int(item["class_name_index"]),
                class_version=int(item["class_version"]),
                class_flags=int(item["class_flags"]),
                field_count=int(item["field_count"]),
            )
            for field_item in item.get("fields", []):
                cdef.fields.append(
                    FieldDef(
                        name_index=int(field_item["name_index"]),
                        type_index=int(field_item["type_index"]),
                        data_offset=int(field_item["data_offset"]),
                        data_size=int(field_item["data_size"]),
                    )
                )
            self.class_defs[cdef.class_name] = cdef

        parent_links: dict[int, int] = {}
        for item in payload.get("objects", []):
            pid_item = item["persistent_id"]
            pid = BSResourceID(
                dir=int(pid_item["dir"]),
                file=int(pid_item["file"]),
                ext=int(pid_item["ext"]),
            )
            obj = MaterialObject(
                persistent_id=pid,
                db_id=int(item["db_id"]),
                base_object_db_id=int(item["base_object_db_id"]),
                has_data=bool(item["has_data"]),
            )
            for component_item in item.get("components", []):
                body = component_item["body"]
                obj.components.append(
                    ComponentBlob(
                        class_name=str(component_item["class_name"]),
                        is_diff=bool(component_item["is_diff"]),
                        key=int(component_item["key"]),
                        body=bytes(body),
                    )
                )
            self.objects_by_db_id[obj.db_id] = obj
            if pid.dir or pid.file or pid.ext:
                self.objects_by_persistent_id[pid] = obj
            parent_db_id = item.get("parent_db_id")
            if parent_db_id is not None:
                parent_links[obj.db_id] = int(parent_db_id)

        for db_id, parent_db_id in parent_links.items():
            obj = self.objects_by_db_id.get(db_id)
            parent = self.objects_by_db_id.get(parent_db_id)
            if obj is not None and parent is not None:
                obj.parent = parent

    def _to_native_payload(self) -> dict:
        return {
            "class_defs": [
                _class_def_payload(cdef)
                for cdef in self.class_defs.values()
            ],
            "objects": [
                _object_payload(obj)
                for obj in self.objects_by_db_id.values()
            ],
            "component_info": [
                {"db_id": db_id, "key": key}
                for db_id, key in self._component_info
            ],
        }

    # -- public lookups ----------------------------------------------------

    def lookup_by_path(self, mat_path: str) -> Optional[MaterialObject]:
        rid = BSResourceID.from_path(mat_path)
        return self.objects_by_persistent_id.get(rid)

    def lookup_by_resource_id(self, rid: BSResourceID) -> Optional[MaterialObject]:
        return self.objects_by_persistent_id.get(rid)

    def list_materials(self) -> list[str]:
        """Return a sorted list of known material paths.

        The path is only recorded when something set ``MaterialObject.path``
        (e.g. a test fixture, or a loader that resolves paths from a JSON
        reference block). Returns an empty list when no paths are attached.
        """
        return sorted(
            obj.path
            for obj in self.objects_by_persistent_id.values()
            if obj.path
        )

    def get_ce2_material(self, mat_path: str) -> Optional[CE2Material]:
        """Return a native-populated ``CE2Material`` view for the given path."""
        obj = self.lookup_by_path(mat_path)
        if obj is None:
            return None
        reason = _unsupported_native_projection_reason(self, obj)
        if reason is not None:
            raise NotImplementedError(
                f"unsupported native CDB projection for {mat_path}: {reason}"
            )
        mat = CE2Material(name=mat_path, material_object=obj)
        payload = native_runtime.project_ce2_material(self._to_native_payload(), obj.db_id)
        if payload is None:
            return mat
        _apply_native_ce2_payload(mat, payload)
        return mat


# ---------------------------------------------------------------------------
# Native projection shims
# ---------------------------------------------------------------------------

def walk_component(
    blob: ComponentBlob,
    class_defs: dict[str, ClassDef],
    stream: object | None,
    objects_by_db_id: dict[int, MaterialObject],
) -> Optional[dict]:
    """Walk a component through the native field interpreter."""
    del stream
    payload = native_runtime.walk_component(
        _component_blob_payload(blob),
        [_class_def_payload(cdef) for cdef in class_defs.values()],
        [_object_payload(obj) for obj in objects_by_db_id.values()],
    )
    return payload


def populate_ce2_material(mat: CE2Material, cdb: "MaterialsCDB") -> None:
    """Populate ``mat`` through the native CE2 projection pipeline."""
    if mat.material_object is None:
        return
    reason = _unsupported_native_projection_reason(cdb, mat.material_object)
    if reason is not None:
        raise NotImplementedError(
            f"unsupported native CDB projection for {mat.name}: {reason}"
        )
    payload = native_runtime.project_ce2_material(
        cdb._to_native_payload(),
        mat.material_object.db_id,
    )
    if payload is not None:
        _apply_native_ce2_payload(mat, payload)


_NATIVE_PROJECTABLE_COMPONENTS = frozenset({
    "BSMaterial::LayerID",
    "BSMaterial::TextureSetID",
    "BSMaterial::MaterialID",
    "BSMaterial::BlenderID",
    "BSMaterial::UVStreamID",
    "BSMaterial::LODMaterialID",
    "BSMaterial::MRTextureFile",
    "BSMaterial::Scale",
    "BSMaterial::AlphaSettingsComponent",
    "BSMaterial::EmissiveSettingsComponent",
    "BSMaterial::LayeredEmissivityComponent",
    "BSComponentDB::CTName",
})

_NATIVE_PROJECTABLE_PRIMITIVE_TYPES = frozenset({
    "string",
    "int8",
    "uint8",
    "int16",
    "uint16",
    "int32",
    "uint32",
    "int64",
    "uint64",
    "bool",
    "float",
    "double",
    "bscomponentdb2::id",
})


def _unsupported_native_projection_reason(
    cdb: "MaterialsCDB",
    root: MaterialObject,
) -> str | None:
    children_map: dict[int, list[MaterialObject]] = {}
    for obj in cdb.objects_by_db_id.values():
        if obj.parent is not None:
            children_map.setdefault(obj.parent.db_id, []).append(obj)

    def _walk(obj: MaterialObject) -> str | None:
        for blob in obj.components:
            reason = _unsupported_component_reason(blob, cdb.class_defs)
            if reason is not None:
                return reason
        for child in children_map.get(obj.db_id, []):
            reason = _walk(child)
            if reason is not None:
                return reason
        return None

    return _walk(root)


def _unsupported_component_reason(
    blob: ComponentBlob,
    class_defs: dict[str, ClassDef],
) -> str | None:
    if blob.class_name not in _NATIVE_PROJECTABLE_COMPONENTS:
        return None

    class_def = class_defs.get(blob.class_name)
    if class_def is None:
        return f"{blob.class_name} has no class definition"
    return _unsupported_class_def_reason(class_def, class_defs)


def _unsupported_class_def_reason(
    class_def: ClassDef,
    class_defs: dict[str, ClassDef],
    seen: set[str] | None = None,
) -> str | None:
    if class_def.class_flags & 4:
        return f"{class_def.class_name} is a user class"

    seen = set() if seen is None else seen
    if class_def.class_name in seen:
        return None
    seen.add(class_def.class_name)

    for field in class_def.fields:
        type_name = field.type_name
        type_name_lower = type_name.lower()
        if type_name_lower in _NATIVE_PROJECTABLE_PRIMITIVE_TYPES:
            continue
        if type_name_lower in {"list", "map", "ref"}:
            return f"{class_def.class_name}.{field.name} uses unsupported {type_name}"
        nested = class_defs.get(type_name)
        if nested is None:
            return f"{class_def.class_name}.{field.name} uses unknown type {type_name}"
        reason = _unsupported_class_def_reason(nested, class_defs, seen)
        if reason is not None:
            return reason
    return None


def _class_def_payload(cdef: ClassDef) -> dict:
    return {
        "class_name": cdef.class_name,
        "class_name_index": cdef.class_name_index,
        "class_version": cdef.class_version,
        "class_flags": cdef.class_flags,
        "field_count": cdef.field_count,
        "fields": [
            {
                "name_index": field.name_index,
                "type_index": field.type_index,
                "data_offset": field.data_offset,
                "data_size": field.data_size,
            }
            for field in cdef.fields
        ],
    }


def _component_blob_payload(blob: ComponentBlob) -> dict:
    return {
        "class_name": blob.class_name,
        "is_diff": blob.is_diff,
        "key": blob.key,
        "body": list(blob.body),
    }


def _object_payload(obj: MaterialObject) -> dict:
    return {
        "persistent_id": {
            "dir": obj.persistent_id.dir,
            "file": obj.persistent_id.file,
            "ext": obj.persistent_id.ext,
        },
        "db_id": obj.db_id,
        "base_object_db_id": obj.base_object_db_id,
        "has_data": obj.has_data,
        "parent_db_id": obj.parent.db_id if obj.parent is not None else None,
        "components": [
            _component_blob_payload(blob)
            for blob in obj.components
        ],
    }

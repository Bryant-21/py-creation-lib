"""
In-memory representation of a NIF file.

NifFile holds a header and a list of NifBlocks. Each block stores its
fields as an ordered list for serialization fidelity and a dict for
fast lookup.
"""
from __future__ import annotations
import copy
import re
from dataclasses import dataclass, field as dc_field
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING:
    from .schema import NifSchema

from .types import to_json


# Bethesda asset roots that mark the start of a Data-relative path. Anything
# preceding (e.g. "D:\\Projects\\SomeMod\\data\\") gets stripped on save.
_DATA_RELATIVE_ROOTS = re.compile(
    r"(?i).*?(?P<rel>(?:textures|materials|meshes|sound|music|video|programs|interface|scripts|strings|lodsettings|grass|terrain)[/\\].*)"
)
_ABSOLUTE_PATH = re.compile(r"^(?:[A-Za-z]:[/\\]|[/\\])")
_FIELD_PATH_TOKEN = re.compile(r"(\[(\d+)\])|\.([A-Za-z_][A-Za-z0-9_ ]*)")


def _split_field_path(name: str) -> tuple[str, tuple[int | str, ...]] | None:
    first_sep = min(
        (idx for idx in (name.find("["), name.find(".")) if idx >= 0),
        default=-1,
    )
    if first_sep <= 0:
        return None

    base_name = name[:first_sep]
    tokens: list[int | str] = []
    pos = first_sep
    while pos < len(name):
        match = _FIELD_PATH_TOKEN.match(name, pos)
        if match is None:
            return None
        index_token = match.group(2)
        field_token = match.group(3)
        tokens.append(int(index_token) if index_token is not None else field_token)
        pos = match.end()
    return (base_name, tuple(tokens)) if tokens else None


def _get_nested_field_value(value: Any, path: tuple[int | str, ...]) -> Any:
    current = value
    for token in path:
        try:
            if isinstance(token, int):
                if not isinstance(current, list):
                    return None
                current = current[token]
            else:
                if not isinstance(current, dict):
                    return None
                current = current[token]
        except (IndexError, KeyError):
            return None
    return current


def _set_nested_field_value(value: Any, path: tuple[int | str, ...], new_value: Any) -> bool:
    current = value
    for token in path[:-1]:
        if isinstance(token, int):
            if not isinstance(current, list) or token < 0 or token >= len(current):
                return False
            current = current[token]
        else:
            if not isinstance(current, dict) or token not in current:
                return False
            current = current[token]

    final = path[-1]
    if isinstance(final, int):
        if not isinstance(current, list) or final < 0 or final >= len(current):
            return False
        current[final] = copy.deepcopy(new_value)
        return True
    if not isinstance(current, dict):
        return False
    current[final] = copy.deepcopy(new_value)
    return True


def _normalize_texture_path(path: str) -> str:
    """Strip an absolute prefix off a Bethesda asset path, leaving it relative
    to the Data directory. Already-relative paths are returned unchanged."""
    if not path or not _ABSOLUTE_PATH.match(path):
        return path
    candidate = path.replace("/", "\\")
    m = _DATA_RELATIVE_ROOTS.match(candidate)
    if m:
        return m.group("rel")
    return candidate


def _normalize_texture_paths(nif: NifFile) -> None:
    """Walk every BSShaderTextureSet block and rewrite absolute texture paths
    to Data-relative form. Bethesda games and NifSkope both expect relative
    paths; absolute paths fail to resolve textures."""
    for block in nif.blocks:
        if block.type_name != "BSShaderTextureSet":
            continue
        textures = block.get_field("Textures")
        if not isinstance(textures, list):
            continue
        normalized = [_normalize_texture_path(str(t or "")) for t in textures]
        if normalized != textures:
            block.set_field("Textures", normalized)


@dataclass
class NifBlock:
    """A single block (node) in the NIF file."""
    block_id: int
    type_name: str
    fields: list[tuple[str, Any]] = dc_field(default_factory=list)
    _field_map: dict[str, Any] = dc_field(default_factory=dict, repr=False)
    _remainder: bytes = dc_field(default=b"", repr=False)

    def __post_init__(self):
        if self.fields and not self._field_map:
            self._field_map = {name: val for name, val in self.fields}

    def get_field(self, name: str) -> Any:
        """Get field value by name. Tries exact match first, then
        checks for disambiguated names (name:suffix)."""
        if name in self._field_map:
            return self._field_map[name]
        # Try matching without suffix
        for key, val in self.fields:
            bare = key.split(":")[0] if ":" in key else key
            if bare == name:
                return val
        nested = _split_field_path(name)
        if nested is not None:
            base_name, path = nested
            base_value = self.get_field(base_name)
            if base_value is not None:
                return _get_nested_field_value(base_value, path)
        return None

    def set_field(self, name: str, value: Any) -> None:
        """Set field value. Updates both ordered list and lookup map."""
        # Try exact match first
        if name in self._field_map:
            self._field_map[name] = value
            for i, (n, _) in enumerate(self.fields):
                if n == name:
                    self.fields[i] = (name, value)
                    return
        # Try bare name match
        for i, (n, _) in enumerate(self.fields):
            bare = n.split(":")[0] if ":" in n else n
            if bare == name:
                self._field_map[n] = value
                self.fields[i] = (n, value)
                return
        nested = _split_field_path(name)
        if nested is not None:
            base_name, path = nested
            for i, (n, val) in enumerate(self.fields):
                bare = n.split(":")[0] if ":" in n else n
                if n == base_name or bare == base_name:
                    updated = copy.deepcopy(val)
                    if _set_nested_field_value(updated, path, value):
                        self._field_map[n] = updated
                        self.fields[i] = (n, updated)
                        return
        # New field — append
        self.fields.append((name, value))
        self._field_map[name] = value

    def get_refs(self, schema: NifSchema) -> list[int]:
        """Return all Ref/Ptr block indices referenced by this block.
        Recursively walks into struct fields (dicts) and arrays of structs."""
        ref_types = {"Ref", "Ptr"}
        refs = []
        all_fields = schema.get_all_fields(self.type_name)
        field_defs = {f.name: f for f in all_fields}
        for f in all_fields:
            if f.suffix:
                field_defs[f"{f.name}:{f.suffix}"] = f

        def _collect_refs_from_value(val: Any, fdef: Any) -> None:
            """Collect Ref/Ptr values from a field value, recursing into structs."""
            if fdef.type in ref_types or fdef.template in ref_types:
                if isinstance(val, int) and val >= 0:
                    refs.append(val)
                elif isinstance(val, list):
                    refs.extend(v for v in val if isinstance(v, int) and v >= 0)
            elif isinstance(val, dict):
                # Struct — walk its fields for Refs
                _collect_refs_from_struct(val, fdef.type)
            elif isinstance(val, list) and val and isinstance(val[0], dict):
                # Array of structs
                for item in val:
                    _collect_refs_from_struct(item, fdef.type)

        def _collect_refs_from_struct(struct_val: dict, struct_type: str) -> None:
            """Walk a struct's fields looking for Ref/Ptr values."""
            struct_def = schema.structs.get(struct_type)
            if struct_def is None:
                return
            struct_fdefs = {f.name: f for f in struct_def.fields}
            for key, val in struct_val.items():
                sfdef = struct_fdefs.get(key)
                if sfdef is None:
                    continue
                _collect_refs_from_value(val, sfdef)

        for name, val in self.fields:
            fdef = field_defs.get(name)
            if fdef is None:
                continue
            _collect_refs_from_value(val, fdef)
        return refs

    def get_all_ref_fields(self, schema: NifSchema) -> list[tuple[str, list[int]]]:
        """Return [(field_name, [block_ids])] for all Ref/Ptr fields with valid refs.

        Like get_refs() but grouped by field name, useful for building tree UIs
        that show which field links to which child blocks.
        """
        ref_types = {"Ref", "Ptr"}
        result = []
        all_fields = schema.get_all_fields(self.type_name)
        fdef_map = {}
        for f in all_fields:
            fdef_map[f.name] = f
            if f.suffix:
                fdef_map[f"{f.name}:{f.suffix}"] = f

        for name, val in self.fields:
            fdef = fdef_map.get(name)
            if fdef is None:
                continue
            if fdef.type in ref_types or fdef.template in ref_types:
                refs = []
                if isinstance(val, int) and val >= 0:
                    refs.append(val)
                elif isinstance(val, list):
                    refs.extend(v for v in val if isinstance(v, int) and v >= 0)
                if refs:
                    result.append((name, refs))
        return result

    def to_json(self) -> dict:
        """JSON-serializable representation. Strips disambiguation suffixes."""
        result = {}
        for name, val in self.fields:
            display_name = name.split(":")[0] if ":" in name else name
            result[display_name] = to_json(val)
        return result


@dataclass
class NifHeader:
    """NIF file header metadata."""
    header_string: str = ""
    version: tuple = (20, 2, 0, 7)     # FO4 default
    version_packed: int = 0x14020007
    user_version: int = 12
    bs_version: int = 130               # FO4
    endian_type: int = 1                # little-endian
    creator: str = ""
    export_info: list[str] = dc_field(default_factory=list)
    sf_export_data: bytes = b""
    num_blocks: int = 0
    block_type_names: list[str] = dc_field(default_factory=list)
    block_type_index: list[int] = dc_field(default_factory=list)
    block_sizes: list[int] = dc_field(default_factory=list)
    strings: list[str] = dc_field(default_factory=list)
    max_string_length: int = 0
    num_groups: int = 0
    groups: list[int] = dc_field(default_factory=list)


class NifFile:
    """Complete NIF file representation."""

    def __init__(self):
        self.header = NifHeader()
        self.blocks: list[NifBlock] = []
        self._schema: NifSchema | None = None
        self._filepath: str = ""
        self._footer_roots: list[int] = []  # root block indices from footer
        self.detected_game = None  # GameProfile | None — set by load()

    @property
    def schema(self) -> NifSchema:
        if self._schema is None:
            from .schema import get_schema
            self._schema = get_schema()
        return self._schema

    @classmethod
    def load(cls, filepath: str) -> NifFile:
        """Read NIF from disk through the native Rust reader."""
        return cls._load_native(filepath)

    @classmethod
    def _load_native(cls, filepath: str) -> NifFile:
        from . import native_runtime
        raw = native_runtime.load_nif_raw(filepath)
        nif = cls._from_native(raw)
        nif._filepath = filepath
        try:
            from creation_lib.core.game_profiles import detect_game
            nif.detected_game = detect_game(nif.header.bs_version)
        except ImportError:
            pass
        return nif

    @classmethod
    def _from_native(cls, native_nif) -> NifFile:
        """Convert a native nif_core_native dict payload to a Python NifFile."""
        nif = cls()
        if not isinstance(native_nif, dict):
            raise TypeError("native NIF payload must be a dict")
        nh = native_nif.get("header", {})
        h = nif.header
        h.header_string = nh.get("header_string", h.header_string)
        h.version = tuple(nh.get("version", h.version))
        h.version_packed = nh.get("version_packed", _pack_version(h.version))
        h.user_version = nh.get("user_version", h.user_version)
        h.bs_version = nh.get("bs_version", h.bs_version)
        h.endian_type = nh.get("endian_type", h.endian_type)
        h.creator = nh.get("creator", h.creator)
        h.export_info = list(nh.get("export_info", h.export_info))
        h.sf_export_data = bytes(nh.get("sf_export_data", h.sf_export_data))
        h.num_blocks = nh.get("num_blocks", h.num_blocks)
        h.block_type_names = list(nh.get("block_type_names", h.block_type_names))
        h.block_type_index = list(nh.get("block_type_index", h.block_type_index))
        h.block_sizes = list(nh.get("block_sizes", h.block_sizes))
        h.strings = list(nh.get("strings", h.strings))
        h.max_string_length = nh.get("max_string_length", h.max_string_length)
        h.num_groups = nh.get("num_groups", h.num_groups)
        h.groups = list(nh.get("groups", h.groups))
        nif._footer_roots = list(nh.get("footer_roots", []))
        for nb in native_nif.get("blocks", []):
            fields_list = list(nb.get("fields", {}).items())
            block = NifBlock(
                block_id=nb["block_id"],
                type_name=nb["type_name"],
                fields=fields_list,
                _remainder=nb.get("remainder", b""),
            )
            nif.blocks.append(block)
        try:
            from creation_lib.core.game_profiles import detect_game
            nif.detected_game = detect_game(nif.header.bs_version)
        except ImportError:
            pass
        return nif

    def _to_native(self) -> dict[str, Any]:
        """Convert this Python NifFile facade into the Rust dict payload."""
        version = tuple(self.header.version or (20, 2, 0, 7))
        version_packed = self.header.version_packed or _pack_version(version)
        header_string = (
            self.header.header_string
            or f"Gamebryo File Format, Version {version[0]}.{version[1]}.{version[2]}.{version[3]}"
        )
        return {
            "header": {
                "header_string": header_string,
                "version": version,
                "version_packed": version_packed,
                "endian_type": self.header.endian_type,
                "user_version": self.header.user_version,
                "bs_version": self.header.bs_version,
                "num_blocks": len(self.blocks),
                "creator": self.header.creator,
                "export_info": list(self.header.export_info),
                "sf_export_data": bytes(self.header.sf_export_data),
                "block_type_names": list(self.header.block_type_names),
                "block_type_index": [int(value) for value in self.header.block_type_index],
                "block_sizes": [int(value) for value in self.header.block_sizes],
                "strings": list(self.header.strings),
                "max_string_length": int(self.header.max_string_length),
                "num_groups": int(self.header.num_groups),
                "groups": [int(value) for value in self.header.groups],
                "footer_roots": [int(value) for value in self._footer_roots],
            },
            "blocks": [
                {
                    "block_id": int(block.block_id),
                    "type_name": block.type_name,
                    "fields": _native_block_fields(block, self.schema),
                    "remainder": bytes(block._remainder),
                }
                for block in self.blocks
            ],
            "path": self._filepath,
        }

    @classmethod
    def load_header(cls, filepath: str) -> NifFile:
        """Read a NIF header through the native Rust reader."""
        nif = cls._load_native(filepath)
        nif.blocks = []
        return nif

    # Map legacy game names to profile IDs
    _GAME_ALIASES = {"FO4": "fo4", "fo4": "fo4", "skyrimse": "skyrimse",
                      "fo76": "fo76", "starfield": "starfield"}

    @classmethod
    def new(cls, game: str = "FO4") -> NifFile:
        """Create a new empty NIF with correct header for the given game.
        Initializes with a BSFadeNode root."""
        from . import native_runtime
        game_id = cls._GAME_ALIASES.get(game, game.lower())
        return cls._from_native(native_runtime.new_nif_raw(game_id))

    def save(self, filepath: str = "") -> None:
        """Write NIF to disk through the native Rust writer."""
        from . import native_runtime
        path = filepath or self._filepath
        if not path:
            raise ValueError("No filepath specified")
        if not self._footer_roots and self.blocks:
            self._footer_roots = [0]
        _normalize_texture_paths(self)
        native_runtime.save_nif_raw(self._to_native(), path)
        self._filepath = path

    def get_block(self, block_id: int) -> NifBlock | None:
        if 0 <= block_id < len(self.blocks):
            return self.blocks[block_id]
        return None

    def find_blocks(self, type_name: str) -> list[NifBlock]:
        """Find all blocks of a given type (including subtypes)."""
        results = []
        for block in self.blocks:
            if self.schema.is_subtype_of(block.type_name, type_name):
                results.append(block)
        return results

    def get_children(self, block_id: int) -> list[NifBlock]:
        """Get direct children (blocks referenced via Ref fields)."""
        block = self.get_block(block_id)
        if block is None:
            return []
        refs = block.get_refs(self.schema)
        return [self.blocks[r] for r in refs if 0 <= r < len(self.blocks)]

    def get_hierarchy(self) -> dict:
        """Build scene graph tree from root nodes (blocks referenced in footer)."""
        visited = set()

        def _build(bid: int) -> dict | None:
            if bid in visited or bid < 0 or bid >= len(self.blocks):
                return None
            visited.add(bid)
            block = self.blocks[bid]
            node = {
                "id": bid,
                "type": block.type_name,
                "name": block.get_field("Name") or "",
            }
            children = []
            refs = block.get_refs(self.schema)
            for ref in refs:
                child = _build(ref)
                if child:
                    children.append(child)
            if children:
                node["children"] = children
            return node

        root_ids = [
            root
            for root in (self._footer_roots or ([0] if self.blocks else []))
            if 0 <= root < len(self.blocks)
        ]
        if not root_ids and self.blocks:
            root_ids = [0]

        roots = []
        for root_id in root_ids:
            tree = _build(root_id)
            if tree:
                roots.append(tree)

        if not roots and self.blocks:
            referenced = set()
            for block in self.blocks:
                referenced.update(block.get_refs(self.schema))
            for block in self.blocks:
                if block.block_id not in referenced:
                    tree = _build(block.block_id)
                    if tree:
                        roots.append(tree)
        if not roots and self.blocks:
            tree = _build(0)
            if tree:
                roots.append(tree)
        return {"roots": roots}

    def add_block(self, type_name: str, fields: dict | None = None) -> NifBlock:
        """Create a new block with optional initial fields."""
        bid = len(self.blocks)
        block = NifBlock(block_id=bid, type_name=type_name)

        # Set default field values from schema
        all_fields = self.schema.get_all_fields(type_name)
        for fdef in all_fields:
            if fdef.is_abstract:
                continue
            default = _get_default_value(fdef, self.schema)
            block.set_field(fdef.name, default)

        # Override with caller-provided fields
        if fields:
            for name, val in fields.items():
                block.set_field(name, val)

        self.blocks.append(block)
        self._update_header_for_new_block(type_name)
        return block

    def remove_blocks(self, block_ids: list[int]) -> None:
        """Remove blocks and remap all Ref/Ptr indices."""
        if not block_ids:
            return
        remove_set = set(block_ids)
        # Build ID mapping: old_id → new_id (-1 for removed)
        id_map = {}
        new_id = 0
        for old_id in range(len(self.blocks)):
            if old_id in remove_set:
                id_map[old_id] = -1
            else:
                id_map[old_id] = new_id
                new_id += 1

        # Remove blocks and reassign IDs
        new_blocks = []
        for block in self.blocks:
            if block.block_id not in remove_set:
                block.block_id = id_map[block.block_id]
                new_blocks.append(block)
        self.blocks = new_blocks
        self.remap_refs(id_map)
        self._rebuild_header()

    def remap_refs(self, id_map: dict[int, int]) -> None:
        """Update all Ref/Ptr values according to the mapping.

        Recurses into struct fields and struct arrays (e.g.
        ``NiDefaultAVObjectPalette.Objs[].AV Object`` or
        ``NiControllerSequence.Controlled Blocks[].Interpolator``).
        Refs not present in ``id_map`` are rewritten to -1.
        """
        for block in self.blocks:
            remap_block_refs(block, id_map, self.schema, missing_default=-1)

    def add_string(self, s: str) -> int:
        """Add string to header string table. Returns index."""
        if s in self.header.strings:
            return self.header.strings.index(s)
        idx = len(self.header.strings)
        self.header.strings.append(s)
        if len(s) > self.header.max_string_length:
            self.header.max_string_length = len(s)
        return idx

    def _update_header_for_new_block(self, type_name: str) -> None:
        """Update header metadata after adding a block."""
        if type_name not in self.header.block_type_names:
            self.header.block_type_names.append(type_name)
        type_idx = self.header.block_type_names.index(type_name)
        self.header.block_type_index.append(type_idx)
        self.header.block_sizes.append(0)
        self.header.num_blocks = len(self.blocks)

    def _rebuild_header(self) -> None:
        """Rebuild header metadata from current block list."""
        type_names = []
        type_index = []
        for block in self.blocks:
            if block.type_name not in type_names:
                type_names.append(block.type_name)
            type_index.append(type_names.index(block.type_name))
        self.header.block_type_names = type_names
        self.header.block_type_index = type_index
        self.header.block_sizes = [0] * len(self.blocks)
        self.header.num_blocks = len(self.blocks)


def _pack_version(version: tuple) -> int:
    parts = tuple(int(part) for part in version[:4])
    if len(parts) != 4:
        parts = (20, 2, 0, 7)
    return (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]


def _native_block_fields(block: NifBlock, schema: NifSchema) -> dict[str, Any]:
    fdefs = {}
    for fdef in schema.get_all_fields(block.type_name):
        fdefs[fdef.name] = fdef
        if getattr(fdef, "suffix", None):
            fdefs[f"{fdef.name}:{fdef.suffix}"] = fdef
    return {
        name: _native_value_for_field(value, fdefs.get(name), schema)
        for name, value in block.fields
    }


def _native_value_for_field(value: Any, fdef: Any | None, schema: NifSchema) -> Any:
    if fdef is None:
        return _native_untyped_value(value)
    field_type = _field_type(fdef)
    field_template = getattr(fdef, "template", None)
    if getattr(fdef, "length", None):
        if getattr(fdef, "width", None):
            return [
                [_native_value_for_type(item, field_type, field_template, schema) for item in row]
                for row in (value or [])
            ]
        return [
            _native_value_for_type(item, field_type, field_template, schema)
            for item in (value or [])
        ]
    return _native_value_for_type(value, field_type, field_template, schema)


def _native_value_for_type(
    value: Any,
    type_name: str,
    template: str | None,
    schema: NifSchema,
) -> Any:
    if type_name == "#T#" and template:
        type_name = template
    if _native_scalar_type(type_name, schema):
        return _native_untyped_value(value)
    if type_name in schema.structs:
        return _native_struct_value(value, type_name, template, schema)
    return _native_untyped_value(value)


def _native_struct_value(
    value: Any,
    type_name: str,
    template: str | None,
    schema: NifSchema,
) -> dict[str, Any]:
    value = _sequence_struct_value(value, type_name)
    if not isinstance(value, dict):
        value = {}
    struct_def = schema.structs.get(type_name)
    if struct_def is None:
        return {str(key): _native_untyped_value(val) for key, val in value.items()}

    result: dict[str, Any] = {}
    for sf in struct_def.fields:
        key = f"{sf.name}:{sf.suffix}" if getattr(sf, "suffix", None) else sf.name
        if key in value:
            raw = value[key]
        elif sf.name in value:
            raw = value[sf.name]
        else:
            continue
        field_type = _field_type(sf)
        field_template = getattr(sf, "template", None)
        if field_type == "#T#" and template:
            field_type = template
        if field_template == "#T#" and template:
            field_template = template
        result[key] = _native_value_for_field(
            raw,
            _FieldView(sf, field_type, field_template),
            schema,
        )
    return result


def _sequence_struct_value(value: Any, type_name: str) -> Any:
    if not isinstance(value, (list, tuple)):
        return value
    if type_name in {"Vector3", "HalfVector3", "ByteVector3"} and len(value) >= 3:
        return {"x": value[0], "y": value[1], "z": value[2]}
    if type_name in {"Vector4", "hkVector4"} and len(value) >= 4:
        return {"x": value[0], "y": value[1], "z": value[2], "w": value[3]}
    if type_name in {"Color3", "ByteColor3"} and len(value) >= 3:
        return {"r": value[0], "g": value[1], "b": value[2]}
    if type_name in {"Color4", "ByteColor4"} and len(value) >= 4:
        return {"r": value[0], "g": value[1], "b": value[2], "a": value[3]}
    if type_name == "ByteColor4BGRA" and len(value) >= 4:
        return {"b": value[0], "g": value[1], "r": value[2], "a": value[3]}
    if type_name in {"TexCoord", "HalfTexCoord"} and len(value) >= 2:
        return {"u": value[0], "v": value[1]}
    if type_name == "Triangle" and len(value) >= 3:
        return {"v1": value[0], "v2": value[1], "v3": value[2]}
    if type_name in {"Quaternion", "hkQuaternion"} and len(value) >= 4:
        return {"w": value[0], "x": value[1], "y": value[2], "z": value[3]}
    if type_name == "Matrix33" and len(value) >= 3:
        return {
            "m11": value[0][0], "m12": value[0][1], "m13": value[0][2],
            "m21": value[1][0], "m22": value[1][1], "m23": value[1][2],
            "m31": value[2][0], "m32": value[2][1], "m33": value[2][2],
        }
    return value


class _FieldView:
    def __init__(self, source: Any, field_type: str, field_template: str | None):
        self.name = source.name
        self.type = field_type
        self.template = field_template
        self.length = getattr(source, "length", None)
        self.width = getattr(source, "width", None)
        self.suffix = getattr(source, "suffix", None)


def _field_type(fdef: Any) -> str:
    return str(getattr(fdef, "type", getattr(fdef, "type_name", "")) or "")


def _native_scalar_type(type_name: str, schema: NifSchema) -> bool:
    return (
        type_name in {"string", "bool", "NiFixedString", "SizedString", "SizedString16"}
        or type_name in schema.basics
        or type_name in schema.enums
        or type_name in schema.bitflags
        or type_name in schema.bitfields
    )


def _native_untyped_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {str(key): _native_untyped_value(val) for key, val in value.items()}
    if isinstance(value, list):
        return [_native_untyped_value(item) for item in value]
    if isinstance(value, tuple):
        return [_native_untyped_value(item) for item in value]
    return value


_STRUCT_DEFAULTS = {
    "Color3": {"r": 0.0, "g": 0.0, "b": 0.0},
    "ByteColor3": {"r": 0, "g": 0, "b": 0},
    "Color4": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
    "ByteColor4": {"r": 0, "g": 0, "b": 0, "a": 255},
    "ByteColor4BGRA": {"b": 0, "g": 0, "r": 0, "a": 255},
    "Vector3": {"x": 0.0, "y": 0.0, "z": 0.0},
    "Vector4": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 0.0},
    "Quaternion": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
    "TexCoord": {"u": 0.0, "v": 0.0},
    "Matrix33": {
        "m11": 1.0, "m12": 0.0, "m13": 0.0,
        "m21": 0.0, "m22": 1.0, "m23": 0.0,
        "m31": 0.0, "m32": 0.0, "m33": 1.0,
    },
}

# Basic integer types
_INT_TYPES = frozenset({
    "uint", "int", "ushort", "short", "byte", "sbyte",
    "int64", "uint64", "ulittle32", "BlockTypeIndex",
    "FileVersion", "StringOffset",
})

# Basic float types
_FLOAT_TYPES = frozenset({"float", "hfloat", "normbyte"})

# String types
_STRING_TYPES = frozenset({
    "string", "SizedString", "SizedString16",
    "HeaderString", "LineString", "char", "NiFixedString",
})


def _get_default_value(fdef, schema=None) -> Any:
    """Get a reasonable default value for a field definition.

    If *schema* is provided, struct types not in _STRUCT_DEFAULTS will be
    recursively built from the schema's compound definitions.  This prevents
    binary-layout corruption when injecting version-conditional fields whose
    types are compound (e.g. TexCoord, BSSPWetnessParams).
    """
    if fdef.length:
        return []  # arrays default to empty
    if fdef.type in _STRUCT_DEFAULTS:
        return copy.deepcopy(_STRUCT_DEFAULTS[fdef.type])
    if fdef.type in _INT_TYPES:
        return 0
    if fdef.type in _FLOAT_TYPES:
        return 0.0
    if fdef.type in ("bool",):
        return 0
    if fdef.type in ("Ref", "Ptr"):
        return -1
    if fdef.type in _STRING_TYPES:
        return ""

    # Schema-aware fallback for compound/struct types
    if schema is not None:
        struct_def = schema.structs.get(fdef.type)
        if struct_def is not None:
            result = {}
            for sf in struct_def.fields:
                if sf.is_abstract or sf.length:
                    continue
                result[sf.name] = _get_default_value(sf, schema)
            return result
        # Enum / bitflags / bitfield → integer 0
        if (fdef.type in schema.enums or fdef.type in schema.bitflags
                or fdef.type in schema.bitfields):
            return 0

    return 0


_MISSING = object()


def remap_block_refs(block, id_map: dict, schema, missing_default=_MISSING) -> None:
    """Remap Ref/Ptr fields in a block using id_map. Recursively walks struct fields.

    If ``missing_default`` is provided, integer refs not present in ``id_map``
    are rewritten to that value (typically -1). Otherwise missing refs are
    left unchanged.
    """
    all_fields = schema.get_all_fields(block.type_name)
    fdef_map = {}
    for f in all_fields:
        key = f"{f.name}:{f.suffix}" if f.suffix else f.name
        fdef_map[key] = f
        fdef_map[f.name] = f

    for name, value in list(block.fields):
        fdef = fdef_map.get(name)
        if fdef is None:
            continue
        new_value = _remap_value(value, fdef, id_map, schema, missing_default)
        if new_value is not value:
            block.set_field(name, new_value)


def _lookup_ref(v, id_map, missing_default):
    if not isinstance(v, int):
        return v
    if v in id_map:
        return id_map[v]
    if missing_default is _MISSING:
        return v
    # Preserve -1 sentinel as-is even when missing_default is provided,
    # since existing code relies on "-1 → -1" passthrough.
    if v < 0:
        return v
    return missing_default


def _remap_value(value, fdef, id_map: dict, schema, missing_default=_MISSING):
    """Recursively remap Ref/Ptr values in a field value."""
    if fdef.type in ("Ref", "Ptr"):
        if isinstance(value, int):
            new_v = _lookup_ref(value, id_map, missing_default)
            return new_v if new_v != value else value
        elif isinstance(value, list):
            new_list = [_lookup_ref(v, id_map, missing_default) for v in value]
            return new_list if new_list != value else value
    elif isinstance(value, dict):
        # Struct — recurse into fields
        struct_def = schema.structs.get(fdef.type)
        if struct_def is None:
            return value
        changed = False
        new_dict = dict(value)
        struct_fdefs = {f.name: f for f in struct_def.fields}
        for key, val in value.items():
            sfdef = struct_fdefs.get(key)
            if sfdef is None:
                continue
            new_val = _remap_value(val, sfdef, id_map, schema, missing_default)
            if new_val is not val:
                new_dict[key] = new_val
                changed = True
        return new_dict if changed else value
    elif isinstance(value, list):
        if not value:
            return value
        if isinstance(value[0], dict):
            # Array of structs
            new_list = []
            changed = False
            struct_def = schema.structs.get(fdef.type)
            if struct_def:
                struct_fdefs = {f.name: f for f in struct_def.fields}
                for item in value:
                    new_item = dict(item)
                    item_changed = False
                    for key, val in item.items():
                        sfdef = struct_fdefs.get(key)
                        if sfdef is None:
                            continue
                        new_val = _remap_value(val, sfdef, id_map, schema, missing_default)
                        if new_val is not val:
                            new_item[key] = new_val
                            item_changed = True
                    new_list.append(new_item if item_changed else item)
                    changed = changed or item_changed
                return new_list if changed else value
        elif isinstance(value[0], int):
            # Array of refs (e.g., Children)
            if fdef.template in ("Ref", "Ptr"):
                new_list = [_lookup_ref(v, id_map, missing_default) for v in value]
                return new_list if new_list != value else value
    return value

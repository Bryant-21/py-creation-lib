"""
Parse nif.xml into an in-memory type registry (NifSchema).

The XML schema defines: basic types, enums, bitflags, bitfields,
compound structs, and niobject block types with full field specifications,
version conditions, and inheritance.
"""
from __future__ import annotations
import os
import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from typing import Any

import logging
from .expr import NifExpr

logger = logging.getLogger(__name__)


# --- Data structures ---

@dataclass
class BasicType:
    name: str
    size: int            # bytes (0 for variable-size)
    integral: bool
    countable: bool
    generic: bool        # takes template parameter (e.g., Ref<T>)


@dataclass
class EnumOption:
    name: str
    value: int


@dataclass
class EnumType:
    name: str
    storage: str         # underlying basic type
    options: list[EnumOption]


@dataclass
class BitflagType:
    name: str
    storage: str
    options: list[EnumOption]  # .value is bit position


@dataclass
class BitfieldMember:
    name: str
    width: int
    pos: int
    mask: int
    type: str            # enum type name or basic type


@dataclass
class BitfieldType:
    name: str
    storage: str
    members: list[BitfieldMember]


@dataclass
class FieldDef:
    name: str
    type: str
    template: str | None = None
    default: str | None = None
    conditional_defaults: list[tuple[str, str]] | None = None
    length: str | None = None
    width: str | None = None
    cond: NifExpr | None = None
    vercond: NifExpr | None = None
    since: tuple | None = None
    until: tuple | None = None
    arg: str | None = None
    is_binary: bool = False
    is_abstract: bool = False
    calc: NifExpr | None = None
    suffix: str | None = None
    only_t: str | None = None
    exclude_t: str | None = None
    recursive: bool = False


@dataclass
class StructType:
    name: str
    fields: list[FieldDef]
    generic: bool = False


@dataclass
class NiObjectType:
    name: str
    inherit: str | None
    fields: list[FieldDef]   # own fields only (not inherited)
    abstract: bool = False
    versions: list[str] = field(default_factory=list)
    generic: bool = False
    description: str = ""


class NifSchema:
    """Parsed nif.xml registry."""

    def __init__(self):
        self.basics: dict[str, BasicType] = {}
        self.enums: dict[str, EnumType] = {}
        self.bitflags: dict[str, BitflagType] = {}
        self.bitfields: dict[str, BitfieldType] = {}
        self.structs: dict[str, StructType] = {}
        self.niobjects: dict[str, NiObjectType] = {}
        self.versions: dict[str, tuple] = {}
        self._tokens: dict[str, str] = {}  # token name → expansion
        # Schema contents are immutable after parse(), so inheritance-derived
        # lookups can be memoized safely. These sit directly on the hot NIF
        # parse path.
        self._all_fields_cache: dict[str, tuple[FieldDef, ...]] = {}
        self._type_hierarchy_cache: dict[str, tuple[str, ...]] = {}
        self._is_subtype_cache: dict[tuple[str, str], bool] = {}

    def get_all_fields(self, type_name: str) -> tuple[FieldDef, ...]:
        """Return all fields including inherited, in read order (parent fields first)."""
        cached = self._all_fields_cache.get(type_name)
        if cached is not None:
            return cached

        obj = self.niobjects.get(type_name) or self.structs.get(type_name)
        if obj is None:
            result: tuple[FieldDef, ...] = ()
        elif isinstance(obj, NiObjectType) and obj.inherit:
            result = self.get_all_fields(obj.inherit) + tuple(obj.fields)
        else:
            result = tuple(obj.fields)

        self._all_fields_cache[type_name] = result
        return result

    def get_type_hierarchy(self, type_name: str) -> tuple[str, ...]:
        """Return (type_name, parent, grandparent, ...) up to root."""
        cached = self._type_hierarchy_cache.get(type_name)
        if cached is not None:
            return cached

        chain = []
        current = type_name
        seen = set()
        while current and current not in seen:
            chain.append(current)
            seen.add(current)
            obj = self.niobjects.get(current)
            if obj is None:
                break
            current = obj.inherit
        result = tuple(chain)
        self._type_hierarchy_cache[type_name] = result
        return result

    def is_subtype_of(self, type_name: str, base_name: str) -> bool:
        """Check if type_name is base_name or inherits from it."""
        cache_key = (type_name, base_name)
        cached = self._is_subtype_cache.get(cache_key)
        if cached is not None:
            return cached

        result = base_name in self.get_type_hierarchy(type_name)
        self._is_subtype_cache[cache_key] = result
        return result

    @classmethod
    def parse(cls, xml_path: str) -> NifSchema:
        """Parse nif.xml file into a NifSchema."""
        schema = cls()
        tree = ET.parse(xml_path)
        root = tree.getroot()

        # Pass 1: Collect tokens (macros for expression expansion)
        schema._parse_tokens(root)

        # Pass 2: Parse versions
        schema._parse_versions(root)

        # Pass 3: Parse all type definitions
        for elem in root:
            tag = elem.tag
            if tag == "basic":
                schema._parse_basic(elem)
            elif tag == "enum":
                schema._parse_enum(elem)
            elif tag == "bitflags":
                schema._parse_bitflags(elem)
            elif tag == "bitfield":
                schema._parse_bitfield(elem)
            elif tag in ("compound", "struct"):
                schema._parse_compound(elem)
            elif tag == "niobject":
                schema._parse_niobject(elem)

        return schema

    # --- Token parsing ---

    def _parse_tokens(self, root: ET.Element) -> None:
        r"""Parse <token> elements into expansion dictionary.

        Actual nif.xml structure:
          <token name="verexpr" attrs="vercond">
              <verexpr token="#BS_GTE_FO4#" string="(#BSVER# >= 130)" />
          </token>
          <token name="global" attrs="cond vercond access">
              <global token="#BSVER#" string="BS Header\BS Version" />
          </token>
          <token name="operator" attrs="cond vercond length width arg calc">
              <operator token="#AND#" string="&amp;&amp;" />
          </token>

        Each child element has a `token` attr (macro name with # delimiters)
        and a `string` attr (expansion text).
        """
        for tok_group in root.findall("token"):
            for child in tok_group:
                token_name = child.attrib.get("token", "")
                token_string = child.attrib.get("string", "")
                if token_name and token_string:
                    # Strip # delimiters: "#BSVER#" -> "BSVER"
                    clean_name = token_name.strip("#")
                    self._tokens[clean_name] = token_string

    def _expand_tokens(self, expr_str: str) -> str:
        """Replace #TOKEN_NAME# with token values, iteratively until stable."""
        if not expr_str or "#" not in expr_str:
            return expr_str
        # Skip pseudo-functions (#LEN[...]#, #LEN2[...]#, #THEN#, #ELSE#, #ARG#, #T#, #SELF#)
        skip = {"LEN", "LEN2", "THEN", "ELSE", "ARG", "T", "SELF"}
        result = expr_str
        for _ in range(10):  # max iterations to prevent infinite loops
            changed = False
            for match in re.finditer(r"#([A-Za-z_][A-Za-z0-9_]*)#", result):
                token_name = match.group(1)
                if token_name in skip:
                    continue
                if token_name in self._tokens:
                    result = result.replace(f"#{token_name}#", self._tokens[token_name])
                    changed = True
            if not changed:
                break
        return result

    def _make_expr(self, expr_str: str | None) -> NifExpr | None:
        """Expand tokens and compile into NifExpr, or None if empty."""
        if not expr_str:
            return None
        expanded = self._expand_tokens(expr_str)
        if not expanded.strip():
            return None
        try:
            return NifExpr(expanded)
        except Exception as e:
            logger.warning("Failed to parse expression: %r: %s", expanded, e)
            return None  # graceful degradation for unparseable expressions

    # --- Version parsing ---

    def _parse_version_string(self, ver_str: str) -> tuple:
        """Parse '20.2.0.7' into (20, 2, 0, 7)."""
        parts = ver_str.strip().split(".")
        return tuple(int(p) for p in parts)

    def _parse_versions(self, root: ET.Element) -> None:
        """Parse <version> elements."""
        for elem in root.iter("version"):
            vid = elem.attrib.get("id", "")
            num = elem.attrib.get("num", "")
            if vid and num:
                self.versions[vid] = self._parse_version_string(num)

    # --- Element parsers ---

    def _parse_basic(self, elem: ET.Element) -> None:
        name = elem.attrib.get("name", "")
        if not name:
            return
        size = int(elem.attrib.get("size", "0"))
        integral = elem.attrib.get("integral", "false").lower() == "true"
        countable = elem.attrib.get("countable", "false").lower() == "true"
        generic = elem.attrib.get("generic", "false").lower() == "true"
        self.basics[name] = BasicType(
            name=name, size=size, integral=integral,
            countable=countable, generic=generic,
        )

    def _parse_enum(self, elem: ET.Element) -> None:
        name = elem.attrib.get("name", "")
        storage = elem.attrib.get("storage", "uint")
        options = []
        for opt in elem.findall("option"):
            opt_name = opt.attrib.get("name", "")
            opt_val_str = opt.attrib.get("value", "0")
            opt_value = int(opt_val_str, 0)  # auto-detect base (handles 0x prefix)
            options.append(EnumOption(name=opt_name, value=opt_value))
        if name:
            self.enums[name] = EnumType(name=name, storage=storage, options=options)

    def _parse_bitflags(self, elem: ET.Element) -> None:
        name = elem.attrib.get("name", "")
        storage = elem.attrib.get("storage", "uint")
        options = []
        for opt in elem.findall("option"):
            opt_name = opt.attrib.get("name", "")
            bit = int(opt.attrib.get("bit", "0"))
            options.append(EnumOption(name=opt_name, value=bit))
        if name:
            self.bitflags[name] = BitflagType(name=name, storage=storage, options=options)

    def _parse_bitfield(self, elem: ET.Element) -> None:
        name = elem.attrib.get("name", "")
        storage = elem.attrib.get("storage", "uint")
        members = []
        pos = 0
        for member in elem.findall("member"):
            m_name = member.attrib.get("name", "")
            m_width = int(member.attrib.get("width", "1"))
            m_type = member.attrib.get("type", storage)
            mask = ((1 << m_width) - 1) << pos
            members.append(BitfieldMember(
                name=m_name, width=m_width, pos=pos, mask=mask, type=m_type,
            ))
            pos += m_width
        if name:
            self.bitfields[name] = BitfieldType(name=name, storage=storage, members=members)

    def _parse_field(self, elem: ET.Element) -> FieldDef:
        """Parse a <field> element (used in both compounds and niobjects)."""
        name = elem.attrib.get("name", "")
        ftype = elem.attrib.get("type", "")
        template = elem.attrib.get("template") or None
        default = elem.attrib.get("default") or None
        length = self._expand_tokens(elem.attrib.get("length") or "") or None
        width = self._expand_tokens(elem.attrib.get("width") or "") or None
        arg = self._expand_tokens(elem.attrib.get("arg") or "") or None
        suffix = elem.attrib.get("suffix") or None
        only_t = elem.attrib.get("onlyT") or None
        exclude_t = elem.attrib.get("excludeT") or None

        # Parse conditional defaults from <default> child elements
        conditional_defaults = None
        default_elems = elem.findall("default")
        if default_elems:
            conditional_defaults = []
            for d in default_elems:
                cond_str = d.attrib.get("cond", "")
                val = (d.text or "").strip()
                if cond_str and val:
                    conditional_defaults.append((cond_str, val))

        # Version range
        since = None
        until = None
        if elem.attrib.get("since"):
            since = self._parse_version_string(elem.attrib["since"])
        if elem.attrib.get("until"):
            until = self._parse_version_string(elem.attrib["until"])

        # Condition expressions (expand tokens first)
        cond = self._make_expr(elem.attrib.get("cond"))
        vercond = self._make_expr(elem.attrib.get("vercond"))
        calc = self._make_expr(elem.attrib.get("calc"))

        # Boolean flags
        is_binary = elem.attrib.get("binary", "false").lower() == "true"
        is_abstract = elem.attrib.get("abstract", "false").lower() == "true"
        recursive = elem.attrib.get("recursive", "false").lower() == "true"

        return FieldDef(
            name=name, type=ftype, template=template, default=default,
            conditional_defaults=conditional_defaults,
            length=length, width=width, cond=cond, vercond=vercond,
            since=since, until=until, arg=arg, is_binary=is_binary,
            is_abstract=is_abstract, calc=calc, suffix=suffix,
            only_t=only_t, exclude_t=exclude_t, recursive=recursive,
        )

    def _parse_compound(self, elem: ET.Element) -> None:
        name = elem.attrib.get("name", "")
        generic = elem.attrib.get("generic", "false").lower() == "true"
        fields = [self._parse_field(f) for f in elem.findall("field")]
        if name:
            self.structs[name] = StructType(name=name, fields=fields, generic=generic)

    def _parse_niobject(self, elem: ET.Element) -> None:
        name = elem.attrib.get("name", "")
        inherit = elem.attrib.get("inherit") or None
        abstract = elem.attrib.get("abstract", "false").lower() == "true"
        generic = elem.attrib.get("generic", "false").lower() == "true"
        versions_str = elem.attrib.get("versions", "")
        versions = [v.strip() for v in versions_str.split() if v.strip()] if versions_str else []
        description = (elem.text or "").strip()
        fields = [self._parse_field(f) for f in elem.findall("field")]
        if name:
            self.niobjects[name] = NiObjectType(
                name=name, inherit=inherit, fields=fields,
                abstract=abstract, versions=versions, generic=generic,
                description=description,
            )


# --- Module-level singleton ---

_SCHEMA_PATH = os.path.join(os.path.dirname(__file__), "nif_xml", "nif.xml")
_schema: NifSchema | None = None


def get_schema() -> NifSchema:
    """Get the cached NifSchema singleton. Parses nif.xml on first call."""
    global _schema
    if _schema is None:
        _schema = NifSchema.parse(os.path.normpath(_SCHEMA_PATH))
    return _schema


def build_field_def_map(schema, type_name: str) -> dict[str, "FieldDef"]:
    """Build a `{field_name: FieldDef}` lookup for displaying / editing fields
    of a given NIF block type.

    When the inheritance chain carries multiple fields with the same name
    (e.g. `BSLightingShaderProperty.Shader Type` exists on `NiObjectNET` with
    `type=BSLightingShaderType` AND on `BSShaderProperty` with the legacy
    `type=BSShaderType`), a naive `{f.name: f}` dict picks whichever was
    iterated last and binds the wrong enum to the displayed value.

    `nif_core`'s reader gates these duplicates with `vercond` / `only_t` so
    only one is actually deserialized — but display code never re-evaluates
    those gates. Prefer the FieldDef whose `only_t` targets the concrete
    block type, since that's the binding the reader honored. Both bare-name
    and `name:suffix` keys are populated.
    """
    fdefs: dict[str, "FieldDef"] = {}

    def _pick(existing, candidate):
        if existing is None:
            return candidate
        ex_only = getattr(existing, "only_t", None)
        cand_only = getattr(candidate, "only_t", None)
        if cand_only == type_name and ex_only != type_name:
            return candidate
        if ex_only == type_name and cand_only != type_name:
            return existing
        return candidate  # default: last-wins (child overrides parent)

    for f in schema.get_all_fields(type_name):
        key = f"{f.name}:{f.suffix}" if f.suffix else f.name
        fdefs[key] = _pick(fdefs.get(key), f)
        fdefs[f.name] = _pick(fdefs.get(f.name), f)
    return fdefs

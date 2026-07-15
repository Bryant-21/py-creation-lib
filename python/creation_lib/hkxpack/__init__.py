"""HKX pack/unpack — thin re-export over creation_lib._native.havok_native.

The Rust `havok_native` module owns the canonical model. This file exists for
backward compatibility — direct imports of `from creation_lib._native.havok_native ...`
are preferred for new code.
"""
import os
import tempfile
from pathlib import Path

from creation_lib._native.havok_native import (
    ClassDescriptor,
    ClassKind,
    DescriptorRegistry,
    EnumDef,
    HKXArrayMember,
    HKXDirectMember,
    HKXEnumMember,
    HKXFile,
    HKXMemberList,
    HKXObject,
    HKXObjectList,
    HKXPointerMember,
    HKXStringMember,
    HKXType,
    HKXTypeFamily,
    HKXValueList,
    MemberTemplate,
    detect_format,
    load_hkx,
    load_hkx_bytes,
    pack_xml_to_hkx,
    save_hkx,
    write_hkx,
    write_xml_file,
    write_xml_string,
)
from creation_lib._native.havok_native import unpack_hkx_to_xml as _native_unpack_hkx_to_xml


def unpack_hkx_to_xml(hkx_path: str) -> str:
    """Unpack a binary HKX file to XML. Returns path to a temp XML file.

    Wraps the native pyfunction (which returns the XML string directly) to
    preserve the legacy path-returning contract of `creation_lib.hkxpack`. Caller is
    responsible for cleaning up the temp directory (os.path.dirname of the
    returned path). New code should call
    `creation_lib._native.havok_native.unpack_hkx_to_xml` directly to skip the
    temp-file detour.
    """
    xml_str = _native_unpack_hkx_to_xml(str(hkx_path))
    tmp_dir = tempfile.mkdtemp(prefix="hkxunpack_")
    base = Path(hkx_path).stem
    xml_path = os.path.join(tmp_dir, base + ".xml")
    Path(xml_path).write_text(xml_str, encoding="utf-8")
    return xml_path

__all__ = [
    "HKXFile", "HKXObject",
    "HKXDirectMember", "HKXArrayMember", "HKXPointerMember",
    "HKXStringMember", "HKXEnumMember",
    "HKXType", "HKXTypeFamily",
    "DescriptorRegistry", "ClassDescriptor", "MemberTemplate", "EnumDef", "ClassKind",
    "HKXObjectList", "HKXMemberList", "HKXValueList",
    "load_hkx", "load_hkx_bytes", "save_hkx", "write_hkx",
    "detect_format", "unpack_hkx_to_xml", "pack_xml_to_hkx",
    "write_xml_string", "write_xml_file",
]

"""Deprecated re-export shim — see creation_lib._native.havok_native.

The in-memory HKXFile / HKXObject / member model now lives in Rust under
`py_creation_lib/native/havok/src/python.rs`. This module re-exports the native pyclasses so
existing `from creation_lib.hkxpack.model import ...` callers keep working. Prefer
direct imports from `creation_lib._native.havok_native` in new code.
"""
from creation_lib._native.havok_native import (
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
)

__all__ = [
    "HKXFile", "HKXObject",
    "HKXDirectMember", "HKXArrayMember", "HKXPointerMember",
    "HKXStringMember", "HKXEnumMember",
    "HKXType", "HKXTypeFamily",
    "HKXObjectList", "HKXMemberList", "HKXValueList",
]

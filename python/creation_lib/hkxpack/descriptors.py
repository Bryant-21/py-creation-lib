"""Deprecated re-export shim — see creation_lib._native.havok_native.

The Havok class descriptor registry now lives in Rust. This module re-exports
the native pyclasses so existing `from creation_lib.hkxpack.descriptors import ...`
callers keep working. Prefer direct imports from `creation_lib._native.havok_native`
in new code.
"""
from creation_lib._native.havok_native import (
    ClassDescriptor,
    ClassKind,
    DescriptorRegistry,
    EnumDef,
    MemberTemplate,
)

__all__ = [
    "DescriptorRegistry", "ClassDescriptor", "MemberTemplate", "EnumDef", "ClassKind",
]

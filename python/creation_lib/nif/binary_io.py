"""Legacy NIF expression/version helpers.

Binary NIF load/save is owned by the Rust `nif_core_native` backend. This
module intentionally keeps only the small helpers still shared by conversion
code and tests.
"""

from __future__ import annotations

from functools import lru_cache


@lru_cache(maxsize=512)
def _compile_nif_expr(expr_string: str):
    from .expr import NifExpr

    return NifExpr(expr_string)


def _version_to_int(ver: tuple) -> int:
    return (ver[0] << 24) | (ver[1] << 16) | (ver[2] << 8) | ver[3]


def _int_to_version(val: int) -> tuple:
    return (
        (val >> 24) & 0xFF,
        (val >> 16) & 0xFF,
        (val >> 8) & 0xFF,
        val & 0xFF,
    )

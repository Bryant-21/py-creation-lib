"""YAML utilities for authoring-dir compatible loading.

Standard yaml.safe_load converts hex integers (0x0400...) to Python ints,
losing the 0x prefix. Authoring-dir YAML requires hex strings for byte blobs.
This module provides a loader that preserves 0x-prefixed values as strings.
"""
from __future__ import annotations

import re

import yaml


class _HexPreservingLoader(yaml.SafeLoader):
    """YAML loader that keeps 0x-prefixed values as strings."""
    pass


# Match hex integers: 0x followed by hex digits
_HEX_RE = re.compile(r"^0[xX][0-9a-fA-F]+$")


def _hex_int_constructor(loader: yaml.Loader, node: yaml.ScalarNode) -> int | str:
    """Keep 0x-prefixed integers as strings, parse others normally."""
    value = loader.construct_scalar(node)
    if isinstance(value, str) and _HEX_RE.match(value):
        return value  # Preserve as string
    # Fall back to normal int parsing
    return int(value, 0) if isinstance(value, str) else value


# Override the int resolver to use our constructor
_HexPreservingLoader.add_constructor(
    "tag:yaml.org,2002:int", _hex_int_constructor
)


def safe_load_hex(stream) -> dict | list | None:
    """Load YAML while preserving 0x-prefixed hex values as strings."""
    return yaml.load(stream, Loader=_HexPreservingLoader)

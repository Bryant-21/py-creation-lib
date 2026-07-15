"""creation_lib must stay standalone: no imports from the host toolkit.

Mirrors the AST scan in the private repo's tests/test_bacup_boundaries.py but
is self-contained so it ships with (and protects) the public py-creation-lib
repo.
"""

from __future__ import annotations

import ast
from pathlib import Path

LIB_ROOT = Path(__file__).resolve().parents[1]  # py_creation_lib/ (repo root when standalone)
PACKAGE = LIB_ROOT / "python" / "creation_lib"

FORBIDDEN_PREFIXES = ("ui", "app", "cli", "bacup_lib", "bacup_ui", "tools")


def _qualified_name(node: ast.AST) -> str:
    parts: list[str] = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        parts.append(node.id)
    return ".".join(reversed(parts))


def _module_matches(module: str | None, prefix: str) -> bool:
    return bool(module) and (module == prefix or module.startswith(f"{prefix}."))


def _references(path: Path, prefix: str) -> list[str]:
    source = path.read_text(encoding="utf-8", errors="replace")
    if prefix not in source:
        return []
    tree = ast.parse(source, filename=str(path))
    refs: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if _module_matches(alias.name, prefix):
                    refs.append(f"line {node.lineno}: import {alias.name}")
        elif isinstance(node, ast.ImportFrom):
            if node.level == 0 and _module_matches(node.module, prefix):
                refs.append(f"line {node.lineno}: from {node.module} import ...")
        elif isinstance(node, ast.Call):
            callee = _qualified_name(node.func)
            if callee not in {
                "import_module",
                "importlib.import_module",
                "patch",
                "mock.patch",
                "unittest.mock.patch",
            }:
                continue
            for value in [*node.args, *(kw.value for kw in node.keywords)]:
                if (
                    isinstance(value, ast.Constant)
                    and isinstance(value.value, str)
                    and _module_matches(value.value, prefix)
                ):
                    refs.append(f"line {node.lineno}: {callee}({value.value!r})")
    return refs


def _runtime_files() -> list[Path]:
    return [
        p
        for p in PACKAGE.rglob("*.py")
        if "tests" not in p.parts and not p.name.startswith("test_")
    ]


def test_creation_lib_runtime_has_no_host_imports() -> None:
    violations: list[str] = []
    for path in _runtime_files():
        for prefix in FORBIDDEN_PREFIXES:
            for ref in _references(path, prefix):
                violations.append(f"{path.relative_to(LIB_ROOT)}: [{prefix}] {ref}")
    assert not violations, "creation_lib must not import host packages:\n" + "\n".join(violations)

"""Small, bounded discovery of real PEX files for smoke tests."""
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[5]

_SEARCH_ROOTS = (
    PROJECT_ROOT / "py_creation_lib/python/creation_lib" / "pex" / "tests" / "fixtures",
    PROJECT_ROOT / "scripts",
    PROJECT_ROOT / "Scripts",
    PROJECT_ROOT / "mods",
)

_SKIP_DIR_NAMES = {
    ".git",
    ".hg",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".uv-cache",
    ".venv",
    "__pycache__",
    "_internal",
    "build",
    "dist",
    "external_mods",
    "extracted",
    "node_modules",
    "output",
    "reports",
    "target",
    "tmp",
}


def find_pex_files(limit: int = 5, *, max_depth: int = 6, max_dirs: int = 500) -> list[Path]:
    """Return a bounded sample of available PEX files without walking the whole repo."""
    found: list[Path] = []
    visited_dirs = 0

    for root in _SEARCH_ROOTS:
        if len(found) >= limit or visited_dirs >= max_dirs or not root.exists():
            continue

        stack: list[tuple[Path, int]] = [(root, 0)]
        while stack and len(found) < limit and visited_dirs < max_dirs:
            directory, depth = stack.pop()
            visited_dirs += 1

            try:
                entries = list(directory.iterdir())
            except OSError:
                continue

            for entry in entries:
                if entry.is_file() and entry.suffix.lower() == ".pex":
                    found.append(entry)
                    if len(found) >= limit:
                        break
                elif (
                    entry.is_dir()
                    and depth < max_depth
                    and entry.name not in _SKIP_DIR_NAMES
                    and not entry.name.startswith(".")
                ):
                    stack.append((entry, depth + 1))

    return found

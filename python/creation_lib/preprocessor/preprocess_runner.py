from __future__ import annotations

import importlib
import io
import os
import sys
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from typing import Callable


_PREPROCESS_MODULES = {
    "preprocess_records.py": "creation_lib.preprocessor.records",
    "preprocess_scripts.py": "creation_lib.preprocessor.scripts",
    "preprocess_wiki.py": "creation_lib.preprocessor.wiki",
    "preprocess_nifs.py": "creation_lib.preprocessor.nifs",
    "preprocess_havok.py": "creation_lib.preprocessor.havok",
    "preprocess_behaviors.py": "creation_lib.preprocessor.havok",
    "preprocess_external.py": "creation_lib.preprocessor.external",
    "preprocess_swf.py": "creation_lib.preprocessor.swf",
}


class _LineWriter(io.TextIOBase):
    def __init__(self, on_line: Callable[[str], None] | None):
        self._on_line = on_line
        self._buffer = ""

    def write(self, text: str) -> int:
        if not text:
            return 0
        self._buffer += text
        while "\n" in self._buffer:
            line, self._buffer = self._buffer.split("\n", 1)
            line = line.rstrip("\r")
            if line and self._on_line:
                self._on_line(line)
        return len(text)

    def flush(self) -> None:
        line = self._buffer.rstrip("\r")
        self._buffer = ""
        if line and self._on_line:
            self._on_line(line)


def resolve_preprocess_module(script: str) -> str:
    name = Path(script).name
    try:
        return _PREPROCESS_MODULES[name]
    except KeyError as exc:
        valid = ", ".join(sorted(_PREPROCESS_MODULES))
        raise FileNotFoundError(
            f"Unknown preprocess script: {name}. Valid: {valid}"
        ) from exc


def run_preprocess(
    script: str,
    *args: str,
    cwd: Path | None = None,
    on_line: Callable[[str], None] | None = None,
) -> int:
    module_name = resolve_preprocess_module(script)
    module = importlib.import_module(module_name)
    main = getattr(module, "main", None)
    if not callable(main):
        raise RuntimeError(f"Preprocess module has no callable main(): {module_name}")

    prev_argv = sys.argv[:]
    prev_cwd = Path.cwd()
    writer = _LineWriter(on_line)

    try:
        sys.argv = [Path(script).name, *args]
        if cwd is not None:
            os.chdir(cwd)
        with redirect_stdout(writer), redirect_stderr(writer):
            try:
                main()
            except SystemExit as exc:
                code = exc.code if isinstance(exc.code, int) else 1
                writer.flush()
                return code
        writer.flush()
        return 0
    finally:
        sys.argv = prev_argv
        os.chdir(prev_cwd)

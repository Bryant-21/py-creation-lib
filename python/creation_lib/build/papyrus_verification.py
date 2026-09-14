import subprocess
import tempfile
from pathlib import Path


def verify_stock_sources(sources, *, compiler, game_root, imports, flags=None, on_progress=None):
    compiler = Path(compiler)
    if not compiler.is_file():
        raise FileNotFoundError(f"Stock Papyrus compiler not found: {compiler}")
    with tempfile.TemporaryDirectory(prefix="modkit_stock_") as temporary:
        for index, source in enumerate(sources):
            source = Path(source).resolve()
            output = Path(temporary) / str(index)
            output.mkdir()
            if on_progress:
                on_progress(f"  [verify-stock] {source.name}")
            command = [str(compiler.resolve()), str(source),
                       f"-i={';'.join(str(Path(path).resolve()) for path in imports)}", f"-o={output}"]
            if flags:
                flag_path = Path(flags)
                command.append(f"-f={flag_path.resolve() if flag_path.is_file() else flags}")
            result = subprocess.run(command, cwd=str(game_root), capture_output=True, text=True, encoding="utf-8", errors="replace")
            produced = any(path.stem.casefold() == source.stem.casefold() for path in output.rglob("*.pex"))
            if result.returncode != 0 or not produced:
                diagnostics = (result.stdout + "\n" + result.stderr).strip()[-6000:]
                raise RuntimeError(f"Stock Papyrus verification failed for {source.name} (exit {result.returncode}, output produced: {produced}):\n{diagnostics}")

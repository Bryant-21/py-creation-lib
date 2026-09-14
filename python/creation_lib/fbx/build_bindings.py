"""Build script for Autodesk FBX SDK Python bindings.

Prerequisites:
  - Autodesk FBX SDK 2020.3.9 installed at:
    C:\\Program Files\\Autodesk\\FBX\\FBX SDK\\2020.3.9
  - Autodesk FBX Python Bindings 2020.3.9 installed at:
    C:\\Program Files\\Autodesk\\FBX\\FBX Python Bindings\\2020.3.9
  - Visual Studio 2022+ with C++ build tools
  - SIP 6.6.2 (installed automatically)

Usage:
  uv run python py_creation_lib/python/creation_lib/fbx/build_bindings.py

Copies the bindings source to build/fbx_bindings/, points its pyproject.toml at
the SDK library path, builds with sipbuild, and copies the .pyd to site-packages.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

SDK_ROOT = Path(r"C:\Program Files\Autodesk\FBX\FBX SDK\2020.3.9")
BINDINGS_ROOT = Path(
    r"C:\Program Files\Autodesk\FBX\FBX Python Bindings\2020.3.9"
)
LIB_DIR = SDK_ROOT / "py_creation_lib/python/creation_lib" / "x64" / "release"
INCLUDE_DIR = SDK_ROOT / "include"


def _site_packages_dirs(project_root: Path) -> list[Path]:
    """Return concrete site-packages directories for the active interpreter."""
    import site

    dirs: list[Path] = []
    for candidate in site.getsitepackages():
        path = Path(candidate)
        if path.name != "site-packages":
            continue
        if not path.exists():
            continue
        dirs.append(path)

    # Prefer the project venv explicitly when present. PyInstaller specs scan it.
    project_site = project_root / ".venv" / "Lib" / "site-packages"
    if project_site.exists() and project_site not in dirs:
        dirs.insert(0, project_site)

    return dirs


def _install_sip() -> None:
    """Install the Autodesk-required SIP version into the active interpreter."""
    try:
        import pip  # noqa: F401

        subprocess.check_call(
            [sys.executable, "-m", "pip", "install", "--force-reinstall", "sip==6.6.2"],
        )
        return
    except ImportError:
        pass

    subprocess.check_call(
        ["uv", "pip", "install", "--python", sys.executable, "--force-reinstall", "sip==6.6.2"],
    )


def main(project_root: str | Path):
    root = Path(project_root)
    build_dir = root / "build" / "fbx_bindings"

    if not SDK_ROOT.exists():
        print(f"FBX SDK not found at {SDK_ROOT}")
        sys.exit(1)
    if not BINDINGS_ROOT.exists():
        print(f"FBX Python Bindings not found at {BINDINGS_ROOT}")
        sys.exit(1)

    # 1. Copy bindings source
    print(f"Copying bindings to {build_dir}...")
    if build_dir.exists():
        shutil.rmtree(build_dir)
    shutil.copytree(BINDINGS_ROOT, build_dir)

    # 2. Patch pyproject.toml
    print("Patching pyproject.toml...")
    toml_path = build_dir / "pyproject.toml"
    text = toml_path.read_text()
    text = text.replace(
        'include-dirs = ["../../../include"]',
        f'include-dirs = ["{INCLUDE_DIR.as_posix()}"]',
    )
    text = text.replace(
        'library-dirs = ["../RelWithDebInfo"]',
        f'library-dirs = ["{LIB_DIR.as_posix()}"]',
    )
    toml_path.write_text(text)

    # 3. Install SIP
    print("Installing SIP 6.6.2...")
    _install_sip()

    # 4. Build
    print("Building FBX Python bindings...")
    env = os.environ.copy()
    env["FBXSDK_ROOT"] = str(SDK_ROOT)
    subprocess.check_call(
        [sys.executable, "-m", "sipbuild.tools.build", "--verbose"],
        cwd=str(build_dir),
        env=env,
    )

    # 5. Find and copy the .pyd
    pyd_files = list(build_dir.rglob("fbx*.pyd"))
    if not pyd_files:
        print("ERROR: No .pyd file found after build")
        sys.exit(1)

    # Find concrete site-packages dirs. Under uv, site.getsitepackages()[0]
    # can be the venv root, which is not importable for extension modules.
    site_packages_dirs = _site_packages_dirs(root)
    if not site_packages_dirs:
        print("ERROR: No site-packages directory found for the active interpreter")
        sys.exit(1)

    copied_to: list[Path] = []
    for sp in site_packages_dirs:
        dest = sp / pyd_files[0].name
        print(f"Copying {pyd_files[0].name} to {dest}...")
        shutil.copy2(pyd_files[0], dest)
        copied_to.append(dest)

    # Verify
    print("Verifying import...")
    verify_script = (
        "import os; "
        f"os.add_dll_directory(r'{LIB_DIR}'); "
        "import fbx; "
        "print('FBX SDK loaded successfully')"
    )
    result = subprocess.run(
        [sys.executable, "-c", verify_script],
        capture_output=True,
        text=True,
    )
    print(result.stdout.strip())
    if result.returncode not in (0, -11):  # -11 is SIGSEGV at exit, harmless
        print(f"WARNING: import test returned {result.returncode}")
        if result.stderr:
            print(result.stderr)
    else:
        for dest in copied_to:
            print(f"Installed binding: {dest}")

    print("Done!")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--project-root", required=True)
    args = parser.parse_args()
    main(args.project_root)

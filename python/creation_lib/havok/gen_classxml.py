"""Generate per-version classxml directories — thin shim over creation_lib._native.havok_native.

Usage:
  modkit build gen-classxml
"""
from __future__ import annotations

import json
from pathlib import Path

TARGETS = [
    ("2012", 46),
    ("2015", 56),
]

FO4_VERSION_ID = 53


def main(project_root: str | Path, resource_dir: str | Path):
    from creation_lib.havok.native_runtime import generate_classxml_native
    from creation_lib.paths import get_resource_dir

    root = Path(project_root)
    resource = get_resource_dir()
    sdk_patches_dir = root / "refs" / "hk2018_1_0_r1" / "Source" / "Common" / "Compat" / "Patches"
    fo4_classxml = resource / "classxml"

    print("Generating per-version class descriptors...")
    print(f"  SDK patches: {sdk_patches_dir}")
    print(f"  FO4 classxml: {fo4_classxml}")

    targets_json = json.dumps([{"suffix": s, "version_id": v} for s, v in TARGETS])
    output_base = str(resource)

    generate_classxml_native(
        source_dir=str(fo4_classxml),
        patches_dir=str(sdk_patches_dir),
        output_base=output_base,
        targets_json=targets_json,
        base_version_id=FO4_VERSION_ID,
    )

    print("Done!")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--project-root", required=True)
    parser.add_argument("--resource-dir", required=True)
    args = parser.parse_args()
    main(args.project_root, args.resource_dir)

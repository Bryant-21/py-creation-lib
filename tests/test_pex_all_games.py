"""Cross-game PEX decompilation test.

Decompiles a sample of .pex scripts from each supported game,
saves the .psc output to tests/results/<game>/, and reports
success/failure stats.

Run:  uv run python tests/test_pex_all_games.py
"""
from __future__ import annotations

import json
import random
import sys
import traceback
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(ROOT))

from creation_lib.pex import decompile_pex, parse_pex

RESULTS_DIR = ROOT / "tests" / "results"

# Game → script directories (relative to extracted/)
GAME_SCRIPT_DIRS: dict[str, list[Path]] = {
    "fo4": [ROOT / "extracted" / "fo4" / "scripts"],
    "skyrimse": [ROOT / "extracted" / "skyrimse" / "scripts"],
    "starfield": [ROOT / "extracted" / "starfield" / "scripts"],
    "fo76": [
        ROOT / "extracted" / "fo76" / "scripts" / "client",
        ROOT / "extracted" / "fo76" / "scripts" / "server",
    ],
}

SAMPLE_SIZE = 20  # scripts per game


def collect_pex_files(dirs: list[Path]) -> list[Path]:
    files = []
    for d in dirs:
        if d.exists():
            files.extend(sorted(d.glob("*.pex")))
    return files


def run_game(game: str, dirs: list[Path]) -> dict:
    out_dir = RESULTS_DIR / game
    out_dir.mkdir(parents=True, exist_ok=True)

    all_pex = collect_pex_files(dirs)
    if not all_pex:
        return {"game": game, "total_available": 0, "sampled": 0,
                "success": 0, "fail": 0, "errors": ["No .pex files found"]}

    # Deterministic sample
    random.seed(42)
    sample = random.sample(all_pex, min(SAMPLE_SIZE, len(all_pex)))

    results = {"game": game, "total_available": len(all_pex),
               "sampled": len(sample), "success": 0, "fail": 0,
               "errors": [], "scripts": []}

    for pex_path in sample:
        name = pex_path.stem
        entry = {"name": name, "pex": str(pex_path)}
        try:
            pex_file = parse_pex(pex_path)
            entry["game_id"] = pex_file.game_id
            entry["source"] = pex_file.source_filename

            source = decompile_pex(pex_path)
            entry["lines"] = source.count("\n")

            psc_path = out_dir / f"{name}.psc"
            psc_path.write_text(source, encoding="utf-8")
            entry["psc"] = str(psc_path)
            entry["status"] = "OK"
            results["success"] += 1

        except Exception as e:
            entry["status"] = "FAIL"
            entry["error"] = f"{type(e).__name__}: {e}"
            entry["traceback"] = traceback.format_exc()
            results["fail"] += 1
            results["errors"].append(f"{name}: {entry['error']}")

        results["scripts"].append(entry)

    return results


def main():
    all_results = {}
    for game, dirs in GAME_SCRIPT_DIRS.items():
        print(f"\n{'='*60}")
        print(f"  {game.upper()}")
        print(f"{'='*60}")
        r = run_game(game, dirs)
        all_results[game] = r

        print(f"  Available: {r['total_available']}")
        print(f"  Sampled:   {r['sampled']}")
        print(f"  Success:   {r['success']}")
        print(f"  Failed:    {r['fail']}")
        if r["errors"]:
            for e in r["errors"]:
                print(f"  ERROR: {e}")

    # Write summary JSON
    summary_path = RESULTS_DIR / "summary.json"
    summary_path.write_text(json.dumps(all_results, indent=2, default=str),
                            encoding="utf-8")
    print(f"\n{'='*60}")
    print(f"  Summary written to {summary_path}")
    print(f"{'='*60}")

    # Exit code: non-zero if any failures
    total_fail = sum(r["fail"] for r in all_results.values())
    if total_fail:
        print(f"\n  TOTAL FAILURES: {total_fail}")
    return 1 if total_fail else 0


if __name__ == "__main__":
    sys.exit(main())

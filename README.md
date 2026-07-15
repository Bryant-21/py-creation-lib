# py-creation-lib

Python + Rust libraries for Bethesda Creation Engine modding: ESP record I/O,
NIF, Havok, BA2 archives, materials, DDS, terrain/LOD generation, Papyrus.

## Build

    git clone --recursive https://github.com/Bryant-21/py-creation-lib.git
    cd py-creation-lib
    uv sync        # builds the native extension via maturin (needs Rust + MSVC)

## Test

    cargo test --workspace
    uv run pytest

Most tests run with no game data. Tests that need game assets skip unless you
point them at your own extracted files:

| Env var | Points at |
|---|---|
| `FO4_DATA` | Fallout 4 `Data/` dir (game install) |
| `FO4_EXTRACTED_DIR` | dir of BA2-extracted FO4 files |
| `FO76_EXTRACTED_DIR` | dir of BA2-extracted FO76 files |
| `STARFIELD_EXTRACTED_DIR` | dir of BA2-extracted Starfield files |
| `FO4_TEST_NIF` | any static FO4 BSTriShape `.nif` |

## License

GPL-3.0 — see LICENSE. Part of the ModBox21 family:
[modkit21](https://github.com/Bryant-21/modkit21) ·
[bacup](https://github.com/Bryant-21/bacup)

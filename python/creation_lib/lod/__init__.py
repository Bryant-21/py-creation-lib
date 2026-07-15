# creation_lib.lod — LOD generation service layer
# Billboard generator: creation_lib.lod.billboards
from creation_lib.lod.native_runtime import (
    LodGenResult,
    generate_lod,
    is_available,
    load_native_module,
)

__all__ = ["is_available", "load_native_module", "generate_lod", "LodGenResult"]

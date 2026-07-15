"""Texture operations -- extract paths, list referenced textures."""
from ..actions import OperationResult


def extract_texture_paths(nif) -> OperationResult:
    """Return all texture file paths referenced by the NIF.

    Searches BSShaderTextureSet blocks for texture paths in the
    Textures array (slots 0-9: diffuse, normal, glow, parallax,
    cubemap, envmask, subsurface, specular/backlight).
    """
    paths = []
    for block in nif.blocks:
        if block.type_name == "BSShaderTextureSet":
            textures = block.get_field("Textures")
            if isinstance(textures, list):
                for tex in textures:
                    if isinstance(tex, str) and tex.strip():
                        paths.append(tex.strip())
            else:
                # Try indexed fields (Textures[0], Textures:0, etc.)
                for i in range(10):
                    for key in (f"Textures[{i}]", f"Textures:{i}"):
                        tex = block.get_field(key)
                        if tex and isinstance(tex, str) and tex.strip():
                            paths.append(tex.strip())
                            break

    # Deduplicate while preserving order
    seen = set()
    unique = []
    for p in paths:
        lower = p.lower()
        if lower not in seen:
            seen.add(lower)
            unique.append(p)

    return OperationResult(True, f"Found {len(unique)} texture path(s)")

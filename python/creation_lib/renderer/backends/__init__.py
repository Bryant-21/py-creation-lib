"""Per-game scene backends.

Each module here implements the ``SceneBackend`` protocol from
``creation_lib.renderer.scene_backend`` for one Bethesda game. Backends are
selected at scene-load time by ``SceneRenderer`` based on the active
game profile.

"""

from __future__ import annotations

from typing import TYPE_CHECKING

from typing import Union

from creation_lib.renderer.backends.fo4_backend import Fo4Backend
from creation_lib.renderer.backends.fo76_backend import Fo76Backend
from creation_lib.renderer.backends.sf_backend import SfBackend

if TYPE_CHECKING:
    from creation_lib.renderer.scene_renderer import SceneRenderer

# Every Bethesda game has its own backend type.
# All concrete backends still satisfy the SceneBackend Protocol
# structurally. Skyrim SE / LE keep using Fo4Backend because they share
# the same shader/material pipeline.
AnyBackend = Union[Fo4Backend, Fo76Backend, SfBackend]


def make_backend(game_id: str, renderer: "SceneRenderer") -> AnyBackend:
    """Construct the appropriate backend for ``game_id``.

    Falls back to ``Fo4Backend`` for any unrecognised game id, which
    matches the existing renderer behaviour (FO4 is the default shader
    program when a per-game program isn't registered). Skyrim SE / LE
    intentionally route through Fo4Backend.
    """
    if game_id == "starfield":
        return SfBackend(renderer)
    if game_id == "fo76":
        return Fo76Backend(renderer)
    return Fo4Backend(renderer)


__all__ = ["make_backend"]

"""Fallout 76 scene backend.

FO76 shares the whole FO4 draw path and differs only in its shader program key
(``"fo76"``) and PBR cubemaps (``_fo76_specular_cube`` / ``_fo76_irradiance_cube``).
``Fo4Backend._draw_node`` (the ``r._current_game_id == "fo76"`` branch) and the
host renderer's per-game program registration handle both, so this subclass
overrides nothing. It gives the factory a distinct type for ``"fo76"``;
FO76-specific overrides belong here, not in ``fo4_backend.py``.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from creation_lib.renderer.backends.fo4_backend import Fo4Backend

if TYPE_CHECKING:
    from creation_lib.renderer.scene_renderer import SceneRenderer


class Fo76Backend(Fo4Backend):
    """FO76 backend — structurally identical to Fo4Backend today.

    Subclasses ``Fo4Backend`` so the entire draw path, shadow walk, and
    overlay set come for free. Override hooks here as FO76-specific
    behaviour diverges from FO4.
    """

    def __init__(self, renderer: "SceneRenderer") -> None:
        super().__init__(renderer)

"""Fallout 76 scene backend.

FO76 shares the entire FO4 draw
path — same composed shader pipeline, same SceneNode tree, same
``_draw_node`` walk, same shadow + selection overlays — and only
differs in:

  * The shader program key it pulls from ``SceneRenderer.programs``
    (``"fo76"`` instead of ``"fo4"``)
  * The PBR cubemaps it binds (``_fo76_specular_cube`` /
    ``_fo76_irradiance_cube`` on the renderer instead of the SF set)

Both deltas are already handled inside ``Fo4Backend._draw_node`` via the
``r._current_game_id == "fo76"`` branch and the host renderer's
per-game program registration loop, so this class doesn't need to
override any methods. It exists primarily as an architectural marker:

  * It gives the factory a concrete type to return for ``"fo76"``,
    which means downstream type narrowing knows the difference.
  * It documents that FO76 is a real, supported game.
  * It's the natural extension point if FO76 ever grows quirks of its
    own (different decal handling, different alpha behaviour, etc.):
    override single methods here without churning Fo4Backend.

If you find yourself adding FO76-specific logic to ``fo4_backend.py``,
move it here instead.
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

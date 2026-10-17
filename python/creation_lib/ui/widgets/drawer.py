"""A slide-in panel over one edge of the window that draws it, closed until asked for."""

from __future__ import annotations

from collections.abc import Callable

from imgui_bundle import imgui

from creation_lib.ui.widgets.modern import heading, scaled

_FLAGS = (imgui.WindowFlags_.no_title_bar | imgui.WindowFlags_.no_resize | imgui.WindowFlags_.no_move
          | imgui.WindowFlags_.no_saved_settings | imgui.WindowFlags_.no_docking
          | imgui.WindowFlags_.no_collapse)


class Drawer:
    """Call `draw()` every frame from inside the host window; `open()` slides it in.

    Clicking outside it or its close button slides it back out."""

    ANIM_SPEED = 8.0  # fraction of the slide per second

    def __init__(self, name: str, side: str = "left", width: float = 460):
        self.name = name
        self.side = side
        self.width = width
        self._open_pct = 0.0
        self._opening = False
        self._content: Callable[[], None] | None = None
        self._title = ""
        self._button_hovered = False

    @property
    def is_open(self) -> bool:
        return self._opening

    def open(self, content: Callable[[], None], title: str) -> None:
        self._content, self._title, self._opening = content, title, True

    def close(self) -> None:
        self._opening = False

    def toggle(self, content: Callable[[], None], title: str) -> None:
        if self._opening:
            self.close()
        else:
            self.open(content, title)

    def toggle_button(self, label: str, content: Callable[[], None], title: str) -> None:
        """A button that opens and closes the drawer; pressing it doesn't count as a click outside."""
        if imgui.button(label):
            self.toggle(content, title)
        self._button_hovered = imgui.is_item_hovered()

    def draw(self) -> None:
        step = imgui.get_io().delta_time * self.ANIM_SPEED
        self._open_pct = min(1.0, self._open_pct + step) if self._opening else max(0.0, self._open_pct - step)
        if self._open_pct <= 0.0 or self._content is None:
            return
        host_pos, host_size = imgui.get_window_pos(), imgui.get_window_size()
        width = min(scaled(self.width), host_size.x - scaled(32)) * self._open_pct
        x = host_pos.x if self.side == "left" else host_pos.x + host_size.x - width
        imgui.set_next_window_pos(imgui.ImVec2(x, host_pos.y))
        imgui.set_next_window_size(imgui.ImVec2(width, host_size.y))
        imgui.push_style_var(imgui.StyleVar_.window_padding, imgui.ImVec2(scaled(16), scaled(14)))
        hovered = False
        if imgui.begin(f"##drawer_{self.name}", None, _FLAGS):
            if self._open_pct >= 0.99:
                heading(self._title)
                imgui.same_line(max(scaled(18), imgui.get_window_width() - scaled(46)))
                if imgui.button(f"X##drawer_close_{self.name}", imgui.ImVec2(scaled(28), 0)):
                    self.close()
                imgui.separator()
                if imgui.begin_child(f"##drawer_content_{self.name}"):
                    self._content()
                imgui.end_child()
            hovered = imgui.is_window_hovered(imgui.HoveredFlags_.root_and_child_windows
                                              | imgui.HoveredFlags_.allow_when_blocked_by_popup)
        imgui.end()
        imgui.pop_style_var()
        clicked = imgui.is_mouse_clicked(imgui.MouseButton_.left)
        if self._opening and clicked and not hovered and not self._button_hovered:
            self.close()

"""Shared User Guide helpers for toolkit and standalone UIs."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

from imgui_bundle import hello_imgui, imgui, icons_fontawesome_6 as fa


@dataclass(frozen=True)
class UserGuide:
    title: str
    body: str
    window_id: str = ""


@runtime_checkable
class UserGuideProvider(Protocol):
    def get_user_guide(self) -> UserGuide | None:
        ...


def get_user_guide(provider: object) -> UserGuide | None:
    getter = getattr(provider, "get_user_guide", None)
    if getter is None:
        return None
    return getter()


def has_user_guide(provider: object) -> bool:
    return get_user_guide(provider) is not None


def toggle_user_guide(provider: object) -> None:
    toggle = getattr(provider, "toggle_user_guide", None)
    if toggle is not None:
        toggle()


def draw_user_guide_menu_item(provider: object) -> None:
    guide = get_user_guide(provider)
    toggle = getattr(provider, "toggle_user_guide", None)
    if guide is None or not callable(toggle):
        imgui.begin_disabled()
        imgui.menu_item("User Guide", "F1", False)
        imgui.end_disabled()
    elif imgui.menu_item("User Guide", "F1", False)[0]:
        toggle_user_guide(provider)


def draw_help_menu(provider: object) -> None:
    if not imgui.begin_menu("Help"):
        return
    draw_user_guide_menu_item(provider)
    imgui.end_menu()


def draw_toolbar_help_button(
    provider: object,
    icon_font=None,
    *,
    same_line: bool = False,
    align_right: bool = True,
) -> bool:
    if not has_user_guide(provider):
        return False
    if same_line:
        imgui.same_line()
    icon = getattr(fa, "ICON_FA_CIRCLE_QUESTION", "?")
    label = f"{icon}##user_guide"
    if icon_font:
        imgui.push_font(icon_font, icon_font.legacy_size)
    label_width = max(
        imgui.get_frame_height(),
        imgui.calc_text_size(icon).x + imgui.get_style().frame_padding.x * 2.0,
    )
    if icon_font:
        imgui.pop_font()
    avail = imgui.get_content_region_avail().x
    if align_right and avail > label_width:
        imgui.set_cursor_pos_x(imgui.get_cursor_pos_x() + max(0.0, avail - label_width))
    if icon_font:
        imgui.push_font(icon_font, icon_font.legacy_size)
    clicked = imgui.button(label, imgui.ImVec2(label_width, 0))
    if icon_font:
        imgui.pop_font()
    imgui.set_item_tooltip("User Guide (F1)")
    if clicked:
        toggle_user_guide(provider)
    return clicked


def _render_markdown(body: str) -> None:
    markdown = getattr(hello_imgui, "markdown", None)
    if callable(markdown):
        markdown(body)
    else:
        imgui.text_wrapped(body)


def draw_generic_user_guide_window(visible: bool, guide: UserGuide) -> bool:
    if not visible:
        return False
    window_id = guide.window_id or "user_guide"
    is_visible, is_open = imgui.begin(f"{guide.title}##{window_id}", visible)
    try:
        if is_visible:
            _render_markdown(guide.body)
    finally:
        imgui.end()
    return is_open


def draw_docked_user_guide_window(label: str, provider: object, on_close=None) -> None:
    guide = get_user_guide(provider)
    if guide is None:
        return
    visible, _ = imgui.begin(label)
    try:
        if visible:
            if on_close is not None:
                close_width = max(
                    imgui.get_frame_height(),
                    imgui.calc_text_size("X").x + imgui.get_style().frame_padding.x * 2.0,
                )
                avail = imgui.get_content_region_avail().x
                if avail > close_width:
                    imgui.set_cursor_pos_x(imgui.get_cursor_pos_x() + max(0.0, avail - close_width))
                if imgui.button("X##close_user_guide", imgui.ImVec2(close_width, 0)):
                    on_close()
                imgui.set_item_tooltip("Close User Guide")
                imgui.separator()
            _render_markdown(guide.body)
    finally:
        imgui.end()

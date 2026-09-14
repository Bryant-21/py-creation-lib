from __future__ import annotations

from dataclasses import dataclass
from functools import lru_cache

from imgui_bundle import hello_imgui, imgui


@dataclass(frozen=True)
class AppearanceTokens:
    background: tuple
    surface: tuple
    input: tuple
    border: tuple
    text: tuple
    muted: tuple
    success: tuple
    warning: tuple
    error: tuple
    padding: float = 16.0
    gap: float = 10.0
    rounding: float = 6.0
    body_size: float = 16.0
    heading_size: float = 21.0
    small_size: float = 14.0


def _rgb(value: str) -> tuple:
    return tuple(int(value[i:i + 2], 16) / 255 for i in (0, 2, 4)) + (1.0,)


@lru_cache(maxsize=2)
def appearance_tokens(light: bool = False) -> AppearanceTokens:
    colors = (
        ("f3f4f6", "ffffff", "e9ecf0", "cbd0d8", "20242b", "555e6b",
         "23703c", "855b09", "b42a38")
        if light else
        ("101114", "222429", "2b2e34", "3a3e46", "eceef2", "a6aeba",
         "79cd92", "edc575", "f28a91")
    )
    return AppearanceTokens(*(_rgb(c) for c in colors))


_status_colors: dict[str, tuple] = {}


def set_status_colors(colors: dict[str, tuple]) -> None:
    _status_colors.clear()
    _status_colors.update(colors)


def status_color(role: str, light: bool) -> tuple:
    return _status_colors.get(role, getattr(appearance_tokens(light), role))


@lru_cache(maxsize=32)
def modern_colors(theme) -> dict:
    t = appearance_tokens(theme.light)
    accent = theme.accent
    def blend(a, b, weight):
        return tuple(a[i] * (1 - weight) + b[i] * weight for i in range(3)) + (1.0,)
    if theme.id == "falloutnv":
        muted_accent, hover = _rgb("795C14"), accent
    elif theme.id == "fallout76":
        muted_accent, hover = _rgb("9B7A1B"), accent
    else:
        muted_accent, hover = blend(t.surface, accent, 0.16), blend(t.input, accent, 0.20)
    control_hover = _rgb("604B14") if theme.id == "falloutnv" else hover
    checkbox_background = _rgb("514015") if theme.id == "falloutnv" else muted_accent
    return {
        imgui.Col_.window_bg: t.background,
        imgui.Col_.child_bg: t.surface,
        imgui.Col_.popup_bg: t.surface,
        imgui.Col_.text: t.text,
        imgui.Col_.text_disabled: t.muted,
        imgui.Col_.border: t.border,
        imgui.Col_.separator: t.border,
        imgui.Col_.frame_bg: t.input,
        imgui.Col_.frame_bg_hovered: control_hover,
        imgui.Col_.frame_bg_active: control_hover,
        imgui.Col_.checkbox_selected_bg: checkbox_background,
        imgui.Col_.button: t.input,
        imgui.Col_.button_hovered: hover,
        imgui.Col_.button_active: muted_accent,
        imgui.Col_.header: muted_accent,
        imgui.Col_.header_hovered: hover,
        imgui.Col_.header_active: muted_accent,
        imgui.Col_.title_bg: t.background,
        imgui.Col_.title_bg_active: t.surface,
        imgui.Col_.menu_bar_bg: t.background,
        imgui.Col_.tab: t.surface,
        imgui.Col_.tab_selected: muted_accent,
        imgui.Col_.tab_hovered: control_hover,
        imgui.Col_.tab_dimmed: t.surface,
        imgui.Col_.tab_dimmed_selected: muted_accent,
        imgui.Col_.tab_selected_overline: accent,
        imgui.Col_.tab_dimmed_selected_overline: accent,
        imgui.Col_.table_header_bg: t.input,
        imgui.Col_.table_border_strong: t.border,
        imgui.Col_.table_border_light: t.border,
        imgui.Col_.scrollbar_bg: t.background,
        imgui.Col_.scrollbar_grab: t.border,
        imgui.Col_.scrollbar_grab_hovered: t.muted,
        imgui.Col_.scrollbar_grab_active: accent,
        imgui.Col_.docking_empty_bg: t.background,
    }


def apply_modern_metrics(scale: float | None = None) -> None:
    style = imgui.get_style()
    if scale is None:
        scale = hello_imgui.dpi_window_size_factor()
    t = appearance_tokens()
    style.window_padding = (t.padding * scale, t.padding * scale)
    style.frame_padding = (10 * scale, 6 * scale)
    style.item_spacing = (t.gap * scale, 9 * scale)
    style.item_inner_spacing = (6 * scale, 5 * scale)
    style.cell_padding = (5 * scale, 6 * scale)
    style.window_rounding = t.rounding * scale
    style.child_rounding = t.rounding * scale
    style.frame_rounding = 4 * scale
    style.popup_rounding = t.rounding * scale
    style.grab_rounding = 4 * scale
    style.tab_rounding = 4 * scale
    style.tab_bar_overline_size = 0.0
    style.scrollbar_rounding = 6 * scale
    style.scrollbar_size = 12 * scale
    style.grab_min_size = 10 * scale
    style.frame_border_size = 1.0
    style.child_border_size = 1.0


@dataclass
class UiFonts:
    body: object = None
    icons: object = None
    small: object = None
    mono: object = None


def load_ui_fonts() -> UiFonts:
    import logging

    fonts = UiFonts()
    loaders = (
        ("body", lambda: hello_imgui.load_font_ttf_with_font_awesome_icons(
            "fonts/Roboto/Roboto-Regular.ttf", 16.0)),
        ("icons", lambda: hello_imgui.load_font("fonts/Font_Awesome_6_Free-Solid-900.otf", 20.0)),
        ("small", lambda: hello_imgui.load_font("fonts/Roboto/Roboto-Regular.ttf", 14.0)),
        ("mono", lambda: hello_imgui.load_font("fonts/Inconsolata-Medium.ttf", 14.0)),
    )
    for role, load in loaders:
        try:
            setattr(fonts, role, load())
        except Exception:
            logging.getLogger(__name__).warning("UI %s font unavailable; retaining default font", role, exc_info=True)
    return fonts


def configure_runner_appearance(params, theme, color_overrides: dict | None = None) -> None:
    from .themes import apply_tab_style, apply_theme

    post_init = params.callbacks.post_init
    post_frame = params.callbacks.post_new_frame

    def initialize():
        apply_theme(theme, color_overrides)
        imgui.get_io().config_flags |= imgui.ConfigFlags_.nav_enable_keyboard
        if post_init:
            post_init()

    def after_new_frame():
        if post_frame:
            post_frame()
        apply_tab_style(theme, color_overrides)

    params.callbacks.post_init = initialize
    params.callbacks.post_new_frame = after_new_frame
    params.callbacks.load_additional_fonts = load_ui_fonts
    params.callbacks.default_icon_font = hello_imgui.DefaultIconFont.font_awesome6

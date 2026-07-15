"""Game-themed color palettes for the ModBox21 toolkit.

Each theme is a dark base with game-colored accents for buttons, tabs, headers,
and interactive elements.  The dark background colors are shared across all
themes — only the accent/interactive colors change.
"""

from __future__ import annotations

from dataclasses import dataclass
from imgui_bundle import imgui


def _hex(hexstr: str) -> tuple[float, float, float, float]:
    """Convert '#RRGGBB' to (r, g, b, 1.0) floats."""
    h = hexstr.lstrip("#")
    return (int(h[0:2], 16) / 255, int(h[2:4], 16) / 255, int(h[4:6], 16) / 255, 1.0)


def _mix(a: tuple, b: tuple, t: float) -> tuple[float, float, float, float]:
    """Linearly interpolate two color tuples."""
    return (
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        1.0,
    )


def _darken(c: tuple, factor: float = 0.3) -> tuple[float, float, float, float]:
    """Darken a color by mixing toward black."""
    return (c[0] * factor, c[1] * factor, c[2] * factor, 1.0)


def _brighten(c: tuple, amount: float = 0.12) -> tuple[float, float, float, float]:
    """Brighten a color additively, clamped to 1.0."""
    return (
        min(c[0] + amount, 1.0),
        min(c[1] + amount, 1.0),
        min(c[2] + amount, 1.0),
        1.0,
    )


# ---------------------------------------------------------------------------
# Theme dataclass
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class GameTheme:
    id: str
    name: str
    light: bool  # True = light backgrounds, dark text

    # Three semantic levels
    accent: tuple[float, float, float, float]       # main
    accent_hover: tuple[float, float, float, float]  # highlight
    accent_active: tuple[float, float, float, float] # brighter highlight

    # Inactive / muted (recessed tabs, dim elements)
    inactive: tuple[float, float, float, float]

    # Buttons (muted accent tint on dark base)
    button: tuple[float, float, float, float]
    button_hover: tuple[float, float, float, float]
    button_active: tuple[float, float, float, float]

    # Tabs
    tab_overline: tuple[float, float, float, float]
    tab_selected: tuple[float, float, float, float]

    # Headers / tree nodes / selectables
    header: tuple[float, float, float, float]
    header_hover: tuple[float, float, float, float]
    header_active: tuple[float, float, float, float]

    # Checkmark, slider grab
    checkmark: tuple[float, float, float, float]
    slider_grab: tuple[float, float, float, float]
    slider_grab_active: tuple[float, float, float, float]

    # Resize grip
    resize_grip: tuple[float, float, float, float]
    resize_grip_hover: tuple[float, float, float, float]
    resize_grip_active: tuple[float, float, float, float]


def _rgb(r: int, g: int, b: int) -> tuple[float, float, float, float]:
    """Convert 0-255 RGB to (r, g, b, 1.0) floats."""
    return (r / 255, g / 255, b / 255, 1.0)


def _make_theme(
    theme_id: str,
    name: str,
    main: tuple,
    inactive: tuple,
    highlight: tuple,
    *,
    light: bool = False,
) -> GameTheme:
    """Build a full GameTheme from three semantic colors.

    Args:
        main:      Primary accent — buttons, active tabs, checkmarks, sliders.
        inactive:  Muted/recessed — inactive tabs, dim headers, scrollbar grabs.
        highlight: Bright hover/active — tab overline, hover states, active press.
    """
    # Buttons: dark version of main for base, main for hover, highlight for active
    btn_base = _darken(main, 0.40)
    return GameTheme(
        id=theme_id,
        name=name,
        light=light,
        # Semantic colors
        accent=main,
        accent_hover=highlight,
        accent_active=_brighten(highlight, 0.10),
        inactive=inactive,
        # Buttons
        button=btn_base,
        button_hover=_darken(main, 0.60),
        button_active=main,
        # Tabs
        tab_overline=highlight,
        tab_selected=_darken(main, 0.50),
        # Headers
        header=_darken(main, 0.35),
        header_hover=_darken(main, 0.50),
        header_active=_darken(highlight, 0.60),
        # Checkmark / sliders
        checkmark=main,
        slider_grab=main,
        slider_grab_active=highlight,
        # Resize grip
        resize_grip=_darken(inactive, 0.50),
        resize_grip_hover=inactive,
        resize_grip_active=main,
    )


# ---------------------------------------------------------------------------
# Concrete themes
# ---------------------------------------------------------------------------

# Fallout 4 — Vault-Tec blue
#   main=#0b89d5  inactive=#504455  highlight=#34a4d4
FALLOUT4 = _make_theme(
    "fallout4", "Fallout 4",
    main=_hex("#0b89d5"),
    inactive=_hex("#504455"),
    highlight=_hex("#34a4d4"),
)

# Starfield — Light theme, white/gray backgrounds with red accents
#   main=#c72138  inactive=#304c7a  highlight=#e06236  bg=#f4f5f7
STARFIELD = _make_theme(
    "starfield", "Starfield",
    main=_hex("#c72138"),
    inactive=_hex("#304c7a"),
    highlight=_hex("#e06236"),
    light=True,
)

# Skyrim SE — Nordic warm browns
#   main=#806653  inactive=#34221b  highlight=#552a24
SKYRIM = _make_theme(
    "skyrimse", "Skyrim SE",
    main=_hex("#806653"),
    inactive=_hex("#34221b"),
    highlight=_hex("#552a24"),
)

# Fallout 76 — Amber/gold wasteland
#   main=#f5cb5b  inactive=#d8b252  highlight=#f9e390
FALLOUT76 = _make_theme(
    "fallout76", "Fallout 76",
    main=_hex("#f5cb5b"),
    inactive=_hex("#d8b252"),
    highlight=_hex("#f9e390"),
)

# Fallout 3 — Pip-Boy green
#   main=#146c11  inactive=#073605  highlight=#199515
FALLOUT3 = _make_theme(
    "fallout3", "Fallout 3",
    main=_hex("#146c11"),
    inactive=_hex("#073605"),
    highlight=_hex("#199515"),
)

# Fallout NV — Pip-Boy amber
#   main=RGB(255,182,66)  inactive=#767455  highlight=#b8b37a
FALLOUTNV = _make_theme(
    "falloutnv", "Fallout: New Vegas",
    main=_rgb(255, 182, 66),
    inactive=_hex("#767455"),
    highlight=_hex("#b8b37a"),
)

# Dracula Dark — purple/pink accents on a deep dark base
#   main=#bd93f9  inactive=#6272a4  highlight=#ff79c6
DRACULA = _make_theme(
    "dracula", "Dracula Dark",
    main=_hex("#bd93f9"),
    inactive=_hex("#6272a4"),
    highlight=_hex("#ff79c6"),
)


# ---------------------------------------------------------------------------
# Registry
# ---------------------------------------------------------------------------

ALL_THEMES: list[GameTheme] = [
    DRACULA,
    FALLOUT76,
    FALLOUT4,
    FALLOUT3,
    FALLOUTNV,
    SKYRIM,
    STARFIELD,
]

THEMES_BY_ID: dict[str, GameTheme] = {t.id: t for t in ALL_THEMES}

DEFAULT_THEME_ID = "falloutnv"


def get_theme(theme_id: str) -> GameTheme:
    """Return theme by id, falling back to the default."""
    return THEMES_BY_ID.get(theme_id, THEMES_BY_ID[DEFAULT_THEME_ID])



# ---------------------------------------------------------------------------
# Apply a theme to the current ImGui style
# ---------------------------------------------------------------------------

def _build_full_colors(theme: GameTheme) -> dict[int, tuple[float, float, float, float]]:
    """Build the complete color map for every ImGui Col_ slot from a theme.

    Uses the three semantic levels stored on the theme:
      accent   = main color (buttons, active elements)
      inactive = muted/recessed (inactive tabs, dim states)
      accent_hover / accent_active = highlight (hovers, overlines)
    """
    a = theme.accent          # main
    hi = theme.accent_hover   # highlight
    ina = theme.inactive      # inactive / muted

    # Darkened variants of inactive for tab backgrounds
    ina_dark = _darken(ina, 0.40)
    ina_mid = _darken(ina, 0.60)

    # Semi-transparent accent for overlays
    accent_t = (a[0], a[1], a[2], 0.40)
    accent_t_strong = (a[0], a[1], a[2], 0.70)

    # --- Background palette (light vs dark) ---
    if theme.light:
        bg = _hex("#f4f5f7")            # main window
        bg_child = (0.92, 0.92, 0.94, 1.0)
        bg_popup = (0.98, 0.98, 0.99, 1.0)
        border = (0.75, 0.75, 0.78, 1.0)
        menubar = (0.90, 0.90, 0.92, 1.0)
        scroll_bg = (0.92, 0.92, 0.94, 1.0)
        text = (0.12, 0.12, 0.14, 1.0)
        text_dis = (0.50, 0.50, 0.52, 1.0)
        sep = (0.78, 0.78, 0.80, 1.0)
        frame = (0.88, 0.88, 0.90, 1.0)
        frame_hov = (0.84, 0.84, 0.86, 1.0)
        frame_act = (0.80, 0.80, 0.83, 1.0)
        title = (0.90, 0.90, 0.92, 1.0)
        title_act = (0.85, 0.85, 0.88, 1.0)
        title_col = (0.92, 0.92, 0.94, 0.75)
        tbl_border = (0.75, 0.75, 0.78, 1.0)
        tbl_border_lt = (0.82, 0.82, 0.84, 1.0)
        tbl_row_alt = (0.0, 0.0, 0.0, 0.03)
        tree_ln = (0.70, 0.70, 0.72, 1.0)
        dock_empty = (0.88, 0.88, 0.90, 1.0)
    else:
        bg = (0.12, 0.12, 0.14, 1.0)
        bg_child = (0.10, 0.10, 0.12, 1.0)
        bg_popup = (0.14, 0.14, 0.16, 1.0)
        border = (0.28, 0.28, 0.30, 1.0)
        menubar = (0.14, 0.14, 0.16, 1.0)
        scroll_bg = (0.10, 0.10, 0.12, 1.0)
        text = (0.85, 0.85, 0.85, 1.0)
        text_dis = (0.50, 0.50, 0.50, 1.0)
        sep = (0.28, 0.28, 0.30, 1.0)
        frame = (0.18, 0.18, 0.20, 1.0)
        frame_hov = (0.22, 0.22, 0.25, 1.0)
        frame_act = (0.25, 0.25, 0.28, 1.0)
        title = (0.10, 0.10, 0.12, 1.0)
        title_act = ina_dark
        title_col = (0.10, 0.10, 0.12, 0.75)
        tbl_border = (0.28, 0.28, 0.30, 1.0)
        tbl_border_lt = (0.22, 0.22, 0.24, 1.0)
        tbl_row_alt = (1.0, 1.0, 1.0, 0.02)
        tree_ln = (0.28, 0.28, 0.30, 1.0)
        dock_empty = (0.08, 0.08, 0.10, 1.0)

    return {
        # --- Backgrounds ---
        imgui.Col_.window_bg:           bg,
        imgui.Col_.child_bg:            bg_child,
        imgui.Col_.popup_bg:            bg_popup,
        imgui.Col_.border:              border,
        imgui.Col_.border_shadow:       (0.0, 0.0, 0.0, 0.0),
        imgui.Col_.menu_bar_bg:         menubar,
        imgui.Col_.scrollbar_bg:        scroll_bg,
        imgui.Col_.text:                text,
        imgui.Col_.text_disabled:       text_dis,
        imgui.Col_.separator:           sep,
        imgui.Col_.modal_window_dim_bg: (0.0, 0.0, 0.0, 0.55),

        # --- Frames (input fields, combo boxes) ---
        imgui.Col_.frame_bg:            frame,
        imgui.Col_.frame_bg_hovered:    frame_hov,
        imgui.Col_.frame_bg_active:     frame_act,

        # --- Title bar ---
        imgui.Col_.title_bg:            title,
        imgui.Col_.title_bg_active:     title_act,
        imgui.Col_.title_bg_collapsed:  title_col,

        # --- Buttons ---
        imgui.Col_.button:              theme.button,
        imgui.Col_.button_hovered:      theme.button_hover,
        imgui.Col_.button_active:       theme.button_active,

        # --- Headers / tree nodes / selectables ---
        imgui.Col_.header:              theme.header,
        imgui.Col_.header_hovered:      theme.header_hover,
        imgui.Col_.header_active:       theme.header_active,

        # --- Tabs (fully themed via inactive / main / highlight) ---
        imgui.Col_.tab:                 ina_dark,
        imgui.Col_.tab_hovered:         _darken(a, 0.50),
        imgui.Col_.tab_selected:        theme.tab_selected,
        imgui.Col_.tab_selected_overline: theme.tab_overline,
        imgui.Col_.tab_dimmed:          ina_dark,
        imgui.Col_.tab_dimmed_selected: ina_mid,
        imgui.Col_.tab_dimmed_selected_overline: _darken(theme.tab_overline, 0.50),

        # --- Checkmark / sliders ---
        imgui.Col_.check_mark:          theme.checkmark,
        imgui.Col_.slider_grab:         theme.slider_grab,
        imgui.Col_.slider_grab_active:  theme.slider_grab_active,

        # --- Scrollbar ---
        imgui.Col_.scrollbar_grab:          ina_mid,
        imgui.Col_.scrollbar_grab_hovered:  ina,
        imgui.Col_.scrollbar_grab_active:   a,

        # --- Resize grip ---
        imgui.Col_.resize_grip:         theme.resize_grip,
        imgui.Col_.resize_grip_hovered: theme.resize_grip_hover,
        imgui.Col_.resize_grip_active:  theme.resize_grip_active,

        # --- Separators (interactive) ---
        imgui.Col_.separator_hovered:   ina,
        imgui.Col_.separator_active:    a,

        # --- Text selection highlight ---
        imgui.Col_.text_selected_bg:    accent_t,

        # --- Nav / focus ---
        imgui.Col_.nav_cursor:              a,
        imgui.Col_.nav_windowing_highlight: (1.0, 1.0, 1.0, 0.70),
        imgui.Col_.nav_windowing_dim_bg:    (0.0, 0.0, 0.0, 0.20),

        # --- Docking ---
        imgui.Col_.docking_preview:     accent_t_strong,
        imgui.Col_.docking_empty_bg:    dock_empty,

        # --- Drag & drop ---
        imgui.Col_.drag_drop_target:    hi,

        # --- Tables ---
        imgui.Col_.table_header_bg:     ina_dark,
        imgui.Col_.table_border_strong: tbl_border,
        imgui.Col_.table_border_light:  tbl_border_lt,
        imgui.Col_.table_row_bg:        (0.0, 0.0, 0.0, 0.0),
        imgui.Col_.table_row_bg_alt:    tbl_row_alt,

        # --- Plot ---
        imgui.Col_.plot_lines:          theme.accent,
        imgui.Col_.plot_lines_hovered:  theme.accent_hover,
        imgui.Col_.plot_histogram:      theme.accent,
        imgui.Col_.plot_histogram_hovered: theme.accent_hover,

        # --- Misc ---
        imgui.Col_.text_link:           theme.accent,
        imgui.Col_.input_text_cursor:   theme.accent,
        imgui.Col_.tree_lines:          tree_ln,
        imgui.Col_.unsaved_marker:      theme.accent,
        imgui.Col_.drag_drop_target_bg: accent_t,
    }


def apply_theme(theme: GameTheme) -> None:
    """Apply a GameTheme to the current ImGui style.

    Call once at startup (in post_init) and then call ``apply_tab_style``
    each frame to keep tab colors stable against hello_imgui resets.
    """
    style = imgui.get_style()
    style.window_rounding = 4.0
    style.frame_rounding = 2.0
    style.grab_rounding = 2.0
    style.scrollbar_rounding = 4.0
    style.frame_border_size = 1.0

    sc = style.set_color_
    for col, rgba in _build_full_colors(theme).items():
        sc(col, imgui.ImVec4(*rgba))


def apply_tab_style(theme: GameTheme) -> None:
    """Reapply themed tab + interactive colors each frame.

    hello_imgui resets many style colors on certain events (focus change,
    docking).  We reapply all accent-derived colors, not just tabs.
    """
    style = imgui.get_style()
    s = style.set_color_
    for col, rgba in _build_full_colors(theme).items():
        s(col.value, imgui.ImVec4(*rgba))


def draw_theme_selector(current_id: str) -> str | None:
    """Draw a theme picker with color preview swatches.

    Returns the new theme id if the user changed it, otherwise None.
    """
    new_id = None
    for theme in ALL_THEMES:
        is_selected = theme.id == current_id

        # Color swatch (accent color)
        draw_list = imgui.get_window_draw_list()
        pos = imgui.get_cursor_screen_pos()
        swatch_size = imgui.get_text_line_height()
        r, g, b, a = theme.accent
        color_u32 = imgui.get_color_u32(imgui.ImVec4(r, g, b, a))
        draw_list.add_rect_filled(
            imgui.ImVec2(pos.x, pos.y + 2),
            imgui.ImVec2(pos.x + swatch_size, pos.y + 2 + swatch_size),
            color_u32,
            rounding=2.0,
        )
        imgui.set_cursor_pos_x(imgui.get_cursor_pos_x() + swatch_size + 8)

        if imgui.selectable(
            theme.name,
            is_selected,
            imgui.SelectableFlags_.none,
            imgui.ImVec2(0, 0),
        )[0]:
            if not is_selected:
                new_id = theme.id

    return new_id

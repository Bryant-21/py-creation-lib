from __future__ import annotations

from copy import deepcopy

from imgui_bundle import imgui

from .themes import ALL_THEMES, get_theme, get_theme_colors, normalize_color_overrides
from ..widgets.modern import action_button, heading, prepare_dialog, scaled, status_indicator


_TAB_LABELS = {
    "tab": "Tab · unselected",
    "tab_selected": "Tab · selected, focused",
    "tab_dimmed": "Tab · unselected, unfocused",
    "tab_dimmed_selected": "Tab · selected, unfocused",
    "tab_hovered": "Tab · hovered",
}
_GROUPS = ("All colors", "Tabs", "Surfaces", "Text", "Controls", "Tables", "Status", "Other")


def color_group(name: str) -> str:
    if name == "tab" or name.startswith("tab_"):
        return "Tabs"
    if name.startswith("table_"):
        return "Tables"
    if name.startswith("status_"):
        return "Status"
    if name.startswith(("text", "nav_")):
        return "Text"
    if name.startswith(("window_", "child_", "popup_", "title_", "menu_", "border", "docking_")):
        return "Surfaces"
    if name.startswith(("frame_", "button", "header", "check", "slider_", "scrollbar_", "separator", "resize_")):
        return "Controls"
    return "Other"


def color_label(name: str) -> str:
    return _TAB_LABELS.get(name, name.replace("_bg", "_background").replace("_", " ").capitalize())


class ThemeEditor:
    def __init__(self):
        self.is_open = False
        self.theme_id = ALL_THEMES[0].id
        self.overrides: dict[str, dict] = {}
        self.search = ""
        self.group = "All colors"
        self._pending_open = False
        self._preview_checked = True
        self._preview_value = 0.65

    def open(self, theme_id: str, overrides: dict[str, dict]) -> None:
        self.theme_id = get_theme(theme_id).id
        self.overrides = deepcopy(overrides)
        self.search = ""
        self.group = "All colors"
        self.is_open = self._pending_open = True

    @property
    def current_overrides(self) -> dict:
        return self.overrides.get(self.theme_id, {})

    def set_color(self, name: str, rgba) -> None:
        value = normalize_color_overrides({name: rgba})
        if value:
            self.overrides.setdefault(self.theme_id, {})[name] = list(value[name])

    def reset_color(self, name: str) -> None:
        self.overrides.get(self.theme_id, {}).pop(name, None)

    def reset_theme(self) -> None:
        self.overrides.pop(self.theme_id, None)

    def visible_colors(self) -> list[tuple[str, tuple]]:
        query = self.search.strip().casefold()
        return [(name, rgba) for name, rgba in get_theme_colors(get_theme(self.theme_id), self.current_overrides).items()
                if (self.group == "All colors" or color_group(name) == self.group)
                and query in f"{name} {color_label(name)}".casefold()]

    def draw(self) -> str | None:
        if not self.is_open:
            return None
        if self._pending_open:
            prepare_dialog(1100, 730)
            imgui.open_popup("Theme##theme_modal")
            self._pending_open = False
        maximum = imgui.get_main_viewport().work_size
        imgui.set_next_window_size_constraints(
            (min(scaled(760), maximum.x - scaled(32)), min(scaled(480), maximum.y - scaled(32))),
            (maximum.x - scaled(32), maximum.y - scaled(32)),
        )
        visible, opened = imgui.begin_popup_modal(
            "Theme##theme_modal", True, imgui.WindowFlags_.no_scrollbar | imgui.WindowFlags_.no_scroll_with_mouse,
        )
        if not visible:
            if not opened:
                self.is_open = False
                return "cancel"
            return None
        result = None
        try:
            heading("Theme colors", large=True)
            imgui.text_wrapped("Preview changes live. Save keeps your colors for each theme; Cancel restores them.")
            imgui.set_next_item_width(scaled(280))
            index = next(i for i, theme in enumerate(ALL_THEMES) if theme.id == self.theme_id)
            changed, index = imgui.combo("##theme_preset", index, [theme.name for theme in ALL_THEMES])
            if changed:
                self.theme_id = ALL_THEMES[index].id
            imgui.same_line()
            imgui.text_disabled(f"{len(self.current_overrides)} customized colors")
            imgui.spacing()

            style = imgui.get_style()
            footer_height = imgui.get_frame_height() + 3 * style.item_spacing.y + 2 * style.cell_padding.y + 1
            height = max(1, imgui.get_content_region_avail().y - footer_height)
            if imgui.begin_table("##theme_layout", 2, imgui.TableFlags_.resizable | imgui.TableFlags_.sizing_stretch_prop):
                imgui.table_setup_column("Colors", imgui.TableColumnFlags_.width_stretch, 0.64)
                imgui.table_setup_column("Preview", imgui.TableColumnFlags_.width_stretch, 0.36)
                imgui.table_next_column()
                if imgui.begin_child("##theme_color_panel", (0, height), window_flags=(
                    imgui.WindowFlags_.no_background | imgui.WindowFlags_.no_scrollbar | imgui.WindowFlags_.no_scroll_with_mouse
                )):
                    self._draw_colors()
                imgui.end_child()
                imgui.table_next_column()
                if imgui.begin_child("##theme_preview", (0, height), imgui.ChildFlags_.borders):
                    self._draw_preview()
                imgui.end_child()
                imgui.end_table()

            imgui.separator()
            if action_button("Reset theme colors"):
                self.reset_theme()
            imgui.same_line()
            right = imgui.get_cursor_pos_x() + imgui.get_content_region_avail().x
            imgui.set_cursor_pos_x(max(imgui.get_cursor_pos_x(), right - scaled(224) - imgui.get_style().item_spacing.x))
            if action_button("Cancel", width=scaled(104)):
                result = "cancel"
            imgui.same_line()
            if action_button("Save", primary=True, width=scaled(120)):
                result = "save"
            if not opened:
                result = "cancel"
            if result:
                self.is_open = False
                imgui.close_current_popup()
        finally:
            imgui.end_popup()
        return result

    def _draw_colors(self) -> None:
        imgui.set_next_item_width(-1)
        _, self.search = imgui.input_text_with_hint("##color_search", "Find a color, e.g. tab_selected or background", self.search)
        imgui.set_next_item_width(-1)
        changed, group = imgui.combo("##color_group", _GROUPS.index(self.group), list(_GROUPS))
        if changed:
            self.group = _GROUPS[group]
        colors = self.visible_colors()
        imgui.text_disabled(f"{len(colors)} colors · hex / RGBA · click a swatch to open its picker")
        flags = imgui.TableFlags_.scroll_y | imgui.TableFlags_.sizing_stretch_prop | imgui.TableFlags_.row_bg
        if imgui.begin_table("##theme_colors", 3, flags, (0, 0)):
            imgui.table_setup_column("Color", imgui.TableColumnFlags_.width_stretch)
            imgui.table_setup_column("Value", imgui.TableColumnFlags_.width_fixed, scaled(154))
            imgui.table_setup_column("", imgui.TableColumnFlags_.width_fixed, scaled(58))
            imgui.table_setup_scroll_freeze(0, 1)
            imgui.table_headers_row()
            for name, rgba in colors:
                imgui.push_id(name)
                imgui.table_next_row()
                imgui.table_set_column_index(0)
                imgui.align_text_to_frame_padding()
                imgui.text_wrapped(color_label(name))
                imgui.set_item_tooltip(name + ("\nTab overlines are disabled." if "overline" in name else ""))
                imgui.table_set_column_index(1)
                imgui.set_next_item_width(-1)
                changed, value = imgui.color_edit4("##value", list(rgba), imgui.ColorEditFlags_.display_hex | imgui.ColorEditFlags_.alpha_preview_half)
                if changed:
                    self.set_color(name, value)
                imgui.table_set_column_index(2)
                imgui.begin_disabled(name not in self.current_overrides)
                if imgui.button("Reset"):
                    self.reset_color(name)
                imgui.end_disabled()
                imgui.pop_id()
            imgui.end_table()

    def _draw_preview(self) -> None:
        heading("Tab backgrounds")
        imgui.text_wrapped("Selected and hovered tabs have separate background colors. Click a sample to find its color.")
        colors = get_theme_colors(get_theme(self.theme_id), self.current_overrides)
        for name, label in _TAB_LABELS.items():
            if imgui.color_button(f"{label}##{name}", colors[name], size=(scaled(52), scaled(26))):
                self.search, self.group = name, "Tabs"
            imgui.same_line()
            imgui.text_wrapped(label.removeprefix("Tab · ").capitalize())
        imgui.spacing()
        imgui.separator()
        heading("Controls")
        action_button("Button")
        imgui.same_line()
        action_button("Primary", primary=True)
        _, self._preview_checked = imgui.checkbox("Checkbox", self._preview_checked)
        imgui.set_next_item_width(-1)
        _, self._preview_value = imgui.slider_float("##preview_slider", self._preview_value, 0, 1)
        imgui.progress_bar(self._preview_value, (-1, 0), "Progress")
        for role in ("success", "warning", "error"):
            status_indicator(role.capitalize(), role)
        imgui.spacing()
        imgui.text_wrapped("Check mark sets the tick color. Slider grab controls the shared accent. Text, surfaces and status colors can also be customized here.")

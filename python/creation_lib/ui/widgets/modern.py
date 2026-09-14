from __future__ import annotations

from contextlib import contextmanager
from collections.abc import Callable
from dataclasses import dataclass, field
import math

from imgui_bundle import imgui

def scaled(value: float) -> float:
    return value * imgui.get_font_size() / 16.0


def semantic_color(role: str) -> imgui.ImVec4:
    slots = {
        "accent": imgui.Col_.slider_grab, "background": imgui.Col_.window_bg,
        "surface": imgui.Col_.child_bg, "input": imgui.Col_.frame_bg,
        "border": imgui.Col_.border, "text": imgui.Col_.text, "muted": imgui.Col_.text_disabled,
    }
    if role in slots:
        return imgui.get_style_color_vec4(slots[role])
    from creation_lib.ui.theme.appearance import status_color

    background = imgui.get_style_color_vec4(imgui.Col_.window_bg)
    return imgui.ImVec4(*status_color(role, background.x > 0.5))


def heading(text: str, *, large: bool = False, size: float | None = None) -> None:
    imgui.push_font(imgui.get_font(), scaled(size if size is not None else 21.0 if large else 17.0))
    try:
        imgui.text_wrapped(text)
    finally:
        imgui.pop_font()


def prepare_dialog(width: float, height: float) -> None:
    viewport = imgui.get_main_viewport()
    imgui.set_next_window_size(
        (min(scaled(width), viewport.work_size.x - scaled(32)),
         min(scaled(height), viewport.work_size.y - scaled(32))), imgui.Cond_.appearing,
    )
    imgui.set_next_window_pos(viewport.get_center(), imgui.Cond_.appearing, (0.5, 0.5))


@contextmanager
def section(identifier: str, title: str = "", *, height: float = 0):
    flags = imgui.ChildFlags_.borders
    if height == 0:
        flags |= imgui.ChildFlags_.auto_resize_y
    visible = imgui.begin_child(identifier, imgui.ImVec2(0, height), flags)
    try:
        if visible and title:
            heading(title)
            imgui.spacing()
        yield visible
    finally:
        imgui.end_child()


@contextmanager
def expandable_section(label: str, flags: int = 0, *, description: str = "",
                       header_actions: Callable[[bool], None] | None = None, actions_width: float = 0):
    pos = imgui.get_cursor_screen_pos()
    width = max(1.0, imgui.get_content_region_avail().x)
    pad, rounding = scaled(18), scaled(10)
    text_width = max(1.0, width - scaled(56) - actions_width)
    title = label.split("##", 1)[0]
    title_size = imgui.calc_text_size(title, wrap_width=text_width)
    detail_size = imgui.calc_text_size(description, wrap_width=text_width) if description else imgui.ImVec2()
    text_height = title_size.y + (detail_size.y + scaled(3) if description else 0)
    height = max(scaled(47), text_height + scaled(22))
    draw = imgui.get_window_draw_list()
    # Each card owns a splitter so nested cards and tables can split independently.
    layers = imgui.ImDrawListSplitter()
    layers.split(draw, 2)
    layers.set_current_channel(draw, 1)
    imgui.begin_group()
    imgui.push_style_var(imgui.StyleVar_.frame_padding, (0, (height - imgui.get_font_size()) / 2))
    imgui.push_style_var(imgui.StyleVar_.frame_border_size, 0)
    for color in (imgui.Col_.header, imgui.Col_.header_hovered, imgui.Col_.header_active,
                  imgui.Col_.text, imgui.Col_.nav_cursor):
        imgui.push_style_color(color, (0, 0, 0, 0))
    try:
        expanded = imgui.collapsing_header(label, flags | (imgui.TreeNodeFlags_.allow_overlap if header_actions else 0))
    finally:
        imgui.pop_style_color(5)
        imgui.pop_style_var(2)
    hovered, focused = imgui.is_item_hovered(), imgui.is_item_focused()
    header_end = imgui.get_item_rect_max().y
    imgui.push_id(label)
    storage = imgui.get_state_storage()
    now = imgui.get_time()
    time_id = imgui.get_id("##section_time")
    elapsed = max(0.0, now - storage.get_float(time_id, now))
    storage.set_float(time_id, now)

    def animate(key, target):
        key_id = imgui.get_id(key)
        previous = storage.get_float(key_id, target)
        value = previous + (target - previous) * (1 - math.exp(-elapsed / .10))
        storage.set_float(key_id, value)
        return value

    opening = animate("##section_open", float(expanded))
    hover = animate("##section_hover", float(hovered))
    accent, text = semantic_color("accent"), semantic_color("text")

    def blend(a, b, amount):
        return imgui.ImVec4(*(x + (y - x) * amount for x, y in zip(a, b)))

    title_accent = blend(accent, text, .5) if semantic_color("background").x > .5 else accent
    title_color = imgui.get_color_u32(blend(text, title_accent, opening))
    draw.push_clip_rect(pos, (pos.x + width, header_end), True)
    text_y = pos.y + (height - text_height) / 2
    draw.add_text(imgui.get_font(), imgui.get_font_size(), (pos.x + scaled(14), text_y),
                  title_color, title, wrap_width=text_width)
    if description:
        draw.add_text(imgui.get_font(), imgui.get_font_size(), (pos.x + scaled(14), text_y + title_size.y + scaled(3)),
                      imgui.get_color_u32(imgui.Col_.text_disabled), description, wrap_width=text_width)
    center = imgui.ImVec2(pos.x + width - scaled(23), pos.y + height / 2)
    angle = opening * math.pi / 2
    points = []
    for x, y in ((-2, -4), (2, 0), (-2, 4)):
        points.append(imgui.ImVec2(center.x + scaled(x * math.cos(angle) - y * math.sin(angle)),
                                  center.y + scaled(x * math.sin(angle) + y * math.cos(angle))))
    draw.add_polyline(points, title_color, scaled(1.5), 0)
    draw.pop_clip_rect()
    if header_actions is not None:
        cursor = imgui.get_cursor_screen_pos()
        imgui.set_cursor_screen_pos((pos.x + width - scaled(40) - actions_width,
                                     pos.y + (height - imgui.get_frame_height()) / 2))
        header_actions(expanded)
        imgui.set_cursor_screen_pos((cursor.x, header_end))
        imgui.dummy((0, 0))
    body = False
    if expanded:
        imgui.push_style_var(imgui.StyleVar_.cell_padding, (pad, scaled(8)))
        body = imgui.begin_table("##section_body", 1,
                                 imgui.TableFlags_.pad_outer_x | imgui.TableFlags_.no_saved_settings
                                 | imgui.TableFlags_.sizing_stretch_same)
        imgui.pop_style_var()
        if body:
            imgui.table_setup_column("##content", imgui.TableColumnFlags_.width_stretch)
            imgui.table_next_column()
            imgui.push_text_wrap_pos(0)
    try:
        yield expanded and body
    finally:
        if body:
            imgui.pop_text_wrap_pos()
            imgui.end_table()
        bottom = max(header_end, imgui.get_cursor_screen_pos().y - imgui.get_style().item_spacing.y)
        layers.set_current_channel(draw, 0)
        surface = blend(semantic_color("surface"), semantic_color("input"), hover * .5)
        draw.add_rect_filled(pos, (pos.x + width, bottom), imgui.get_color_u32(surface), rounding)
        border = blend(semantic_color("border"), accent, opening * .22)
        draw.add_rect(pos, (pos.x + width, bottom), imgui.get_color_u32(border), rounding)
        if body:
            draw.add_line((pos.x + pad, header_end), (pos.x + width - pad, header_end),
                          imgui.get_color_u32(imgui.Col_.border))
        indicator = imgui.ImVec4(accent.x, accent.y, accent.z, accent.w * opening)
        draw.add_rect_filled((pos.x, pos.y + scaled(12)), (pos.x + scaled(3), header_end - scaled(12)),
                             imgui.get_color_u32(indicator), scaled(2))
        if focused:
            draw.add_rect((pos.x + scaled(3), pos.y + scaled(3)),
                          (pos.x + width - scaled(3), header_end - scaled(3)),
                          imgui.get_color_u32(imgui.Col_.nav_cursor), scaled(7))
        layers.merge(draw)
        imgui.pop_id()
        imgui.end_group()


def action_button(label: str, *, primary: bool = False, width: float = 0,
                  height: float = 0, icon: str = "", enabled: bool = True) -> bool:
    if not enabled:
        imgui.begin_disabled()
    if primary:
        accent = semantic_color("accent")
        luminance = 0.2126 * accent.x + 0.7152 * accent.y + 0.0722 * accent.z
        text = (0.08, 0.08, 0.09, 1) if luminance > 0.45 else (1, 1, 1, 1)
        imgui.push_style_color(imgui.Col_.button, accent)
        imgui.push_style_color(imgui.Col_.button_hovered,
                               imgui.ImVec4(min(accent.x + .08, 1), min(accent.y + .08, 1), min(accent.z + .08, 1), 1))
        imgui.push_style_color(imgui.Col_.button_active, accent)
        imgui.push_style_color(imgui.Col_.text, imgui.ImVec4(*text))
    imgui.push_style_var(imgui.StyleVar_.frame_rounding, scaled(6))
    try:
        if icon and imgui.get_font_baked().find_glyph_no_fallback(ord(icon[0])):
            caption = f"{icon}   {label.split('##', 1)[0]}"
            text_size = imgui.calc_text_size(caption)
            if width == 0:
                width = text_size.x + 2 * imgui.get_style().frame_padding.x
            # Keep the native button's ID, focus handling and disabled behavior.
            imgui.push_style_color(imgui.Col_.text, imgui.ImVec4(0, 0, 0, 0))
            clicked = imgui.button(label, imgui.ImVec2(width, height))
            imgui.pop_style_color()
            lo, hi = imgui.get_item_rect_min(), imgui.get_item_rect_max()
            draw = imgui.get_window_draw_list()
            draw.push_clip_rect(lo, hi, True)
            draw.add_text(((lo.x + hi.x - text_size.x) / 2, (lo.y + hi.y - text_size.y) / 2),
                          imgui.get_color_u32(imgui.Col_.text), caption)
            draw.pop_clip_rect()
        else:
            clicked = imgui.button(label, imgui.ImVec2(width, height))
        return clicked and enabled
    finally:
        imgui.pop_style_var()
        if primary:
            imgui.pop_style_color(4)
        if not enabled:
            imgui.end_disabled()


@dataclass
class InteractionState:
    values: dict = field(default_factory=dict)

    def animate(self, identifier: int, target: float, now: float) -> float:
        value, previous = self.values.get(identifier, (target, now))
        value += (target - value) * (1 - math.exp(-max(0, now - previous) / .10))
        self.values[identifier] = (value, now)
        if len(self.values) > 256:
            self.values = {key: state for key, state in self.values.items() if now - state[1] < 2}
        return value


def navigation_item(identifier: str, label: str, *, selected: bool = False,
                    icon: str = "", detail: str = "", running: bool = False,
                    state: InteractionState | None = None) -> bool:
    imgui.push_id(identifier)
    try:
        width = imgui.get_content_region_avail().x
        pad = scaled(12)
        icon_width = scaled(28) if icon else 0
        wrap_width = max(scaled(40), width - 2 * pad - icon_width)
        text_size = imgui.calc_text_size(label, wrap_width=wrap_width)
        detail_text = (detail or "Running…") if running else detail
        detail_height = imgui.calc_text_size(detail_text, wrap_width=wrap_width).y + scaled(3) if detail_text else 0
        height = max(scaled(48), text_size.y + detail_height + 2 * pad)
        pos = imgui.get_cursor_screen_pos()
        clicked = imgui.selectable("##select", selected, size=imgui.ImVec2(width, height))[0]
        hovered = imgui.is_item_hovered()
        if state is not None:
            alpha = state.animate(imgui.get_id("##animation"), 1.0 if selected else .4 if hovered else 0.0, imgui.get_time())
        else:
            alpha = 1.0 if selected else 0.0
        draw = imgui.get_window_draw_list()
        accent = semantic_color("accent")
        accent.w *= alpha * imgui.get_style().alpha
        draw.add_rect_filled((pos.x, pos.y + scaled(5)), (pos.x + scaled(3), pos.y + height - scaled(5)),
                             imgui.get_color_u32(accent), scaled(2))
        color = imgui.get_color_u32(imgui.Col_.text)
        if icon:
            glyph = imgui.get_font_baked().find_glyph_no_fallback(ord(icon[0]))
            draw.add_text((pos.x + pad, pos.y + pad), color, icon if glyph else label[:1])
        draw.add_text(imgui.get_font(), imgui.get_font_size(), (pos.x + pad + icon_width, pos.y + pad),
                      color, label, wrap_width=wrap_width)
        if detail or running:
            draw.add_text(imgui.get_font(), imgui.get_font_size(), (pos.x + pad + icon_width, pos.y + pad + text_size.y + scaled(3)),
                          imgui.get_color_u32(imgui.Col_.slider_grab if running else imgui.Col_.text_disabled),
                          detail_text, wrap_width=wrap_width)
        return clicked
    finally:
        imgui.pop_id()


def toggle(label: str, value: bool) -> tuple[bool, bool]:
    pos = imgui.get_cursor_screen_pos()
    width = imgui.get_content_region_avail().x
    track_width, track_height, gap = scaled(34), scaled(20), scaled(10)
    text = label.split("##", 1)[0]
    wrap = max(scaled(20), width - track_width - gap)
    text_height = imgui.calc_text_size(text, wrap_width=wrap).y if text else 0
    height = max(track_height, text_height)
    pressed = imgui.invisible_button(label, (width, height), imgui.ButtonFlags_.enable_nav)
    if pressed:
        value = not value
    draw = imgui.get_window_draw_list()
    top = pos.y + (height - track_height) / 2
    color = imgui.Col_.slider_grab if value else imgui.Col_.frame_bg
    draw.add_rect_filled((pos.x, top), (pos.x + track_width, top + track_height),
                         imgui.get_color_u32(color), track_height / 2)
    radius = track_height / 2 - scaled(3)
    center_x = pos.x + (track_width - track_height / 2 if value else track_height / 2)
    draw.add_circle_filled((center_x, top + track_height / 2), radius, imgui.get_color_u32(imgui.Col_.text))
    if imgui.is_item_focused():
        draw.add_rect(pos, (pos.x + width, pos.y + height), imgui.get_color_u32(imgui.Col_.nav_cursor), scaled(3))
    if text:
        draw.add_text(imgui.get_font(), imgui.get_font_size(), (pos.x + track_width + gap, pos.y),
                      imgui.get_color_u32(imgui.Col_.text), text, wrap_width=wrap)
    return pressed, value


def status_indicator(label: str, role: str = "muted") -> None:
    imgui.push_style_color(imgui.Col_.text, semantic_color(role))
    imgui.text_wrapped(label)
    imgui.pop_style_color()


def ring_stat(identifier: str, label: str, fraction: float | None, *,
              detail: str = "", role: str = "accent", tooltip: str = "") -> None:
    imgui.push_id(identifier)
    imgui.begin_group()
    diameter = scaled(72)
    thickness = scaled(7)
    pos = imgui.get_cursor_screen_pos()
    center = (pos.x + diameter / 2, pos.y + diameter / 2)
    radius = (diameter - thickness) / 2
    draw = imgui.get_window_draw_list()
    draw.add_circle(center, radius, imgui.get_color_u32(imgui.Col_.frame_bg), 64, thickness)
    value = "—" if fraction is None else f"{fraction:.0%}"
    if fraction is not None and fraction > 0:
        draw.path_arc_to(center, radius, -math.pi / 2,
                         -math.pi / 2 + 2 * math.pi * min(1, fraction), 64)
        draw.path_stroke(imgui.get_color_u32(semantic_color(role)), thickness)
    size = imgui.calc_text_size(value)
    draw.add_text((center[0] - size.x / 2, center[1] - size.y / 2),
                  imgui.get_color_u32(imgui.Col_.text), value)
    imgui.dummy((diameter, diameter))
    if imgui.get_content_region_avail().x >= diameter + scaled(120):
        imgui.same_line()
    imgui.begin_group()
    imgui.text_wrapped(label)
    if detail:
        status_indicator(detail, "muted")
    imgui.end_group()
    imgui.end_group()
    if tooltip and imgui.is_item_hovered():
        imgui.set_tooltip(tooltip)
    imgui.pop_id()


def progress_row(identifier: str, label: str, status: str, fraction: float | None,
                 *, detail: str = "", count: str = "", detail_tooltip: str = "") -> None:
    imgui.push_id(identifier)
    try:
        role = {"completed": "success", "error": "error", "running": "accent"}.get(status, "muted")
        status_indicator(label, role)
        if fraction is not None or status == "running":
            value = -imgui.get_time() if fraction is None else fraction
            imgui.progress_bar(value, imgui.ImVec2(-1, scaled(7)), "")
        if count:
            imgui.text_disabled(count)
        if detail:
            imgui.push_style_color(imgui.Col_.text, semantic_color("muted"))
            imgui.text_wrapped(detail)
            imgui.pop_style_color()
            if detail_tooltip and imgui.is_item_hovered():
                imgui.set_tooltip(detail_tooltip)
        imgui.spacing()
    finally:
        imgui.pop_id()


def loading_panel(title: str, message: str, fraction: float | None, *,
                  history: list[str] | tuple[str, ...] = (), bounds: tuple | None = None) -> None:
    pos, size = bounds if bounds is not None else (imgui.get_window_pos(), imgui.get_window_size())
    if size.x <= 0 or size.y <= 0:
        return
    storage = imgui.get_state_storage()
    frame_id, start_id = imgui.get_id("##loader_frame"), imgui.get_id("##loader_start")
    frame, now = imgui.get_frame_count(), imgui.get_time()
    if frame - storage.get_int(frame_id, -2) > 1:
        storage.set_float(start_id, now)
    storage.set_int(frame_id, frame)
    elapsed = max(0, int(now - storage.get_float(start_id, now)))
    timer = f"{elapsed // 60:02}:{elapsed % 60:02}"

    pad, gap, bar_height = scaled(16), scaled(10), scaled(10)
    margin = min(scaled(24), size.x * .04, size.y * .04)
    width = max(1, min(scaled(560), size.x - 2 * margin))
    text_width = max(1, width - 2 * pad)
    line_height = imgui.get_text_line_height()
    timer_width = imgui.calc_text_size(timer).x
    title_width = max(1, text_width - timer_width - gap)
    title = title or "Working"
    message = message or "Starting..."
    title_height = imgui.calc_text_size(title, wrap_width=title_width).y
    message_height = imgui.calc_text_size(message, wrap_width=text_width).y
    fraction = min(1, max(0, fraction)) if fraction is not None else None
    caption = f"{fraction:.0%} complete" if fraction is not None else "Progress will update as stages report back."
    caption_height = imgui.calc_text_size(caption, wrap_width=text_width).y
    fixed_height = 2 * pad + title_height + 4 * gap + 1 + bar_height + caption_height
    message_height = min(message_height, max(line_height, size.y - 2 * margin - fixed_height))
    entries = [entry for entry in history if entry][-3:]

    def history_height():
        return (line_height + 2 * gap + sum(imgui.calc_text_size(entry, wrap_width=max(1, text_width - scaled(16))).y + gap
                                           for entry in entries)) if entries else 0

    while entries and fixed_height + message_height + history_height() > size.y - 2 * margin:
        entries.pop(0)
    height = fixed_height + message_height + history_height()
    left, top = pos.x + (size.x - width) / 2, pos.y + (size.y - height) / 2
    draw = imgui.get_foreground_draw_list()
    draw.push_clip_rect(pos, (pos.x + size.x, pos.y + size.y), True)
    draw.add_rect_filled(pos, (pos.x + size.x, pos.y + size.y), imgui.get_color_u32((0, 0, 0, .58)))
    draw.add_rect_filled((left, top), (left + width, top + height), imgui.get_color_u32(imgui.Col_.popup_bg), scaled(6))
    draw.add_rect((left, top), (left + width, top + height), imgui.get_color_u32(imgui.Col_.border), scaled(6))
    x, y = left + pad, top + pad
    text_color, muted_color = imgui.get_color_u32(imgui.Col_.text), imgui.get_color_u32(imgui.Col_.text_disabled)
    draw.add_text(imgui.get_font(), imgui.get_font_size(), (x, y), text_color, title, wrap_width=title_width)
    draw.add_text((left + width - pad - timer_width, y), muted_color, timer)
    y += title_height + gap
    draw.add_line((x, y), (x + text_width, y), imgui.get_color_u32(imgui.Col_.border))
    y += gap
    draw.push_clip_rect((x, y), (x + text_width, y + message_height), True)
    draw.add_text(imgui.get_font(), imgui.get_font_size(), (x, y), text_color, message, wrap_width=text_width)
    draw.pop_clip_rect()
    y += message_height + gap
    draw.add_rect_filled((x, y), (x + text_width, y + bar_height), imgui.get_color_u32(imgui.Col_.frame_bg), bar_height / 2)
    if fraction is not None:
        start, end = 0, fraction
    else:
        start = (math.sin(now * 2) + 1) * .4
        end = start + .2
    if end > start:
        draw.add_rect_filled((x + text_width * start, y), (x + text_width * end, y + bar_height),
                             imgui.get_color_u32(imgui.Col_.plot_histogram), bar_height / 2)
    draw.add_rect((x, y), (x + text_width, y + bar_height), imgui.get_color_u32(imgui.Col_.border), bar_height / 2)
    y += bar_height + gap
    draw.add_text(imgui.get_font(), imgui.get_font_size(), (x, y), muted_color, caption, wrap_width=text_width)
    if entries:
        y += caption_height + 2 * gap
        draw.add_text((x, y), muted_color, "Recent stages")
        y += line_height + gap
        for entry in entries:
            draw.add_circle_filled((x + scaled(4), y + line_height / 2), scaled(3), text_color)
            draw.add_text(imgui.get_font(), imgui.get_font_size(), (x + scaled(16), y), text_color, entry,
                          wrap_width=max(1, text_width - scaled(16)))
            y += imgui.calc_text_size(entry, wrap_width=max(1, text_width - scaled(16))).y + gap
    draw.pop_clip_rect()

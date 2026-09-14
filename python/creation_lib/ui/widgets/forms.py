from imgui_bundle import imgui

from .modern import action_button, scaled
from .pick_folder import pick_file, pick_save_file

LABEL_COL_W = 160


def begin_form(identifier: str, label_width: float = LABEL_COL_W) -> bool:
    width = imgui.get_content_region_avail().x
    if not imgui.begin_table(identifier, 2, imgui.TableFlags_.sizing_fixed_fit):
        return False
    imgui.table_setup_column("Label", imgui.TableColumnFlags_.width_fixed, min(scaled(label_width), width * .36))
    imgui.table_setup_column("Value", imgui.TableColumnFlags_.width_stretch)
    return True


def end_form() -> None:
    imgui.end_table()


def form_row_label(label: str) -> None:
    imgui.table_next_row()
    imgui.table_set_column_index(0)
    imgui.align_text_to_frame_padding()
    imgui.text_wrapped(label)
    imgui.table_set_column_index(1)


def draw_combo_field(label: str, items: list[str], index: int) -> tuple[bool, int]:
    form_row_label(label)
    imgui.set_next_item_width(-1)
    return imgui.combo(f"##{label}", index, items)


def draw_path_row(label: str, path: str, btn_label: str = "Browse…") -> tuple[str, bool]:
    form_row_label(label)
    available = imgui.get_content_region_avail().x
    button_width = imgui.calc_text_size(btn_label).x + imgui.get_style().frame_padding.x * 2
    gap = imgui.get_style().item_spacing.x
    inline = available > button_width + gap + scaled(90)
    imgui.set_next_item_width(available - button_width - gap if inline else -1)
    _, path = imgui.input_text(f"##{label}_path", path, imgui.InputTextFlags_.read_only)
    if imgui.is_item_hovered():
        imgui.set_tooltip(path)
    if inline:
        imgui.same_line()
    return path, imgui.button(f"{btn_label}##{label}_browse")


def draw_int_field(label: str, value: int, step: int = 1, step_fast: int = 10,
                   min_val: int | None = None, max_val: int | None = None) -> tuple[bool, int]:
    """Draw a labeled integer input as a form table row."""
    form_row_label(label)
    imgui.set_next_item_width(-1)
    changed, new_val = imgui.input_int(f"##{label}", value, step, step_fast)
    if min_val is not None:
        new_val = max(min_val, new_val)
    if max_val is not None:
        new_val = min(max_val, new_val)
    return changed, new_val


def draw_float_field(label: str, value: float, step: float = 0.1, step_fast: float = 1.0,
                     fmt: str = "%.2f", min_val: float | None = None,
                     max_val: float | None = None) -> tuple[bool, float]:
    """Draw a labeled float input as a form table row."""
    form_row_label(label)
    imgui.set_next_item_width(-1)
    changed, new_val = imgui.input_float(f"##{label}", value, step, step_fast, fmt)
    if min_val is not None:
        new_val = max(min_val, new_val)
    if max_val is not None:
        new_val = min(max_val, new_val)
    return changed, new_val


def draw_text_field(label: str, value: str) -> tuple[bool, str]:
    """Draw a labeled text input as a form table row."""
    form_row_label(label)
    imgui.set_next_item_width(-1)
    changed, new_val = imgui.input_text(f"##{label}", value)
    return changed, new_val


def draw_run_cancel_buttons(running: bool, can_run: bool = True) -> tuple[bool, bool]:
    run = action_button("Run", primary=True, width=scaled(120), enabled=can_run and not running)
    cancel = False
    if running:
        if imgui.get_content_region_avail().x >= scaled(250):
            imgui.same_line()
        cancel = action_button("Cancel", width=scaled(120))
    return run, cancel

from pathlib import Path
import subprocess
import sys

import pytest


@pytest.mark.parametrize("theme,scale", [("falloutnv", 1), ("starfield", 1.5), ("dracula", 2)])
def test_expandable_section_with_real_imgui(theme, scale):
    result = subprocess.run(
        [sys.executable, str(Path(__file__)), theme, str(scale)],
        capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def _check_native_ui(theme, scale, screenshot=None):
    from imgui_bundle import hello_imgui, imgui, immapp
    from creation_lib.ui.theme import get_theme
    from creation_lib.ui.theme.appearance import configure_runner_appearance
    from creation_lib.ui.widgets.modern import expandable_section, scaled

    params = hello_imgui.RunnerParams()
    params.ini_disable = True
    params.app_window_params.hidden = True
    params.app_window_params.window_geometry.size = (int(800 * scale), int(760 * scale))
    params.app_window_params.window_geometry.window_size_measure_mode = hello_imgui.WindowSizeMeasureMode.screen_coords
    params.dpi_aware_params.dpi_window_size_factor = scale
    state = {"frame": 0, "workers": 4, "action": False}
    positions = {}

    def remember(key):
        lo, hi = imgui.get_item_rect_min(), imgui.get_item_rect_max()
        positions[key] = ((lo.x + hi.x) / 2, (lo.y + hi.y) / 2)

    def events():
        frame = state["frame"]
        io = imgui.get_io()
        io.add_focus_event(True)
        for key, down, up in ((imgui.Key.space, 3, 4), (imgui.Key.left_arrow, 7, 8),
                              (imgui.Key.enter, 19, 20)):
            if frame in (down, up):
                io.add_key_event(key, frame == down)
        for key, down, up in (("header", 11, 12), ("slider", 15, 16),
                              ("disabled", 23, 24), ("action", 27, 28)):
            if frame == down - 1:
                io.add_mouse_pos_event(*positions[key])
            if frame in (down, up):
                if key == "slider":
                    io.add_key_event(imgui.Key.mod_ctrl, frame == down)
                io.add_mouse_button_event(0, frame == down)
        if frame == 17:
            io.add_input_characters_utf8("7")

    def header_actions(expanded):
        state["open"] = expanded
        remember("header")

    def draw():
        frame = state["frame"]
        imgui.get_io().config_nav_cursor_visible_always = True
        if frame == 1:
            imgui.set_keyboard_focus_here()
        with expandable_section("Advanced##keyboard", description="Archive layout, compression & textures",
                                header_actions=header_actions) as expanded:
            if expanded:
                assert imgui.get_content_region_avail().x > scaled(600), "Card body collapsed to its content width"
                _, state["workers"] = imgui.slider_int("Workers", state["workers"], 0, 16)
                remember("slider")
                with expandable_section("Installation details", imgui.TreeNodeFlags_.default_open,
                                        description="Nested tables retain their own layout") as details:
                    if details and imgui.begin_table("paths", 2):
                        imgui.table_next_column()
                        imgui.text("Source")
                        imgui.table_next_column()
                        imgui.text_wrapped("C:/Games/A very long installation path with additional folder names/Data")
                        imgui.end_table()
        if frame in (5, 14, 30):
            assert state["open"], f"Header did not open at frame {frame}"
        if frame == 9:
            assert not state["open"], "Left arrow did not collapse the focused header"

        imgui.begin_disabled()
        with expandable_section("Disabled section", header_actions=lambda _open: remember("disabled")) as expanded:
            assert not expanded, "Disabled header accepted input"
        imgui.end_disabled()

        def action_header(expanded):
            assert not expanded, "Header action unexpectedly toggled the section"
            if imgui.button("Action"):
                state["action"] = True
            remember("action")

        with expandable_section("Header with an independent action", header_actions=action_header,
                                actions_width=scaled(80)) as expanded:
            assert not expanded

        with expandable_section("Same title##closed") as expanded:
            assert not expanded
        with expandable_section("Same title##open", imgui.TreeNodeFlags_.default_open) as expanded:
            assert expanded
            if expanded:
                imgui.text_wrapped("Distinct IDs keep their open states independent.")
        imgui.set_next_item_open(True)
        with expandable_section("A longer heading that wraps when its card has limited horizontal space") as expanded:
            assert expanded
            imgui.text_wrapped("Open state can still be set with the public ImGui API.")

        state["frame"] += 1
        if state["frame"] == 34:
            assert state["workers"] == 7, "Ctrl-click exact-value entry failed inside the card"
            assert state["action"], "Header action did not receive its click"
            params.app_shall_exit = True

    params.callbacks.pre_new_frame = events
    params.callbacks.show_gui = draw
    configure_runner_appearance(params, get_theme(theme))
    immapp.run(params)
    if screenshot:
        from PIL import Image

        Image.fromarray(hello_imgui.final_app_window_screenshot()).save(screenshot)


if __name__ == "__main__":
    _check_native_ui(sys.argv[1], float(sys.argv[2]), sys.argv[3] if len(sys.argv) > 3 else None)

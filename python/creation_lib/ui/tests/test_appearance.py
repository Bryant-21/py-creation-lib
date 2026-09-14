from pathlib import Path
import subprocess
import sys


def test_modern_appearance_with_real_imgui():
    result = subprocess.run(
        [sys.executable, str(Path(__file__))], capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def _check_native_ui():
    from unittest.mock import patch

    from imgui_bundle import hello_imgui, icons_fontawesome_6 as fa, imgui, immapp
    from creation_lib.ui.theme import apply_theme, apply_tab_style, get_theme
    from creation_lib.ui.theme.appearance import configure_runner_appearance, load_ui_fonts
    from creation_lib.ui.widgets.modern import action_button, heading, navigation_item, section, toggle

    params = hello_imgui.RunnerParams()
    params.ini_disable = True
    params.app_window_params.hidden = True
    params.app_window_params.window_geometry.size = (800, 600)
    params.dpi_aware_params.dpi_window_size_factor = 1
    state = {"frame": 0, "toggle": False, "disabled": False, "nav": False, "action": False, "workers": 4}

    def checks():
        imgui.get_io().config_nav_cursor_visible_always = True
        style = imgui.get_style()
        for scale in (1, 1.5, 2, 1):
            with patch("creation_lib.ui.theme.appearance.hello_imgui.dpi_window_size_factor", return_value=scale):
                for theme_id in ("falloutnv", "starfield", "dracula"):
                    theme = get_theme(theme_id)
                    apply_theme(theme)
                    for _ in range(5):
                        apply_tab_style(theme)
                    assert tuple(style.window_padding) == (16 * scale, 16 * scale)
                    assert style.frame_rounding == 4 * scale
                    background = imgui.get_style_color_vec4(imgui.Col_.window_bg)
                    assert (background.x > .5) == theme.light
                    assert tuple(imgui.get_style_color_vec4(imgui.Col_.check_mark)) == tuple(imgui.ImVec4(*theme.checkmark))
        with patch("creation_lib.ui.theme.appearance.hello_imgui.load_font_ttf_with_font_awesome_icons", side_effect=RuntimeError("fixture font unavailable")):
            assert load_ui_fonts().body is None
        apply_theme(get_theme("falloutnv"))

    def events():
        frame = state["frame"]
        io = imgui.get_io()
        io.add_focus_event(True)
        if frame in (3, 9, 15):
            io.add_key_event(imgui.Key.space, True)
        if frame in (4, 10, 16):
            io.add_key_event(imgui.Key.space, False)
        if frame == 17:
            io.add_key_event(imgui.Key.mod_ctrl, True)
            io.add_mouse_pos_event(*state["slider_center"])
            io.add_mouse_button_event(0, True)
        if frame == 18:
            io.add_mouse_button_event(0, False)
            io.add_key_event(imgui.Key.mod_ctrl, False)
        if frame == 19:
            io.add_input_characters_utf8("7")
        if frame == 20:
            io.add_key_event(imgui.Key.enter, True)
        if frame == 21:
            io.add_key_event(imgui.Key.enter, False)

    def draw():
        frame = state["frame"]
        body_size = imgui.get_font_size()
        heading("Shared components", large=True)
        assert imgui.get_font_size() == body_size
        with section("keyboard", "Keyboard interaction") as visible:
            if visible:
                if frame == 1:
                    imgui.set_keyboard_focus_here()
                _, state["toggle"] = toggle("Toggle with Space", state["toggle"])
                imgui.begin_disabled()
                _, state["disabled"] = toggle("Disabled toggle", state["disabled"])
                imgui.end_disabled()
                assert not action_button("Disabled action", enabled=False, primary=True,
                                         height=48, icon=fa.ICON_FA_PLAY)
                if frame == 7:
                    imgui.set_keyboard_focus_here()
                if navigation_item("project", "Select with Space", selected=state["nav"]):
                    state["nav"] = True
                if frame == 13:
                    imgui.set_keyboard_focus_here()
                if action_button("Action with Space", height=52, width=220, icon=fa.ICON_FA_PLAY):
                    state["action"] = True
                _, state["workers"] = imgui.slider_int("Workers", state["workers"], 0, 16)
                lo, hi = imgui.get_item_rect_min(), imgui.get_item_rect_max()
                state["slider_center"] = ((lo.x + hi.x) / 2, (lo.y + hi.y) / 2)
        state["frame"] += 1
        if state["frame"] == 25:
            assert state["toggle"], "Space did not activate the focused toggle"
            assert state["nav"], "Space did not activate the focused navigation item"
            assert state["action"], "Space did not activate the focused icon button"
            assert not state["disabled"]
            assert state["workers"] == 7, "Ctrl-click exact-value entry failed"
            params.app_shall_exit = True

    params.callbacks.post_init = checks
    params.callbacks.pre_new_frame = events
    params.callbacks.show_gui = draw
    configure_runner_appearance(params, get_theme("falloutnv"))
    immapp.run(params)


if __name__ == "__main__":
    _check_native_ui()

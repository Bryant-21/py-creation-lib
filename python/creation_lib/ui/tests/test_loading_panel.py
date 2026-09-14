from pathlib import Path
import subprocess
import sys

import pytest


@pytest.mark.parametrize("theme,scale", [("falloutnv", 1), ("fallout76", 1.5), ("starfield", 2)])
def test_loading_panel_native(theme, scale, tmp_path):
    result = subprocess.run(
        [sys.executable, str(Path(__file__)), theme, str(scale), str(tmp_path / "loader.png")],
        capture_output=True, text=True, timeout=30,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def _check_native(theme_id, scale, output):
    from unittest.mock import patch

    from imgui_bundle import hello_imgui, imgui, immapp
    from PIL import Image

    from creation_lib.ui.theme import get_theme
    from creation_lib.ui.theme.appearance import configure_runner_appearance
    from creation_lib.ui.widgets.modern import loading_panel, scaled

    state = {"frame": 0, "positions": []}
    get_draw_list = imgui.get_foreground_draw_list

    class DrawRecorder:
        def __init__(self):
            self.draw = get_draw_list()
            self.rects = []
            self.text = []

        def add_rect_filled(self, lo, hi, color, *args):
            self.rects.append((tuple(lo), tuple(hi), color))
            self.draw.add_rect_filled(lo, hi, color, *args)

        def add_text(self, *args, **kwargs):
            self.text.append(args[2] if len(args) == 3 else args[4])
            self.draw.add_text(*args, **kwargs)

        def __getattr__(self, name):
            return getattr(self.draw, name)

    params = hello_imgui.RunnerParams()
    params.ini_disable = True
    params.app_window_params.hidden = True
    params.app_window_params.window_geometry.size = (int(640 * scale), int(420 * scale))
    params.dpi_aware_params.dpi_window_size_factor = scale

    def draw():
        frame = state["frame"]
        if frame == 8:
            state["frame"] += 1
            return
        fraction = {4: 0, 5: .5, 6: 1.5, 7: -.2}.get(frame, None if frame < 4 else .65)
        if frame == 5:
            imgui.push_style_color(imgui.Col_.plot_histogram, (.8, .3, .6, 1))
        foreground = imgui.get_color_u32(imgui.Col_.plot_histogram)
        background = imgui.get_color_u32(imgui.Col_.frame_bg)
        recorder = DrawRecorder()
        with patch.object(imgui, "get_foreground_draw_list", return_value=recorder):
            loading_panel(
                "Preparing assets", "Processing a long folder name with spaces / textures / architecture / stone.dds",
                fraction, history=["Reading source files", "Preparing output", "Processing assets"],
            )
        track = next((lo, hi) for lo, hi, color in recorder.rects
                     if color == background and hi[1] - lo[1] == pytest.approx(scaled(10)))
        fills = [(lo, hi) for lo, hi, color in recorder.rects if color == foreground]
        expected = .2 if fraction is None else max(0, min(1, fraction))
        if expected:
            assert len(fills) == 1
            lo, hi = fills[0]
            assert hi[0] - lo[0] == pytest.approx((track[1][0] - track[0][0]) * expected)
            assert track[0][0] <= lo[0] <= hi[0] <= track[1][0]
            if fraction is None:
                state["positions"].append(lo[0])
        else:
            assert not fills
        assert "Recent stages" in recorder.text
        if fraction is None:
            assert "Progress will update as stages report back." in recorder.text
            assert not any("% complete" in text for text in recorder.text)
        else:
            assert f"{expected:.0%} complete" in recorder.text
        if frame == 5:
            imgui.pop_style_color()
        if frame == 9:
            assert imgui.get_state_storage().get_float(imgui.get_id("##loader_start")) == pytest.approx(imgui.get_time())
        if frame == 11:
            assert len(set(state["positions"])) > 1
            params.app_shall_exit = True
        state["frame"] += 1

    params.callbacks.show_gui = draw
    configure_runner_appearance(params, get_theme(theme_id))
    immapp.run(params)
    output.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(hello_imgui.final_app_window_screenshot()).save(output)


if __name__ == "__main__":
    _check_native(sys.argv[1], float(sys.argv[2]), Path(sys.argv[3]))

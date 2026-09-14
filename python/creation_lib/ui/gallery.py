from __future__ import annotations

import argparse
from pathlib import Path

from imgui_bundle import hello_imgui, imgui, immapp

from creation_lib.ui.theme import get_theme
from creation_lib.ui.theme.appearance import configure_runner_appearance
from creation_lib.ui.widgets.forms import begin_form, draw_combo_field, draw_path_row, end_form
from creation_lib.ui.widgets.modern import (
    InteractionState, action_button, expandable_section, heading, navigation_item, progress_row, ring_stat, scaled, section, toggle,
)


class ComponentGallery:
    def __init__(self):
        self.selected = "overview"
        self.enabled = True
        self.choice = 0
        self.workers = 4
        self.interaction = InteractionState()

    def draw(self):
        if not imgui.begin_table("gallery", 2):
            return
        imgui.table_setup_column("Navigation", imgui.TableColumnFlags_.width_fixed, scaled(230))
        imgui.table_setup_column("Components", imgui.TableColumnFlags_.width_stretch)
        imgui.table_next_column()
        for key, label in (("overview", "Overview"), ("long", "A project with a longer name"), ("run", "Background task")):
            if navigation_item(key, label, selected=key == self.selected, running=key == "run", state=self.interaction):
                self.selected = key
        imgui.table_next_column()
        heading("Shared ImGui components", large=True)
        with section("form", "Configuration") as visible:
            if visible:
                if begin_form("options"):
                    _, self.choice = draw_combo_field("Output", ["Packed", "Loose"], self.choice)
                    draw_path_row("Folder", "C:/Projects/A project with a long folder name/output")
                    end_form()
                _, self.enabled = toggle("Enable processing", self.enabled)
                _, self.workers = imgui.slider_int("Workers", self.workers, 0, 16)
                action_button("Run", primary=True, width=scaled(120))
                imgui.same_line()
                action_button("Unavailable", enabled=False)
        with section("capacity", "Capacity comparison") as visible:
            if visible and imgui.begin_table("capacity_stats", 2):
                imgui.table_next_column()
                ring_stat("current", "Current use", .61, detail="780 GB free")
                imgui.table_next_column()
                ring_stat("planned", "After processing", .85, detail="480 GB estimated", role="success")
                imgui.end_table()
        with expandable_section("Advanced", imgui.TreeNodeFlags_.default_open,
                                description="Processing and output options") as expanded:
            if expanded:
                _, self.enabled = toggle("Enable processing##advanced", self.enabled)
                _, self.workers = imgui.slider_int("Workers##advanced", self.workers, 0, 16)
                with expandable_section("Installation details", description="Folders and diagnostics") as details:
                    if details:
                        if begin_form("installation"):
                            draw_path_row("Folder", "C:/Projects/A project with a long folder name/output")
                            end_form()
        with section("progress", "Task progress") as visible:
            if visible:
                progress_row("complete", "Preparation", "completed", 1.0, count="Completed")
                progress_row("running", "Processing", "running", .62, count="62 / 100", detail="example.asset")
                progress_row("unknown", "Waiting for results", "running", None)
                progress_row("failure", "Failed operation", "error", None, detail="Example error message")
        imgui.end_table()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--theme", default="falloutnv")
    parser.add_argument("--scale", type=float, default=1)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=900)
    parser.add_argument("--screenshot", type=Path)
    args = parser.parse_args()
    gallery = ComponentGallery()
    params = hello_imgui.RunnerParams()
    params.app_window_params.window_title = "creation_lib.ui — Component gallery"
    params.app_window_params.window_geometry.size = (args.width, args.height)
    params.app_window_params.window_geometry.window_size_measure_mode = hello_imgui.WindowSizeMeasureMode.screen_coords
    params.app_window_params.hidden = args.screenshot is not None
    params.ini_disable = True
    params.dpi_aware_params.dpi_window_size_factor = args.scale
    frames = 0

    def draw():
        nonlocal frames
        gallery.draw()
        frames += 1
        if args.screenshot and frames >= 8:
            params.app_shall_exit = True

    params.callbacks.show_gui = draw
    configure_runner_appearance(params, get_theme(args.theme))
    immapp.run(params)
    if args.screenshot:
        from PIL import Image

        args.screenshot.parent.mkdir(parents=True, exist_ok=True)
        Image.fromarray(hello_imgui.final_app_window_screenshot()).save(args.screenshot)


if __name__ == "__main__":
    main()

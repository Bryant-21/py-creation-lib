# Shared ImGui UI

The modern appearance and widgets are reusable Python components for ImGui
1.92.8 (`imgui-bundle>=1.92.801`). They have no BACUP project, conversion,
runner, or settings dependencies. Install `py-creation-lib[ui]` when using the
library outside this repository.

`apply_theme(theme)` and `apply_tab_style(theme)` apply graphite surfaces,
semantic dark/light colors, rounded controls, and consistent spacing. ModBox21,
its standalone tools, and BACUP use this single appearance. The former
`appearance` argument and classic rendering branches have been removed.
Existing theme IDs still control the accent; `falloutnv` is the default amber.

Canvas windows override `StyleVar_.window_padding` to `(0, 0)` around their
`imgui.begin()` call, then immediately pop the override. User guides use a compact
inset of half the current font size (8 logical pixels). Controls, child dialogs
and other panels retain the theme's spacing.

## Small standalone application

```python
from imgui_bundle import hello_imgui, immapp
from creation_lib.ui.theme import configure_runner_appearance, get_theme
from creation_lib.ui.widgets.modern import action_button, section, toggle

enabled = True

def draw():
    global enabled
    with section("options", "Configuration") as visible:
        if visible:
            _, enabled = toggle("Enable processing", enabled)
            if action_button("Run", primary=True, enabled=enabled):
                print("Run requested")

params = hello_imgui.RunnerParams()
params.callbacks.show_gui = draw
configure_runner_appearance(params, get_theme("falloutnv"))
immapp.run(params)
```

The bootstrap chains existing initialization/frame callbacks and installs the
bundled font loader. It reapplies appearance after each new frame so backend
theme resets do not change the controls. Set callbacks before calling it.
Hosts managing their own theme selection can call `apply_tab_style` with the
current theme every frame instead. Style dimensions are assigned from logical
values and the current DPI factor; they are never repeatedly multiplied.
Hosts with a separate UI zoom preference can call `apply_modern_metrics(scale)`
after applying the theme, passing the combined DPI and zoom factor.

## Theme editor

`creation_lib.ui.theme.editor.ThemeEditor` provides a resizable dialog with a
searchable list of every ImGui color plus the shared success, warning and error
colors. It includes hex/RGBA entry, native color pickers, tab-state swatches,
control previews, individual resets, and Save/Cancel. Selected tab backgrounds
are separate resolved colors (`tab_selected` and `tab_dimmed_selected`).
New Vegas uses dark gold tabs (`#795C14`) with darker tab/input hover states
(`#604B14`) and checkbox fills (`#514015`) to keep checkmarks readable.
Fallout 76 uses yellow gold (`#9B7A1B`) with brighter hover states (`#C9A227`),
while the remaining presets blend the theme accent into the surface color.
Checkmarks use their own color; New Vegas uses brighter amber (`#FFB642`) for
contrast, and Fallout 76 uses pale gold (`#FFF0BE`). Shared component accents
follow `slider_grab`.

Call `editor.open(theme_id, overrides_by_theme)` to create an isolated draft.
While `editor.is_open`, apply `get_theme(editor.theme_id)` with
`color_overrides=editor.current_overrides` each frame. `editor.draw()` returns
`"save"`, `"cancel"`, or `None`. On Save, persist `editor.theme_id` and
`editor.overrides`; on Cancel, resume applying the original theme and overrides.
The editor does not access settings files itself.

`get_theme_colors(theme, color_overrides=None)` returns the complete resolved
palette. `apply_theme`, `apply_tab_style`, and `configure_runner_appearance` accept
the same optional `color_overrides` dictionary. Keys are stable ImGui names such
as `tab_selected`, or `status_success`, `status_warning`, and `status_error`.
Values are four normalized RGBA channels. Unknown names and invalid values are
ignored. ModBox21 and BACUP persist overrides per theme in shared settings and
apply them in their setup windows too.

## Loading screens

Use `loading_panel(title, message, fraction, history=(), bounds=None)` for busy
overlays. It draws a themed card with elapsed time, wrapped messages, a slim
progress bar, and up to three recent stages. Pass `None` for an animated bar
when progress is unknown; known fractions show a percentage. Optional `bounds`
are `(position, size)` for a viewport or content region; otherwise the overlay
stays inside the current window. Call it only while the operation is active.
The host retains control of input blocking, cancellation and runner state.

The bar uses `plot_histogram`, matching native ImGui progress bars and the theme
editor. Surface, border and text colors also come from the active palette.
Builder, archive, voice-browser and editor overlays share this component with
BACUP setup and cleanup. The shared standalone bootstrap applies the modern
default theme on startup and subsequent frames.

## Components

| API | Purpose |
| --- | --- |
| `appearance_tokens(light)` | Semantic surfaces, text/status colors, spacing, typography and rounding |
| `load_ui_fonts()` | Bundled Roboto, Font Awesome 6, small text and Inconsolata; default-font fallback |
| `scaled(value)` | Logical size relative to the current 16 px body font |
| `heading(text, large=False, size=None)` | Wrapped heading with an optional logical font size; restores the previous font |
| `section(id, title, height=0)` | Balanced child/card context; automatic height or a scrolling fixed-height card |
| `expandable_section(label, flags=0, description="")` | Rounded disclosure card with a compact header, padded body, wrapped text and animated accent/chevron |
| `navigation_item(id, label, ...)` | Native selectable with icon, wrapped label, detail, active indicator and optional running state |
| `InteractionState` | Per-view hover/selection interpolation; uses elapsed time and prunes inactive entries |
| `toggle(label, value)` | Switch returning `(changed, value)`; mouse, keyboard, disabled state and focus indicator |
| `action_button(label, primary=False, enabled=True, width=0, height=0, icon="")` | Native button with scoped primary/disabled styling, optional icon and explicit size |
| `ring_stat(id, label, fraction, detail="", role="accent", tooltip="")` | Compact doughnut statistic with an explicit percentage; `None` shows an unknown value |
| `status_indicator(label, role)` | Wrapped semantic status text |
| `progress_row(id, label, status, fraction, ...)` | Compact determinate or indeterminate progress with detail/count/tooltip |
| `loading_panel(title, message, fraction)` | Foreground progress presentation; caller owns operation state and disabling input |
| `prepare_dialog(width, height)` | Centered, DPI-scaled initial size constrained to the viewport |
| `forms.begin_form/end_form` | Two-column form with responsive label width |
| `forms.draw_combo_field/draw_path_row` | Native value controls and responsive file/path rows |
| `forms.draw_int_field/draw_float_field/draw_text_field` | Value fields preserving numeric bounds and exact entry |
| `forms.draw_run_cancel_buttons` | Shared Run/Cancel controls with native disabled states |

`creation_lib.ui.widgets.images.image_in_box(path, position, size)` draws an
image in a bounding box while preserving its aspect ratio and alpha. Encoded
data and GPU textures are cached; HelloImGui releases textures at shutdown.
Use `trim_padding=True` to crop uniform alpha padding through texture coordinates
without modifying the source PNG. `opacity` controls display transparency.

IDs must be stable and unique in their ImGui scope. Keep `InteractionState`
with the view instance. Components do not save values, start jobs, pick folders,
or invoke application callbacks. Consume their return values in the host.
Sliders remain native ImGui sliders, including Ctrl-click exact-value entry.
Use `expandable_section` for expandable settings, FAQs, effect cards and diagnostic
sections across applications. Keep `tree_node` for hierarchical navigation and
structured data trees. The card retains native header IDs, keyboard navigation,
disabled handling, `TreeNodeFlags_.default_open`, and `set_next_item_open` support:

```python
from creation_lib.ui.widgets.modern import expandable_section

with expandable_section("Advanced##output", description="Processing options") as expanded:
    if expanded:
        _, enabled = toggle("Enable processing", enabled)
```

For headers with independent actions, pass `header_actions(open: bool)` and reserve
their width in pixels with `actions_width`. The callback runs with the native header
as the last item (for drag/drop) and the cursor positioned for right-aligned controls.
Use its open value to persist state even when the body is outside the scroll area.
Application callbacks own persistence and effects; the component only presents them.

Animations run only for interaction transitions or active indeterminate work.
Fonts are supplied by `imgui_bundle` assets; icons fall back to the label's first
letter if the current font lacks the requested glyph.
Each font role loads independently, so an unavailable body font does not
prevent the terminal or code editor from receiving its monospace font.

Tool panels import forms from `creation_lib.ui.widgets.forms`. The old
`ui.tools.imgui_helpers` implementation has been removed after migrating its
callers. Application menus, docking, runner ownership and settings remain
the host's responsibility.

## Gallery and checks

```powershell
uv run --no-sync python -m creation_lib.ui.gallery
uv run --no-sync python -m creation_lib.ui.gallery --theme starfield --scale 1.5 --width 1920 --height 1140 --screenshot tmp/gallery.png
uv run --no-sync python -m pytest py_creation_lib/python/creation_lib/ui/tests/test_appearance.py -q
uv run --no-sync python -m pytest py_creation_lib/python/creation_lib/ui/tests/test_expandable_section.py -q
```

The native integration test checks dark/light theme changes, repeated DPI
application, font restoration/failure, Space activation of navigation and
switches, disabled controls, and Ctrl-click slider entry. It runs in a separate
process so application tests that mock `imgui_bundle` cannot hide binding errors.
Disclosure checks cover open/close by keyboard and mouse, disabled sections, nested
tables, independent header actions and exact slider entry at 100%, 150%, and 200%
scaling in dark and light themes.

The implementation uses the [public Dear ImGui controls and drawing API](https://github.com/ocornut/imgui/tree/v1.92.8).
The supplied menu examples informed the visual direction; no modified ImGui
fork or third-party menu assets are included.

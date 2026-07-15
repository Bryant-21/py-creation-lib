"""Shared hello_imgui bootstrap for all Fallout 4 MCP desktop tools.

Provides:
- create_runner_params() — configure hello_imgui.RunnerParams with dark theme + docking
- run_app() — wraps immapp.run()
- AsyncWorker — run a function in a background thread, poll for result
- CommandRunner — stream subprocess output line-by-line via queue
"""

import ctypes
import logging
import os
import queue
import subprocess
import threading
from pathlib import Path

from imgui_bundle import hello_imgui, imgui, immapp

_log = logging.getLogger("imgui_app")


def _apply_darcula_theme():
    """Apply a dark theme similar to Darcula/VS Code Dark."""
    style = imgui.get_style()
    style.window_rounding = 4.0
    style.frame_rounding = 2.0
    style.grab_rounding = 2.0
    style.scrollbar_rounding = 4.0
    style.frame_border_size = 1.0

    sc = style.set_color_
    sc(imgui.Col_.window_bg, imgui.ImVec4(0.12, 0.12, 0.14, 1.0))
    sc(imgui.Col_.child_bg, imgui.ImVec4(0.10, 0.10, 0.12, 1.0))
    sc(imgui.Col_.popup_bg, imgui.ImVec4(0.14, 0.14, 0.16, 1.0))
    sc(imgui.Col_.border, imgui.ImVec4(0.28, 0.28, 0.30, 1.0))
    sc(imgui.Col_.frame_bg, imgui.ImVec4(0.18, 0.18, 0.20, 1.0))
    sc(imgui.Col_.frame_bg_hovered, imgui.ImVec4(0.22, 0.22, 0.25, 1.0))
    sc(imgui.Col_.frame_bg_active, imgui.ImVec4(0.25, 0.25, 0.28, 1.0))
    sc(imgui.Col_.title_bg, imgui.ImVec4(0.10, 0.10, 0.12, 1.0))
    sc(imgui.Col_.title_bg_active, imgui.ImVec4(0.16, 0.16, 0.18, 1.0))
    sc(imgui.Col_.menu_bar_bg, imgui.ImVec4(0.14, 0.14, 0.16, 1.0))
    sc(imgui.Col_.scrollbar_bg, imgui.ImVec4(0.10, 0.10, 0.12, 1.0))
    sc(imgui.Col_.scrollbar_grab, imgui.ImVec4(0.30, 0.30, 0.32, 1.0))
    sc(imgui.Col_.scrollbar_grab_hovered, imgui.ImVec4(0.40, 0.40, 0.42, 1.0))
    sc(imgui.Col_.check_mark, imgui.ImVec4(0.40, 0.70, 1.0, 1.0))
    sc(imgui.Col_.button, imgui.ImVec4(0.22, 0.22, 0.25, 1.0))
    sc(imgui.Col_.button_hovered, imgui.ImVec4(0.30, 0.30, 0.35, 1.0))
    sc(imgui.Col_.button_active, imgui.ImVec4(0.35, 0.50, 0.75, 1.0))
    sc(imgui.Col_.header, imgui.ImVec4(0.22, 0.22, 0.25, 1.0))
    sc(imgui.Col_.header_hovered, imgui.ImVec4(0.28, 0.28, 0.32, 1.0))
    sc(imgui.Col_.header_active, imgui.ImVec4(0.30, 0.45, 0.70, 1.0))
    sc(imgui.Col_.separator, imgui.ImVec4(0.28, 0.28, 0.30, 1.0))
    sc(imgui.Col_.tab, imgui.ImVec4(0.16, 0.16, 0.18, 1.0))
    sc(imgui.Col_.tab_hovered, imgui.ImVec4(0.28, 0.28, 0.32, 1.0))
    sc(imgui.Col_.tab_selected, imgui.ImVec4(0.22, 0.22, 0.28, 1.0))
    sc(imgui.Col_.text, imgui.ImVec4(0.85, 0.85, 0.85, 1.0))
    sc(imgui.Col_.text_disabled, imgui.ImVec4(0.50, 0.50, 0.50, 1.0))


def set_native_dark_title_bar() -> None:
    """Request the native Windows title bar/menu chrome to use dark mode."""
    if os.name != "nt":
        return

    try:
        window_address = hello_imgui.get_glfw_window_address()
        if not window_address:
            return

        import imgui_bundle

        dwmapi = ctypes.WinDLL("dwmapi")
        glfw_dll = os.path.join(os.path.dirname(imgui_bundle.__file__), "glfw3.dll")
        glfw = ctypes.CDLL(glfw_dll)
        glfw.glfwGetWin32Window.restype = ctypes.c_void_p
        glfw.glfwGetWin32Window.argtypes = [ctypes.c_void_p]
        hwnd = glfw.glfwGetWin32Window(ctypes.c_void_p(window_address))
        if not hwnd:
            return

        value = ctypes.c_int(1)
        size = ctypes.sizeof(value)

        # Try the newer and older immersive dark mode attributes.
        for attribute in (20, 19):
            try:
                hr = dwmapi.DwmSetWindowAttribute(
                    ctypes.c_void_p(hwnd),
                    ctypes.c_int(attribute),
                    ctypes.byref(value),
                    ctypes.c_int(size),
                )
                if hr == 0:
                    break
            except Exception:
                continue
    except Exception:
        _log.exception("Could not enable dark title bar")


def set_ini_folder(params: hello_imgui.RunnerParams, app_name: str, ini_dir: Path) -> None:
    """Configure RunnerParams to store .ini in the shared settings/ folder.

    Args:
        params: The RunnerParams to configure.
        app_name: Short name used as the .ini filename (e.g. "toolkit", "setup").
        ini_dir: Directory in which to store the .ini file.
    """
    params.ini_folder_type = hello_imgui.IniFolderType.absolute_path
    params.ini_filename = str(ini_dir / f"{app_name}.ini")


def create_runner_params(
    title: str,
    width: int = 1200,
    height: int = 800,
    gui_fn=None,
    layout_fn=None,
    on_exit_fn=None,
    ini_name: str = "",
) -> hello_imgui.RunnerParams:
    """Create hello_imgui RunnerParams with standard dark theme and docking.

    Args:
        title: Window title.
        width: Initial window width.
        height: Initial window height.
        gui_fn: Callable for the main GUI loop (called each frame).
        layout_fn: Optional callable returning hello_imgui.DockingParams.
        on_exit_fn: Optional callable invoked on app exit.
        ini_name: Short name for the .ini file (stored in settings/ folder).
                  If empty, derived from title.
    """
    params = hello_imgui.RunnerParams()
    params.app_window_params.window_title = title
    params.app_window_params.window_geometry.size = (width, height)

    # Store ini in consolidated settings/ folder
    name = ini_name or title.lower().replace(" ", "_").replace("—", "").strip("_")
    from creation_lib.ui.host import get_host

    set_ini_folder(params, name, get_host().get_ini_dir())

    # Enable docking
    params.imgui_window_params.default_imgui_window_type = (
        hello_imgui.DefaultImGuiWindowType.provide_full_screen_dock_space
    )
    params.imgui_window_params.enable_viewports = False
    params.imgui_window_params.show_menu_bar = False
    params.imgui_window_params.show_status_bar = True

    if gui_fn:
        params.callbacks.show_gui = gui_fn

    # Apply theme on first frame
    _theme_applied = False

    def _post_init():
        nonlocal _theme_applied
        if not _theme_applied:
            _apply_darcula_theme()
            set_native_dark_title_bar()
            _theme_applied = True

    params.callbacks.post_init = _post_init

    if layout_fn:
        params.docking_params = layout_fn()

    if on_exit_fn:
        params.callbacks.before_exit = on_exit_fn

    return params


def run_app(params: hello_imgui.RunnerParams, addons: immapp.AddOnsParams | None = None):
    """Run the hello_imgui application."""
    if addons is None:
        addons = immapp.AddOnsParams()
        addons.with_markdown = True
    immapp.run(runner_params=params, add_ons_params=addons)


# ---------------------------------------------------------------------------
# AsyncWorker — run a function in a background thread
# ---------------------------------------------------------------------------
class AsyncWorker:
    """Run a function in a background thread. Poll done/result each frame.

    Usage:
        worker = AsyncWorker(target_fn=my_search, args=(query,))
        worker.start()
        # Each frame:
        if worker.done:
            result = worker.result
            error = worker.error
    """

    def __init__(self, target_fn, args=(), kwargs=None):
        self.target_fn = target_fn
        self.args = args
        self.kwargs = kwargs or {}
        self.result = None
        self.error = None
        self.done = False
        self._thread = None

    def start(self):
        self.done = False
        self.result = None
        self.error = None
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self):
        try:
            self.result = self.target_fn(*self.args, **self.kwargs)
        except Exception as e:
            self.error = e
        finally:
            self.done = True


# ---------------------------------------------------------------------------
# CommandRunner — stream subprocess output
# ---------------------------------------------------------------------------
class CommandRunner:
    """Run a shell command and stream output line-by-line.

    Usage:
        runner = CommandRunner(["bash", "deploy.sh", "MyMod"])
        runner.start()
        # Each frame:
        for line in runner.drain():
            log_lines.append(line)
        if runner.finished:
            print(f"Exit code: {runner.exit_code}")
    """

    def __init__(self, cmd: list[str], cwd: str | None = None, env: dict[str, str] | None = None):
        self.cmd = cmd
        if cwd is None:
            from creation_lib.ui.host import get_host

            cwd = str(get_host().get_app_root())
        self.cwd = cwd
        self._env = env
        self._queue: queue.Queue[str] = queue.Queue()
        self._thread = None
        self.exit_code: int | None = None
        self.finished = False

    def start(self):
        self.finished = False
        self.exit_code = None
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self):
        try:
            run_env = None
            if self._env:
                run_env = {**os.environ, **self._env}
            proc = subprocess.Popen(
                self.cmd,
                cwd=self.cwd,
                env=run_env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )
            for line in proc.stdout:
                self._queue.put(line.rstrip("\n"))
            proc.wait()
            self.exit_code = proc.returncode
        except Exception as e:
            self._queue.put(f"ERROR: {e}")
            self.exit_code = 1
        finally:
            self.finished = True

    def drain(self) -> list[str]:
        """Drain all pending output lines (call from main thread each frame)."""
        lines = []
        while True:
            try:
                lines.append(self._queue.get_nowait())
            except queue.Empty:
                break
        return lines

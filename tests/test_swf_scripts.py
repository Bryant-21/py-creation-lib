"""`.swfproj` `scripts`: compiling ActionScript into a packed SWF.

These need `compile_as3_do_abc` in the native extension. When the `.pyd` on disk
predates it -- a rebuild can be blocked by another process holding the file open
-- they skip rather than fail, so the gate is real as soon as the extension is
current.
"""
from __future__ import annotations

import json

import pytest

from creation_lib.swf import native_runtime, project
from creation_lib.swf.writer import write_swf

def _has_compiler() -> bool:
    try:
        return hasattr(native_runtime.load_native_module(), "compile_as3_do_abc")
    except Exception:
        return False


pytestmark = pytest.mark.skipif(
    not _has_compiler(),
    reason="native extension predates compile_as3_do_abc; rebuild _native.pyd",
)

IHUDWIDGET = (
    "package hudframework {\n"
    "    public interface IHUDWidget {\n"
    "        function processMessage(command:String, params:Array):void;\n"
    "    }\n"
    "}\n"
)

WIDGET = """
package {
    import flash.display.MovieClip;
    import hudframework.IHUDWidget;

    public class B21_TestWidget extends MovieClip implements IHUDWidget {
        public function B21_TestWidget() { }

        public function processMessage(command:String, params:Array):void {
            if (command == "set") {
                this.gotoAndStop(params[0]);
            }
        }
    }
}
"""


def _write_project(tmp_path, scripts, exports):
    (tmp_path / "Widget.as").write_text(WIDGET, encoding="utf-8")
    (tmp_path / "IHUDWidget.as").write_text(IHUDWIDGET, encoding="utf-8")
    proj = {
        "canvas": [64, 24],
        "stage": [{}],
        "scripts": scripts,
        "exports": exports,
    }
    path = tmp_path / "widget.swfproj"
    path.write_text(json.dumps(proj), encoding="utf-8")
    return path


def test_a_widget_project_packs_with_its_class_backed(tmp_path):
    path = _write_project(
        tmp_path,
        ["Widget.as", "IHUDWidget.as"],
        [{"character": 0, "class": "B21_TestWidget"}],
    )
    data = write_swf(project.load_project_file(path))

    # The document class is really defined, and nothing dangles.
    assert "B21_TestWidget" in native_runtime.abc_class_names(data)
    assert native_runtime.unbacked_symbol_classes(data) == []

    # It is a real implementation, not a synthesized shell: the interface, the
    # method and the payload access all reached the constant pool.
    strings = [s for pool in native_runtime.abc_string_pools(data) for s in pool[-1]]
    for expected in ("IHUDWidget", "processMessage", "gotoAndStop", "set"):
        assert expected in strings, f"{expected!r} missing from the packed ABC"


def test_an_export_the_scripts_do_not_define_is_refused(tmp_path):
    path = _write_project(
        tmp_path,
        ["Widget.as", "IHUDWidget.as"],
        [{"character": 0, "class": "NotDefinedAnywhere"}],
    )
    with pytest.raises(ValueError, match="NotDefinedAnywhere"):
        project.load_project_file(path)


def test_a_missing_script_file_names_itself(tmp_path):
    path = _write_project(tmp_path, ["Absent.as"], [])
    with pytest.raises(ValueError, match="Absent.as"):
        project.load_project_file(path)


def test_a_compile_error_surfaces_with_its_position(tmp_path):
    (tmp_path / "Bad.as").write_text("package { public class }\n", encoding="utf-8")
    path = tmp_path / "bad.swfproj"
    path.write_text(json.dumps({"stage": [{}], "scripts": ["Bad.as"]}), encoding="utf-8")
    with pytest.raises(Exception, match="expected an identifier"):
        project.load_project_file(path)


def test_scripts_without_exports_still_emit_the_code(tmp_path):
    """A SWF may carry classes with no SymbolClass binding at all."""
    path = _write_project(tmp_path, ["Widget.as", "IHUDWidget.as"], [])
    data = write_swf(project.load_project_file(path))
    assert "B21_TestWidget" in native_runtime.abc_class_names(data)
    # No exports means no SymbolClass, so there is nothing to dangle.
    assert native_runtime.list_symbols(data) == []


def test_the_synthesized_path_still_works_without_scripts(tmp_path):
    """Existing projects that name bare classes must keep building."""
    proj = {"canvas": [64, 24], "stage": [{}],
            "exports": [{"character": 0, "class": "B21_Plain"}]}
    path = tmp_path / "plain.swfproj"
    path.write_text(json.dumps(proj), encoding="utf-8")
    data = write_swf(project.load_project_file(path))
    assert native_runtime.abc_class_names(data) == ["B21_Plain"]
    assert native_runtime.unbacked_symbol_classes(data) == []

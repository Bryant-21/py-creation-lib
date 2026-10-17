import pytest

from creation_lib.swf import native_runtime as native
from creation_lib.swf.project import build_document
from creation_lib.swf.writer import write_swf


def test_getter_and_setter_patch_independently(tmp_path):
    source = tmp_path / "AccessorMovie.as"
    source.write_text('''package {
        import flash.display.MovieClip;
        public class AccessorMovie extends MovieClip {
            public var value:int;
            public var getterValue:int;
            public var setterValue:int;
            public function get selectedIndex():int { return value; }
            public function set selectedIndex(index:int):void { value = index; }
        }
    }''')
    movie = write_swf(build_document({
        "scripts": [str(source)], "exports": [{"character": 0, "class": "AccessorMovie"}],
    }, tmp_path))
    pattern = [["getproperty", "value"]]
    with pytest.raises(ValueError, match="Expected one method"):
        native.patch_as3_method(movie, "AccessorMovie", "selectedIndex", pattern, [["keep", 0]])

    movie = native.patch_as3_method(movie, "AccessorMovie", "get selectedIndex", pattern,
                                    [["getproperty", "getterValue"]])
    movie = native.patch_as3_method(movie, "AccessorMovie", "set selectedIndex", [["setproperty", "value"]],
                                    [["setproperty", "setterValue"]])
    accessors = [method for method in native.abc_disassemble(movie, "AccessorMovie")
                 if method["method"] in ("get selectedIndex", "set selectedIndex")]
    assert len(accessors) == 2
    getter = next(method for method in accessors if method["signature"]["params"] == [])
    setter = next(method for method in accessors if method["signature"]["params"] == ["int"])
    assert any("GetProperty getterValue" in line for line in getter["code"])
    assert not any("setterValue" in line for line in getter["code"])
    assert any("SetProperty setterValue" in line for line in setter["code"])
    assert not any("getterValue" in line for line in setter["code"])

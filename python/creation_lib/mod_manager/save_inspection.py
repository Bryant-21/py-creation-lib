from dataclasses import asdict, dataclass
import json

from .backend import call


@dataclass(frozen=True)
class Inspection:
    path: str
    stamp: tuple
    basic: dict
    form_version: int
    game_version: str
    width: int
    height: int
    regular: tuple[str, ...]
    light: tuple[str, ...]
    notices: tuple[str, ...] = ()
    image_offset: int = 0


@dataclass(frozen=True)
class Comparison:
    target: str
    missing: tuple[str, ...]
    disabled: tuple[str, ...]
    added: tuple[str, ...]
    moved: tuple[tuple[str, str, str], ...]
    type_changed: tuple[str, ...]
    unknown: tuple[str, ...]
    issues: tuple[str, ...] = ()

    same_plugins: bool = False


def validate(inspection, cancel=None):
    call("save_inspection_validate", json.dumps(asdict(inspection)), cancel)


def inspect(save, cancel=None):
    value = json.loads(call("save_inspect", save, cancel))
    return Inspection(**{**value, **{name: tuple(value[name]) for name in ("stamp", "regular", "light", "notices")}})


def screenshot(inspection, cancel=None):
    return call("save_screenshot", json.dumps(asdict(inspection)), cancel)


def _comparison(value):
    return Comparison(**{**value, **{name: tuple(value[name]) for name in
        ("missing", "disabled", "added", "type_changed", "unknown", "issues")},
        "moved": tuple(tuple(item) for item in value["moved"])})


def compare_saved(left, right, cancel=None):
    return _comparison(json.loads(call("save_compare_saved", json.dumps(asdict(left)), json.dumps(asdict(right)), cancel)))


def compare_profile(inspection, catalog, cancel=None):
    return _comparison(json.loads(call("save_compare_profile", json.dumps(asdict(inspection)), json.dumps(asdict(catalog), default=str), cancel)))

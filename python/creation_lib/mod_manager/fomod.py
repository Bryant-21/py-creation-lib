from __future__ import annotations

from dataclasses import dataclass, field
import json
import xml.etree.ElementTree as ET

from .backend import call
from .paths import ManagerError
from .fomod_compatibility import decode

EXTENDERS = {"foseDependency": "fose", "nvseDependency": "nvse", "skseDependency": "skse", "f4seDependency": "f4se"}


class FileStates(dict):
    """Canonical mutable states with an O(1) revision for the cached wizard view."""
    def __init__(self, values):
        super().__init__(values)
        self.revision = 0

    def __setitem__(self, name, value):
        return call("fomod_states_action", self, "set", (name, value), {})

    def __delitem__(self, name):
        return call("fomod_states_action", self, "delete", (name,), {})

    def update(self, values=(), **kwargs):
        return call("fomod_states_action", self, "update", (values,), kwargs)

    def clear(self):
        return call("fomod_states_action", self, "clear", (), {})

    def pop(self, name, *default):
        return call("fomod_states_action", self, "pop", (name, *default), {})

    def popitem(self):
        return call("fomod_states_action", self, "popitem", (), {})

    def setdefault(self, name, default=None):
        return call("fomod_states_action", self, "setdefault", (name, default), {})

    def __ior__(self, other):
        self.update(other)
        return self


@dataclass
class Environment:
    files: dict[str, str] = field(default_factory=dict)
    game_version: str = ""
    fomm_version: str = ""
    complete: bool = True
    extender_versions: dict[str, str] = field(default_factory=dict)

    def __post_init__(self):
        self.files = FileStates(dict(call("fomod_file_state", name, value) for name, value in self.files.items()))


def matches(node, environment, flags):
    return call("fomod_matches", node, environment, json.dumps(flags))


@dataclass(frozen=True)
class OptionView:
    id: str
    name: str
    description: str
    type: str
    selected: bool
    image: str = ""


@dataclass(frozen=True)
class GroupView:
    id: str
    name: str
    type: str
    options: tuple[OptionView, ...]


@dataclass(frozen=True)
class StepView:
    name: str
    groups: tuple[GroupView, ...]


@dataclass
class Evaluation:
    steps: list[StepView]
    flags: dict[str, str]
    errors: list[str]
    rules: list[ET.Element]


class Fomod:
    _checkpoint = None

    def __init__(self, data: bytes, *, compatibility="schema"):
        self.compatibility = compatibility
        text, self.encoding, self.notices = decode(data, compatibility)
        try:
            self.root = ET.fromstring(text.encode("utf-8"))
        except ET.ParseError as exc:
            raise ManagerError(f"Invalid FOMOD XML: {exc}") from exc
        self._sequence = {node: index for index, node in enumerate(self.root.iter())}
        self.version, self.name, self.image = call("fomod_prepare", self.root)
        from .installer_decoration import parse
        self.decoration = parse(self.root)

    def evaluate(self, choices=None, environment=None, cancel=None):
        choices = {key: list(values) for key, values in (choices or {}).items()}
        steps, flags, errors, rules = call("fomod_evaluate", self.root, self.compatibility, json.dumps(choices),
                                         environment or Environment(), cancel, self._checkpoint)
        views = [StepView(step["name"], tuple(GroupView(group["id"], group["name"], group["type"],
                 tuple(OptionView(**option) for option in group["options"])) for group in step["groups"]))
                 for step in json.loads(steps)]
        return Evaluation(views, json.loads(flags), errors, rules)

    def plan(self, package, choices=None, environment=None, cancel=None, *, repairs=None):
        from .installer_repair import validate
        evaluation = self.evaluate(choices, environment, cancel)
        return json.loads(call("fomod_install_plan", package, evaluation, self._sequence, repairs, validate, cancel))

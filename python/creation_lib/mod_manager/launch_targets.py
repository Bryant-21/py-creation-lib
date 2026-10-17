"""Multiple FO4/F4SE installations share one modlist, with isolated runtime state."""
from dataclasses import asdict, dataclass, field
import json
from pathlib import Path

from .launcher import Executable, fingerprint
from .mo2 import read_text
from .backend import call

PROFILE_FIELDS = frozenset(call("target_profile_fields"))


@dataclass(frozen=True)
class Target:
    id: str
    name: str
    game_root: str
    binary: str
    working_directory: str
    arguments: str
    defaults: dict
    f4se: dict = field(default_factory=dict)

    @property
    def executable(self):
        return Executable(**json.loads(call("target_executable", json.dumps(asdict(self)))))


@dataclass(frozen=True)
class Review:
    library_id: str
    target: Target
    expected: str
    inputs: tuple


def records(library, db=None):
    if db is None:
        with library._db() as connection:
            return records(library, connection)
    return [Target(**record) for record in json.loads(call("target_records", db.native))]


def review(library, name, game_root, binary="", working_directory="", arguments="", identifier="", cancel=None):
    request = {"name": name, "game_root": str(game_root), "binary": binary,
               "working_directory": working_directory, "arguments": arguments, "identifier": identifier}
    with library._db() as db:
        result = json.loads(call("target_review", db.native, library, json.dumps(request), fingerprint, read_text, cancel))
    result["target"] = Target(**result["target"])
    result["inputs"] = tuple((path, tuple(stamp)) for path, stamp in result["inputs"])
    return Review(**result)


def _checkpoint():
    pass


def apply(library, plan, cancel=None):
    with library._db() as db:
        return call("target_apply", db.native, library, json.dumps(asdict(plan)), fingerprint, cancel, _checkpoint)


class TargetLibrary:
    def __init__(self, library, target_id):
        self.base = library
        with library._db() as db:
            state = json.loads(call("target_state_root", db.native, library, target_id))
        self.target_id = state["target_id"]
        self.library_id = state["library_id"]
        self.root = Path(state["root"])

    def __getattr__(self, name):
        return getattr(self.base, name)

    def target(self, db=None):
        if db is None:
            with self.base._db() as connection:
                return self.target(connection)
        return Target(**json.loads(call("target_record", db.native, self.target_id)))

    def profile_files(self, db, profile_id, base_files):
        return json.loads(call("target_profile_files", db.native, self.target_id, profile_id, json.dumps(base_files)))

    def owned_save_root(self, profile_id):
        with self.base._db() as db:
            return Path(call("target_save_root", db.native, self.target_id, profile_id, self.base.root, self.root))

    def save_namespace(self, profile_id):
        with self.base._db() as db:
            return call("target_save_namespace", db.native, self.target_id, profile_id)

    def profile_snapshot(self, profile_id, *, include_signature=False):
        from .library import Profile, ModRow
        with self.base._db() as db:
            result = json.loads(call("target_profile_snapshot", db.native, self.target_id, profile_id, include_signature))
        result[0] = Profile(**result[0])
        for row in result[1]:
            row["categories"] = tuple(row["categories"])
        result[1] = [ModRow(**row) for row in result[1]]
        return tuple(result)

    def profiles(self):
        from .library import Profile
        with self.base._db() as db:
            return [Profile(**row) for row in json.loads(call("target_profiles", db.native, self.target_id))]

    def database_signature(self, db, profile_id):
        return call("target_profile_signature", db.native, self.target_id, profile_id)

    def write_profile_fields(self, db, profile_id, files):
        call("target_write_profile", db.native, self.target_id, profile_id, json.dumps(files))

    def save_ini(self, profile_id, name, text, expected, cancel=None):
        with self.base._db() as db:
            return call("target_save_ini", db.native, self.base, self.target_id, profile_id, name, text,
                        json.dumps(expected), cancel, _checkpoint)


def recover(library):
    return call("target_recover", library)

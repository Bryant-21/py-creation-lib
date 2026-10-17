from dataclasses import asdict, dataclass
import json

from .backend import call


@dataclass(frozen=True)
class Review:
    library_id: str
    target_id: str
    profile_id: str
    record: dict
    expected: str


def value(library, target_id, db=None):
    if db is None:
        with library._db() as connection:
            return value(library, target_id, connection)
    return json.loads(call("target_setting_value", db.native, target_id, "preferences"))


def review(library, target_id, profile_id, *, ini="target", saves="target", cancel=None):
    with library._db() as db:
        result = call("target_setting", db.native, library, "review", "preferences", target_id, profile_id,
                      json.dumps({"ini": ini, "saves": saves}), cancel=cancel)
    return Review(**json.loads(result))


def _checkpoint():
    pass


def apply(library, plan, cancel=None):
    with library.owned.lock(), library._db() as db:
        return json.loads(call("target_setting", db.native, library, "apply", "preferences", plan.target_id,
                               plan.profile_id, plan=json.dumps(asdict(plan)), checkpoint=_checkpoint, cancel=cancel))


class SaveLibrary:
    """Shared owned save ownership, with the selected target's profile metadata for comparisons."""
    def __init__(self, context):
        self.context, self.base = context, context.base
        self.root, self.library_id = self.base.root, self.base.library_id

    def __getattr__(self, name):
        return getattr(self.base, name)

    def profile_snapshot(self, profile_id):
        return self.context.profile_snapshot(profile_id)

    def profiles(self):
        return self.context.profiles()

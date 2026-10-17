from dataclasses import asdict, dataclass, replace
import json

from .backend import call
from .launcher import definition, fingerprint


@dataclass(frozen=True)
class Review:
    library_id: str
    target_id: str
    arguments: str
    expected: str
    inputs: tuple


def value(library, target_id, db=None):
    if db is None:
        with library._db() as connection:
            return value(library, target_id, connection)
    return json.loads(call("target_setting_value", db.native, target_id, "options"))


def validate(arguments):
    return call("target_arguments", arguments)


def executable(library, target, db=None):
    return definition(**replace(target.executable, arguments=value(library, target.id, db)["arguments"]).record())


def review(library, target_id, arguments, cancel=None):
    with library._db() as db:
        result = json.loads(call("target_setting", db.native, library, "review", "options", target_id,
                                 record=json.dumps({"arguments": arguments}), fingerprint=fingerprint, cancel=cancel))
    result["inputs"] = tuple((path, tuple(stamp)) for path, stamp in result["inputs"])
    return Review(**result)


def _checkpoint():
    pass


def apply(library, plan, cancel=None):
    with library.owned.lock(), library._db() as db:
        return json.loads(call("target_setting", db.native, library, "apply", "options", plan.target_id,
                               plan=json.dumps(asdict(plan)), fingerprint=fingerprint, checkpoint=_checkpoint, cancel=cancel))

from dataclasses import dataclass
import json

from .backend import call, invoke


@dataclass(frozen=True)
class Missing:
    source: str
    folder: bool
    destinations: tuple


def available(package, source, folder):
    return invoke(package._index.available, source, folder)


def missing(package, evaluation, cancel=None):
    rows = json.loads(call("fomod_missing", package._index, package.prefix, evaluation.rules, cancel))
    return tuple(Missing(row["source"], row["folder"], tuple(row["destinations"])) for row in rows)


def validate(package, evaluation, bindings, cancel=None):
    return json.loads(call("fomod_repair", package._index, package.prefix, evaluation.rules, json.dumps(bindings), cancel))

from dataclasses import dataclass
import json
from .backend import call


@dataclass(frozen=True)
class Context:
    library_root: str
    library_id: str
    profile_id: str
    plugin_signature: str
    database_signature: str
    file_signature: str
    file_count: int
    providers: tuple
    ccc: tuple | None
    plugins: tuple
    requested: tuple[str, ...]
    archives: object = None
    versions: object = None



def _verify_winners(winners, cancel, progress):
    return call("installer_verify_winners", json.dumps(winners), cancel, progress)


def _scan(library, profile_id, requested, cancel, progress, version_requests=()):
    return call("installer_context_scan", library, profile_id, list(requested), cancel, progress, list(version_requests))


def capture(library, profile_id, fomod, cancel=None, progress=None):
    return call("installer_context_capture", library, profile_id, fomod, cancel, progress)


def validate(library, context, cancel=None):
    return call("installer_context_validate", library, context, cancel)

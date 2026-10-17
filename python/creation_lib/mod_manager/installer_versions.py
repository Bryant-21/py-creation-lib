from dataclasses import dataclass

from .backend import call
from .launcher import fingerprint
from .pe_metadata import file_version


@dataclass(frozen=True)
class Snapshot:
    requested: tuple = ()
    versions: tuple = ()
    inputs: tuple = ()
    issues: tuple = ()


def capture(library, configuration, game, requested, cancel=None):
    return call("installer_versions", library, configuration, game, list(requested), cancel, fingerprint, file_version)

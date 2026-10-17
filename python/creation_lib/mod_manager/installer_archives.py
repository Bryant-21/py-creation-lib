from dataclasses import dataclass
from .backend import call


@dataclass(frozen=True)
class Snapshot:
    states: tuple = ()
    archives: tuple = ()
    issues: tuple = ()



def capture(library, profile_id, requested, loose, game, game_available, cancel=None, progress=None):
    return call("installer_archive_capture", library, profile_id, list(requested), loose, game, game_available, cancel, progress)

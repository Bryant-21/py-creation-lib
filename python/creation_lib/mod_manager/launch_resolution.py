from dataclasses import dataclass
from .backend import call


@dataclass(frozen=True)
class Resolution:
    target_id: str
    game_root: str
    executable: object



def resolve(library, executable, *, selected_target="", cancel=None):
    return call("launch_resolve", library, executable, selected_target, cancel)

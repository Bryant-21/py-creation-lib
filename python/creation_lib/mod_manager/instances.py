from dataclasses import dataclass
import json
from pathlib import Path

from .backend import call


@dataclass(frozen=True)
class Game:
    id: str
    name: str
    binary: str
    extender: str = ""
    ini_names: tuple[str, ...] = ()
    native: bool = False


def _game(record):
    record["ini_names"] = tuple(record["ini_names"])
    return Game(**record)


GAMES = tuple(_game(record) for record in json.loads(call("manager_games")))


def game_definition(game_id):
    return _game(json.loads(call("manager_game", game_id)))


def validate_root(game_id, root, library_root):
    game, path = call("instance_validate_root", game_id, Path(root), Path(library_root))
    return _game(json.loads(game)), Path(path)


def create(library, game_id, root, name="Default", cancel=None):
    from .mo2 import read_text
    with library._db() as db:
        return call("instance_create", db.native, library.root, game_id, Path(root), name, read_text, cancel)

"""Explicit browser navigation from known local metadata, without account actions."""
import json
from webbrowser import open as open_browser

from .backend import call
from .paths import ManagerError


def game_page(library, profile_id):
    with library._db() as db:
        return call("nexus_page", db.native, profile_id)


def mod_page(library, profile_id, mod_id):
    with library._db() as db:
        return call("nexus_page", db.native, profile_id, mod_id)


def browse(url):
    if not open_browser(url, new=2, autoraise=True):
        raise ManagerError("The default browser did not accept this Nexus page.")


def file_page(game, mod_id, file_id):
    return call("nexus_file_page", game, json.dumps(mod_id), json.dumps(file_id))


def open_game(library, profile_id):
    browse(game_page(library, profile_id))


def open_mod(library, profile_id, mod_id):
    browse(mod_page(library, profile_id, mod_id))

from .backend import call
from .plugin_state import _catalog_json


def move(catalog, order, selected, destination, locks):
    return call("plugin_state_move", _catalog_json(catalog), list(order), list(selected), destination, list(locks))

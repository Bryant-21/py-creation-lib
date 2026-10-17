from pathlib import Path

from .backend import call
from .runtime import known_folder
from .saves import inventory_folder

DOCUMENTS = "FDD39AD0-238F-46AF-ADB4-6C85480369C7"


def folder(library, profile_id):
    return Path(call("saves_global_folder", library, profile_id, known_folder))


def inventory(library, profile_id, cancel=None, progress=None):
    return call("saves_global_inventory", library, profile_id, known_folder, inventory_folder, cancel, progress)

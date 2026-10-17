import json
from .backend import call


def preferences(library, instance_id="", db=None):
    return json.loads(call("executable_catalog_preferences", library, instance_id, db))


def ordered(library, items, instance_id=""):
    return call("executable_catalog_ordered", library, items, instance_id)


def snapshot(library, instance_id="", db=None):
    return call("executable_catalog_snapshot", library, instance_id, db)


def _checkpoint():
    pass


def save(library, instance_id, expected, items, default="", cancel=None):
    return call("executable_catalog_save", library, instance_id, expected, items, default, cancel, _checkpoint)


def instance(manager):
    return call("executable_catalog_instance", manager)

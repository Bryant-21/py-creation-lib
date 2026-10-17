import json

from .backend import call


def fo4_runtime_ini(name, text, profile_id):
    return call("profile_runtime_ini", name, text, profile_id)


def isolate_documents(library, profile_id, enabled=None):
    with library._db() as db:
        return call("profile_isolate_documents", db.native, profile_id, enabled)


def effective_ini(library, profile, name):
    from .mo2 import read_text
    return call("profile_effective_ini", library.root, profile.id, getattr(library, "target_id", ""),
                name, profile.files, read_text)


def validate_ini(name, text):
    return call("profile_validate_ini", name, text)


def _edit(library, action, profile_id, name, text="", expected=None, cancel=None):
    with library._db() as db:
        call("profile_edit", db.native, db, library, action, profile_id, name, text, json.dumps(expected), cancel)


def save_ini(library, profile_id, name, text, expected, cancel=None):
    return call("profile_save_scoped", library, profile_id, name, text, expected, cancel)


def restore_ini(library, profile_id, name, expected, cancel=None):
    _edit(library, "restore", profile_id, name, expected=expected, cancel=cancel)


def rename(library, profile_id, name, cancel=None):
    _edit(library, "rename", profile_id, name, cancel=cancel)

from .backend import call


def toggle_hidden(library, provider, cancel=None, progress=None):
    return call("file_toggle_hidden", library, provider, cancel, progress, False)


def parent_folder(library, provider):
    return call("file_parent_folder", library, provider)


def toggle_parent_hidden(library, provider, cancel=None, progress=None):
    return call("file_toggle_hidden", library, provider, cancel, progress, True)

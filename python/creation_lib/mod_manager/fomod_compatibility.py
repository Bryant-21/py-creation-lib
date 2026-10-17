from .backend import call

MODES = ("mo2", "schema")


def _checkpoint():
    pass


def decode(data, mode):
    text, label, notices = call("fomod_decode", data, mode, _checkpoint)
    return text, label, tuple(notices)

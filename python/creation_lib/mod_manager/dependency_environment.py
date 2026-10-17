from .backend import call


def known_game_files(library, profile_id, fomod, cancel=None):
    return call("known_game_files", library, profile_id, fomod, cancel)

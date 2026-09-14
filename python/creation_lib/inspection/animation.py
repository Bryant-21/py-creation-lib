import json


def _native():
    from creation_lib._native import ck_native
    return ck_native


def race_subgraphs(path, game, form_id=None):
    return json.loads(_native().ck_race_subgraphs(str(path), game, form_id))


def subgraph_id(behavior, paths):
    return _native().ck_subgraph_id(behavior, list(paths))

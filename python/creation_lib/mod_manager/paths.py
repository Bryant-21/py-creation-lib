from __future__ import annotations

from pathlib import Path


class ManagerError(ValueError):
    pass


class Cancelled(ManagerError):
    pass


def check_cancel(cancel=None):
    from .backend import call
    return call("manager_check_cancel", cancel)


def relative_path(raw: str, *, empty=False) -> str:
    from .backend import call
    return call("relative_path", raw, empty)


def reject_links(path: Path):
    from .backend import call
    return call("reject_links", path)


def tree_files(root: Path, cancel=None, directories=None, directory_limit=200_000, *, with_info=False):
    from .backend import call, invoke
    iterator = call("FileTree", root, cancel, directories, directory_limit)
    while True:
        try:
            name, path = invoke(next, iterator)
        except StopIteration:
            return
        path = Path(path)
        yield (name, path, path.lstat()) if with_info else (name, path)


def validate_destinations(paths):
    from .backend import call
    return call("validate_destinations", list(paths))

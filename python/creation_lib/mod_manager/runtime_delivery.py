"""Package the project's own native filespace; no upstream runtime acquisition."""
from .backend import call

from . import native_builder


def freeze_datas(project_root, *, public=False, onefile=False):
    return call("runtime_freeze_datas", project_root, public, onefile, native_builder)

import shutil

from .backend import call
from .read_process import ReadProcess


def run_json(command, *, game="", cancel=None, max_bytes=8 * 1024**2, timeout=30):
    return call("cli_read_json", list(command), game, cancel, max_bytes, timeout, shutil.which, ReadProcess)

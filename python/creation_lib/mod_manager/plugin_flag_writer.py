import shutil

from .backend import call
from .read_process import ReadProcess


def write(source, output, action, cancel=None):
    return call("plugin_write_flag", source, output, action, cancel, shutil.which, ReadProcess)

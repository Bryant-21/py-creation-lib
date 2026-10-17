from .backend import NativeContext, call


def executable():
    return call("sevenzip_executable")


def process(arguments, cancel=None, timeout=120):
    return NativeContext(call("archive_process", arguments, cancel, timeout))


def parse_listing(data, limits, archive_bytes):
    return call("sevenzip_listing", data, limits, archive_bytes)


class SevenZip:
    def __init__(self, archive, limits, cancel=None):
        call("sevenzip_initialize", self, archive, limits, cancel)

    def open(self, source, cancel=None):
        return NativeContext(call("sevenzip_open", self, source, cancel))

    def read_config(self, source):
        return call("sevenzip_read_config", self, source)

from .backend import call
from .launcher import fingerprint


def file_version(path):
    return call("pe_file_version", path, fingerprint)


def exported_data(path, symbol, size):
    return call("pe_exported_data", path, symbol, size, fingerprint)


def packed_runtime(version):
    return call("pe_packed_runtime", version)

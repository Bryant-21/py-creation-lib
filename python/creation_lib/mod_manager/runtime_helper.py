import sys
import time

from .backend import call


def write_status(path, **fields):
    return call("helper_write_status", path, fields)


def validate_files(plan):
    return call("helper_validate_files", plan)


def run(request_path, *, native_factory=None, wait=time.sleep):
    return call("helper_run", request_path, native_factory, wait)


if __name__ == "__main__":
    raise SystemExit(run(sys.argv[1]))

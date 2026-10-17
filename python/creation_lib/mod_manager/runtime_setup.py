from app.paths import get_code_root
from .backend import call

APP_ROOT = get_code_root()


def resolve(value, app_root=APP_ROOT):
    return call("runtime_bundle_resolve", value, app_root)


def supported_bundle(value, app_root=APP_ROOT):
    return call("runtime_supported_bundle", value, app_root)


def discover(configuration, app_root=APP_ROOT):
    return call("runtime_discover_bundle", configuration, app_root)

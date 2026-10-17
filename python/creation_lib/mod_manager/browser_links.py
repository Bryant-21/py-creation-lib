from .backend import call


def protocol_command():
    return call("browser_protocol_command")


def current_handler(registry=None):
    return call("browser_current_handler", registry)


def register_handler(*, confirmed=False, registry=None):
    return call("browser_register_handler", confirmed, registry)

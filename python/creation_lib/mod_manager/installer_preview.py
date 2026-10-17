from .backend import call


def metadata(package, cancel=None):
    return call("installer_preview_metadata", package, cancel)


def image(package, source, cancel=None):
    return call("installer_preview_image", package, source, cancel)

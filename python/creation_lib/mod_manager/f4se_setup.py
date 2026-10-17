from urllib.request import HTTPRedirectHandler
from .package import Limits
from .backend import call

CATALOG = {
    (1, 10, 163): {"version": "0.6.23", "url": "https://f4se.silverlock.org/beta/f4se_0_06_23.7z", "steam_loader": True},
    (1, 10, 984): {"version": "0.7.2", "file_id": 316287, "steam_loader": False},
    (1, 11, 240): {"version": "0.7.9", "file_id": 407709, "steam_loader": False},
}
SITE = "https://f4se.silverlock.org/"
LIMITS = Limits(files=4000, total_bytes=64 * 1024**2, file_bytes=16 * 1024**2)


def runtime_name(build):
    return call("f4se_runtime_name", build)


def inspect_folder(game, cancel=None):
    return call("f4se_inspect_folder", game, cancel)


def _official(raw):
    return call("f4se_official", raw)


class _Redirects(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        _official(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _download(url, destination, cancel, progress, *, opener=None, validator=_official, md5=""):
    return call("f4se_download", url, destination, cancel, progress, opener, validator, md5)


def _checkpoint():
    pass


def prepare(library, target_id, *, archive="", client=None, nxm="", cancel=None, progress=None, opener=None):
    return call("f4se_prepare", library, target_id, archive, client, nxm, cancel, progress, opener)


def validate_ready(target, cancel=None, *, owner_root=None):
    return call("f4se_validate_ready", target, cancel, owner_root)


def validate_launch(target, cancel=None, *, owner_root=None):
    return call("f4se_validate_launch", target, cancel, owner_root)


def plugin_findings(files, build, f4se_version, cancel=None):
    return tuple(call("f4se_plugin_findings", files, build, f4se_version, cancel))

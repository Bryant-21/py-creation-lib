from __future__ import annotations

from dataclasses import asdict, dataclass, field
import json
import time
from urllib.request import HTTPRedirectHandler

from .paths import ManagerError
from .backend import call


GAMES = {"Fallout 4": "fallout4", "Fallout 76": "fallout76", "Skyrim Special Edition": "skyrimspecialedition",
         "Skyrim": "skyrim", "Fallout 3": "fallout3", "Fallout New Vegas": "newvegas", "Starfield": "starfield"}


class NexusError(ManagerError):
    def __init__(self, message, retry_after=3600, *, http_status=None):
        super().__init__(message)
        self.retry_after = retry_after
        self.http_status = http_status


def game_domain(name):
    return call("nexus_game_domain", name)

@dataclass(frozen=True)
class NxmLink:
    game: str
    mod_id: int
    file_id: int
    key: str = field(default="", repr=False)
    expires: int = field(default=0, repr=False)
    user_id: int = field(default=0, repr=False)

    @property
    def identity(self):
        return self.game, self.mod_id, self.file_id

    @property
    def label(self):
        return f"{self.game} / mod {self.mod_id} / file {self.file_id}"

    def check_expiry(self, now=None):
        call("nexus_expiry", self.expires, time.time() if now is None else now)


def parse_nxm(raw: str):
    return NxmLink(**json.loads(call("nexus_parse_nxm", raw)))


def validate_cdn_url(raw):
    return call("nexus_cdn_url", raw)


class _Redirects(HTTPRedirectHandler):
    def __init__(self, allowed):
        self.allowed = allowed

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        call("nexus_redirect", self.allowed, newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class NexusClient:
    def __init__(self, api_key, *, version="1.0.0", transport=None):
        call("nexus_client_initialize", self, api_key, version, transport)

    def forget(self):
        self._api_key, self.user = "", None

    def for_transfer(self):
        return call("nexus_transfer_client", self)

    def _open(self, request, api=True, *, accepted_statuses=()):
        return call("nexus_open", self, request, api, accepted_statuses)

    def _json(self, endpoint, parameters=None, *, method="GET", body=None, allow_empty=False):
        return call("nexus_json", self, endpoint, parameters, method, body, allow_empty)

    @staticmethod
    def _read(response, size):
        return call("nexus_read", response, size)

    def _operation(self, action, **arguments):
        return json.loads(call("nexus_operation", action, json.dumps(arguments), self._json))

    def connect(self):
        self.user = self._operation("connect")
        return self.user

    def resolve(self, link: NxmLink):
        return tuple(self._operation("resolve", **asdict(link), user=self.user, now=time.time()))

    def download(self, url, destination, cancel=None, progress=None, *, md5=""):
        return call("nexus_client_download", self, url, destination, cancel, progress, md5)

    def mod_info(self, game, mod_id):
        return self._operation("mod_info", game=game, mod_id=mod_id)

    def categories(self, game):
        return self._operation("categories", game=game)

    def files(self, game, mod_id):
        return self._operation("files", game=game, mod_id=mod_id)

    def tracked_mods(self):
        return self._operation("tracked_mods")

    def endorsements(self):
        return self._operation("endorsements")

    def track(self, game, mod_id, enabled=True):
        return self._operation("track", game=game, mod_id=mod_id, enabled=enabled)

    def endorse(self, game, mod_id, version=""):
        return self._endorsement(game, mod_id, version, "endorse")

    def unendorse(self, game, mod_id, version=""):
        return self._endorsement(game, mod_id, version, "abstain")

    def _endorsement(self, game, mod_id, version, action):
        return self._operation(action, game=game, mod_id=mod_id, version=version)


def _mod_identity(game, mod_id):
    call("nexus_mod_identity", game, json.dumps(mod_id))

"""Owned, credential-free download checkpoints and reviewed HTTP range resumes."""
import json
from pathlib import Path

from .nexus import NexusError, NxmLink
from .backend import call
from dataclasses import asdict
from .paths import Cancelled


LIMIT = 8 * 1024**3


class DownloadPaused(Cancelled):
    def __init__(self):
        super().__init__("Download paused. Owned partial data was retained; resume with a connected account and, when required, a fresh browser link.")


class DownloadFailure(NexusError):
    pass


def strong_etag(value):
    return call("download_etag", json.dumps(value))


class DownloadQueue:
    def __init__(self, library, observer=None):
        self.library, self.observer = library, observer

    def records(self):
        with self.library._db() as db:
            return json.loads(call("download_records", db.native))

    def get(self, identifier):
        with self.library._db() as db:
            return json.loads(call("download_records", db.native, identifier))

    def enqueue(self, link, cancel=None):
        with self.library.owned.lock(), self.library._db() as db:
            return call("download_enqueue", db.native, self.library.root, json.dumps(asdict(link)), cancel)

    def _store(self, job, state, record):
        return call("download_store_observed", self, job, state, record)

    def _paths(self, job):
        return tuple(Path(path) for path in call("download_paths", self.library.root, job["id"]))

    @staticmethod
    def _hash(path, size=None, cancel=None):
        return call("download_hash", path, size, cancel)

    def _finish(self, job, partial, folder, cancel=None):
        return call("download_finish", self.library._db, json.dumps(job), partial, folder, cancel, self._checkpoint)

    def _checkpoint(self, name):
        pass

    @staticmethod
    def _verify_md5(partial, md5, cancel):
        call("download_verify_md5", partial, md5, cancel)

    def run(self, identifier, client, link=None, cancel=None, progress=None):
        from .download_locks import job_lock
        with job_lock(self.library, identifier):
            return call("download_run", self, identifier, client, NxmLink, link, cancel, progress)

    def remove(self, identifier):
        from .download_locks import job_lock
        with job_lock(self.library, identifier):
            call("download_remove", self, identifier)

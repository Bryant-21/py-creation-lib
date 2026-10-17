"""Recoverable transactions for strictly manager-owned mod payloads."""
from __future__ import annotations

from dataclasses import asdict, dataclass
import json
from pathlib import Path

from .backend import call, NativeContext
from .paths import ManagerError


def key(value):
    return call("transaction_key", value)


def digest(value):
    return call("transaction_digest", json.dumps(value))


@dataclass(frozen=True)
class OwnedReview:
    mod_id: str
    name: str
    token: str
    profiles: tuple[str, ...]
    changed: tuple[str, ...]
    file_count: int
    bytes: int


class CommittedTransaction(ManagerError):
    def __init__(self, record):
        super().__init__("The change committed, but finalization was interrupted. Reopen the library to complete recovery; backups were retained.")
        self.mod_id, self.kind = record["mod_id"], record["kind"]


class OwnedTransactions:
    def __init__(self, library):
        call("transactions_initialize", self, library)

    def lock(self, *, shared=False):
        return NativeContext(call("transaction_lock", self, shared))


    def _paths(self, record):
        return tuple(Path(path) for path in call("transaction_paths", self.root, self.library.library_id, json.dumps(record)))

    def _begin(self, kind, mod_id, old_present, *, origin=None):
        record = json.loads(call("transaction_begin", self.root, self.library.library_id, kind, mod_id, old_present, origin))
        self._checkpoint("journal")
        return record

    def _checkpoint(self, name):
        """Fault-injection seam; the production implementation deliberately does nothing."""

    def _finish(self, record):
        call("transaction_finish", self.root, self.library.library_id, json.dumps(record))

    def _quarantine(self, path, record, label):
        call("transaction_quarantine", self.root, path, json.dumps(record), label)

    def _rollback(self, record):
        with self.library._db() as db:
            call("transaction_rollback", db.native, self.root, self.library.library_id, json.dumps(record))

    def _recover_locked(self):
        with self.library._db() as db:
            return call("transaction_recover", db.native, self.root, self.library.library_id)

    def recover(self):
        return call("transaction_owned_recover", self)

    def _snapshot(self, db, mod_id):
        return json.loads(call("transaction_snapshot", db.native, self.root, mod_id))

    def _manifest(self, source, cancel=None, progress=None):
        return json.loads(call("transaction_manifest", source, cancel, progress))

    def _review(self, db, mod_id, cancel=None):
        review, snapshot = json.loads(call("transaction_review", db.native, self.root, mod_id, cancel))
        review["profiles"], review["changed"] = tuple(review["profiles"]), tuple(review["changed"])
        return OwnedReview(**review), snapshot

    def review(self, mod_id, cancel=None):
        review, _ = json.loads(call("transaction_owned_review", self, mod_id, False, cancel))
        review["profiles"], review["changed"] = tuple(review["profiles"]), tuple(review["changed"])
        return OwnedReview(**review)

    def _record(self, db, record, snapshot):
        call("transaction_record", db.native, json.dumps(record), json.dumps(snapshot))

    def _abort(self, record):
        return call("transaction_abort", self.library._db, self.root, self.library.library_id, json.dumps(record))

    def install(self, package, plan, name, profile_id, cancel=None, progress=None, *, enabled=True, nexus=None,
                review=None, restored=None, expected_manifest_token=None, restore_directories=(), validate_before_commit=None, attach_metadata=None):
        options = dict(plan=plan, name=name, profile_id=profile_id, enabled=enabled, nexus=nexus,
                       review=asdict(review) if review else None, restored=restored,
                       expected_manifest_token=expected_manifest_token, restore_directories=list(restore_directories))
        with self.lock():
            return call("transaction_install", self.library, self._checkpoint, package, json.dumps(options),
                        cancel, progress, validate_before_commit, attach_metadata)

    def uninstall(self, review, cancel=None):
        with self.lock():
            return call("transaction_uninstall", self.library, self._checkpoint, json.dumps(asdict(review)), cancel)

    def history(self):
        with self.library._db() as db:
            return json.loads(call("transaction_history", db.native, self.root))

    def review_restore(self, operation_id, cancel=None):
        entry, review = json.loads(call("transaction_owned_review", self, operation_id, True, cancel))
        if review is not None:
            review["profiles"], review["changed"] = tuple(review["profiles"]), tuple(review["changed"])
            review = OwnedReview(**review)
        return entry, review

    def restore(self, operation_id, review=None, cancel=None, progress=None, *, backup_token=None):
        return call("transaction_owned_restore", self, operation_id, json.dumps(asdict(review) if review else None), cancel, progress, backup_token)

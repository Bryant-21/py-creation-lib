from .backend import call
from .nexus import NexusClient


class CredentialStore:
    def __init__(self, target="ModBox21/Nexus/account"):
        self.target = target

    def read(self):
        return call("nexus_credential", "read", self.target, "")

    def write(self, key):
        call("nexus_credential", "write", self.target, key)

    def delete(self):
        call("nexus_credential", "delete", self.target, "")


def authenticate(api_key="", remember=False, *, store=None, factory=NexusClient, cancel=None):
    return call("nexus_authenticate", api_key, remember, store or CredentialStore(), factory, cancel)

from dataclasses import asdict
import json

from .backend import call


def can_advance(evaluation, step):
    return call("fomod_can_advance", json.dumps([asdict(value) for value in evaluation.steps] if evaluation else []), step)

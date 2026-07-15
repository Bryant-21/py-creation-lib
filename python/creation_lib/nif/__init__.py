"""Shared NIF library — NIF file parsing, schema, and operations."""
from .nif_file import NifFile, NifBlock, NifHeader
from .schema import NifSchema, get_schema
from .actions import NifAction, OperationResult, SetFieldAction, SnapshotAction, CompositeAction

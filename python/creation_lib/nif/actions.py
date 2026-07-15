"""Action dispatcher — serializable command objects for NIF mutations.

Actions are stateless: execute() and undo() take the NifFile as a parameter
rather than holding a reference to it. UndoManager holds the NifFile and
passes it in.
"""
import copy
import logging
import time
from dataclasses import dataclass, field
from typing import Any

_log = logging.getLogger("creation_lib.nif.actions")


@dataclass
class OperationResult:
    """Returned by all operations."""
    success: bool
    description: str
    modified_block_ids: list[int] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


class NifAction:
    """Base class for all NIF mutations."""

    def execute(self, nif) -> OperationResult:
        raise NotImplementedError

    def undo(self, nif) -> OperationResult:
        raise NotImplementedError

    def description(self) -> str:
        return ""


@dataclass
class SetFieldAction(NifAction):
    """Set a single field on a NIF block, with undo support."""

    block_id: int
    field_name: str
    old_value: Any
    new_value: Any
    _description: str = ""

    def __post_init__(self):
        self.old_value = copy.deepcopy(self.old_value)
        self.new_value = copy.deepcopy(self.new_value)
        if not self._description:
            self._description = f"Set {self.field_name} on block {self.block_id}"

    def execute(self, nif) -> OperationResult:
        block = nif.get_block(self.block_id)
        if block:
            block.set_field(self.field_name, copy.deepcopy(self.new_value))
            return OperationResult(True, self._description, [self.block_id])
        return OperationResult(False, f"Block {self.block_id} not found")

    def undo(self, nif) -> OperationResult:
        block = nif.get_block(self.block_id)
        if block:
            block.set_field(self.field_name, copy.deepcopy(self.old_value))
            return OperationResult(True, f"Undo: {self._description}", [self.block_id])
        return OperationResult(False, f"Block {self.block_id} not found")

    def description(self) -> str:
        return self._description


@dataclass
class SnapshotAction(NifAction):
    """Full NIF state before/after for destructive operations."""

    _description: str = ""
    _before_blocks: list = field(default_factory=list, repr=False)
    _before_header: Any = field(default=None, repr=False)
    _after_blocks: list = field(default_factory=list, repr=False)
    _after_header: Any = field(default=None, repr=False)

    def capture_before(self, nif):
        self._before_blocks = copy.deepcopy(nif.blocks)
        self._before_header = copy.deepcopy(nif.header)

    def capture_after(self, nif):
        self._after_blocks = copy.deepcopy(nif.blocks)
        self._after_header = copy.deepcopy(nif.header)

    def execute(self, nif) -> OperationResult:
        if self._after_blocks:
            nif.blocks = copy.deepcopy(self._after_blocks)
            nif.header = copy.deepcopy(self._after_header)
            return OperationResult(True, self._description)
        return OperationResult(False, "No after-state captured")

    def undo(self, nif) -> OperationResult:
        if self._before_blocks:
            nif.blocks = copy.deepcopy(self._before_blocks)
            nif.header = copy.deepcopy(self._before_header)
            return OperationResult(True, f"Undo: {self._description}")
        return OperationResult(False, "No before-state captured")

    def description(self) -> str:
        return self._description


@dataclass
class CompositeAction(NifAction):
    """Groups multiple NifActions into one undo step."""

    children: list[NifAction] = field(default_factory=list)
    _description: str = ""

    def execute(self, nif) -> OperationResult:
        results = [child.execute(nif) for child in self.children]
        all_ids = [bid for r in results for bid in r.modified_block_ids]
        return OperationResult(True, self._description, all_ids)

    def undo(self, nif) -> OperationResult:
        results = [child.undo(nif) for child in reversed(self.children)]
        all_ids = [bid for r in results for bid in r.modified_block_ids]
        return OperationResult(True, f"Undo: {self._description}", all_ids)

    def description(self) -> str:
        return self._description


@dataclass
class VertexEditAction(NifAction):
    """Stores sparse deltas (vertex_index -> old/new values) for efficient
    vertex editing undo. Coalesces edits within a 500ms window."""

    block_id: int
    _description: str = ""
    _deltas: dict[int, tuple[dict, dict]] = field(default_factory=dict)  # idx -> (old, new)
    _last_edit_time: float = 0.0
    _coalesce_ms: float = 500.0

    def add_delta(self, vertex_index: int, old_value: dict, new_value: dict):
        """Add a vertex edit delta. If within coalesce window, merges into existing."""
        now = time.time() * 1000
        if vertex_index in self._deltas:
            # Keep original old_value, update new_value
            orig_old = self._deltas[vertex_index][0]
            self._deltas[vertex_index] = (orig_old, copy.deepcopy(new_value))
        else:
            self._deltas[vertex_index] = (copy.deepcopy(old_value), copy.deepcopy(new_value))
        self._last_edit_time = now

    @property
    def is_coalescing(self) -> bool:
        now = time.time() * 1000
        return (now - self._last_edit_time) < self._coalesce_ms

    def execute(self, nif) -> OperationResult:
        block = nif.get_block(self.block_id)
        if not block:
            return OperationResult(False, f"Block {self.block_id} not found")
        vdata = block.get_field("Vertex Data")
        if not vdata:
            return OperationResult(False, "No vertex data")
        for idx, (_, new_val) in self._deltas.items():
            if 0 <= idx < len(vdata):
                vdata[idx].update(copy.deepcopy(new_val))
        block.set_field("Vertex Data", vdata)
        return OperationResult(True, self._description, [self.block_id])

    def undo(self, nif) -> OperationResult:
        block = nif.get_block(self.block_id)
        if not block:
            return OperationResult(False, f"Block {self.block_id} not found")
        vdata = block.get_field("Vertex Data")
        if not vdata:
            return OperationResult(False, "No vertex data")
        for idx, (old_val, _) in self._deltas.items():
            if 0 <= idx < len(vdata):
                vdata[idx].update(copy.deepcopy(old_val))
        block.set_field("Vertex Data", vdata)
        return OperationResult(True, f"Undo: {self._description}", [self.block_id])

    def description(self) -> str:
        return self._description

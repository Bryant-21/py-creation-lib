"""PoseDelta — single source of truth for bone editor user edits.

All deltas are stored in **parent-local space** (the same space HKX
animation tracks live in). No world-space conversion happens at apply
time. The viewport preview composes deltas with the bind pose at draw
time; the apply pipeline composes deltas with each animation frame's
existing local transforms at write time. Both consume the same data.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Optional

import numpy as np


_IDENTITY_QUAT = np.array([0.0, 0.0, 0.0, 1.0])
_ZERO_TRANS = np.array([0.0, 0.0, 0.0])

# Threshold for "this delta is effectively identity / zero, drop it"
_ROT_EPSILON = 1e-6
_TRANS_EPSILON = 1e-6


@dataclass
class PoseDelta:
    """Per-bone parent-local rotation and translation deltas.

    rotations[bone] is composed onto the existing local rotation:
        new_local_rot = rotations[bone] * existing_local_rot
    translations[bone] is added to the existing local translation:
        new_local_pos = existing_local_pos + translations[bone]
    """

    rotations: Dict[str, np.ndarray] = field(default_factory=dict)
    translations: Dict[str, np.ndarray] = field(default_factory=dict)

    def is_empty(self) -> bool:
        return not self.rotations and not self.translations

    def edited_bones(self) -> set[str]:
        return set(self.rotations.keys()) | set(self.translations.keys())

    def set_rotation(self, bone: str, local_quat: np.ndarray) -> None:
        """Store a parent-local rotation delta. Identity is removed."""
        q = np.asarray(local_quat, dtype=np.float64)
        # Drop if effectively identity
        if abs(q[0]) < _ROT_EPSILON and abs(q[1]) < _ROT_EPSILON \
                and abs(q[2]) < _ROT_EPSILON and abs(abs(q[3]) - 1.0) < _ROT_EPSILON:
            self.rotations.pop(bone, None)
            return
        self.rotations[bone] = q

    def set_translation(self, bone: str, local_vec: np.ndarray) -> None:
        """Store a parent-local translation delta. Zero is removed."""
        v = np.asarray(local_vec, dtype=np.float64)
        if all(abs(c) < _TRANS_EPSILON for c in v):
            self.translations.pop(bone, None)
            return
        self.translations[bone] = v

    def clear_bone(self, bone: str) -> None:
        self.rotations.pop(bone, None)
        self.translations.pop(bone, None)

    def get_local_transform(
        self, bone: str
    ) -> Optional[tuple[np.ndarray, np.ndarray]]:
        """Return (rotation_quat, translation_vec) for *bone*, filling
        identity/zero for whichever component isn't set. Returns None if
        the bone has no edits at all.
        """
        if bone not in self.rotations and bone not in self.translations:
            return None
        rot = self.rotations.get(bone, _IDENTITY_QUAT).copy()
        trans = self.translations.get(bone, _ZERO_TRANS).copy()
        return rot, trans

    def copy(self) -> "PoseDelta":
        return PoseDelta(
            rotations={k: v.copy() for k, v in self.rotations.items()},
            translations={k: v.copy() for k, v in self.translations.items()},
        )

    def equals(self, other: "PoseDelta", atol: float = 1e-9) -> bool:
        """Value equality used by undo bookkeeping to drop no-op drags."""
        if set(self.rotations.keys()) != set(other.rotations.keys()):
            return False
        if set(self.translations.keys()) != set(other.translations.keys()):
            return False
        for k, v in self.rotations.items():
            if not np.allclose(v, other.rotations[k], atol=atol):
                return False
        for k, v in self.translations.items():
            if not np.allclose(v, other.translations[k], atol=atol):
                return False
        return True

    def to_json(self) -> dict:
        return {
            "rotations": {k: v.tolist() for k, v in self.rotations.items()},
            "translations": {k: v.tolist() for k, v in self.translations.items()},
        }

    @classmethod
    def from_json(cls, data: dict) -> "PoseDelta":
        return cls(
            rotations={
                k: np.asarray(v, dtype=np.float64)
                for k, v in (data.get("rotations") or {}).items()
            },
            translations={
                k: np.asarray(v, dtype=np.float64)
                for k, v in (data.get("translations") or {}).items()
            },
        )

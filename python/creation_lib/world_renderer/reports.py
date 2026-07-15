from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class WorldReport:
    ok: bool
    errors: list[str] = field(default_factory=list)
    warnings: list[dict[str, Any]] = field(default_factory=list)
    timings_ms: dict[str, float] = field(default_factory=dict)
    counts: dict[str, int] = field(default_factory=dict)
    data: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_json(cls, value: dict[str, Any]) -> "WorldReport":
        return cls(
            ok=bool(value.get("ok")),
            errors=list(value.get("errors") or []),
            warnings=list(value.get("warnings") or []),
            timings_ms=dict(value.get("timings_ms") or {}),
            counts=dict(value.get("counts") or {}),
            data=dict(value.get("data") or {}),
        )

    def to_json(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "errors": self.errors,
            "warnings": self.warnings,
            "timings_ms": self.timings_ms,
            "counts": self.counts,
            "data": self.data,
        }

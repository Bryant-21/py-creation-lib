"""FormID normalization helpers and multi-plugin resolution."""

from __future__ import annotations

from dataclasses import dataclass, field
from creation_lib.esp.model import FormRef

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from creation_lib.esp.plugin import Plugin


def normalize_form_id(raw: int, plugin: "Plugin") -> FormRef:
    return FormRef.from_raw(raw, plugin_name=plugin.plugin_name, masters=plugin.masters)


def encode_form_ref(form_ref: FormRef, plugin: "Plugin", *, add_missing: bool = False) -> int:
    return form_ref.to_raw(
        target_plugin_name=plugin.plugin_name,
        masters=plugin.masters,
        add_missing=add_missing,
    )


@dataclass(slots=True)
class PluginSet:
    """A simple load-order-aware plugin collection."""

    plugins: list["Plugin"] = field(default_factory=list)

    def __post_init__(self) -> None:
        for index, plugin in enumerate(self.plugins):
            plugin.load_order = index

    def add(self, plugin: "Plugin") -> None:
        plugin.load_order = len(self.plugins)
        self.plugins.append(plugin)

    def by_name(self, name: str) -> "Plugin" | None:
        wanted = name.lower()
        for plugin in self.plugins:
            if plugin.plugin_name.lower() == wanted:
                return plugin
        return None


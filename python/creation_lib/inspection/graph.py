from __future__ import annotations

from contextlib import ExitStack
from pathlib import Path

from creation_lib.esp import Plugin, native_runtime
from creation_lib.inspection.records import vmad_bindings, record_catalog
from creation_lib.inspection.assets import asset_dependencies


def references(value, path=""):
    if isinstance(value, dict):
        if "plugin" in value and "object_id" in value:
            oid = value["object_id"]
            number = int(oid, 16) if isinstance(oid, str) else int(oid)
            if number:
                yield path, f"{value['plugin']}:{number:06X}"
        else:
            for key, item in value.items():
                yield from references(item, f"{path}.{key}" if path else key)
    elif isinstance(value, list):
        for index, item in enumerate(value):
            yield from references(item, f"{path}.{index}")


class PluginGraph:
    def __init__(self, game, search_paths=()):
        self.game = game
        self.search_paths = [Path(p).resolve() for p in search_paths]
        self.plugins = []
        self.issues = []
        self._stack = ExitStack()
        self._indexes = {}
        self._records = {}

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return self._stack.__exit__(*args)

    def find(self, filename):
        path = Path(filename)
        if path.is_absolute() and path.is_file():
            return path
        for root in self.search_paths:
            candidate = root / path
            if candidate.is_file():
                return candidate
            if path.parent == Path(".") and root.is_dir():
                hit = next((p for p in root.iterdir() if p.name.casefold() == filename.casefold() and p.is_file()), None)
                if hit:
                    return hit
        return None

    def load(self, path, visiting=None):
        path = Path(path).resolve()
        key = path.name.casefold()
        visiting = set() if visiting is None else visiting
        if key in visiting:
            raise ValueError(f"Cyclic plugin master: {path.name}")
        existing = next((p for p in self.plugins if p.plugin_name.casefold() == key), None)
        if existing:
            if Path(existing.file_path).resolve() != path:
                raise ValueError(f"Two different paths supplied for {path.name}")
            return existing
        plugin = self._stack.enter_context(Plugin.load(path, game=self.game, backend="native", lazy_index=True))
        if path.parent not in self.search_paths:
            self.search_paths.append(path.parent)
        for name in plugin.header.masters:
            master_path = self.find(name)
            if master_path:
                self.load(master_path, visiting | {key})
            else:
                issue = {"code": "MISSING_MASTER", "plugin": path.name, "master": name}
                if issue not in self.issues:
                    self.issues.append(issue)
        self.plugins.append(plugin)
        return plugin

    def index(self, plugin):
        name = plugin.plugin_name.casefold()
        if name not in self._indexes:
            self._indexes[name] = {row["form_key"].casefold(): row for row in record_catalog(plugin)}
        return self._indexes[name]

    def resolve(self, key):
        versions = []
        for plugin in self.plugins:
            row = self.index(plugin).get(key.casefold())
            if row:
                versions.append((plugin, row))
        return versions

    def record(self, plugin, fid):
        inspected = self.inspect(plugin, fid)
        return inspected["record"] if inspected else None

    def inspect(self, plugin, fid):
        key = plugin.plugin_name, fid
        if key not in self._records:
            self._records[key] = native_runtime.plugin_handle_inspect_record(plugin._rust_handle, fid)
        return self._records[key]


def explain_record(graph, root_key, resolver, *, depth=2, max_records=100, max_assets=500):
    records, edges, assets, issues = [], [], [], list(graph.issues)
    pending = [(root_key, 0)]
    seen, asset_seen = set(), set()
    asset_pending = []
    truncated_records = truncated_assets = False
    while pending:
        key, level = pending.pop(0)
        if key.casefold() in seen:
            continue
        if len(records) >= max_records:
            truncated_records = True
            break
        seen.add(key.casefold())
        versions = graph.resolve(key)
        if not versions:
            issues.append({"code": "UNRESOLVED_REFERENCE", "form_key": key})
            continue
        plugin, row = versions[-1]
        record = graph.record(plugin, row["form_id"])
        bindings, vmad_issues = vmad_bindings(record)
        has_vmad = any("VirtualMachineAdapter" in f or "VMAD" in f for f in record.get("fields", []))
        if has_vmad:
            issues.extend({"code": "INCOMPLETE_VMAD", "form_key": key, "message": message} for message in vmad_issues)
        records.append({"form_key": key, "signature": row["signature"], "editor_id": row["editor_id"],
                        "winner": plugin.plugin_name, "overrides": [p.plugin_name for p, _ in versions],
                        "depth": level, "scripts": bindings if has_vmad else []})
        for field, target in references(record):
            edges.append({"from": key, "to": target, "field": field, "followed": level < depth})
            if level < depth:
                pending.append((target, level + 1))
        direct_assets = graph.inspect(plugin, row["form_id"])["assets"]
        for asset in direct_assets:
            asset_pending.append((asset["path"], key, asset["field"], 0))
        for binding in bindings if has_vmad else []:
            asset_pending.append(("scripts/" + binding["script"].replace(":", "/") + ".pex", key, binding["path"], 0))
    while asset_pending:
        path, owner, field, level = asset_pending.pop(0)
        key = path.replace("\\", "/").casefold()
        edges.append({"from": owner, "to": key, "field": field, "kind": "asset"})
        if key in asset_seen:
            continue
        if len(assets) >= max_assets:
            truncated_assets = True
            break
        asset_seen.add(key)
        result = dict(resolver.resolve(path))
        assets.append(result)
        suffix = Path(path).suffix.casefold()
        if result["status"] == "available" and suffix in {".nif", ".bgsm", ".bgem"}:
            try:
                for dependency in asset_dependencies(path, resolver.read(result)):
                    asset_pending.append((dependency["path"], key, dependency["field"], level + 1))
            except (OSError, ValueError, RuntimeError) as error:
                result["inspection_error"] = str(error)
                issues.append({"code": "ASSET_PARSE_FAILED", "path": path, "message": str(error)})
    return {"root": root_key, "records": records, "edges": edges, "assets": assets, "issues": issues,
            "limits": {"record_depth": depth, "max_records": max_records, "max_assets": max_assets,
                       "records_truncated": truncated_records, "assets_truncated": truncated_assets},
            "resolution": resolver.describe()}

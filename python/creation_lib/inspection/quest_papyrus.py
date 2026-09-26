from __future__ import annotations

from collections import Counter, deque
from hashlib import sha256
import json
from pathlib import Path

from creation_lib.esp import native_runtime
from creation_lib.inspection.graph import references
from creation_lib.inspection.records import form_key, vmad_bindings
from creation_lib.inspection.papyrus import audit_binding, evaluate, walk


def quest_stages(record, bindings):
    stages, current, item = [], None, None
    for field in record.get("fields", []):
        if "INDX" in field and isinstance(field["INDX"], dict) and "StageIndex" in field["INDX"]:
            current = {"stage": field["INDX"]["StageIndex"], "flags": field["INDX"].get("Flags", []), "items": []}
            stages.append(current)
            item = None
        elif "StageFlags" in field and current is not None:
            item = {"index": len(current["items"]), "flags": field["StageFlags"], "fields": [], "callbacks": []}
            current["items"].append(item)
        elif any(key in field for key in ("QOBJ", "Objective", "AliasID", "ALST", "ALLS", "ANAM")):
            current, item = None, None
        elif item is not None and any(key in field for key in ("Note", "SCFC", "CTDA", "Conditions")):
            item["fields"].append(field)
    unmatched = []
    for binding in bindings:
        fragment = binding.get("fragment", {})
        if "Quest Stage" not in fragment:
            continue
        stage, index = fragment["Quest Stage"], fragment.get("Quest Stage Index", 0)
        matches = [s for s in stages if s["stage"] == stage]
        callback = {"script": binding["script"], "name": fragment.get("FragmentName"),
                    "status": binding.get("callback_status"), "bodies": binding.get("callbacks", [])}
        if len(matches) == 1 and 0 <= index < len(matches[0]["items"]):
            matches[0]["items"][index]["callbacks"].append(callback)
        else:
            unmatched.append({"stage": stage, "item": index, **callback})
    return stages, unmatched


class QuestPapyrusAudit:
    def __init__(self, graph, sources, resolver, *, max_records=10000, max_scripts=10000):
        self.graph, self.sources, self.resolver = graph, sources, resolver
        self.max_records, self.max_scripts = max_records, max_scripts
        self._reverse = {}
        self._topics = None
        self._vmad_reverse = None
        self._index_issues = []

    def vmad_reverse(self):
        if self._vmad_reverse is None:
            self._vmad_reverse = {}
            for plugin in self.graph.plugins:
                candidates = native_runtime.plugin_handle_record_form_ids_with_subrecords(plugin._rust_handle, ["VMAD"])
                for fid in candidates:
                    key = form_key(plugin, fid)
                    if self.graph.resolve(key)[-1][0] is not plugin:
                        continue
                    try:
                        record = self.graph.record(plugin, fid)
                        attached, errors = vmad_bindings(record)
                        self._index_issues.extend({"code": "REVERSE_VMAD_INCOMPLETE", "record": key, "message": error} for error in errors)
                        for binding in attached:
                            for _, target in references(binding):
                                self._vmad_reverse.setdefault(target.casefold(), set()).add(key)
                    except (ValueError, RuntimeError, OSError) as error:
                        self._index_issues.append({"code": "REVERSE_VMAD_INCOMPLETE", "record": key, "message": str(error)})
        return self._vmad_reverse

    def reverse(self, key):
        if key.casefold() not in self._reverse:
            candidates = set(self.vmad_reverse().get(key.casefold(), ()))
            for plugin in self.graph.plugins:
                candidates.update(plugin.get_referencing_form_keys(key))
            # An overridden record must still reference this quest in its winning version.
            found = []
            for candidate in sorted(candidates):
                versions = self.graph.resolve(candidate)
                if versions and (candidate in self.vmad_reverse().get(key.casefold(), ()) or
                                 key.casefold() in {r.casefold() for r in versions[-1][0].get_referenced_form_keys(candidate)}):
                    found.append(candidate)
            self._reverse[key.casefold()] = found
        return self._reverse[key.casefold()]

    def topics(self):
        if self._topics is None:
            self._topics = {}
            for plugin in self.graph.plugins:
                topics = json.loads(native_runtime.plugin_handle_extract_dialogue_text(plugin._rust_handle, "json"))
                for topic in topics:
                    key = form_key(plugin, int(topic["dial_form_id"], 16))
                    # Child groups can be distributed across plugins that do not override DIAL.
                    children = self._topics.setdefault(key.casefold(), set())
                    children.update(form_key(plugin, int(i["form_id"], 16)) for i in topic.get("infos", []))
        return self._topics

    def output_status(self, source):
        asset = self.resolver.resolve("scripts/" + source["script"].replace(":", "/") + ".pex")
        result = {"resolution": asset, "status": "missing" if asset["status"] != "available" else "unverified"}
        if asset["status"] == "available":
            try:
                result["sha256"] = sha256(self.resolver.read(asset)).hexdigest()
                if asset["winner"]["kind"] == "loose":
                    result["mtime_ns"] = Path(asset["winner"]["path"]).stat().st_mtime_ns
                    if source.get("mtime_ns", 0) > result["mtime_ns"]:
                        result["status"] = "older_than_source"
                # Newer timestamps cannot establish which source was compiled.
            except (OSError, ValueError, RuntimeError) as error:
                result.update(status="unreadable", error=str(error))
        return result

    def audit(self, root_key):
        versions = self.graph.resolve(root_key)
        if not versions or versions[-1][1]["signature"] != "QUST":
            raise ValueError(f"Quest not found: {root_key}")
        root = versions[-1][1]
        records, bindings, edges, issues = [], [], [], list(self.graph.issues)
        pending, scripts_pending = deque([root_key]), deque()
        seen, script_seen, script_reports = set(), set(), []
        owners = {}
        producers = []
        stage_records = {}
        queued = {root_key.casefold()}

        def enqueue(key, origin, kind, field=None):
            edges.append({"from": origin, "to": key, "kind": kind, **({"field": field} if field else {})})
            if key.casefold() not in queued:
                pending.append(key)
                queued.add(key.casefold())

        while pending or scripts_pending:
            while pending and (not self.max_records or len(seen) < self.max_records):
                key = pending.popleft()
                if key.casefold() in seen:
                    continue
                seen.add(key.casefold())
                versions = self.graph.resolve(key)
                if not versions:
                    issues.append({"code": "UNRESOLVED_RECORD", "record": key})
                    continue
                plugin, row = versions[-1]
                signature = row["signature"]
                try:
                    inspected = self.graph.inspect(plugin, row["form_id"])
                    if not inspected:
                        raise ValueError("Native record inspection returned no record")
                    record = inspected["record"]
                    native_refs = plugin.get_referenced_form_keys(key)
                    decoded_refs = list(references(record))
                    fields_by_target = {}
                    for field, target in decoded_refs:
                        fields_by_target.setdefault(target.casefold(), []).append(field)
                    for target in sorted(set(native_refs) | {r for _, r in decoded_refs}):
                        enqueue(target, key, "record_reference", fields_by_target.get(target.casefold(), ["native_reference"]))
                    if signature == "QUST":
                        owners[key.casefold()] = key
                        for dependent in self.reverse(key):
                            enqueue(dependent, key, "quest_referenced_by")
                    elif signature in {"DIAL", "SCEN", "PACK"}:
                        qs = plugin.get_referenced_form_keys_by_subrecord(key, "PNAM" if signature == "SCEN" else "QNAM")
                        if len(qs) == 1:
                            owners[key.casefold()] = qs[0]
                    if signature == "DIAL":
                        children = sorted(self.topics().get(key.casefold(), set()))
                        expected = next((f["InfoCount"] for f in record.get("fields", []) if "InfoCount" in f), None)
                        if expected is not None and expected != len(children):
                            issues.append({"code": "DIALOGUE_COUNT_MISMATCH", "record": key,
                                           "expected": expected, "found": len(children)})
                        for child in children:
                            if key.casefold() in owners:
                                owners[child.casefold()] = owners[key.casefold()]
                            enqueue(child, key, "topic_child")
                    has_vmad = any(k in {"VMAD", "VirtualMachineAdapter"} for f in record.get("fields", []) for k in f)
                    attached, vmad_issues = vmad_bindings(record) if has_vmad else ([], [])
                    issues.extend({"code": "INCOMPLETE_VMAD", "record": key, "message": message} for message in vmad_issues)
                    records.append({"form_key": key, "signature": signature, "editor_id": row["editor_id"],
                                    "winner": plugin.plugin_name, "overrides": [p.plugin_name for p, _ in versions],
                                    "bindings": len(attached)})
                    for binding in attached:
                        # Fragment callback rows share the script-instance properties, not an empty new instance.
                        if "fragment" in binding:
                            containers = [b for b in attached if "fragment" not in b and b["script"].casefold() == binding["script"].casefold()
                                          and b.get("alias") == binding.get("alias")]
                            if len(containers) == 1:
                                binding = {**binding, "properties": containers[0]["properties"]}
                        bindings.append({"record": key, "signature": signature, **binding})
                        scripts_pending.append((binding["script"], key, "attachment"))
                    if signature == "QUST":
                        stage_records[key] = record
                        for field in record.get("fields", []):
                            if "INDX" in field and (not isinstance(field["INDX"], dict) or "StageIndex" not in field["INDX"]):
                                issues.append({"code": "UNDECODED_STAGE", "record": key, "value": field["INDX"]})
                    for index, field in enumerate(record.get("fields", [])):
                        for label in ("SCQS", "TIQS", "Set Parent Quest Stage"):
                            if label not in field:
                                continue
                            value = field[label]
                            entries = value.items() if isinstance(value, dict) else [("stage", value)]
                            for trigger, stage in entries:
                                if type(stage) is int and stage <= 0:
                                    continue
                                producers.append({"kind": "record", "record": key, "field": f"fields.{index}.{label}.{trigger}",
                                    "value": stage, "target": owners.get(key.casefold()), "stage": stage if type(stage) is int else None,
                                    "status": "resolved" if type(stage) is int and key.casefold() in owners else "unresolved"})
                except (RuntimeError, ValueError, OSError) as error:
                    issues.append({"code": "RECORD_INSPECTION_FAILED", "record": key, "message": str(error)})
            while scripts_pending:
                name, origin, kind = scripts_pending.popleft()
                if name.casefold() in script_seen:
                    edges.append({"from": origin, "to": name, "kind": kind})
                    continue
                if self.max_scripts and len(script_seen) >= self.max_scripts:
                    scripts_pending.appendleft((name, origin, kind))
                    break
                edges.append({"from": origin, "to": name, "kind": kind})
                script_seen.add(name.casefold())
                source = self.sources.get(name)
                script_reports.append({**{k: v for k, v in source.items() if not k.startswith("_")},
                                       "output": self.output_status(source)})
                for dep in source["dependencies"]:
                    scripts_pending.append((dep["script"], name, dep["kind"]))
                for node in walk(source.get("_ast", {})):
                    if node.get("node") == "DotCallExpr" and node.get("method", "").casefold() == "getformfromfile":
                        target = evaluate(node, {})
                        if target:
                            enqueue(target, name, "source_form_literal")
                        else:
                            issues.append({"code": "DYNAMIC_FORM_REFERENCE", "script": name, "line": node["pos"]["line"]})
            if pending and self.max_records and len(seen) >= self.max_records or scripts_pending and self.max_scripts and len(script_seen) >= self.max_scripts:
                break

        audited_bindings = []
        for binding in bindings:
            if not self.sources.cached_lineage(binding["script"], script_seen):
                audited_bindings.append({**binding, "callback_status": "incomplete_source", "inheritance_issue": "script_limit",
                                         "property_coverage": [], "callbacks": []})
                continue
            audited = audit_binding(binding, self.sources, record_key=binding["record"],
                                    owner=owners.get(binding["record"].casefold()), signature=binding["signature"])
            producers.extend({"record": binding["record"], "binding_path": binding["path"], **p}
                             for p in audited.pop("stage_producers"))
            audited_bindings.append(audited)
        represented = {(p["script"].casefold(), p["state"].casefold(), p["line"], p["column"]) for p in producers if p["kind"] == "papyrus"}
        for source in script_reports:
            if not self.sources.cached_lineage(source["script"], script_seen):
                continue
            dependency = audit_binding({"script": source["script"], "scope": "dependency"}, self.sources,
                                       record_key=None, owner=None, signature=None)
            for producer in dependency["stage_producers"]:
                site = (producer["script"].casefold(), producer["state"].casefold(), producer["line"], producer["column"])
                if site not in represented:
                    producers.append({"record": None, "binding_path": None, **producer})
                    represented.add(site)
        for producer in producers:
            if producer["kind"] == "record":
                producer["target"] = owners.get(producer["record"].casefold())
                if producer["target"] and type(producer["stage"]) is int:
                    producer["status"] = "resolved"
            if producer["status"] != "unresolved":
                target = self.graph.resolve(producer["target"])
                if not target or target[-1][1]["signature"] != "QUST":
                    producer["status"] = "unresolved"
                    producer["reason"] = "Stage target does not resolve to a quest"
        # Preserve call sites once per script instance, even when VMAD lists several callbacks on it.
        producers = list({json.dumps(p, sort_keys=True): p for p in producers}.values())
        stages = []
        for key, record in stage_records.items():
            inventory, unmatched = quest_stages(record, [b for b in audited_bindings if b["record"] == key])
            stages.append({"quest": key, "stages": inventory})
            issues.extend({"code": "UNMATCHED_STAGE_CALLBACK", "record": key, **entry} for entry in unmatched)
        stage_indices = {q["quest"].casefold(): {s["stage"] for s in q["stages"]} for q in stages}
        for producer in producers:
            indices = stage_indices.get((producer.get("target") or "").casefold())
            producer["target_stage_declared"] = producer["stage"] in indices if indices is not None and producer["stage"] is not None else None
            if producer["target_stage_declared"] is False:
                producer["status"] = "missing_stage"
        for stage_record in stages:
            for stage in stage_record["stages"]:
                stage["producers"] = [p for p in producers if p.get("target", "") and p["target"].casefold() == stage_record["quest"].casefold()
                                       and p.get("stage") == stage["stage"]]
        if pending:
            issues.append({"code": "RECORD_LIMIT", "pending": len(pending)})
        if scripts_pending:
            scripts_pending = deque(p for p in scripts_pending if p[0].casefold() not in script_seen)
            if scripts_pending:
                issues.append({"code": "SCRIPT_LIMIT", "pending": len(scripts_pending)})
        issues.extend(self._index_issues)
        completeness = {
            "records": not pending and not issues,
            "sources": not scripts_pending and all(s["status"] == "parsed" for s in script_reports),
            "callbacks": all(b.get("callback_status", "covered") == "covered" and not b["inheritance_issue"] for b in audited_bindings),
            "properties": all(p["status"] != "undeclared" for b in audited_bindings for p in b["property_coverage"]),
            "stage_targets": all(p["status"] not in {"unresolved", "missing_stage"} for p in producers),
        }
        return {"quest": {"form_key": root_key, "editor_id": root["editor_id"]},
                "complete": all(completeness.values()), "completeness": completeness,
                "counts": {"records": len(records), "scripts": len(script_reports), "bindings": len(bindings),
                           "stage_entries": sum(len(s["stages"]) for s in stages), "stage_producers": len(producers)},
                "records": records, "edges": edges, "scripts": script_reports, "bindings": audited_bindings,
                "quests": stages, "stage_producers": producers, "issues": issues,
                "freshness": {"pex": dict(Counter(s["output"]["status"] for s in script_reports)),
                              "generated_source": dict(Counter(s.get("generated_source", {}).get("status", "not_requested") for s in script_reports)),
                              "bytecode_matches_source": "unverified"},
                "limits": {"max_records": self.max_records, "max_scripts": self.max_scripts,
                           "pending_records": list(pending), "pending_scripts": [p[0] for p in scripts_pending]}}

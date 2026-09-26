from __future__ import annotations

import re
from pathlib import Path

from creation_lib.esp import native_runtime
from creation_lib.inspection.graph import references
from creation_lib.inspection.papyrus import PapyrusSources
from creation_lib.inspection.records import form_key, record_catalog, vmad_bindings

LINKED_SIGNATURES = ("TERM", "SCEN", "INFO", "PACK", "PERK")
# Read raw: the lazy plugin index carries no reverse references, and SCEN.PNAM rarely decodes.
_OWNER_SUBRECORD = {"SCEN": "PNAM", "PACK": "QNAM", "DIAL": "QNAM", "SMQN": "NNAM"}
_MEMBER_HEADER = re.compile(r"^\s*(?:[\w:\[\]]+\s+)?(?:Function|Event)\s+([\w.]+)\s*\(", re.IGNORECASE | re.MULTILINE)


def _english(value):
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        values = value.get("Values") or []
        hit = next((v for v in values if v.get("Language") == value.get("TargetLanguage", "English")), None)
        return (hit or (values[0] if values else {})).get("String", "")
    return ""


def _field(record, name):
    return next((f[name] for f in record.get("fields", []) if name in f), None)


def _script_key(name):
    return name.replace("/", ":").replace("\\", ":").casefold()


def _topic_children(path):
    """INFO object id -> parent DIAL object id, from GRUP headers (type 7 = topic children).

    The dialogue extractor needs a fully parsed handle; this reads only record headers.
    """
    import mmap
    import struct

    result = {}
    with open(path, "rb") as handle, mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_READ) as data:
        stack = [(24 + struct.unpack_from("<I", data, 4)[0], len(data), None)]
        while stack:
            offset, end, dial = stack.pop()
            while offset < end:
                size = struct.unpack_from("<I", data, offset + 4)[0]
                if data[offset:offset + 4] == b"GRUP":
                    label, kind = struct.unpack_from("<Ii", data, offset + 8)
                    stack.append((offset + size, end, dial))
                    offset, end, dial = offset + 24, offset + size, label if kind == 7 else dial
                    continue
                if data[offset:offset + 4] == b"INFO" and dial is not None:
                    result[struct.unpack_from("<I", data, offset + 12)[0]] = dial
                offset += 24 + size
    return result


def _pending_index(roots):
    """Method-only patch fragments: script key -> member names they define."""
    index = {}
    for root in (Path(r).resolve() for r in roots):
        for path in root.rglob("*.psc"):
            key = ":".join(path.relative_to(root).with_suffix("").parts).casefold()
            text = path.read_text(encoding="utf-8-sig", errors="replace")
            index.setdefault(key, set()).update(m.casefold() for m in _MEMBER_HEADER.findall(text))
    return index


class QuestStatusAudit:
    """Batch triage of converted quests: start wiring plus empty/missing Papyrus bodies.

    Cheaper than QuestPapyrusAudit: it inspects each quest, its own VMAD, and the
    TERM/SCEN/INFO/PACK/PERK fragments owned by or bound to it, without following
    the full record dependency closure.
    """

    def __init__(self, plugin, source_roots, *, base_roots=(), pending_roots=()):
        self.plugin = plugin
        self.base_roots = [Path(p).resolve() for p in base_roots]
        self.sources = PapyrusSources([*self.base_roots, *source_roots])
        self.pending = _pending_index(pending_roots)
        self.catalog = {row["form_key"].casefold(): row for row in record_catalog(plugin)}
        self._linked = None
        self._nodes = None

    def _owners(self, form_id, signature):
        subrecords = native_runtime.plugin_handle_record_subrecords(self.plugin._rust_handle, form_id) or []
        return [form_key(self.plugin, int.from_bytes(data[:4], "little"))
                for sig, data, _ in subrecords if sig == _OWNER_SUBRECORD[signature] and len(data) >= 4
                and int.from_bytes(data[:4], "little")]

    def story_manager_nodes(self):
        if self._nodes is None:
            self._nodes = {}
            for row in self.catalog.values():
                if row["signature"] == "SMQN":
                    for quest in self._owners(row["form_id"], "SMQN"):
                        self._nodes.setdefault(quest.casefold(), []).append(row["form_key"])
        return self._nodes

    def resolve(self, value):
        text = value.strip().removeprefix("0x")
        if re.fullmatch(r"[0-9A-Fa-f]{1,8}", text):
            key = form_key(self.plugin, int(text, 16) if len(text) > 6 else
                           (self.plugin_index() << 24) | int(text, 16))
            return self.catalog.get(key.casefold())
        return next((r for r in self.catalog.values() if (r["editor_id"] or "").casefold() == text.casefold()), None)

    def plugin_index(self):
        return len(self.plugin.header.masters)

    def _inspect(self, form_id):
        inspected = native_runtime.plugin_handle_inspect_record(self.plugin._rust_handle, form_id)
        return inspected["record"] if inspected else {}

    def _origin(self, source):
        if source["status"] == "missing":
            return "missing"
        path = Path(source.get("path") or source["locations"][-1])
        return "base" if any(path.is_relative_to(root) for root in self.base_roots) else "mod"

    def _member_status(self, script, member):
        definitions, _, issue = self.sources.effective(script)
        found = [(s, m) for (_, name), (s, _, _, m) in definitions.items() if name == member.casefold()]
        pending = member.casefold() in self.pending.get(_script_key(script), set())
        if not found:
            return {"status": "missing", "pending_patch": pending, "inheritance_issue": issue}
        source, node = found[-1]
        status = "native" if node.get("is_native") else "implemented" if node.get("body") else "empty"
        return {"status": status, "pending_patch": pending, "defined_in": source["script"]}

    def _script_report(self, binding):
        source = self.sources.get(binding["script"])
        origin = self._origin(source)
        report = {"script": binding["script"], "scope": binding["scope"], "origin": origin, "source_status": source["status"]}
        if "alias" in binding:
            report["alias"] = binding["alias"].get("Alias") if isinstance(binding["alias"], dict) else binding["alias"]
        if origin == "mod":
            members = source.get("members", [])
            pending = self.pending.get(_script_key(binding["script"]), set())
            empty = [m["name"] for m in members if m["status"] == "empty"]
            report.update(members=len(members), empty_members=empty,
                          pending_members=sorted(n for n in empty if n.casefold() in pending),
                          has_patch=_script_key(binding["script"]) in self.pending)
        return report

    def _fragments(self, bindings):
        rows = []
        for binding in bindings:
            fragment = binding.get("fragment")
            if not fragment or not fragment.get("FragmentName"):
                continue
            row = {"script": binding["script"], "fragment": fragment["FragmentName"]}
            if "Quest Stage" in fragment:
                row.update(stage=fragment["Quest Stage"], item=fragment.get("Quest Stage Index", 0))
            if self._origin(self.sources.get(binding["script"])) == "missing":
                row.update(status="missing_script", pending_patch=_script_key(binding["script"]) in self.pending)
            else:
                row.update(self._member_status(binding["script"], fragment["FragmentName"]))
            rows.append(row)
        return rows

    def linked_index(self):
        """quest form_key (casefold) -> linked TERM/SCEN/INFO/PACK/PERK script records."""
        if self._linked is not None:
            return self._linked
        handle = self.plugin._rust_handle
        info_dial = _topic_children(self.plugin.file_path)
        dial_owner = {}
        quests = {k for k, r in self.catalog.items() if r["signature"] == "QUST"}
        self._linked = {}
        for fid in native_runtime.plugin_handle_record_form_ids_with_subrecords(handle, ["VMAD"]):
            key = form_key(self.plugin, fid)
            row = self.catalog.get(key.casefold())
            if not row or row["signature"] not in LINKED_SIGNATURES:
                continue
            bindings, _ = vmad_bindings(self._inspect(fid))
            if not bindings:
                continue
            links = {}
            if row["signature"] == "INFO":
                dial = info_dial.get(fid)
                if dial is not None and dial not in dial_owner:
                    owners = self._owners(dial, "DIAL")
                    dial_owner[dial] = owners[0] if len(owners) == 1 else None
                if dial_owner.get(dial):
                    links[dial_owner[dial].casefold()] = "owner"
            elif row["signature"] in _OWNER_SUBRECORD:
                links.update({o.casefold(): "owner" for o in self._owners(fid, row["signature"])})
            for binding in bindings:
                for _, target in references(binding.get("properties", [])):
                    if target.casefold() in quests:
                        links.setdefault(target.casefold(), "property")
            for quest, via in links.items():
                self._linked.setdefault(quest, []).append(
                    {"record": key, "signature": row["signature"], "editor_id": row["editor_id"], "via": via,
                     "fragments": self._fragments(bindings),
                     "scripts": sorted({b["script"] for b in bindings if "fragment" not in b})})
        return self._linked

    def audit(self, value):
        row = self.resolve(value)
        if not row or row["signature"] != "QUST":
            return {"query": value, "found": False,
                    "issues": ["not_found" if not row else f"not_a_quest:{row['signature']}"]}
        key = row["form_key"]
        record = self._inspect(row["form_id"])
        bindings, vmad_issues = vmad_bindings(record) if _field(record, "VirtualMachineAdapter") else ([], [])
        general = _field(record, "General") or {}
        enam = _field(record, "ENAM")
        nodes = self.story_manager_nodes().get(key.casefold(), [])
        stages = sorted({f["INDX"]["StageIndex"] for f in record.get("fields", [])
                         if isinstance(f.get("INDX"), dict) and "StageIndex" in f["INDX"]})
        scripts = [self._script_report(b) for b in bindings if "fragment" not in b]
        fragments = self._fragments(bindings)
        linked = self.linked_index().get(key.casefold(), [])
        unfinished = {"empty", "missing", "missing_script"}

        def open_count(rows):
            return sum(r["status"] in unfinished and not r["pending_patch"] for r in rows)

        linked_open = {}
        for entry in linked:
            count = open_count(entry["fragments"])
            if count:
                linked_open[entry["signature"]] = linked_open.get(entry["signature"], 0) + count
        summary = {
            "stages": len(stages),
            "objectives": sum(1 for f in record.get("fields", []) if "ObjectiveIndex" in f),
            "aliases": sum(1 for f in record.get("fields", []) if "ALST" in f or "ALLS" in f or "ALCS" in f),
            "quest_fragments": len(fragments),
            "open_quest_fragments": open_count(fragments),
            "pending_quest_fragments": sum(r["pending_patch"] and r["status"] in unfinished for r in fragments),
            "missing_scripts": [s["script"] for s in scripts if s["origin"] == "missing"],
            "hollow_scripts": [s["script"] for s in scripts if s["origin"] == "mod" and s.get("members")
                               and len(s["empty_members"]) - len(s["pending_members"]) > 0],
            "open_linked_fragments": linked_open,
        }
        issues = list(vmad_issues)
        if enam is not None and not nodes:
            issues.append("event_scoped_without_story_manager_node")
        if not bindings:
            issues.append("no_vmad")
        return {"query": value, "found": True, "form_key": key, "editor_id": row["editor_id"],
                "name": _english(_field(record, "Name") or _field(record, "FULL")),
                "start": {"event_scoped": enam is not None,
                          "event": enam.to_bytes(4, "little").decode("latin-1") if isinstance(enam, int) else enam,
                          "flags": general.get("Flags", []), "type": general.get("Type"),
                          "story_manager_nodes": nodes},
                "summary": summary, "scripts": scripts, "fragments": fragments, "linked": linked, "issues": issues}

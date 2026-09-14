from __future__ import annotations

import fnmatch

from creation_lib.esp import native_runtime


def form_key(plugin, form_id):
    index = (int(form_id) >> 24) & 0xFF
    masters = plugin.header.masters
    owner = masters[index] if index < len(masters) else plugin.plugin_name
    return f"{owner}:{int(form_id) & 0xFFFFFF:06X}"


def named_fields(record):
    result, repeated = {}, set()
    for field in record.get("fields", []):
        for name, value in field.items():
            if name in repeated:
                result[name].append(value)
            elif name in result:
                result[name] = [result[name], value]
                repeated.add(name)
            else:
                result[name] = value
    return {**{k: v for k, v in record.items() if k != "raw_payload_hex"}, "fields": result}


def record_catalog(plugin, signatures=()):
    return [{"signature": row[2], "editor_id": row[1], "form_key": row[0], "form_id": row[4]}
            for row in native_runtime.plugin_handle_record_index_rows(plugin._rust_handle, signatures=list(signatures) or None)]


def record_rows(plugin, *, signatures=(), pattern="*", record_ids=()):
    if record_ids:
        candidates = [row for row in record_catalog(plugin, signatures) if row["form_id"] in record_ids]
    else:
        candidates = [row for row in record_catalog(plugin, signatures)
                      if fnmatch.fnmatchcase((row["editor_id"] or "").casefold(), pattern.casefold())]
    for row in candidates:
        inspected = native_runtime.plugin_handle_inspect_record(plugin._rust_handle, row["form_id"])
        if inspected is not None:
            yield {**named_fields(inspected["record"]), "signature": row["signature"], "form_key": row["form_key"]}


def vmad_bindings(record):
    bindings, issues = [], []
    fields = record.get("fields", [])
    vmads = [(name, value) for field in fields for name, value in field.items()
             if name in {"VMAD", "VirtualMachineAdapter"} or isinstance(value, dict) and value.get("kind") == "vmad"]
    if not vmads:
        return [], ["VMAD could not be decoded into the authoring schema"]

    def walk(node, path, alias=None):
        if isinstance(node, list):
            for index, item in enumerate(node):
                walk(item, f"{path}.{index}", alias)
        elif isinstance(node, dict):
            if "Alias Scripts" in node:
                alias = node.get("Object")
            script = node.get("ScriptName")
            if script:
                scope = "alias" if alias is not None else "fragment" if "FragmentName" in node else "fragment_script" if ".Script Fragments." in path else "record"
                binding = {"script": script, "scope": scope, "path": path,
                           "flags": node.get("Flags", 0), "properties": node.get("Properties", [])}
                if alias is not None:
                    binding["alias"] = alias
                if "FragmentName" in node:
                    binding["fragment"] = {k: v for k, v in node.items() if k != "ScriptName"}
                bindings.append(binding)
            for key, value in node.items():
                if key not in {"Properties", "Value"}:
                    walk(value, f"{path}.{key}", alias)

    for index, (name, vmad) in enumerate(vmads):
        if not isinstance(vmad, dict) or not isinstance(vmad.get("Scripts"), list):
            issues.append(f"{name}[{index}]: undecoded VMAD payload")
            continue
        if vmad.get("tail_hex") or vmad.get("raw_hex"):
            issues.append(f"{name}[{index}]: decoder retained raw bytes; attachment coverage may be incomplete")
        walk(vmad, f"fields.{name}.{index}")
    return bindings, issues


def vmad_rows(plugin, diagnostics, *, signatures=(), record_ids=(), script=None, script_glob=None, property_name=None):
    handle = plugin._rust_handle
    candidates = native_runtime.plugin_handle_record_form_ids_with_subrecords(handle, ["VMAD"])
    requested = set(record_ids)
    diagnostics.update(candidate_records=len(candidates), scanned_records=0, issues=[], complete=True)
    for fid in candidates:
        if requested and fid not in requested:
            continue
        context = native_runtime.plugin_handle_call(handle, "record_context_for_form_id", fid)
        signature = context["record_signature"] if context else None
        if signatures and signature not in signatures:
            continue
        diagnostics["scanned_records"] += 1
        identity = {"form_key": form_key(plugin, fid), "form_id": f"{fid:08X}",
                    "signature": signature, "editor_id": None}
        try:
            inspected = native_runtime.plugin_handle_inspect_record(handle, fid)
            record = inspected["record"] if inspected else None
            identity["editor_id"] = (record or {}).get("eid")
            bindings, issues = vmad_bindings(record or {})
        except (RuntimeError, ValueError) as error:
            bindings, issues = [], [str(error)]
        if issues:
            diagnostics["complete"] = False
            diagnostics["issues"].append({**identity, "messages": issues})
        for binding in bindings:
            if script and binding["script"].casefold() != script.casefold():
                continue
            if script_glob and not fnmatch.fnmatchcase(binding["script"].casefold(), script_glob.casefold()):
                continue
            if property_name and not any(p.get("propertyName", "").casefold() == property_name.casefold() for p in binding["properties"]):
                continue
            yield {**identity, **binding}


def normalize_references(value, own_plugin):
    if isinstance(value, list):
        return [normalize_references(item, own_plugin) for item in value]
    if isinstance(value, dict):
        result = {key: normalize_references(item, own_plugin) for key, item in value.items()}
        if "plugin" in result and "object_id" in result:
            name = str(result["plugin"]).casefold()
            result["plugin"] = "$self" if name == own_plugin.casefold() else name
            oid = result["object_id"]
            result["object_id"] = f"{int(oid, 16) if isinstance(oid, str) else oid:06X}"
        return result
    return value


_MISSING = object()


def field_differences(before, after, path="", *, fields=(), exclude=()):
    def included(field):
        return not fields or any(field == p or field.startswith(p + ".") or fnmatch.fnmatchcase(field, p) for p in fields)

    def ignored(field):
        return any(field == p or field.startswith(p + ".") or fnmatch.fnmatchcase(field, p) for p in exclude)

    if ignored(path):
        return []
    dictionaries = isinstance(before, dict) and isinstance(after, dict)
    added_dictionary = before is _MISSING and isinstance(after, dict) and bool(after)
    removed_dictionary = after is _MISSING and isinstance(before, dict) and bool(before)
    if dictionaries or added_dictionary or removed_dictionary:
        before = {} if added_dictionary else before
        after = {} if removed_dictionary else after
        result = []
        for key in sorted(before.keys() | after.keys()):
            child = f"{path}.{key}" if path else key
            result.extend(field_differences(before.get(key, _MISSING), after.get(key, _MISSING), child, fields=fields, exclude=exclude))
        return result
    if isinstance(before, list) and isinstance(after, list):
        return [change for index in range(max(len(before), len(after)))
                for change in field_differences(before[index] if index < len(before) else _MISSING,
                                                after[index] if index < len(after) else _MISSING,
                                                f"{path}.{index}", fields=fields, exclude=exclude)]
    if type(before) is type(after) and before == after:
        return []
    return [{"path": path, "before": None if before is _MISSING else before, "after": None if after is _MISSING else after,
             "before_present": before is not _MISSING, "after_present": after is not _MISSING}] if included(path) else []

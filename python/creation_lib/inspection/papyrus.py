from __future__ import annotations

from hashlib import sha256
from pathlib import Path

from creation_lib.papyrus_lsp.native_runtime import parse_text_native


def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def members(ast):
    for scope in [ast, *ast.get("states", [])]:
        state = "" if scope is ast else scope["name"]
        for kind in ("functions", "events"):
            for member in scope.get(kind, []):
                yield state, kind[:-1], member
    for prop in ast.get("properties", []):
        for accessor in ("getter", "setter"):
            if prop.get(accessor):
                yield "", accessor, {**prop[accessor], "name": prop["name"] + "." + accessor}


def source_slice(text, node):
    pos = node.get("pos", {})
    return "\n".join(text.splitlines()[max(0, pos.get("line", 1) - 1):pos.get("end_line", 0)])


def member_summary(script, state, kind, member, text):
    return {"script": script, "state": state, "kind": kind, "name": member["name"],
            "line": member["pos"]["line"], "end_line": member["pos"]["end_line"],
            "status": "native" if member.get("is_native") else "implemented" if member.get("body") else "empty",
            "body": source_slice(text, member)}


class PapyrusSources:
    def __init__(self, roots, generated_roots=()):
        self.roots = [Path(p).resolve() for p in roots]
        self.generated_roots = [Path(p).resolve() for p in generated_roots]
        self.files = self._index(self.roots)
        self.generated = self._index(self.generated_roots)
        self.cache = {}

    @staticmethod
    def _index(roots):
        result = {}
        for priority, root in enumerate(roots):
            for path in sorted(root.rglob("*")):
                if path.is_file() and path.suffix.casefold() == ".psc":
                    name = ":".join(path.relative_to(root).with_suffix("").parts).casefold()
                    result.setdefault(name, []).append((priority, path))
        return result

    def get(self, name):
        key = name.casefold()
        if key in self.cache:
            return self.cache[key]
        locations = self.files.get(key, [])
        result = {"script": name, "status": "missing", "locations": [str(p) for _, p in locations],
                  "dependencies": [], "members": [], "properties": [], "errors": []}
        self.cache[key] = result
        if not locations:
            return result
        priority, path = locations[-1]
        if sum(p == priority for p, _ in locations) != 1:
            result["status"] = "ambiguous"
            return result
        result["path"] = str(path)
        try:
            raw = path.read_bytes()
            text = raw.decode("utf-8-sig")
            parsed = parse_text_native(text)
            ast = parsed.get("ast") or {}
            result.update(status="parsed", sha256=sha256(raw).hexdigest(), mtime_ns=path.stat().st_mtime_ns,
                          errors=parsed.get("errors", []), parent=ast.get("parent"), _ast=ast, _text=text)
            if result["errors"] or ast.get("name", "").casefold() != key:
                result["status"] = "invalid"
                if ast.get("name", "").casefold() != key:
                    result["errors"].append({"message": "Scriptname does not match the source-relative path"})
            result["members"] = [member_summary(name, state, kind, member, text)
                                 for state, kind, member in members(ast)]
            result["properties"] = ast.get("properties", [])
            dependencies = set()
            if ast.get("parent"):
                dependencies.add((ast["parent"], "extends"))
            dependencies.update((i["script_name"], "import") for i in ast.get("imports", []))
            locals_ = {s["name"].casefold() for s in ast.get("structs", [])}
            names = {n["name"].casefold() for n in ast.get("properties", []) + ast.get("variables", [])}
            for _, _, member in members(ast):
                names.update(p["name"].casefold() for p in member.get("params", []))
                names.update(n["name"].casefold() for n in walk(member) if n.get("node") == "LocalVarStmt")
            for node in walk(ast):
                for field in ("type", "return_type", "target_type", "element_type"):
                    value = node.get(field, "").removesuffix("[]")
                    if value and value.casefold() not in {"int", "float", "bool", "string", "none", "var", *locals_}:
                        dependencies.add((value.split("#", 1)[0], "type"))
                if node.get("node") == "DotCallExpr" and node.get("object", {}).get("node") == "NameExpr":
                    value = node["object"]["name"]
                    if value.casefold() not in names | {"self", "parent"}:
                        dependencies.add((value, "static_call"))
            result["dependencies"] = [{"script": dep, "kind": kind} for dep, kind in sorted(dependencies)
                                      if dep.casefold() != key]
            generated = self.generated.get(key, [])
            if self.generated_roots:
                result["generated_source"] = {"status": "missing"}
                if generated:
                    g_priority, g_path = generated[-1]
                    g_hash = sha256(g_path.read_bytes()).hexdigest()
                    status = "identical" if g_hash == result["sha256"] else "different"
                    if sum(p == g_priority for p, _ in generated) != 1:
                        status = "ambiguous"
                    result["generated_source"] = {"path": str(g_path), "sha256": g_hash, "status": status}
        except (OSError, UnicodeError, ValueError, RuntimeError) as error:
            result.update(status="invalid", errors=[{"message": str(error)}])
        return result

    def lineage(self, name):
        chain, seen = [], set()
        while name:
            if name.casefold() in seen:
                return chain, "inheritance_cycle"
            seen.add(name.casefold())
            source = self.get(name)
            chain.append(source)
            if source["status"] != "parsed":
                return chain, "incomplete_inheritance"
            name = source.get("parent")
        return chain, None

    def effective(self, name):
        chain, issue = self.lineage(name)
        definitions, properties = {}, {}
        for source in reversed(chain):
            for state, kind, member in members(source.get("_ast", {})):
                definitions[(state.casefold(), member["name"].casefold())] = (source, state, kind, member)
            for prop in source.get("properties", []):
                properties[prop["name"].casefold()] = (source, prop)
        return definitions, properties, issue

    def cached_lineage(self, name, allowed):
        seen = set()
        while name and name.casefold() not in seen:
            key = name.casefold()
            if key not in allowed:
                return False
            seen.add(key)
            name = self.cache[key].get("parent")
        return True


def property_uses(member, name):
    local_names = {n["name"].casefold() for n in member.get("params", [])}
    local_names.update(n["name"].casefold() for n in walk(member.get("body", []))
                       if n.get("node") == "LocalVarStmt")
    lines = set()
    for node in walk(member.get("body", [])):
        if node.get("node") == "NameExpr" and node["name"].casefold() == name and name not in local_names:
            lines.add(node["pos"]["line"])
        if node.get("node") == "DotExpr" and node["member"].casefold() == name:
            obj = node.get("object", {})
            if obj.get("node") == "NameExpr" and obj["name"].casefold() == "self":
                lines.add(node["pos"]["line"])
    return sorted(lines)


def evaluate(expr, values, owner=None, self_quest=None):
    if not isinstance(expr, dict):
        return None
    kind = expr.get("node")
    if kind == "LiteralExpr":
        return expr.get("value")
    if kind == "ParentExpr":
        return self_quest
    if kind == "NameExpr":
        return self_quest if expr["name"].casefold() == "self" else values.get(expr["name"].casefold())
    if kind == "CastExpr":
        return evaluate(expr["expr"], values, owner, self_quest)
    if kind == "ArrayAccessExpr":
        array = evaluate(expr["array"], values, owner, self_quest)
        index = evaluate(expr["index"], values, owner, self_quest)
        return array[index] if isinstance(array, list) and type(index) is int and 0 <= index < len(array) else None
    if kind == "DotExpr" and expr.get("object", {}).get("name", "").casefold() == "self":
        return values.get(expr["member"].casefold())
    if kind in {"CallExpr", "DotCallExpr"}:
        method = expr.get("function", expr.get("method", "")).casefold()
        obj = expr.get("object", {})
        if method in {"getowningquest", "getquest"}:
            if kind == "CallExpr" or obj.get("name", "").casefold() in {"self", "parent"} or obj.get("node") == "ParentExpr":
                return owner
            value = evaluate(obj, values, owner, self_quest)
            if isinstance(value, dict) and "alias_quest" in value:
                return value["alias_quest"]
        if method == "getformfromfile" and obj.get("name", "").casefold() == "game":
            args = expr.get("args", [])
            if len(args) >= 2:
                oid = evaluate(args[0], values)
                plugin = evaluate(args[1], values)
                if type(oid) is int and 0 < oid <= 0xFFFFFF and isinstance(plugin, str):
                    return f"{plugin}:{oid:06X}"
    return None


def binding_value(value):
    from creation_lib.inspection.graph import references
    if isinstance(value, list):
        return [binding_value(v) for v in value]
    if isinstance(value, dict):
        refs = list(references(value))
        if len(refs) == 1:
            return {"alias_quest": refs[0][1], "alias": value["Alias"]} if value.get("Alias", -1) >= 0 else refs[0][1]
        return None
    return value


def audit_binding(binding, sources, *, record_key, owner, signature):
    name = binding["script"]
    definitions, props, inheritance_issue = sources.effective(name)
    values = {key: evaluate(prop.get("default"), {}) for key, (_, prop) in props.items()}
    values.update({p["propertyName"].casefold(): binding_value(p.get("Value")) for p in binding.get("properties", [])})
    chain, _ = sources.lineage(name)
    self_quest = record_key if signature == "QUST" and binding["scope"] != "alias" else None
    result = {**binding, "record": record_key, "inheritance": [s["script"] for s in chain],
              "inheritance_issue": inheritance_issue, "property_coverage": [], "callbacks": [], "stage_producers": []}
    callback = binding.get("fragment", {}).get("FragmentName")
    bound = {p["propertyName"].casefold(): p for p in binding.get("properties", [])}
    for key in sorted(props.keys() | bound.keys()) if not callback else []:
        prop = bound.get(key)
        declaration = props.get(key)
        uses = [{"script": s["script"], "member": m["name"], "state": state, "lines": lines}
                for s, state, _, m in definitions.values() if (lines := property_uses(m, key))]
        result["property_coverage"].append({"name": prop["propertyName"] if prop else declaration[1]["name"],
                                            "value": prop.get("Value") if prop else None,
                                            "binding": "vmad" if prop else "source_default_or_runtime",
                                            "declared_by": declaration[0]["script"] if declaration else None,
                                            "status": "used" if declaration and uses else "unused" if declaration else "undeclared",
                                            "uses": uses})
    for source, state, kind, member in definitions.values():
        if callback and member["name"].casefold() == callback.casefold() or not callback and kind == "event":
            result["callbacks"].append({**member_summary(source["script"], state, kind, member, source["_text"]),
                                        "path": source["path"], "inherited": source["script"].casefold() != name.casefold()})
        if callback:
            continue
        local_values = dict(values)
        for param in member.get("params", []):
            local_values[param["name"].casefold()] = None
        # Only immutable local initializers can be used without control-flow analysis.
        assignments = set()
        for node in walk(member.get("body", [])):
            if node.get("node") == "AssignStmt":
                target = node.get("target", {})
                assignments.add(target.get("name", target.get("member", "")).casefold())
        for node in walk(member.get("body", [])):
            if node.get("node") == "LocalVarStmt":
                key = node["name"].casefold()
                local_values[key] = None if key in assignments else evaluate(node.get("value"), local_values, owner, self_quest)
        for key in assignments:
            local_values[key] = None
        for node in walk(member.get("body", [])):
            if node.get("node") not in {"CallExpr", "DotCallExpr"}:
                continue
            method = node.get("function", node.get("method", "")).casefold()
            if method not in {"setstage", "setcurrentstageid"}:
                continue
            target = self_quest if node["node"] == "CallExpr" else evaluate(node["object"], local_values, owner, self_quest)
            args = node.get("args", [])
            stage = evaluate(args[0], local_values, owner, self_quest) if args else None
            resolved = isinstance(target, str) and ":" in target and type(stage) is int
            result["stage_producers"].append({"kind": "papyrus", "script": source["script"], "path": source["path"],
                "member": member["name"], "state": state, "line": node["pos"]["line"], "column": node["pos"]["col"],
                "target": target if isinstance(target, str) else None, "stage": stage if type(stage) is int else None,
                "status": "resolved_initial_values" if resolved else "unresolved", "expression": source_slice(source["_text"], node)})
    if callback:
        result["callback_status"] = "missing" if not result["callbacks"] else "empty" if any(
            c["status"] == "empty" for c in result["callbacks"]) else "covered"
        if inheritance_issue or sources.get(name)["status"] != "parsed":
            result["callback_status"] = "incomplete_source"
    return result

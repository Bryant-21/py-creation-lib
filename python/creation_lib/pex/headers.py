"""Synthesize header-only `.psc` from compiled `.pex`.

Papyrus resolves every type from source, and Fallout 4 ships only compiled
`.pex` — the vanilla `.psc` arrive with the Creation Kit. Rather than requiring
the CK, rebuild a type-only view of the base game from the `.pex` every install
already has, and hand it to the ordinary import path.

The emitted text is never executed. It exists so the compiler can answer "what
is this type, what does this function return, what are its parameters" while
compiling real scripts against it.

Two facts are absent from `.pex` and come from a manifest generated once from CK
sources by `tools/gen_papyrus_defaults.py`:

* **Default parameter values.** Not encoded at all. Without them every optional
  parameter becomes required and any caller that omits one fails to compile.
* **Which declarations are events.** The debug table types `OnInit` and
  `GetFormID` identically, yet the compiler validates every overriding `Event`
  against its inherited signature.
"""
from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any, Iterable

DATA_DIR = Path(__file__).with_name("data")
_DATA_PATH = DATA_DIR / "fo4_papyrus_defaults.json"
FLAGS_FILE_NAME = "Institute_Papyrus_Flags.flg"


@lru_cache(maxsize=1)
def load_manifest(path: str | None = None) -> dict[str, Any]:
    """Load the API metadata `.pex` cannot express."""
    source = Path(path) if path else _DATA_PATH
    if not source.is_file():
        return {"defaults": {}, "events": {}}
    with source.open(encoding="utf-8") as handle:
        payload = json.load(handle)
    return {
        "defaults": payload.get("defaults", {}),
        "events": payload.get("events", {}),
    }


def _source_type(pex_type: str, owner: str) -> str:
    """Translate a PEX type name into Papyrus source spelling.

    `.pex` qualifies a struct as `script#struct`; source uses `script:struct`,
    or the bare struct name inside its own script. `#` is a parse error, and a
    single bad declaration takes the whole header down with it — the resolver
    then caches a `None` AST for that script and every call into it resolves to
    void, which surfaces far away as "cannot assign None to Int".
    """
    if "#" not in pex_type:
        return pex_type
    array = "[]" if pex_type.endswith("[]") else ""
    script, _, struct = pex_type[: len(pex_type) - len(array)].partition("#")
    if script.casefold() == owner.casefold():
        return f"{struct}{array}"
    return f"{script}:{struct}{array}"


def _params_text(
    function: Any,
    defaults: dict[str, str],
    owner: str,
) -> str:
    rendered: list[str] = []
    for index, param in enumerate(function.params):
        text = f"{_source_type(param.type, owner)} {param.name}"
        default = defaults.get(str(index))
        if default is not None:
            text = f"{text} = {default}"
        rendered.append(text)
    return ", ".join(rendered)


def _unique_callables(obj: Any) -> list[Any]:
    """One declaration per name.

    A name may appear in several states with the same signature; the type
    universe only needs it once, and the empty (default) state is authoritative.
    """
    by_name: dict[str, Any] = {}
    for state in sorted(obj.states, key=lambda s: s.name != ""):
        for function in state.functions:
            # `::remote_*` and friends are compiler-generated mangled names.
            # `::` is not legal in source, and emitting one does not merely fail
            # to parse — it sends the parser into a runaway allocation that
            # aborts the process.
            if function.name.startswith("::"):
                continue
            by_name.setdefault(function.name.lower(), function)
    return [by_name[key] for key in sorted(by_name)]


def _emit_structs(obj: Any) -> list[str]:
    lines: list[str] = []
    for struct in obj.structs:
        lines.append(f"Struct {struct.name}")
        for member in struct.members:
            lines.append(f"    {_source_type(member.type, obj.name)} {member.name}")
        lines.append("EndStruct")
        lines.append("")
    return lines


def _emit_properties(obj: Any) -> list[str]:
    lines: list[str] = []
    for prop in obj.properties:
        if prop.name.startswith("::"):
            continue
        # Auto for every property regardless of accessor flags: the import path
        # consumes name and type only, and a read-only property rendered
        # writable can at worst admit an assignment the game would reject —
        # never miscompile one that is correct.
        lines.append(
            f"{_source_type(prop.type, obj.name)} Property {prop.name} Auto"
        )
    if lines:
        lines.append("")
    return lines


def _emit_callables(obj: Any, manifest: dict[str, Any]) -> list[str]:
    script_key = obj.name.lower()
    defaults_for = manifest["defaults"].get(script_key, {})
    events_for = set(manifest["events"].get(script_key, ()))

    lines: list[str] = []
    for function in _unique_callables(obj):
        name_key = function.name.lower()
        params = _params_text(function, defaults_for.get(name_key, {}), obj.name)
        if name_key in events_for:
            lines.append(f"Event {function.name}({params})")
            lines.append("EndEvent")
            lines.append("")
            continue
        # `Native` for every function: it needs no body, so the header carries
        # no synthesized return statement that could differ from the real one.
        returns = function.return_type
        prefix = (
            ""
            if returns in (None, "", "None")
            else f"{_source_type(returns, obj.name)} "
        )
        keywords = " Global" if function.is_global else ""
        lines.append(f"{prefix}Function {function.name}({params}){keywords} Native")
    return lines


def emit_header(obj: Any, manifest: dict[str, Any] | None = None) -> str:
    """Render one PEX object as a header-only `.psc`."""
    manifest = manifest if manifest is not None else load_manifest()

    header = f"Scriptname {obj.name}"
    if obj.parent:
        header = f"{header} Extends {obj.parent}"

    lines = [header, ""]
    lines.extend(_emit_structs(obj))
    lines.extend(_emit_properties(obj))
    lines.extend(_emit_callables(obj, manifest))
    return "\n".join(lines).rstrip() + "\n"


def emit_headers_for_file(
    pex_path: str | Path,
    manifest: dict[str, Any] | None = None,
) -> Iterable[tuple[str, str]]:
    """Yield `(script_name, header_text)` for each object in a `.pex`."""
    from creation_lib.pex.native_runtime import parse_pex_file_native

    manifest = manifest if manifest is not None else load_manifest()
    for obj in parse_pex_file_native(pex_path).objects:
        yield obj.name, emit_header(obj, manifest)

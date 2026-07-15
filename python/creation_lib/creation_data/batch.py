"""Batch execution: run multiple creation-data queries in parallel."""

from __future__ import annotations

import os
from concurrent.futures import ThreadPoolExecutor


def _get_dispatch() -> dict:
    """Lazy-import sibling modules and return tool dispatch table."""
    from .search import search, semantic_search, search_by_keyword
    from .content import get_content, get_behavior_xml
    from .records import get_record, get_references, lookup_editor_id, resolve_keywords, count_references
    from .scripts import get_function, list_functions, get_script_api, get_script_hierarchy
    from .listing import list_items

    return {
        "search": search,
        "get_content": get_content,
        "list_items": list_items,
        "semantic_search": semantic_search,
        "get_record": get_record,
        "get_references": get_references,
        "lookup_editor_id": lookup_editor_id,
        "search_by_keyword": search_by_keyword,
        "resolve_keywords": resolve_keywords,
        "count_references": count_references,
        "get_function": get_function,
        "list_functions": list_functions,
        "get_script_api": get_script_api,
        "get_script_hierarchy": get_script_hierarchy,
        "get_behavior_xml": get_behavior_xml,
    }


def _exec_command(cmd: dict, dispatch: dict) -> dict:
    """Execute a single batch command."""
    if not isinstance(cmd, dict):
        return {"tool": "", "args": {}, "result": {"error": "Each command must be a dict with 'tool' and 'args' keys."}}
    if "tool" not in cmd:
        return {"tool": "", "args": cmd.get("args", {}), "result": {"error": "Missing required 'tool' key in command."}}
    tool_name = cmd["tool"]
    args = cmd.get("args", {})
    fn = dispatch.get(tool_name)
    if fn is None:
        return {
            "tool": tool_name,
            "args": args,
            "result": {"error": f"Unknown tool '{tool_name}'. Valid: {', '.join(sorted(dispatch))}"},
        }
    try:
        result = fn(**args)
    except TypeError as e:
        msg = str(e)
        if "missing" in msg and "required" in msg:
            result = {"error": f"Missing required arguments for '{tool_name}'. Check the tool's parameter list."}
        elif "unexpected keyword argument" in msg:
            result = {"error": f"Unknown argument passed to '{tool_name}': {msg.split('unexpected keyword argument ')[-1]}"}
        else:
            result = {"error": f"Invalid arguments for '{tool_name}'."}
    except Exception as e:
        result = {"error": f"Error in '{tool_name}': {e}"}
    return {"tool": tool_name, "args": args, "result": result}


def batch(
    commands: list[dict],
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """Execute multiple creation-data queries in parallel to avoid serial round-trips.

    Each command reuses the same tool names and arguments as the individual tools.

    Args:
        commands: List of command dicts, each with:
            - "tool": tool name (e.g. "search", "get_content", "get_record")
            - "args": dict of arguments for that tool
        game: Default game for commands that don't specify their own game arg.
        db_dir: Default db_dir for commands that don't specify their own.

    Returns list of results in the same order, each with "tool", "args", and "result" keys.
    If a command fails, its "result" will contain an "error" key.
    """
    dispatch = _get_dispatch()

    # Inject default game and db_dir into commands that don't specify them
    for cmd in commands:
        args = cmd.get("args", {})
        if game and "game" not in args:
            args["game"] = game
        if db_dir and "db_dir" not in args:
            args["db_dir"] = db_dir
        cmd["args"] = args

    if len(commands) <= 1:
        return [_exec_command(cmd, dispatch) for cmd in commands]

    max_workers = min(len(commands), max(1, (os.cpu_count() or 2) // 2))
    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        results = list(pool.map(lambda cmd: _exec_command(cmd, dispatch), commands))
    return results

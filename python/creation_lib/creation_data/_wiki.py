"""Helpers for parsing the current wiki database schema."""

from __future__ import annotations

import re
from collections.abc import Mapping


_SECTION_RE_TEMPLATE = r"(?is)##\s+{heading}\s*(?P<body>.*?)(?=\n##\s+|\Z)"
_MEMBER_OF_RE = re.compile(r"\*\*Member of:\*\*\s*\[(?P<script>[^\]]+?)\s+Script\]\(", re.IGNORECASE | re.DOTALL)
_EXTENDS_RE = re.compile(r"\*\*Extends:\*\*\s*\[(?P<script>[^\]]+?)\]\(", re.IGNORECASE | re.DOTALL)


def _page_dict(page: Mapping[str, object]) -> dict:
    return dict(page)


def _strip_script_suffix(value: str) -> str:
    value = value.strip()
    for suffix in ("_(Papyrus)", "_Script", " Script"):
        if value.endswith(suffix):
            value = value[: -len(suffix)]
    return value.strip()


def script_type_from_page(page: Mapping[str, object]) -> str:
    data = _page_dict(page)
    for candidate in (data.get("title", ""), data.get("filename", "")):
        if not isinstance(candidate, str) or not candidate.strip():
            continue
        script_type = _strip_script_suffix(candidate)
        if script_type and script_type != candidate:
            return script_type
    return ""


def function_name_from_page(page: Mapping[str, object]) -> str:
    data = _page_dict(page)
    title = str(data.get("title", "") or "").strip()
    if " - " in title:
        return title.split(" - ", 1)[0].strip()
    filename = str(data.get("filename", "") or "").strip()
    for marker in ("_-_", "_(Papyrus)"):
        if marker in filename:
            return filename.split(marker, 1)[0].strip()
    return title or filename


def function_member_script_from_page(page: Mapping[str, object]) -> str:
    content = str(_page_dict(page).get("content", "") or "")
    match = _MEMBER_OF_RE.search(content)
    if match:
        return match.group("script").strip()
    return ""


def script_extends_from_page(page: Mapping[str, object]) -> str:
    content = str(_page_dict(page).get("content", "") or "")
    match = _EXTENDS_RE.search(content)
    if match:
        return match.group("script").strip()
    return ""


def section_text_from_page(page: Mapping[str, object], heading: str) -> str:
    content = str(_page_dict(page).get("content", "") or "")
    pattern = re.compile(_SECTION_RE_TEMPLATE.format(heading=re.escape(heading)))
    match = pattern.search(content)
    if not match:
        return ""
    return match.group("body").strip()

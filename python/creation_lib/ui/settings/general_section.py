"""General settings section (default game, addon node range, Gitea, setup, .env sync)."""
from __future__ import annotations

import logging

from imgui_bundle import imgui

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.ui.shell.settings_section import SettingsContext, SettingsSection

_log = logging.getLogger("toolkit.settings.general")

_ENV_KEY_MAP: dict[str, dict[str, str]] = {
    "fo4": {"root": "FO4_DIR", "extracted": "FO4_EXTRACTED_DIR"},
    "skyrimse": {"root": "SKYRIMSE_DIR", "extracted": "SKYRIMSE_EXTRACTED_DIR"},
    "starfield": {"root": "STARFIELD_DIR", "extracted": "STARFIELD_EXTRACTED_DIR"},
    "fo76": {"root": "FO76_DIR", "extracted": "FO76_EXTRACTED_DIR"},
    "fo3": {"root": "FO3_DIR", "extracted": "FO3_EXTRACTED_DIR"},
    "fnv": {"root": "FONV_DIR", "extracted": "FONV_EXTRACTED_DIR"},
}


class _State:
    active_game: str = "fo4"
    gitea_url: str = ""
    gitea_username: str = ""
    gitea_orgs: dict = {}
    gitea_token: str = ""
    env_sync_status: str = ""
    env_path: object = None  # Path | None — set by make_section()
    # callbacks set by SettingsWindow/app (optional)
    rerun_setup_cb: object = None  # callable() | None
    close_cb: object = None        # callable() | None
    paths_changed_cb: object = None  # callable() | None
    paths_commit_cb: object = None  # callable() | None


_state = _State()


def _parse_env_file() -> dict[str, str]:
    try:
        env_path = _state.env_path
        if env_path is None or not env_path.exists():
            return {}
        result = {}
        for line in env_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, _, v = line.partition("=")
                result[k.strip()] = v.strip().strip('"').strip("'")
        return result
    except Exception:
        return {}


def _update_env_file(updates: dict[str, str]) -> bool:
    try:
        env_path = _state.env_path
        if env_path is None or not env_path.exists():
            return False
        lines = env_path.read_text(encoding="utf-8").splitlines()
        written: set[str] = set()
        result = []
        for line in lines:
            stripped = line.strip()
            if stripped and not stripped.startswith("#") and "=" in stripped:
                key = stripped.split("=", 1)[0].strip()
                if key in updates:
                    result.append(f'{key}="{updates[key]}"')
                    written.add(key)
                    continue
            result.append(line)
        for key, val in updates.items():
            if key not in written:
                result.append(f'{key}="{val}"')
        env_path.write_text("\n".join(result) + "\n", encoding="utf-8")
        return True
    except Exception:
        return False


def _import_from_env(settings) -> None:
    env = _parse_env_file()
    if not env:
        _state.env_sync_status = "No .env file found"
        return
    count = 0
    for game_id, field_map in _ENV_KEY_MAP.items():
        for field, env_key in field_map.items():
            if env.get(env_key):
                settings._paths[game_id][field + "_dir" if field in ("root", "extracted") else field] = env[env_key]
                count += 1
    if env.get("DEFAULT_GAME"):
        _state.active_game = env["DEFAULT_GAME"]
        count += 1
    if env.get("MOD_PREFIX"):
        settings.mod_prefix = env["MOD_PREFIX"]
        count += 1
    if env.get("ADDON_NODE_INDEX_START"):
        try:
            settings.addon_node_index_start = int(env["ADDON_NODE_INDEX_START"])
            count += 1
        except ValueError:
            pass
    if env.get("CONVERSION_ADDON_NODE_INDEX_START"):
        try:
            settings.conversion_addon_node_index_start = int(env["CONVERSION_ADDON_NODE_INDEX_START"])
            count += 1
        except ValueError:
            pass
    for key, attr in (
        ("GITEA_URL", "gitea_url"),
        ("GITEA_USER", "gitea_username"),
        ("GITEA_TOKEN", "gitea_token"),
    ):
        if env.get(key) is not None:
            setattr(_state, attr, env[key])
            count += 1
    for game_id in GAME_PROFILES:
        org_key = f"GITEA_ORG_{game_id.upper()}"
        if env.get(org_key) is not None:
            _state.gitea_orgs[game_id] = env[org_key]
            count += 1
    if callable(_state.paths_changed_cb):
        _state.paths_changed_cb()
    _state.env_sync_status = f"Imported {count} value(s) from .env"
    _log.info("ENV→UI: %s", _state.env_sync_status)


def _export_to_env(settings) -> None:
    if callable(_state.paths_commit_cb):
        _state.paths_commit_cb()
    updates: dict[str, str] = {}
    for game_id, field_map in _ENV_KEY_MAP.items():
        gp = settings.get_game_paths(game_id)
        for field, env_key in field_map.items():
            db_field = field + "_dir" if field in ("root", "extracted") else field
            updates[env_key] = gp.get(db_field, "")
    updates["DEFAULT_GAME"] = _state.active_game
    if settings.mod_prefix:
        updates["MOD_PREFIX"] = settings.mod_prefix
    updates["ADDON_NODE_INDEX_START"] = str(settings.addon_node_index_start)
    updates["CONVERSION_ADDON_NODE_INDEX_START"] = str(settings.conversion_addon_node_index_start)
    updates["GITEA_URL"] = _state.gitea_url
    updates["GITEA_USER"] = _state.gitea_username
    updates["GITEA_TOKEN"] = _state.gitea_token
    for game_id in GAME_PROFILES:
        updates[f"GITEA_ORG_{game_id.upper()}"] = _state.gitea_orgs.get(game_id, "")
    if _update_env_file(updates):
        _state.env_sync_status = f"Exported {len(updates)} value(s) to .env"
        _log.info("UI→ENV: %s", _state.env_sync_status)
    else:
        _state.env_sync_status = "Export failed: .env not found"


def _draw(ctx: SettingsContext) -> None:
    settings = ctx.settings

    imgui.text("Default Game")
    imgui.separator()
    imgui.spacing()

    game_ids = list(GAME_PROFILES.keys())
    game_labels = [GAME_PROFILES[g].display_name for g in game_ids]
    current_idx = game_ids.index(_state.active_game) if _state.active_game in game_ids else 0
    imgui.set_next_item_width(200)
    changed, new_idx = imgui.combo("##default_game", current_idx, game_labels)
    if changed:
        _state.active_game = game_ids[new_idx]
        settings.set_active_game(_state.active_game)

    imgui.spacing()
    imgui.separator()
    imgui.spacing()

    imgui.text("AddonNode Index Range")
    imgui.separator()
    imgui.spacing()
    imgui.set_next_item_width(120)
    changed, val = imgui.input_int("Start Index##addon_start", settings.addon_node_index_start)
    if changed:
        settings.addon_node_index_start = max(1, val)
    imgui.set_item_tooltip(
        "Starting index for new AddonNode allocations.\n"
        "Reserve a unique range to avoid collisions with other modders.\n"
        "Your current range starts at this value."
    )

    imgui.spacing()
    imgui.set_next_item_width(120)
    changed, val = imgui.input_int("Conversion Start Index##addon_conv_start", settings.conversion_addon_node_index_start)
    if changed:
        settings.conversion_addon_node_index_start = max(1, val)
    imgui.set_item_tooltip(
        "Starting index for AddonNode allocations created by the conversion workspace.\n"
        "This range is separate from regular mods."
    )

    imgui.spacing()
    imgui.separator()
    imgui.spacing()

    imgui.text("Gitea Server")
    imgui.separator()
    imgui.spacing()
    imgui.text_disabled("Auto-create a git repo on your Gitea server when setting up a new mod.")
    imgui.spacing()

    _gitea_label_w = imgui.calc_text_size("Username").x + 12
    _gitea_input_w = imgui.get_content_region_avail().x - _gitea_label_w

    imgui.text("URL")
    imgui.same_line(_gitea_label_w)
    imgui.set_next_item_width(_gitea_input_w)
    _, _state.gitea_url = imgui.input_text("##gitea_url", _state.gitea_url)
    if imgui.is_item_hovered():
        imgui.set_tooltip("Gitea server URL, e.g. https://192.168.1.252:3100")

    imgui.text("Username")
    imgui.same_line(_gitea_label_w)
    imgui.set_next_item_width(_gitea_input_w)
    _, _state.gitea_username = imgui.input_text("##gitea_username", _state.gitea_username)
    if imgui.is_item_hovered():
        imgui.set_tooltip("Your Gitea username — used for authentication and fallback repo owner")

    imgui.spacing()
    imgui.text_disabled("Orgs (optional) — repos are created under the org for that game, or your username if blank.")
    imgui.spacing()
    _org_label_w = imgui.calc_text_size("Starfield").x + 12
    _org_input_w = imgui.get_content_region_avail().x - _org_label_w
    for game_id, profile in GAME_PROFILES.items():
        imgui.text(profile.display_name)
        imgui.same_line(_org_label_w)
        imgui.set_next_item_width(_org_input_w)
        current = _state.gitea_orgs.get(game_id, "")
        changed, new_val = imgui.input_text(f"##gitea_org_{game_id}", current)
        if changed:
            _state.gitea_orgs[game_id] = new_val
    imgui.spacing()

    imgui.text("Token")
    imgui.same_line(_gitea_label_w)
    imgui.set_next_item_width(_gitea_input_w)
    _, _state.gitea_token = imgui.input_text(
        "##gitea_token", _state.gitea_token, flags=imgui.InputTextFlags_.password.value
    )
    if imgui.is_item_hovered():
        imgui.set_tooltip("Optional API token — fallback if push-to-create is not enabled")

    imgui.spacing()
    imgui.separator()
    imgui.spacing()

    imgui.text("Setup")
    imgui.separator()
    imgui.spacing()
    if imgui.button("Rerun Setup Wizard", imgui.ImVec2(200, 0)):
        if callable(_state.rerun_setup_cb):
            _state.rerun_setup_cb()
    if imgui.is_item_hovered():
        imgui.set_tooltip("Close settings and rerun the first-time setup wizard")
    imgui.spacing()
    imgui.separator()
    imgui.spacing()

    imgui.text(".env Sync")
    imgui.separator()
    imgui.spacing()
    imgui.text_disabled("Sync settings with the .env file used by CLI scripts and MCP servers.")
    imgui.spacing()
    if imgui.button("Import from .env", imgui.ImVec2(180, 0)):
        _import_from_env(settings)
    if imgui.is_item_hovered():
        imgui.set_tooltip("Load game paths, Gitea settings, and MOD_PREFIX from .env into the UI")
    imgui.same_line()
    if imgui.button("Export to .env", imgui.ImVec2(160, 0)):
        _export_to_env(settings)
    if imgui.is_item_hovered():
        imgui.set_tooltip("Write current UI settings back to .env (also happens automatically on Save)")
    if _state.env_sync_status:
        imgui.same_line()
        imgui.text_disabled(_state.env_sync_status)


def _load(saved: dict) -> None:
    _state.active_game = saved.get("active_game", "fo4")
    g = saved.get("gitea", {})
    _state.gitea_url = g.get("url", "")
    _state.gitea_username = g.get("username", "")
    _state.gitea_orgs = dict(g.get("orgs", {}))
    _state.gitea_token = g.get("token", "")


def _save() -> dict:
    return {
        "active_game": _state.active_game,
        "gitea": {
            "url": _state.gitea_url,
            "username": _state.gitea_username,
            "orgs": dict(_state.gitea_orgs),
            "token": _state.gitea_token,
        },
    }


def make_section(env_path=None) -> SettingsSection:
    _state.env_path = env_path
    return SettingsSection(id="general", label="General", draw=_draw, load=_load, save=_save)

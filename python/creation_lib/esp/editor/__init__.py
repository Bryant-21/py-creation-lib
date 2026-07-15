"""ESP editor — multi-plugin coordinator for the toolkit's xEdit-style workspace.

This module sits on top of `creation_lib.esp.native_runtime` and provides:
- `EditorSession` for managing multiple loaded plugins and a load order
- `validate` for producing error reports across the load order
- `copy_as_override` / `copy_as_override_deep` for cloning records into an active plugin
- `decode_record` / `encode_field` for schema-driven field UI
"""

from creation_lib.esp.editor.session import (
    EditorSession,
    LoadedPlugin,
    ConflictStatus,
    detect_game,
)
from creation_lib.esp.editor.validate import (
    Issue,
    IssueCategory,
    Severity,
    ValidationReport,
    validate,
)
from creation_lib.esp.editor.override import copy_as_new, copy_as_override
from creation_lib.esp.editor.masters_ops import add_masters, clean_masters, sort_masters
from creation_lib.esp.editor.cleanup import remove_itm_records, undelete_and_disable_refs
from creation_lib.esp.editor.reference_info import build_reference_index
from creation_lib.esp.editor.formid_ops import (
    apply_object_id_mapping,
    change_form_id,
    compact_for_esl,
    inject_into_master,
    renumber_form_ids_from,
)
from creation_lib.esp.editor.apply_script import ScriptContext, run_script
from creation_lib.esp.editor.reachable import build_reachable_set, find_orphan_records
from creation_lib.esp.editor.header_flags import (
    FLAG_LIGHT,
    FLAG_MASTER,
    FLAG_MEDIUM,
    is_light,
    is_master,
    is_medium,
    set_esl,
    set_light,
    set_master,
    set_medium,
)
from creation_lib.esp.editor.patch import (
    PatchError,
    add_winner_to_patch,
    add_winners_to_patch,
    automerge_to_patch,
    clear_patch_target,
    create_patch_plugin,
    ensure_patch_masters,
)
from creation_lib.esp.editor.conflicts import (
    ConflictReport,
    ConflictScan,
    ConflictScanner,
    OverrideEntry,
)
from creation_lib.esp.editor.fields import (
    Field,
    FieldKind as UiFieldKind,
    decode_record,
    encode_field,
    decode_bytes,
    encode_bytes,
    decode_struct,
    encode_struct,
    decode_array,
    encode_array,
    make_array_row,
    clone_array_row,
)

__all__ = [
    "EditorSession",
    "LoadedPlugin",
    "ConflictStatus",
    "detect_game",
    "Issue",
    "IssueCategory",
    "Severity",
    "ValidationReport",
    "validate",
    "copy_as_new",
    "copy_as_override",
    "add_masters",
    "clean_masters",
    "sort_masters",
    "set_esl",
    "set_light",
    "set_master",
    "set_medium",
    "is_light",
    "is_master",
    "is_medium",
    "FLAG_LIGHT",
    "FLAG_MASTER",
    "FLAG_MEDIUM",
    "remove_itm_records",
    "undelete_and_disable_refs",
    "build_reference_index",
    "apply_object_id_mapping",
    "change_form_id",
    "compact_for_esl",
    "inject_into_master",
    "renumber_form_ids_from",
    "ScriptContext",
    "run_script",
    "build_reachable_set",
    "find_orphan_records",
    "PatchError",
    "add_winner_to_patch",
    "add_winners_to_patch",
    "automerge_to_patch",
    "clear_patch_target",
    "create_patch_plugin",
    "ensure_patch_masters",
    "ConflictReport",
    "ConflictScan",
    "ConflictScanner",
    "OverrideEntry",
    "Field",
    "UiFieldKind",
    "decode_record",
    "encode_field",
    "decode_bytes",
    "encode_bytes",
    "decode_struct",
    "encode_struct",
    "decode_array",
    "encode_array",
    "make_array_row",
    "clone_array_row",
]

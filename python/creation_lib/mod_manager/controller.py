from __future__ import annotations

from dataclasses import dataclass

from .backend import call
from .fomod import Environment, Fomod
from .package import Package
from .nexus import NexusClient
from .transactions import OwnedReview


@dataclass
class PreparedInstall:
    package: Package
    fomod: Fomod | None
    environment: Environment
    nexus: dict | None = None
    replacement_review: OwnedReview | None = None
    source_tree: object = None
    metadata: dict | None = None
    manual: bool = False
    context: object | None = None
    download_review: object | None = None
    header_image: object | None = None
    overlay_review: object | None = None

    def plan(self, choices=None, root="", cancel=None, *, repairs=None):
        return call("prepared_install_plan", self, choices, root, cancel, repairs)

    def validate(self, library, profile_id, cancel=None):
        return call("prepared_install_validate", self, library, profile_id, cancel)


class ManagerController:
    def __init__(self, *, extensions=None):
        call("controller_initialize", self, extensions)

    @property
    def busy(self):
        return self._future is not None

    def submit(self, label, work, complete, *, allow_runtime=False):
        return call("controller_submit", self, label, work, complete, allow_runtime)

    def _report(self, fraction, detail):
        self.progress = (fraction, detail)

    def poll(self):
        return call("controller_poll", self)

    def cancel(self):
        return call("controller_cancel", self)

    def resolve_destination(self, path=""):
        return call("controller_destination", self, path)

    def _validate_destination(self, destination, preview=None):
        return call("controller_validate_destination", destination, preview)

    @staticmethod
    def _prepare_library(destination, *, create):
        return call("controller_prepare_library", destination, create)

    def open_library(self, path="", create=False):
        return call("controller_open_library", self, path, create)

    def refresh(self):
        return call("controller_refresh", self)

    @property
    def conflict_index(self):
        return call("controller_conflict_index", self)

    def refresh_conflicts(self, *, delay=.15):
        return call("controller_refresh_conflicts", self, delay)

    def _accept_conflicts(self, generation, result, *, final=False):
        return call("controller_accept_conflicts", self, generation, result, final)

    def _poll_conflicts(self):
        return call("controller_poll_conflicts", self)

    def select_profile(self, profile_id):
        return call("controller_select_profile", self, profile_id)

    @property
    def runtime_active(self):
        return call("controller_runtime_active", self)

    @property
    def context_library(self):
        return call("controller_context", self)

    def select_target(self, identifier):
        return call("controller_select_target", self, identifier)

    @property
    def save_library(self):
        return call("controller_save_context", self)

    def review_target(self, name, game_root, identifier=""):
        return call("controller_review_target", self, name, game_root, identifier)

    def apply_target(self, *, automatic=True, archive="", nxm=""):
        return call("controller_apply_target", self, automatic, archive, nxm)

    def prepare_target_f4se(self, *, archive="", nxm=""):
        return call("controller_prepare_target_f4se", self, archive, nxm)

    def review_target_root_setup(self, *, restore=False):
        return call("controller_review_root_setup", self, restore)

    def apply_target_root_setup(self, *, accept_root_writes=False):
        return call("controller_apply_root_setup", self, accept_root_writes)

    def poll_runtime(self):
        return call("controller_poll_runtime", self)


    def save_executable(self, executable):
        return call("controller_save_executable", self, executable)

    def remove_executable(self, executable_id):
        return call("controller_remove_executable", self, executable_id)

    def save_executables(self, instance_id, expected, items, default=""):
        return call("controller_save_executables", self, instance_id, expected, items, default)

    def prepare_launch(self, executable_id, game_root="", bundle_root=""):
        return call("controller_prepare_launch", self, executable_id, bundle_root)

    def start_launch(self, *, accept_source_writes=False, accept_minimal_ini=False, accept_experimental=False, disable_mcp=False):
        return call("controller_start_launch", self, accept_source_writes, accept_minimal_ini, accept_experimental, disable_mcp)

    def status_rows(self):
        return call("controller_status_rows", self)

    def filtered(self, query="", group="", enabled_only=False, *, sort_by="priority", descending=False, origin="", category=None, statuses=None):
        return call("controller_filtered", self, query, group, enabled_only, sort_by, descending, origin, category, statuses)

    def inspect_mo2(self, path, source_mappings=None):
        return call("controller_inspect_mo2", self, path, source_mappings)

    def _accept_migration(self, result):
        return call("controller_accept_migration", self, result)

    def migrate_mo2(self, path, destination="", source_mappings=None):
        return call("controller_migrate_mo2", self, path, destination, source_mappings)

    def import_profiles(self, selected, destination=""):
        return call("controller_import_profiles", self, selected, destination)

    def create_instance(self, game_id, game_root, destination="", name="Default"):
        return call("controller_create_instance", self, game_id, game_root, destination, name)

    def create_profile(self, name, clone=False):
        return call("controller_profile_action", self, "create_profile", (name, clone,), {})

    def customize(self, mod_id, **changes):
        return call("controller_profile_action", self, "customize", (mod_id,), changes)

    def delete_profile(self, profile_id):
        return call("controller_profile_action", self, "delete_profile", (profile_id,), {})

    def restore_profile(self, profile_id):
        return call("controller_profile_action", self, "restore_profile", (profile_id,), {})

    def rename_profile(self, name):
        return call("controller_profile_action", self, "rename_profile", (name,), {})

    def save_profile_ini(self, name, text, expected):
        return call("controller_profile_action", self, "save_profile_ini", (name, text, expected,), {})

    def restore_profile_ini(self, name, expected):
        return call("controller_profile_action", self, "restore_profile_ini", (name, expected,), {})

    def backup_profile(self):
        return call("controller_profile_action", self, "backup_profile", (), {})

    def export_recipe(self):
        return call("controller_profile_action", self, "export_recipe", (), {})

    def inspect_recipe(self, path):
        return call("controller_inspect_recipe", self, path)

    def review_recipe(self, mapping, name):
        return call("controller_profile_action", self, "review_recipe", (mapping, name,), {})

    def apply_recipe(self):
        return call("controller_profile_action", self, "apply_recipe", (), {})

    def review_profile_backup(self, identifier):
        return call("controller_profile_action", self, "review_profile_backup", (identifier,), {})

    def restore_profile_backup(self):
        return call("controller_profile_action", self, "restore_profile_backup", (), {})

    def remove_profile_backup(self, identifier):
        return call("controller_profile_action", self, "remove_profile_backup", (identifier,), {})

    def reorder(self, mod_id, priority):
        return call("controller_profile_action", self, "reorder", (mod_id, priority,), {})

    def bulk_organize(self, mod_ids, **changes):
        return call("controller_profile_action", self, "bulk_organize", (mod_ids,), changes)

    def set_target_enabled(self, mod_ids, enabled):
        return call("controller_profile_action", self, "set_target_enabled", (mod_ids, enabled,), {})

    def undo_organization(self):
        return call("controller_profile_action", self, "undo_organization", (), {})

    def create_empty_mod(self, name, after=""):
        return call("controller_profile_action", self, "create_empty_mod", (name, after), {})

    def create_group(self, name, group=""):
        return call("controller_profile_action", self, "create_group", (name, group,), {})

    def review_download(self, path, *, imported=False):
        return call("controller_profile_action", self, "review_download", (path, imported,), {})

    def prepare_install(self, path, replace_id="", *, manual=False, download_review=None, overlay_id=""):
        return call("controller_prepare_install", self, path, replace_id, manual, download_review, overlay_id)

    def install(self, name, root=""):
        return call("controller_install", self, name, root)

    def review_install(self, root=""):
        return call("controller_review_install", self, root)

    def _install_review_state(self, root=""):
        return call("controller_install_review_state", self, root)

    def change_fomod_policy(self, compatibility):
        return call("controller_fomod_policy", self, compatibility)

    def preview_installer_image(self, source):
        return call("controller_installer_image", self, source)

    def review_owned(self, mod_id):
        return call("controller_owned_action", self, "review_owned", (mod_id,))

    def inspect_mod_name(self, mod_id):
        return call("controller_owned_action", self, "inspect_mod_name", (mod_id,))

    def review_reference_source(self, mod_id, source):
        return call("controller_owned_action", self, "review_reference_source", (mod_id, source,))

    def relink_source(self, *, accept_identity=False):
        return call("controller_owned_action", self, "relink_source", (accept_identity,))

    def undo_source_relink(self, mod_id):
        return call("controller_owned_action", self, "undo_source_relink", (mod_id,))

    def inspect_mod_field(self, mod_id, field):
        return call("controller_owned_action", self, "inspect_mod_field", (mod_id, field,))

    def review_source_metadata(self, mod_id, category_file=""):
        return call("controller_owned_action", self, "review_source_metadata", (mod_id, category_file,))

    def refresh_source_metadata(self, selected):
        return call("controller_owned_action", self, "refresh_source_metadata", (selected,))

    def undo_source_metadata(self, mod_id):
        return call("controller_owned_action", self, "undo_source_metadata", (mod_id,))

    def edit_mod_field(self, value):
        return call("controller_owned_action", self, "edit_mod_field", (value,))

    def inspect_group_name(self, group):
        return call("controller_owned_action", self, "inspect_group_name", (group,))

    def rename_group(self, replacement):
        return call("controller_owned_action", self, "rename_group", (replacement,))

    def rename_mod_name(self, name):
        return call("controller_owned_action", self, "rename_mod_name", (name,))

    def uninstall(self):
        return call("controller_owned_action", self, "uninstall", ())

    def review_restore(self, operation_id):
        return call("controller_owned_action", self, "review_restore", (operation_id,))

    def restore(self):
        return call("controller_owned_action", self, "restore", ())

    def review_prune(self, operation_id):
        return call("controller_owned_action", self, "review_prune", (operation_id,))

    def prune_backup(self):
        return call("controller_owned_action", self, "prune_backup", ())

    def discard_install(self):
        return call("controller_discard_install", self)

    def preview_export(self):
        return call("controller_inventory_action", self, "preview_export", ())

    def review_migration(self, mod_ids):
        return call("controller_inventory_action", self, "review_migration", (mod_ids,))

    def migrate(self):
        return call("controller_inventory_action", self, "migrate", ())

    def inspect_files(self, include_disabled=True):
        return call("controller_inventory_action", self, "inspect_files", (include_disabled,))

    def inspect_plugins(self, *, metadata=False):
        return call("controller_inventory_action", self, "inspect_plugins", (metadata,))

    def inspect_archives(self):
        return call("controller_inventory_action", self, "inspect_archives", ())

    def inspect_combined_data(self):
        return call("controller_inventory_action", self, "inspect_combined_data", ())

    def review_reference_removal(self, mod_ids):
        return call("controller_inventory_action", self, "review_reference_removal", (mod_ids,))

    def remove_references(self):
        return call("controller_inventory_action", self, "remove_references", ())

    def inspect_overwrite(self):
        return call("controller_overwrite_action", self, "inspect_overwrite", ())

    def review_retained_backup(self, kind, identifier):
        return call("controller_overwrite_action", self, "review_retained_backup", (kind, identifier,))

    def remove_retained_backup(self):
        return call("controller_overwrite_action", self, "remove_retained_backup", ())

    def change_overwrite(self, action, name=""):
        return call("controller_overwrite_action", self, "change_overwrite", (action, name,))

    def review_overwrite_restore(self, identifier):
        return call("controller_overwrite_action", self, "review_overwrite_restore", (identifier,))

    def restore_overwrite(self):
        return call("controller_overwrite_action", self, "restore_overwrite", ())

    def apply_plugins(self, plan):
        return call("controller_inventory_action", self, "apply_plugins", (plan,))


    def export(self):
        return call("controller_inventory_action", self, "export", ())

    def accept_nxm(self, raw, *, destination=None):
        return call("controller_download_action", self, "accept_nxm", (raw, destination,))

    def connect_nexus(self, api_key="", factory=NexusClient, *, remember=False):
        return call("controller_download_action", self, "connect_nexus", (api_key, factory, remember))

    def disconnect_nexus(self):
        return call("controller_download_action", self, "disconnect_nexus", ())

    def download_nxm(self, identity):
        return call("controller_download_action", self, "download_nxm", (identity,))

    def resume_download(self, identifier):
        return call("controller_download_action", self, "resume_download", (identifier,))

    @staticmethod
    def download_local_recovery(job):
        return call("controller_download_local_recovery", job)

    def remove_download_job(self, identifier):
        return call("controller_download_action", self, "remove_download_job", (identifier,))

    @property
    def downloads_active(self):
        return call("controller_download_action", self, "downloads_active", ())

    def transfer_active(self, identity_or_id):
        return call("controller_download_action", self, "transfer_active", (identity_or_id,))

    def _assert_transfer_idle(self):
        return call("controller_download_action", self, "_assert_transfer_idle", ())

    def set_download_slots(self, slots):
        return call("controller_download_action", self, "set_download_slots", (slots,))

    def _schedule_download(self, client, link=None, identifier=""):
        return call("controller_download_action", self, "_schedule_download", (client, link, identifier,))

    def pause_download(self, identity_or_id=None):
        return call("controller_download_action", self, "pause_download", (identity_or_id,))

    def _poll_downloads(self):
        return call("controller_download_action", self, "_poll_downloads", ())

    def inspect_nexus_mod(self, mod_id):
        return call("controller_nexus_action", self, "inspect_nexus_mod", (mod_id,))

    def inspect_saves(self):
        return call("controller_save_action", self, "inspect_saves", ())

    def inspect_source_saves(self, folder):
        return call("controller_save_action", self, "inspect_source_saves", (folder,))

    def inspect_global_saves(self):
        return call("controller_save_action", self, "inspect_global_saves", ())

    def inspect_categories(self):
        return call("controller_category_action", self, "inspect_categories", ())

    def import_nexus_categories(self, refresh=True):
        return call("controller_category_action", self, "import_nexus_categories", (refresh,))

    def edit_category(self, action, value, replacement="", nexus_id=0):
        return call("controller_category_action", self, "edit_category", (action, value, replacement, nexus_id,))

    def assign_category(self, mod_ids, category):
        return call("controller_category_action", self, "assign_category", (mod_ids, category,))

    def review_save_copy(self, path, target_profile=""):
        return call("controller_save_action", self, "review_save_copy", (path, target_profile,))

    def apply_save_copy(self):
        return call("controller_save_action", self, "apply_save_copy", ())

    def inspect_save_backups(self):
        return call("controller_save_action", self, "inspect_save_backups", ())

    def review_save_backup(self, identifier):
        return call("controller_save_action", self, "review_save_backup", (identifier,))

    def inspect_save_header(self, path):
        return call("controller_save_action", self, "inspect_save_header", (path,))

    def _selected_save(self, path):
        return call("controller_selected_save", self, path, False)

    def inspect_save_plugins(self, path):
        return call("controller_save_action", self, "inspect_save_plugins", (path,))

    def preview_save_screenshot(self, path):
        return call("controller_save_action", self, "preview_save_screenshot", (path,))

    def compare_save_profile(self, path):
        return call("controller_save_action", self, "compare_save_profile", (path,))

    def compare_save_to(self, path, target):
        return call("controller_save_action", self, "compare_save_to", (path, target,))

    def toggle_owned_hidden(self, provider, *, parent=False):
        return call("controller_category_action", self, "toggle_owned_hidden", (provider, parent,))

    def nexus_account_action(self, mod_id, action):
        return call("controller_nexus_action", self, "nexus_account_action", (mod_id, action,))

    def _nexus_context(self):
        return call("controller_nexus_context", self)

    def check_nexus_batch(self, *, manual=True):
        return call("controller_nexus_action", self, "check_nexus_batch", (manual,))

    def check_updates(self, *, manual=True):
        return call("controller_nexus_action", self, "check_updates", (manual,))

    def tick_updates(self, now=None):
        return call("controller_nexus_action", self, "tick_updates", (now,))

    def cleanup(self):
        return call("controller_cleanup", self)

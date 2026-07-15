"""DEPRECATED - app_paths has moved to creation_lib.paths.

This shim re-exports from creation_lib.paths so existing callers continue to work.
All imports of this module MUST be migrated to explicit config params.
"""
import warnings

warnings.warn(
    "creation_lib.core.app_paths is deprecated; import from creation_lib.paths instead. "
    "py_creation_lib/python/creation_lib/ modules must migrate to explicit config params.",
    DeprecationWarning,
    stacklevel=2,
)

from creation_lib.paths import *  # noqa: F401,F403,E402
from creation_lib.paths import (  # noqa: E402
    is_frozen,
    find_project_root,
    load_dotenv_into_environ,
    get_app_root,
    get_code_root,
    get_resource_dir,
    get_db_dir,
    get_settings_path,
    get_logs_dir,
    get_ini_dir,
)

"""Manager diagnostics use the Toolkit's existing root LogPanel handler."""
import logging


LOG = logging.getLogger("toolkit.mod_manager")
LOG.setLevel(logging.INFO)


def emit(manager):
    state = (manager.status, manager.error, manager.job_state)
    if state == getattr(manager, "_diagnostic_state", None):
        return
    manager._diagnostic_state = state
    message = manager.error or manager.status
    if message:
        level = logging.ERROR if manager.error else logging.WARNING if manager.job_state == "cancelled" else logging.INFO
        LOG.log(level, "%s: %s", manager.job_state, message)

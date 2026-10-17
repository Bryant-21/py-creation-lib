from creation_lib._native import mod_manager_native


def invoke(function, *args, **kwargs):
    from .paths import ManagerError, Cancelled
    try:
        return function(*args, **kwargs)
    except mod_manager_native.DownloadInterrupted as exc:
        from .download_queue import DownloadPaused, DownloadFailure
        paused, message = exc.args
        raise (DownloadPaused() if paused else DownloadFailure(message)) from exc
    except mod_manager_native.NexusFailure as exc:
        from .nexus import NexusError
        message, retry, status = exc.args
        raise NexusError(message, retry, http_status=status) from exc
    except mod_manager_native.TransactionCommitted as exc:
        import json
        from .transactions import CommittedTransaction
        raise CommittedTransaction(json.loads(str(exc))) from exc
    except mod_manager_native.OperationCancelled as exc:
        raise Cancelled(str(exc)) from exc
    except ManagerError:
        raise
    except ValueError as exc:
        raise ManagerError(str(exc)) from exc


def call(name, *args, **kwargs):
    return invoke(getattr(mod_manager_native, name), *args, **kwargs)


class NativeContext:
    def __init__(self, context):
        self.context = context

    def __enter__(self):
        return invoke(self.context.__enter__)

    def __exit__(self, kind, error, trace):
        return invoke(self.context.__exit__, kind, error, trace)

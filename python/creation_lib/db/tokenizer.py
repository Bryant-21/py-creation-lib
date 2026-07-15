"""CamelCase/underscore tokenizer — thin shim over Rust db_native."""

from .native_runtime import tokenize

__all__ = ["tokenize"]

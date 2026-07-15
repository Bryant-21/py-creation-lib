"""GIL-free smoke test for the native Papyrus core.

Asserts that calling `papyrus_core.parse_text` from N Python threads scales
sub-linearly with thread count, proving that `py.detach(...)` is correctly
wired in `py_creation_lib/native/papyrus_core/src/bindings.rs`.

If the GIL were held during parsing, wall-clock for N threads would scale
linearly with N (because Python serializes them on the GIL). With detach
working, wall-clock should be roughly constant up to the number of cores —
the test asserts the ratio is at most ~1.6× single-threaded for 4 threads,
which is well below 4× and gives a generous margin against scheduler noise.

Skipped when the native module isn't built/available — this is a guard for
the GIL contract, not a baseline correctness test.
"""
from __future__ import annotations

import os
import threading
import time

import pytest

from creation_lib.papyrus_lsp import native_runtime as nr


@pytest.fixture(scope="module")
def native():
    if not nr.is_available():
        pytest.skip("papyrus_core native module not installed")
    mod = nr.load_native_module()
    if not hasattr(mod, "parse_text"):
        pytest.skip("native module missing parse_text")
    # Sanity check: a stub build (where parse_text was a no-op) returned empty
    # AST instantly — meaningless for a GIL test. Make sure the loaded build
    # actually parses by feeding it a simple known-good script.
    import json as _json
    payload = _json.loads(mod.parse_text("ScriptName Foo extends Bar\n"))
    if not payload.get("ast") or payload["ast"].get("name") != "Foo":
        pytest.skip(
            "loaded papyrus_core build is a stub or older than the parser; "
            "rebuild via `uv sync --reinstall-package modbox21-native` "
            "(may require closing existing Python processes that hold the .pyd)"
        )
    return mod


SAMPLE = "\n".join(
    [
        "Scriptname StressMe extends ObjectReference",
        "",
    ]
    + [
        f"Int Function Computation_{i}(Int x, Int y)\n"
        f"  Int z = x * y + {i}\n"
        f"  If z > {i}\n"
        f"    Return z - {i}\n"
        f"  EndIf\n"
        f"  Return 0\n"
        f"EndFunction\n"
        for i in range(40)
    ]
)


def _bench(parse, iters: int) -> float:
    start = time.perf_counter()
    for _ in range(iters):
        parse(SAMPLE)
    return time.perf_counter() - start


def test_parse_text_releases_gil(native):
    iters_per_thread = 200

    # Warmup so JIT/cache effects don't dominate the single-threaded baseline.
    _bench(native.parse_text, 20)

    # Single-threaded baseline.
    serial = _bench(native.parse_text, iters_per_thread)

    # Parallel: 4 threads doing the same work each.
    n_threads = 4
    durations: list[float] = [0.0] * n_threads

    def worker(idx: int):
        durations[idx] = _bench(native.parse_text, iters_per_thread)

    start = time.perf_counter()
    threads = [threading.Thread(target=worker, args=(i,)) for i in range(n_threads)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    parallel_wall = time.perf_counter() - start

    # If the GIL were held during parsing, parallel_wall would be ~n_threads * serial.
    # With detach working, parallel_wall should be roughly serial (capped by core count).
    # Allow a generous margin (1.6×) for scheduler noise on the build agent.
    ratio = parallel_wall / serial if serial > 0 else float("inf")
    assert ratio < 1.6, (
        f"parallel/serial wall-time ratio = {ratio:.2f} suggests the GIL is "
        f"held during parse. serial={serial:.3f}s, "
        f"parallel({n_threads} threads)={parallel_wall:.3f}s. "
        f"Per-thread wall: {[f'{d:.3f}' for d in durations]}"
    )

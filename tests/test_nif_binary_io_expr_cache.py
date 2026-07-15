from creation_lib.nif.binary_io import _compile_nif_expr


def test_compile_nif_expr_reuses_compiled_instances():
    _compile_nif_expr.cache_clear()

    first = _compile_nif_expr("1 + 2")
    second = _compile_nif_expr("1 + 2")

    assert first is second
    assert _compile_nif_expr.cache_info().hits >= 1

"""Synthetic byte-exact write/read/write roundtrip for HKX packfiles.

Behavior graphs (`hkbBehaviorGraph` + derived classes) exercise `hkbBindable`
-derived classes with `SERIALIZE_IGNORED` array members (`cachedBindables`,
`uniqueIdPool`, etc). Vanilla FO4 packs those arrays with
`capacity = 0x80000000` (the "owns memory" flag at the high byte) so the
runtime allocator treats them as heap-owned empty arrays. The writer must
match, or CK crashes reconciling the bindable table on load.

Uses a synthetic HKXFile/HKXObject graph (no real game data) so this test
runs anywhere without an extracted corpus; the byte-exact contract is
checked by writing, reading the bytes back, and writing again.
"""
from __future__ import annotations

from creation_lib.hkxpack import DescriptorRegistry, HKXFile, HKXObject, load_hkx_bytes, write_hkx


def test_behavior_graph_write_read_write_is_byte_stable():
    hkx = HKXFile(class_version=11, contents_version="hk_2014.1.0-r1")
    root = HKXObject(name="#0001", class_name="hkRootLevelContainer")
    graph = HKXObject(name="#0002", class_name="hkbBehaviorGraph")
    hkx.objects.append(root)
    hkx.objects.append(graph)

    registry = DescriptorRegistry()
    first_pass = write_hkx(hkx, registry)

    reloaded, reloaded_registry = load_hkx_bytes(first_pass)
    second_pass = write_hkx(reloaded, reloaded_registry)

    assert second_pass == first_pass, "write(load(write(x))) must be byte-identical to write(x)"
    assert [obj.class_name for obj in reloaded.objects] == ["hkRootLevelContainer", "hkbBehaviorGraph"]

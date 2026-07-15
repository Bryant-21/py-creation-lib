"""Regression tests for NifFile.remap_refs / sanitize._remap_refs.

Covers struct-nested Ref/Ptr fields (e.g. NiDefaultAVObjectPalette.Objs[].AV Object
and NiControllerSequence.Controlled Blocks[].Interpolator/Controller/String Palette).
If these are not remapped after remove_blocks(), stale block indices crash
FO4 on load.
"""
from creation_lib.nif.nif_file import NifBlock, NifFile


def _build_nif_with_palette():
    """Build a minimal NIF: root NiNode + 3 BSTriShape children + palette
    referencing each child, plus a NiControllerSequence with Controlled Blocks."""
    nif = NifFile()

    root = NifBlock(0, "NiNode", fields=[
        ("Name", "Root"),
        ("Num Children", 3),
        ("Children", [1, 2, 3]),
    ])
    tri_a = NifBlock(1, "BSTriShape", fields=[("Name", "Mesh_A")])
    tri_b = NifBlock(2, "BSTriShape", fields=[("Name", "Mesh_B")])
    tri_c = NifBlock(3, "BSTriShape", fields=[("Name", "Mesh_C")])

    # Palette with entries for each tri-shape
    palette = NifBlock(4, "NiDefaultAVObjectPalette", fields=[
        ("Scene", 0),
        ("Num Objs", 3),
        ("Objs", [
            {"Name": "Mesh_A", "AV Object": 1},
            {"Name": "Mesh_B", "AV Object": 2},
            {"Name": "Mesh_C", "AV Object": 3},
        ]),
    ])

    # A pair of interpolator + controller blocks used by the sequence
    interp_a = NifBlock(5, "NiTransformInterpolator", fields=[("Value", {})])
    ctrl_a = NifBlock(6, "NiTransformController", fields=[("Flags", 0)])
    interp_b = NifBlock(7, "NiTransformInterpolator", fields=[("Value", {})])
    ctrl_b = NifBlock(8, "NiTransformController", fields=[("Flags", 0)])

    # NiControllerSequence with two controlled blocks referencing the above
    seq = NifBlock(9, "NiControllerSequence", fields=[
        ("Name", "Seq"),
        ("Num Controlled Blocks", 2),
        ("Controlled Blocks", [
            {
                "Interpolator": 5,
                "Controller": 6,
                "Blend Interpolator": -1,
                "Blend Index": 0,
                "Priority": 0,
                "Node Name": "Mesh_A",
                "Property Type": "",
                "Controller Type": "",
                "Controller ID": "",
                "Interpolator ID": "",
                "String Palette": -1,
            },
            {
                "Interpolator": 7,
                "Controller": 8,
                "Blend Interpolator": -1,
                "Blend Index": 0,
                "Priority": 0,
                "Node Name": "Mesh_B",
                "Property Type": "",
                "Controller Type": "",
                "Controller ID": "",
                "Interpolator ID": "",
                "String Palette": -1,
            },
        ]),
    ])

    nif.blocks = [root, tri_a, tri_b, tri_c, palette, interp_a, ctrl_a, interp_b, ctrl_b, seq]
    nif.header.num_blocks = len(nif.blocks)
    nif.header.block_type_names = sorted({b.type_name for b in nif.blocks})
    nif.header.block_type_index = [nif.header.block_type_names.index(b.type_name) for b in nif.blocks]
    nif.header.block_sizes = [0] * len(nif.blocks)
    return nif


def test_remap_refs_recurses_into_palette_objs():
    """After removing a middle block, palette AV Object Ptrs must shift."""
    nif = _build_nif_with_palette()

    # Remove Mesh_B (block 2). Remaining: 0,1,3,4,5,6,7,8,9 → renumbered 0..8
    nif.remove_blocks([2])

    palette = next(b for b in nif.blocks if b.type_name == "NiDefaultAVObjectPalette")
    objs = palette.get_field("Objs")

    # Mesh_B entry's AV Object should now be -1 (removed)
    # Mesh_A (was 1) → still 1; Mesh_C (was 3) → now 2
    by_name = {o["Name"]: o for o in objs}

    assert by_name["Mesh_A"]["AV Object"] == 1, f"Mesh_A AV ptr wrong: {by_name['Mesh_A']}"
    assert by_name["Mesh_B"]["AV Object"] == -1, f"Mesh_B AV ptr should be -1: {by_name['Mesh_B']}"
    assert by_name["Mesh_C"]["AV Object"] == 2, f"Mesh_C AV ptr wrong: {by_name['Mesh_C']}"

    # And every valid AV Object must point to a block that still exists,
    # with matching Name.
    for entry in objs:
        av = entry["AV Object"]
        if av < 0:
            continue
        assert 0 <= av < len(nif.blocks), f"AV Object {av} out of range after remove"
        assert nif.blocks[av].get_field("Name") == entry["Name"]


def test_remap_refs_recurses_into_controlled_blocks():
    """Controlled Blocks struct array fields (Interpolator, Controller) must remap."""
    nif = _build_nif_with_palette()

    # Sanity: initial refs
    seq = next(b for b in nif.blocks if b.type_name == "NiControllerSequence")
    before = seq.get_field("Controlled Blocks")
    assert before[0]["Interpolator"] == 5
    assert before[1]["Controller"] == 8

    # Remove block 6 (ctrl_a). New indices: 0,1,2,3,4,5(=interp_a),6(=interp_b),7(=ctrl_b),8(=seq)
    nif.remove_blocks([6])

    after = seq.get_field("Controlled Blocks")
    # First controlled block: Interpolator was 5 → still 5; Controller was 6 → -1
    assert after[0]["Interpolator"] == 5
    assert after[0]["Controller"] == -1
    # Second controlled block: Interpolator was 7 → now 6; Controller was 8 → now 7
    assert after[1]["Interpolator"] == 6
    assert after[1]["Controller"] == 7


def test_remap_refs_top_level_still_works():
    """Top-level Ref/Ptr (Children array, Scene Ptr) still remap correctly."""
    nif = _build_nif_with_palette()

    nif.remove_blocks([2])  # drop Mesh_B

    root = nif.blocks[0]
    # Children was [1,2,3] → after dropping 2: [1,-1,2]
    children = root.get_field("Children")
    assert children == [1, -1, 2], f"Children remap wrong: {children}"

    palette = next(b for b in nif.blocks if b.type_name == "NiDefaultAVObjectPalette")
    assert palette.get_field("Scene") == 0

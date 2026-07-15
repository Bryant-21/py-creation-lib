from creation_lib.nif.nif_file import NifBlock, NifFile


def test_get_hierarchy_uses_block_zero_when_collision_targets_root():
    nif = NifFile()
    nif.blocks = [
        NifBlock(
            0,
            "NiNode",
            fields=[
                ("Name", "Root"),
                ("Collision Object", 2),
                ("Children", [1]),
            ],
        ),
        NifBlock(1, "BSTriShape", fields=[("Name", "Mesh")]),
        NifBlock(
            2,
            "bhkNPCollisionObject",
            fields=[
                ("Target", 0),
                ("Data", 3),
                ("Body ID", 0),
            ],
        ),
        NifBlock(3, "bhkPhysicsSystem"),
        NifBlock(4, "BSShaderTextureSet"),
    ]

    hierarchy = nif.get_hierarchy()

    assert hierarchy["roots"][0]["id"] == 0
    assert hierarchy["roots"][0]["type"] == "NiNode"
    assert hierarchy["roots"][0]["children"][0]["id"] == 2
    assert hierarchy["roots"][0]["children"][0]["children"][0]["id"] == 3
    assert all(root["id"] != 4 for root in hierarchy["roots"])

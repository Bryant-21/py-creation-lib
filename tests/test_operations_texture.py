import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.operations.texture import extract_texture_paths


def test_extract_finds_deduped_paths_skipping_empty_slots():
    nif = NifFile()
    root = NifBlock(block_id=0, type_name="BSFadeNode")
    root.set_field("Children", [1, 2])
    nif.blocks.append(root)

    tex_set1 = NifBlock(block_id=1, type_name="BSShaderTextureSet")
    tex_set1.set_field("Textures", [
        "textures\\actors\\character\\basehuman\\basehuman_d.dds",
        "Textures\\actors\\character\\basehuman\\basehuman_d.dds",  # dup, different case
        "",  # empty slot
        "textures\\actors\\character\\basehuman\\basehuman_s.dds",
    ])
    nif.blocks.append(tex_set1)

    tex_set2 = NifBlock(block_id=2, type_name="BSShaderTextureSet")
    tex_set2.set_field("Textures", [
        "textures\\weapons\\10mmPistol\\10mmPistol_d.dds",
        "textures\\weapons\\10mmPistol\\10mmPistol_n.dds",
    ])
    nif.blocks.append(tex_set2)

    result = extract_texture_paths(nif)
    assert result.success
    assert "4 texture" in result.description  # empty slot skipped, case-dup collapsed


def test_extract_empty_nif_reports_zero_textures():
    nif = NifFile()
    result = extract_texture_paths(nif)
    assert result.success
    assert "0 texture" in result.description

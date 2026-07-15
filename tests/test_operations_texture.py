import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.operations.texture import extract_texture_paths


def _make_textured_nif() -> NifFile:
    """Create a NIF with BSShaderTextureSet blocks containing texture paths."""
    nif = NifFile()

    root = NifBlock(block_id=0, type_name="BSFadeNode")
    root.set_field("Children", [1])
    nif.blocks.append(root)

    tex_set1 = NifBlock(block_id=1, type_name="BSShaderTextureSet")
    tex_set1.set_field("Textures", [
        "textures\\actors\\character\\basehuman\\basehuman_d.dds",
        "textures\\actors\\character\\basehuman\\basehuman_n.dds",
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

    return nif


def test_extract_finds_all_paths():
    nif = _make_textured_nif()
    result = extract_texture_paths(nif)
    assert result.success
    assert "5 texture" in result.description


def test_extract_skips_empty():
    nif = _make_textured_nif()
    result = extract_texture_paths(nif)
    # The empty string should not be counted
    assert "5 texture" in result.description


def test_extract_deduplicates():
    nif = NifFile()
    tex = NifBlock(block_id=0, type_name="BSShaderTextureSet")
    tex.set_field("Textures", [
        "textures\\test.dds",
        "Textures\\Test.dds",  # same path, different case
        "textures\\other.dds",
    ])
    nif.blocks.append(tex)
    result = extract_texture_paths(nif)
    assert "2 texture" in result.description


def test_extract_empty_nif():
    nif = NifFile()
    result = extract_texture_paths(nif)
    assert result.success
    assert "0 texture" in result.description


def test_extract_no_texture_sets():
    nif = NifFile()
    root = NifBlock(block_id=0, type_name="BSFadeNode")
    nif.blocks.append(root)
    result = extract_texture_paths(nif)
    assert result.success
    assert "0 texture" in result.description

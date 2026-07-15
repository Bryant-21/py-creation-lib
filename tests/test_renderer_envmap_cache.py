from pathlib import Path
from unittest.mock import MagicMock, patch

from creation_lib.renderer import dds_loader, material_pipeline, shader_pipeline


def test_cached_load_cubemap_reuses_predecoded_texture() -> None:
    ctx = MagicMock()
    resolved = Path(r"C:\tmp\env.dds")
    decoded = MagicMock()
    tex = MagicMock()

    with (
        patch.dict(material_pipeline._decode_cache, {str(resolved): decoded}, clear=True),
        patch.dict(material_pipeline._tex_cache, {}, clear=True),
        patch.object(material_pipeline, "upload_decoded", return_value=tex) as mock_upload,
        patch.object(material_pipeline, "load_cubemap") as mock_load,
    ):
        result = material_pipeline._cached_load_cubemap(ctx, resolved)

    assert result is tex
    mock_upload.assert_called_once_with(ctx, decoded, build_mipmaps=False)
    mock_load.assert_not_called()


def test_load_cubemap_skips_mipmap_generation(tmp_path) -> None:
    cube = tmp_path / "env.dds"
    cube.write_bytes(b"DDS ")
    ctx = MagicMock()
    decoded = MagicMock()
    tex = MagicMock()

    with (
        patch.object(dds_loader, "decode_texture", return_value=decoded),
        patch.object(dds_loader, "upload_decoded", return_value=tex) as mock_upload,
    ):
        result = dds_loader.load_cubemap(ctx, str(cube))

    assert result is tex
    mock_upload.assert_called_once_with(ctx, decoded, build_mipmaps=False)


def test_create_default_env_uses_fast_loose_file_lookup(tmp_path) -> None:
    cube = (
        tmp_path
        / "Textures"
        / "Shared"
        / "Cubemaps"
        / "mipblur_DefaultOutside1.dds"
    )
    cube.parent.mkdir(parents=True)
    cube.write_bytes(b"DDS ")
    ctx = MagicMock()
    ba2_mgr = MagicMock()
    tex = MagicMock()
    tex.size = (128, 128)

    with patch(
        "creation_lib.renderer.material_pipeline._cached_load_cubemap",
        return_value=tex,
    ) as mock_cached:
        result_tex, is_real = shader_pipeline.create_default_env(
            ctx,
            [tmp_path],
            ba2_mgr=ba2_mgr,
            game_id="fo4",
        )

    assert result_tex is tex
    assert is_real is True
    mock_cached.assert_called_once_with(ctx, cube)
    ba2_mgr.find.assert_not_called()

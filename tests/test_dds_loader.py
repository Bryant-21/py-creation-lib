import pytest
import moderngl


@pytest.fixture
def ctx():
    """Create a standalone ModernGL context for testing."""
    return moderngl.create_standalone_context()


def test_load_nonexistent_returns_error_texture(ctx):
    from creation_lib.renderer.dds_loader import load_texture
    tex = load_texture(ctx, "/nonexistent/path.dds")
    assert tex.size == (1, 1)  # error texture


def test_load_png_texture(ctx, tmp_path):
    """PNG files should load via Pillow."""
    from PIL import Image
    from creation_lib.renderer.dds_loader import load_texture
    img = Image.new("RGBA", (4, 4), (255, 0, 0, 255))
    path = tmp_path / "test.png"
    img.save(path)
    tex = load_texture(ctx, str(path))
    assert tex.size == (4, 4)

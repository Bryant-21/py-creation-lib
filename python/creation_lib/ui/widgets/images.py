from functools import lru_cache
from io import BytesIO
from pathlib import Path

from imgui_bundle import hello_imgui, imgui
from PIL import Image


@lru_cache(maxsize=32)
def _image_source(path: str, trim_padding: bool):
    data = Path(path).read_bytes()
    with Image.open(BytesIO(data)) as source:
        width, height = source.size
        bounds = (0, 0, width, height)
        if trim_padding and "A" in source.getbands():
            alpha = source.getchannel("A")
            baseline = alpha.getextrema()[0]
            bounds = alpha.point(lambda value: 255 if value > baseline else 0).getbbox() or bounds
    left, top, right, bottom = bounds
    return data, (left / width, top / height), (right / width, bottom / height), (right - left) / (bottom - top)


def image_in_box(path: str | Path, position, size, *, opacity: float = 1.0,
                 trim_padding: bool = False) -> None:
    path = Path(path).as_posix()
    data, uv0, uv1, aspect = _image_source(path, trim_padding)
    texture = hello_imgui.image_and_size_from_encoded_data(data, f"creation_lib.ui/image/{path}")
    width = min(size[0], size[1] * aspect)
    height = width / aspect
    left = position[0] + (size[0] - width) / 2
    top = position[1] + (size[1] - height) / 2
    draw = imgui.get_window_draw_list()
    draw.push_clip_rect(position, (position[0] + size[0], position[1] + size[1]), True)
    draw.add_image(imgui.ImTextureRef(texture.texture_id), (left, top), (left + width, top + height),
                   uv0, uv1, imgui.get_color_u32((1, 1, 1, opacity)))
    draw.pop_clip_rect()

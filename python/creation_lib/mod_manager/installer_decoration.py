from dataclasses import dataclass

from .backend import call


@dataclass(frozen=True)
class Decoration:
    position: str = "Left"
    colour: tuple | None = None
    image: str = ""
    fade: bool = True
    height: int = -1


def parse(root):
    return Decoration(**call("fomod_decoration", root))

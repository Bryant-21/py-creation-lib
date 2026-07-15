"""Ground plane grid using ModernGL line rendering."""
from pathlib import Path
import numpy as np
import moderngl

_SHADER_DIR = Path(__file__).parent / "shaders"


def compile_grid_shader(ctx: moderngl.Context) -> moderngl.Program:
    """Compile the grid shader from py_creation_lib/python/creation_lib/renderer/shaders/."""
    vert = (_SHADER_DIR / "grid.vert").read_text()
    frag = (_SHADER_DIR / "grid.frag").read_text()
    return ctx.program(vertex_shader=vert, fragment_shader=frag)


class Grid:
    def __init__(self, ctx: moderngl.Context, program: moderngl.Program,
                 size: float = 200.0, step: float = 4.0):
        self.ctx = ctx
        self.program = program

        lines = []
        # Generate grid lines using float step (supports sub-unit grids)
        n = int(size / step) if step > 0 else 50
        for j in range(-n, n + 1):
            v = j * step
            if abs(v) < step * 0.01:
                continue  # skip center lines — drawn separately as axes
            lines.extend([v, -size, 0.0,
                          v,  size, 0.0])
            lines.extend([-size, v, 0.0,
                           size, v, 0.0])

        data = np.array(lines, dtype=np.float32)
        self.vbo = ctx.buffer(data.tobytes())
        self.vao = ctx.vertex_array(program, [(self.vbo, "3f", "in_position")])
        self.num_vertices = len(lines) // 3
        self.color = (0.3, 0.3, 0.3, 0.85)

        # X axis (red): runs along X at Y=0
        x_axis = np.array([-size, 0.0, 0.0, size, 0.0, 0.0], dtype=np.float32)
        self.x_axis_vbo = ctx.buffer(x_axis.tobytes())
        self.x_axis_vao = ctx.vertex_array(program, [(self.x_axis_vbo, "3f", "in_position")])

        # Y axis (green): runs along Y at X=0
        y_axis = np.array([0.0, -size, 0.0, 0.0, size, 0.0], dtype=np.float32)
        self.y_axis_vbo = ctx.buffer(y_axis.tobytes())
        self.y_axis_vao = ctx.vertex_array(program, [(self.y_axis_vbo, "3f", "in_position")])

    def render(self, mvp_tuple):
        self.program["u_mvp"].value = mvp_tuple
        self.program["u_color"].value = self.color
        self.vao.render(moderngl.LINES)

        self.program["u_color"].value = (0.8, 0.15, 0.15, 1.0)
        self.x_axis_vao.render(moderngl.LINES)

        self.program["u_color"].value = (0.15, 0.75, 0.15, 1.0)
        self.y_axis_vao.render(moderngl.LINES)

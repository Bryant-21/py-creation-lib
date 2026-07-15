#version 330
// Fullscreen triangle — no VAO needed (gl_VertexID trick).
out vec2 vTexCoord;
void main() {
    // Three vertices covering the full screen:
    //   id=0: (-1,-1), id=1: (3,-1), id=2: (-1,3)
    float x = float((gl_VertexID & 1) << 2) - 1.0;
    float y = float((gl_VertexID & 2) << 1) - 1.0;
    vTexCoord = vec2(x + 1.0, y + 1.0) * 0.5;
    gl_Position = vec4(x, y, 0.0, 1.0);
}

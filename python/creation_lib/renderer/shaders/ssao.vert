#version 330
// Fullscreen quad vertex shader for SSAO and post-processing passes.
// Renders a screen-filling triangle (3 vertices, no VBO needed).

out vec2 vTexCoord;

void main() {
    // Fullscreen triangle: vertex 0=(−1,−1), 1=(3,−1), 2=(−1,3)
    float x = float((gl_VertexID & 1) << 2) - 1.0;
    float y = float((gl_VertexID & 2) << 1) - 1.0;
    vTexCoord = vec2((x + 1.0) * 0.5, (y + 1.0) * 0.5);
    gl_Position = vec4(x, y, 0.0, 1.0);
}

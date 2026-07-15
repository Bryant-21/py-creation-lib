#version 330 core
// Starfield vertex shader — direct port of tools/sf_render_test.py VERT_SRC
// (lines 876-916), adapted to use the editor's standard attribute names
// (in_position, in_normal, ... — see shared_default.vert) so it can bind to
// the existing VAO interleave without a new buffer layout.
//
// Works in view space (same as NifSkope + the standalone) for lighting math.
// The fragment shader (starfield_default.frag) consumes these outputs.

uniform mat4 u_model;        // model -> world
uniform mat4 u_view;         // world -> view
uniform mat4 u_mvp;          // model*view*projection (provided by the editor)
uniform vec3 u_lightDirWorld;  // main light dir in world space

in vec3 in_position;
in vec3 in_normal;
in vec3 in_tangent;
in vec3 in_bitangent;
in vec4 in_color;
in vec2 in_texcoord;
in vec2 in_texcoord2;

out vec4 vTexCoord;  // (uv1.xy, uv2.xy) — matches standalone's packed layout
out vec4 vColor;
out mat3 vBtn;       // columns (B, T, N) — view space
out vec3 vViewDir;   // view-space "to camera" (really -viewPos)
out vec3 vLightDir;  // view-space light direction

void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);

    // Compute view-space position / basis.
    mat4 mv = u_view * u_model;
    vec4 viewPos = mv * vec4(in_position, 1.0);

    // Normal matrix in view space (inverse-transpose of mv's 3x3).
    mat3 normalMat = transpose(inverse(mat3(mv)));
    vec3 N = normalize(normalMat * in_normal);
    vec3 T = normalize(normalMat * in_tangent);
    vec3 B = normalize(normalMat * in_bitangent);
    vBtn = mat3(B, T, N);  // matches NifSkope stf_default.vert column order

    vViewDir  = -viewPos.xyz;
    vLightDir = mat3(u_view) * u_lightDirWorld;

    vTexCoord = vec4(in_texcoord, in_texcoord2);
    vColor    = in_color;
}

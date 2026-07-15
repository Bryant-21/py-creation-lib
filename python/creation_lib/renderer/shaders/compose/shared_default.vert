#version 330
// Shared default vertex shader — used by all games.
// Transforms position, passes UVs, normals, tangents, vertex colors.

uniform mat4 u_mvp;       // model*view*projection
uniform mat4 u_model;     // model -> world
uniform mat4 u_view;      // world -> view (for SSAO normals)
uniform vec4 uvScaleOffset;  // .xy=scale .zw=offset

in vec3 in_position;
in vec3 in_normal;
in vec2 in_texcoord;
in vec2 in_texcoord2;     // Starfield second UV channel (zero when absent)
in vec3 in_tangent;
in vec3 in_bitangent;
in vec4 in_color;         // vertex color (default 1,1,1,1 when absent)

out vec2  vTexCoord;
out vec2  vTexCoord2;     // raw second UV; per-layer transforms applied in fragment shader
out vec3  vNormalWorld;
out vec3  vPosWorld;
out vec3  vTangentWorld;
out vec3  vBitangentWorld;
out vec4  vVertexColor;
out vec3  vNormalView;   // view-space normal for SSAO

void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);

    vec2 rawUV = in_texcoord;
    vTexCoord = rawUV * uvScaleOffset.xy + uvScaleOffset.zw;
    vTexCoord2 = in_texcoord2;

    vec4 pw = u_model * vec4(in_position, 1.0);
    vPosWorld = pw.xyz / pw.w;

    mat3 modelRot = mat3(u_model);
    vNormalWorld    = normalize(modelRot * in_normal);
    vTangentWorld   = normalize(modelRot * in_tangent);
    vBitangentWorld = normalize(modelRot * in_bitangent);

    vVertexColor = in_color;

    mat3 viewModelRot = mat3(u_view) * modelRot;
    vNormalView = normalize(viewModelRot * in_normal);
}

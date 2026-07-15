#version 330
// Shared effect vertex shader — used by all games.

uniform mat4 u_mvp;
uniform mat4 u_model;
uniform vec4 uvScaleOffset;

in vec3 in_position;
in vec3 in_normal;
in vec2 in_texcoord;
in vec3 in_tangent;
in vec3 in_bitangent;
in vec4 in_color;

out vec2  vTexCoord;
out vec3  vNormalWorld;
out vec3  vPosWorld;
out vec3  vTangentWorld;
out vec3  vBitangentWorld;
out vec4  vVertexColor;

void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);

    vec2 rawUV = in_texcoord;
    vTexCoord = rawUV * uvScaleOffset.xy + uvScaleOffset.zw;

    vec4 pw = u_model * vec4(in_position, 1.0);
    vPosWorld = pw.xyz / pw.w;

    mat3 modelRot = mat3(u_model);
    vNormalWorld    = normalize(modelRot * in_normal);
    vTangentWorld   = normalize(modelRot * in_tangent);
    vBitangentWorld = normalize(modelRot * in_bitangent);

    vVertexColor = in_color;
}

#version 330

uniform mat4 u_mvp;
uniform mat4 u_model;
uniform mat3 u_normal_matrix;
uniform mat4 u_light_space_matrix;

in vec3 in_position;
in vec3 in_normal;
in vec2 in_texcoord;
in vec3 in_tangent;

out vec3 v_world_pos;
out vec3 v_normal;
out vec2 v_texcoord;
out vec3 v_tangent;
out vec4 v_light_space_pos;

void main() {
    vec4 world = u_model * vec4(in_position, 1.0);
    v_world_pos = world.xyz;
    v_normal = normalize(u_normal_matrix * in_normal);
    v_texcoord = in_texcoord;
    v_tangent = normalize(u_normal_matrix * in_tangent);
    v_light_space_pos = u_light_space_matrix * world;
    gl_Position = u_mvp * vec4(in_position, 1.0);
}

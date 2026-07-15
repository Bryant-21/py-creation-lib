#version 330

in vec3 in_center;
in vec2 in_corner;
in vec2 in_uv;
in vec4 in_color;
in float in_size;
in float in_rotation;

uniform mat4 u_vp;
uniform vec3 u_camera_right;
uniform vec3 u_camera_up;

out vec2 v_uv;
out vec4 v_color;

void main() {
    float c = cos(in_rotation);
    float s = sin(in_rotation);
    vec2 rotated_corner = vec2(
        in_corner.x * c - in_corner.y * s,
        in_corner.x * s + in_corner.y * c
    );
    vec3 world_pos = in_center
        + u_camera_right * rotated_corner.x * in_size
        + u_camera_up * rotated_corner.y * in_size;

    v_uv = in_uv;
    v_color = in_color;
    gl_Position = u_vp * vec4(world_pos, 1.0);
}

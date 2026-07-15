#version 330
uniform mat4 u_light_mvp;
in vec3 in_position;
void main() {
    gl_Position = u_light_mvp * vec4(in_position, 1.0);
}

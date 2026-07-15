#version 330
// Shadow map depth pass — renders scene from light's perspective.

uniform mat4 u_light_mvp;    // lightProjection * lightView * model

in vec3 in_position;

void main() {
    gl_Position = u_light_mvp * vec4(in_position, 1.0);
}

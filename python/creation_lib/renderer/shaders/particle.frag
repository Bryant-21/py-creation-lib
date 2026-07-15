#version 330

uniform sampler2D ParticleTexture;
uniform sampler2D GreyscaleTexture;
uniform bool hasParticleTexture;
uniform bool hasGreyscaleTexture;
uniform bool greyscaleColor;
uniform bool greyscaleAlpha;

in vec2 v_uv;
in vec4 v_color;

layout(location = 0) out vec4 fragColor;
layout(location = 1) out vec4 fragNormal;

vec4 color_lookup(sampler2D greyscale_map, float x, float y) {
    return textureLod(greyscale_map, vec2(clamp(x, 0.0, 1.0), clamp(y, 0.0, 1.0)), 0.0);
}

void main() {
    vec4 tex_color = hasParticleTexture ? texture(ParticleTexture, v_uv) : vec4(1.0);
    vec4 color = v_color * tex_color;
    if (hasParticleTexture && hasGreyscaleTexture && greyscaleColor) {
        vec4 palette_color = color_lookup(GreyscaleTexture, tex_color.g, v_color.r);
        color.rgb = palette_color.rgb * v_color.rgb;
    }
    if (hasParticleTexture && hasGreyscaleTexture && greyscaleAlpha) {
        color.a = color_lookup(GreyscaleTexture, tex_color.a, color.a).a;
    }
    if (color.a <= 0.001) {
        discard;
    }
    fragColor = color;
    fragNormal = vec4(0.5, 0.5, 1.0, color.a);
}

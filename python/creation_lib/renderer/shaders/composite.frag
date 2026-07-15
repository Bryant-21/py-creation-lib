#version 330
// Final compositing pass — multiplies scene color by AO factor.

in vec2 vTexCoord;
out vec4 fragColor;

uniform sampler2D sceneTex;   // rendered scene color
uniform sampler2D aoTex;      // blurred SSAO (single channel)
uniform float aoEnabled;      // 1.0 = apply AO, 0.0 = bypass

void main() {
    vec4 sceneColor = texture(sceneTex, vTexCoord);

    if (aoEnabled > 0.5) {
        float ao = texture(aoTex, vTexCoord).r;
        // AO affects diffuse/ambient but not emissive/specular highlights.
        // Since we can't separate them here, use a gentle blend.
        sceneColor.rgb *= mix(1.0, ao, 0.85);
    }

    fragColor = sceneColor;
}

#version 330
// Bilateral blur for SSAO — smooths AO noise while preserving depth edges.

in vec2 vTexCoord;
out float fragAO;

uniform sampler2D aoTex;      // raw SSAO output
uniform sampler2D depthTex;   // scene depth (for edge detection)
uniform vec2 texelSize;       // 1.0 / screenSize
uniform vec2 blurDir;         // (1,0) for horizontal, (0,1) for vertical

void main() {
    float centerDepth = texture(depthTex, vTexCoord).r;
    float centerAO = texture(aoTex, vTexCoord).r;

    float result = 0.0;
    float totalWeight = 0.0;

    // 5-tap bilateral blur
    for (int i = -2; i <= 2; i++) {
        vec2 offset = blurDir * texelSize * float(i);
        vec2 sampleUV = vTexCoord + offset;

        float sampleAO = texture(aoTex, sampleUV).r;
        float sampleDepth = texture(depthTex, sampleUV).r;

        // Depth-aware weight: reject samples across depth discontinuities
        float depthDiff = abs(centerDepth - sampleDepth);
        float depthWeight = exp(-depthDiff * 1000.0);

        // Spatial weight (simple box)
        float weight = depthWeight;

        result += sampleAO * weight;
        totalWeight += weight;
    }

    fragAO = result / max(totalWeight, 0.001);
}

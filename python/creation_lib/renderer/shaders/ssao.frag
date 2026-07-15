#version 330
// Screen-Space Ambient Occlusion (SSAO) — hemisphere sampling.
//
// Reads depth buffer and view-space normals to estimate how occluded
// each pixel is by nearby geometry.

in vec2 vTexCoord;
out float fragAO;

uniform sampler2D depthTex;       // scene depth buffer
uniform sampler2D normalTex;      // view-space normals (RGB16F)
uniform sampler2D noiseTex;       // 4x4 random rotation texture
uniform vec3 samples[32];         // hemisphere sample kernel
uniform int numSamples;           // actual sample count (16 or 32)
uniform mat4 projection;          // camera projection matrix
uniform mat4 invProjection;       // inverse projection
uniform vec2 screenSize;          // viewport dimensions
uniform float radius;             // AO sampling radius (world units)
uniform float bias;               // depth comparison bias
uniform float intensity;          // AO strength multiplier

// Reconstruct view-space position from depth
vec3 viewPosFromDepth(vec2 uv, float depth) {
    // NDC: xy in [-1,1], z in [-1,1]
    vec4 ndc = vec4(uv * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    vec4 viewPos = invProjection * ndc;
    return viewPos.xyz / viewPos.w;
}

void main() {
    float depth = texture(depthTex, vTexCoord).r;
    if (depth >= 1.0) {
        fragAO = 1.0;
        return;
    }

    vec3 fragPos = viewPosFromDepth(vTexCoord, depth);
    vec3 normal = texture(normalTex, vTexCoord).rgb;
    // Normals stored as [0,1], remap to [-1,1]
    normal = normalize(normal * 2.0 - 1.0);

    // Tile the 4x4 noise texture across the screen
    vec2 noiseScale = screenSize / 4.0;
    vec3 randomVec = texture(noiseTex, vTexCoord * noiseScale).rgb * 2.0 - 1.0;

    // Gram-Schmidt to build TBN from normal + random vector
    vec3 tangent = normalize(randomVec - normal * dot(randomVec, normal));
    vec3 bitangent = cross(normal, tangent);
    mat3 TBN = mat3(tangent, bitangent, normal);

    float occlusion = 0.0;
    for (int i = 0; i < numSamples; i++) {
        // Orient sample in hemisphere along the surface normal
        vec3 samplePos = fragPos + TBN * samples[i] * radius;

        // Project sample to screen space
        vec4 offset = projection * vec4(samplePos, 1.0);
        offset.xyz /= offset.w;
        offset.xy = offset.xy * 0.5 + 0.5;

        // Sample depth at the projected position
        float sampleDepth = texture(depthTex, offset.xy).r;
        vec3 sampleViewPos = viewPosFromDepth(offset.xy, sampleDepth);

        // Range check: only occlude if the sample is within radius
        float rangeCheck = smoothstep(0.0, 1.0, radius / abs(fragPos.z - sampleViewPos.z));

        // Occlusion: sample is behind the test point (closer to camera)
        occlusion += (sampleViewPos.z >= samplePos.z + bias ? 1.0 : 0.0) * rangeCheck;
    }

    occlusion = 1.0 - (occlusion / float(numSamples)) * intensity;
    fragAO = clamp(occlusion, 0.0, 1.0);
}

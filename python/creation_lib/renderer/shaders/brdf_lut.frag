#version 330
// BRDF LUT generation — precompute split-sum Fresnel scale/bias.
// One-time 512x512 render. Output: R=scale, G=bias.
// Reference: Epic Games, "Real Shading in Unreal Engine 4" (SIGGRAPH 2013)
in vec2 vTexCoord;
out vec2 fragColor;

const float PI = 3.14159265359;

// Van der Corput radical inverse (bit reversal)
float radicalInverse_VdC(uint bits) {
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10; // / 0x100000000
}

vec2 hammersley(uint i, uint N) {
    return vec2(float(i) / float(N), radicalInverse_VdC(i));
}

// GGX importance sampling — returns half-vector in tangent space
vec3 importanceSampleGGX(vec2 Xi, float roughness) {
    float a = roughness * roughness;
    float phi = 2.0 * PI * Xi.x;
    float cosTheta = sqrt((1.0 - Xi.y) / (1.0 + (a * a - 1.0) * Xi.y));
    float sinTheta = sqrt(1.0 - cosTheta * cosTheta);
    return vec3(cos(phi) * sinTheta, sin(phi) * sinTheta, cosTheta);
}

// Schlick-GGX geometry (direct lighting k)
float geometrySchlickGGX(float NdotV, float roughness) {
    float k = (roughness * roughness) / 2.0;  // IBL remapping
    return NdotV / (NdotV * (1.0 - k) + k);
}

float geometrySmith(float NdotV, float NdotL, float roughness) {
    return geometrySchlickGGX(NdotV, roughness) * geometrySchlickGGX(NdotL, roughness);
}

void main() {
    float NdotV = vTexCoord.x;  // X axis = cos(theta_v)
    float roughness = vTexCoord.y;  // Y axis = roughness

    // Clamp to avoid singularities
    NdotV = max(NdotV, 0.001);
    roughness = max(roughness, 0.001);

    vec3 V;
    V.x = sqrt(1.0 - NdotV * NdotV);  // sin(theta_v)
    V.y = 0.0;
    V.z = NdotV;  // cos(theta_v)

    vec3 N = vec3(0.0, 0.0, 1.0);

    float A = 0.0;  // Fresnel scale
    float B = 0.0;  // Fresnel bias

    const uint SAMPLE_COUNT = 1024u;
    for (uint i = 0u; i < SAMPLE_COUNT; ++i) {
        vec2 Xi = hammersley(i, SAMPLE_COUNT);
        vec3 H = importanceSampleGGX(Xi, roughness);
        vec3 L = normalize(2.0 * dot(V, H) * H - V);

        float NdotL = max(L.z, 0.0);
        float NdotH = max(H.z, 0.0);
        float VdotH = max(dot(V, H), 0.0);

        if (NdotL > 0.0) {
            float G = geometrySmith(NdotV, NdotL, roughness);
            float G_Vis = (G * VdotH) / (NdotH * NdotV);
            float Fc = pow(1.0 - VdotH, 5.0);

            A += (1.0 - Fc) * G_Vis;
            B += Fc * G_Vis;
        }
    }

    fragColor = vec2(A, B) / float(SAMPLE_COUNT);
}

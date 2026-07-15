#version 330
// Starfield custom PBR LUT — matches NifSkope lib/libfo76utils/src/pbr_lut.cpp.
// 4-channel RGBA16F, NOT the standard Epic split-sum LUT.
//
//   R = indirect specular F (fresnel * G, normalized by sum of G)
//   G = indirect specular G (geometry sum)
//   B = fresnel_n(NdotV)  — polynomial n=1.5 approximation
//   A = fresnel_n(roughness)
//
// Sampled by stf_default.frag as:
//   fDirect = LUT(LdotH, NdotL0).ba   // direct lighting fresnel factors
//   envLUT  = LUT(NdotV, roughness).rgba
//   refl *= f * envLUT.g;
//   ambient *= (1 - f0) * fDiffEnv;   // fDiffEnv from envLUT.b
//
// NOTE the k formula differs from Epic: NifSkope uses k = a * 0.5 (where
// a = roughness^2), not (r+1)^2/8. And their geometry normalization uses
// V·H, N·L, N·H directly instead of N·V paired with N·L.

in vec2 vTexCoord;
out vec4 fragColor;

const float PI = 3.14159265359;
const uint SAMPLE_COUNT = 1024u;

float radicalInverse_VdC(uint bits) {
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10;
}

vec2 hammersley(uint i, uint N) {
    return vec2(float(i) / float(N), radicalInverse_VdC(i));
}

// Importance-sampled GGX half-vector, normal = (0, 0, 1). a2 = roughness^4.
vec3 importanceSampleGGX(vec2 xi, float a2) {
    float cosTheta = sqrt((1.0 - xi.y) / (a2 * xi.y + (1.0 - xi.y)));
    float sinTheta = sqrt(1.0 - cosTheta * cosTheta);
    float phi = xi.x * 2.0 * PI;
    return vec3(cos(phi) * sinTheta, sin(phi) * sinTheta, cosTheta);
}

// Schlick fresnel polynomial (NifSkope fresnel_n with n=1.5), squared.
float fresnel_n(float x) {
    float y = ((((x * -1.03202882 + 3.64690610) * x - 5.46708684) * x
                + 4.84513712) * x - 2.99292756) * x + 1.0;
    return y * y;
}

void main() {
    float nDotV = vTexCoord.x;
    float roughness = vTexCoord.y;
    nDotV = max(nDotV, 0.001);
    roughness = max(roughness, 0.001);

    float a = roughness * roughness;
    float a2 = a * a;
    float k = a * 0.5;

    // View direction at the sampled nDotV
    vec3 V = vec3(sqrt(1.0 - nDotV * nDotV), 0.0, nDotV);

    float s1 = 0.0;  // sum of (f * g)
    float s2 = 0.0;  // sum of g

    for (uint i = 0u; i < SAMPLE_COUNT; ++i) {
        vec2 xi = hammersley(i, SAMPLE_COUNT);
        vec3 H = importanceSampleGGX(xi, a2);
        // NifSkope computes nDotL as h_z * (2 * vDotH) - v_z directly
        float h_x = H.x;
        float nDotH = H.z;
        float vDotH = max(h_x * V.x + nDotH * V.z, 0.0);
        float nDotL = max(nDotH * (vDotH + vDotH) - V.z, 0.0);
        nDotH = max(nDotH, 0.0);

        if (nDotH > 0.0) {
            float g = nDotL * vDotH
                    / ((nDotL * (1.0 - k) + k) * nDotH);
            float f = fresnel_n(vDotH);
            s1 += f * g;
            s2 += g;
        }
    }

    // Normalization: NifSkope divides by (nSpec * (nDotV*(1-k)+k))
    float denom = float(SAMPLE_COUNT) * (nDotV * (1.0 - k) + k);
    float S1 = s1 / denom;
    float S2 = s2 / denom;

    // R = F (normalized by G), G = G, B = fresnel_n(nDotV), A = fresnel_n(roughness)
    fragColor = vec4(
        (S2 > 0.0) ? (S1 / S2) : 0.0,
        S2,
        fresnel_n(nDotV),
        fresnel_n(roughness)
    );
}

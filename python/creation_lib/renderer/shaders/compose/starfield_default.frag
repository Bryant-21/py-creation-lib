#version 330 core
// Starfield default fragment shader — direct wholesale port of
// tools/sf_render_test.py FRAG_SRC (lines 923-1221).
//
// Only deviation from the standalone: the 6 separate GGX-prefiltered cube
// samplers (uCubeMip0..uCubeMip5) are collapsed into a single samplerCube
// whose mip chain already holds the prefiltered levels, since the editor's
// build_environment_cubes (shader_pipeline.py:1275) stitches them that way.
// sampleSpecCube therefore uses textureLod instead of the 5-branch ladder.
//
// Everything else (uniform names, math, blend logic, alpha path, tonemap)
// matches the standalone byte-for-byte. The editor-specific extras
// (shadows, MRT, point lights, dbg knobs, toggles) have been dropped for
// the Starfield path — the handoff said rip it out and implement the
// standalone's logic.

#define MAX_LAYERS 3
#define MAX_TEX_UNITS 32

in vec4 vTexCoord;
in vec4 vColor;
in mat3 vBtn;
in vec3 vViewDir;
in vec3 vLightDir;

out vec4 fragColor;

uniform sampler2D uTextures[MAX_TEX_UNITS];
uniform sampler2D uBrdfLUT;

// Single GGX-prefiltered specular cube (mip i = roughness i/5).
uniform samplerCube uEnvSpec;
uniform samplerCube uEnvIrradiance;
uniform bool  uHasCubeMap;
uniform bool  uHasSpecular;
uniform float uEnvLodBias;

// Per-layer material slots (-1 = unused)
uniform int   uNumLayers;
uniform int   uLayerAlbedoUnit[MAX_LAYERS];
uniform int   uLayerNormalUnit[MAX_LAYERS];
uniform int   uLayerRoughUnit [MAX_LAYERS];
uniform int   uLayerMetalUnit [MAX_LAYERS];
uniform int   uLayerAoUnit    [MAX_LAYERS];
uniform int   uLayerOpacityUnit[MAX_LAYERS];
uniform bool  uHasOpacity;
uniform float uAlphaThreshold;
uniform bool  uAlphaTest;

uniform vec2  uLayerUvScale    [MAX_LAYERS];
uniform vec2  uLayerUvOffset   [MAX_LAYERS];
uniform int   uLayerUvChannel  [MAX_LAYERS];
uniform vec3  uLayerTint       [MAX_LAYERS];
uniform float uLayerNormalScale[MAX_LAYERS];

// Blenders — one entry per layer i > 0
uniform int   uBlenderCount;
uniform int   uBlenderMaskUnit[MAX_LAYERS];
uniform int   uBlenderMode    [MAX_LAYERS]; // 0 lerp,1 add,2 poscontrast,3 none
uniform int   uBlenderVcChan  [MAX_LAYERS]; // -1 disabled
uniform float uBlenderMaskInt [MAX_LAYERS];
uniform bool  uBlendAlbedo    [MAX_LAYERS];
uniform bool  uBlendNormal    [MAX_LAYERS];
uniform bool  uBlendMetal     [MAX_LAYERS];
uniform bool  uBlendRough     [MAX_LAYERS];
uniform bool  uBlendAO        [MAX_LAYERS];
uniform bool  uBlendAddNormal [MAX_LAYERS];

uniform vec3  uLightSourceDiffuse;
uniform vec3  uLightSourceAmbient;
uniform float uToneMapScale;
uniform float uBrightnessScale;
uniform float uEnvIntensity;
uniform mat3  uEnvRotation;

// -- NifSkope helpers (verbatim from stf_default.frag via the standalone) --
float LightingFuncGGX_REF(float NdotH, float NdotL, float NdotV, float roughness) {
    float alpha = roughness * roughness;
    float alphaSqr = alpha * alpha;
    float denom = NdotH * NdotH;
    denom = (denom * alphaSqr) + (1.0 - denom);
    float D = alphaSqr / (denom * denom * 4.0);
    float k = alpha * 0.5;
    float G = NdotL / (mix(NdotL, 1.0, k) * mix(NdotV, 1.0, k));
    return D * G;
}

vec3 sampleSpecCube(vec3 dir, float rough) {
    // Editor's uEnvSpec holds GGX-prefiltered mip chain — mip i == roughness i/5.
    float lod = clamp(rough + uEnvLodBias, 0.0, 1.0) * 5.0;
    return textureLod(uEnvSpec, dir, lod).rgb;
}

vec3 sampleIrradiance(vec3 dir) {
    return texture(uEnvIrradiance, dir).rgb;
}

vec3 sfTonemap(vec3 x, float y) {
    float a = 0.15, b = 0.50, c = 0.10, d = 0.20, e = 0.02, f = 0.30;
    vec3 z = x * (y * 4.22978723);
    z = (z * (a * z + b * c) + d * e) / (z * (a * z + b) + d * f) - e / f;
    return z / (y * 0.93333333);
}

vec4 sampleUnit(int unit, vec2 uv) {
    // Dynamic indexing into a sampler array requires constant expressions on
    // GL 3.3 / ES — the switch ladder is the portable workaround, verbatim
    // from the standalone.
    switch (unit) {
        case  0: return texture(uTextures[ 0], uv);
        case  1: return texture(uTextures[ 1], uv);
        case  2: return texture(uTextures[ 2], uv);
        case  3: return texture(uTextures[ 3], uv);
        case  4: return texture(uTextures[ 4], uv);
        case  5: return texture(uTextures[ 5], uv);
        case  6: return texture(uTextures[ 6], uv);
        case  7: return texture(uTextures[ 7], uv);
        case  8: return texture(uTextures[ 8], uv);
        case  9: return texture(uTextures[ 9], uv);
        case 10: return texture(uTextures[10], uv);
        case 11: return texture(uTextures[11], uv);
        case 12: return texture(uTextures[12], uv);
        case 13: return texture(uTextures[13], uv);
        case 14: return texture(uTextures[14], uv);
        case 15: return texture(uTextures[15], uv);
        case 16: return texture(uTextures[16], uv);
        case 17: return texture(uTextures[17], uv);
        case 18: return texture(uTextures[18], uv);
        case 19: return texture(uTextures[19], uv);
        case 20: return texture(uTextures[20], uv);
        case 21: return texture(uTextures[21], uv);
        case 22: return texture(uTextures[22], uv);
        case 23: return texture(uTextures[23], uv);
        case 24: return texture(uTextures[24], uv);
        case 25: return texture(uTextures[25], uv);
        case 26: return texture(uTextures[26], uv);
        case 27: return texture(uTextures[27], uv);
        case 28: return texture(uTextures[28], uv);
        case 29: return texture(uTextures[29], uv);
        case 30: return texture(uTextures[30], uv);
        default: return texture(uTextures[31], uv);
    }
}

vec2 layerUV(int i) {
    vec2 base = (uLayerUvChannel[i] == 0) ? vTexCoord.st : vTexCoord.pq;
    return base * uLayerUvScale[i] + uLayerUvOffset[i];
}

float getBlenderMask(int i) {
    float r = 1.0;
    if (uBlenderMaskUnit[i] >= 0)
        r = sampleUnit(uBlenderMaskUnit[i], layerUV(0)).r;
    if (uBlenderVcChan[i] >= 0)
        r *= vColor[uBlenderVcChan[i]];
    return r * uBlenderMaskInt[i];
}

void main() {
    // -------- Layer 0 --------
    vec2 uv0 = layerUV(0);
    vec3 baseMap = uLayerTint[0];
    if (uLayerAlbedoUnit[0] >= 0)
        baseMap = sampleUnit(uLayerAlbedoUnit[0], uv0).rgb * uLayerTint[0];

    vec3 normal = vec3(0.0, 0.0, 1.0);
    if (uLayerNormalUnit[0] >= 0) {
        vec2 nrg = sampleUnit(uLayerNormalUnit[0], uv0).rg * 2.0 - 1.0;
        nrg *= uLayerNormalScale[0];
        normal.rg = nrg;
        normal.b = sqrt(max(1.0 - dot(nrg, nrg), 0.0));
    }

    vec3 pbrMap = vec3(1.0, 0.0, 1.0); // roughness, metalness, AO
    if (uLayerRoughUnit[0] >= 0)
        pbrMap.r = sampleUnit(uLayerRoughUnit[0], uv0).r;
    if (uLayerMetalUnit[0] >= 0)
        pbrMap.g = sampleUnit(uLayerMetalUnit[0], uv0).r;
    if (uLayerAoUnit[0] >= 0)
        pbrMap.b = sampleUnit(uLayerAoUnit[0], uv0).r;

    // -------- Layers 1..N --------
    int numLayers = min(uNumLayers, MAX_LAYERS);
    for (int i = 1; i < MAX_LAYERS; ++i) {
        if (i >= numLayers) break;
        int bi = i - 1;
        int mode = uBlenderMode[bi];
        if (mode == 3) continue; // "None"

        vec2 uvI = layerUV(i);

        vec3 layerBase = uLayerTint[i];
        if (uLayerAlbedoUnit[i] >= 0)
            layerBase = sampleUnit(uLayerAlbedoUnit[i], uvI).rgb * uLayerTint[i];

        vec3 layerNormal = vec3(0.0, 0.0, 1.0);
        if (uLayerNormalUnit[i] >= 0) {
            vec2 nrg = sampleUnit(uLayerNormalUnit[i], uvI).rg * 2.0 - 1.0;
            nrg *= uLayerNormalScale[i];
            layerNormal.rg = nrg;
            layerNormal.b = sqrt(max(1.0 - dot(nrg, nrg), 0.0));
        }
        vec3 layerPBR = pbrMap;
        if (uLayerRoughUnit[i] >= 0) layerPBR.r = sampleUnit(uLayerRoughUnit[i], uvI).r;
        if (uLayerMetalUnit[i] >= 0) layerPBR.g = sampleUnit(uLayerMetalUnit[i], uvI).r;
        if (uLayerAoUnit[i]   >= 0)  layerPBR.b = sampleUnit(uLayerAoUnit[i],   uvI).r;

        float srcMask = clamp(getBlenderMask(bi), 0.0, 1.0);

        if (uBlendAlbedo[bi])
            baseMap = mix(baseMap, layerBase, srcMask);
        if (uBlendMetal[bi])
            pbrMap.g = mix(pbrMap.g, layerPBR.g, srcMask);
        if (uBlendRough[bi])
            pbrMap.r = mix(pbrMap.r, layerPBR.r, srcMask);
        if (uBlendAO[bi])
            pbrMap.b = mix(pbrMap.b, layerPBR.b, srcMask);
        if (uBlendNormal[bi]) {
            if (uBlendAddNormal[bi]) {
                normal.rg += layerNormal.rg * srcMask;
                normal.b = sqrt(max(1.0 - dot(normal.rg, normal.rg), 0.0));
            } else {
                normal = normalize(mix(normal, layerNormal, srcMask));
            }
        }
    }

    // -------- Lighting (NifSkope stf_default.frag:611-693 verbatim-ish) --------
    mat3 btn = mat3(normalize(vBtn[0]), normalize(vBtn[1]), normalize(vBtn[2]));
    if (!gl_FrontFacing) normal.z *= -1.0;
    vec3 N = normalize(btn * normal);

    vec3 V = normalize(vViewDir);
    vec3 L = normalize(vLightDir);
    vec3 R = reflect(-V, N);
    vec3 H = normalize(L + V);

    float NdotL  = dot(N, L);
    float NdotL0 = max(NdotL, 0.0);
    float NdotH  = clamp(dot(N, H), 0.0, 1.0);
    float NdotV  = abs(dot(N, V));
    float LdotH  = dot(L, H);

    vec3 reflectedWS = uEnvRotation * R;
    vec3 normalWS    = uEnvRotation * N;

    vec3 f0     = mix(vec3(0.04), baseMap, pbrMap.g);
    vec3 albedo = baseMap * (1.0 - pbrMap.g);

    float roughness = pbrMap.r;
    vec3 spec = uLightSourceDiffuse;
    spec *= LightingFuncGGX_REF(NdotH, NdotL0, NdotV, clamp(roughness, 0.045, 0.95));

    vec3 diffuse = vec3(NdotL0);
    vec2 fDirect = textureLod(uBrdfLUT, vec2(LdotH, NdotL0), 0.0).ba;
    spec *= mix(f0, vec3(1.0), fDirect.x);
    vec4 envLUT = textureLod(uBrdfLUT, vec2(NdotV, roughness), 0.0);
    vec2 fDiff = vec2(fDirect.y, envLUT.b);
    fDiff = fDiff * (LdotH * LdotH * roughness * 2.0 - 0.5) + 1.0;
    diffuse *= (vec3(1.0) - f0) * fDiff.x * fDiff.y;

    vec3 refl = vec3(0.0);
    vec3 ambient = uLightSourceAmbient;
    if (uHasCubeMap) {
        refl = sampleSpecCube(reflectedWS, clamp(roughness, 0.0, 1.0)) * uEnvIntensity;
        refl *= ambient;
        ambient *= sampleIrradiance(normalWS) * uEnvIntensity;
    } else {
        ambient *= 0.08;
        refl = ambient;
    }
    vec3 f = mix(f0, vec3(1.0), envLUT.r);
    if (!uHasSpecular) {
        albedo = baseMap;
        diffuse = vec3(NdotL0);
        spec = vec3(0.0);
        f = vec3(0.0);
    } else {
        float fDiffEnv = envLUT.b * ((NdotV + 1.0) * roughness - 0.5) + 1.0;
        ambient *= (vec3(1.0) - f0) * fDiffEnv;
    }
    float ao = pbrMap.b;
    float specOcc = max((ao - 1.0) * ((NdotV * 1.125 - 2.625) * NdotV + 2.5) + 1.0, 0.0);
    refl *= f * envLUT.g;

    vec3 color = (diffuse * uLightSourceDiffuse + ambient) * albedo * ao;
    color += (spec + refl) * specOcc;

    color = sfTonemap(color * uBrightnessScale, uToneMapScale);

    float alpha = 1.0;
    if (uHasOpacity && uLayerOpacityUnit[0] >= 0) {
        alpha = sampleUnit(uLayerOpacityUnit[0], layerUV(0)).r;
    }
    if (uAlphaTest && alpha < uAlphaThreshold)
        discard;
    fragColor = vec4(color, alpha);
}

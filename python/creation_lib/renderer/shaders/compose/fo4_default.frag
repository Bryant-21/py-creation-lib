#version 330
// FO4 TBR fragment shader — composed from modular includes.
// Regression-safe: produces identical output to the original monolithic shader.
//
// TBR (Todd-Based Rendering) = PBR SpecGloss without height maps.

#include "includes/common.glsl"
#include "includes/fresnel.glsl"
#include "includes/cubemap.glsl"
#include "includes/lighting.glsl"
#include "includes/normal_decode.glsl"
#include "includes/tonemapping.glsl"
#include "materials/specgloss.glsl"

// ---------- Textures ----------
uniform sampler2D diffuseMap;
uniform sampler2D specMap;
uniform sampler2D normalMap;
uniform sampler2D envMap;
uniform sampler2D   envMaskMap;
uniform sampler2D   greyscaleMap;
uniform sampler2D   glowMap;
uniform float     hasSpecularMap;
uniform float     hasNormalMap;
uniform float     hasEnvMap;
uniform float     hasEnvMask;
uniform float     hasEmit;
uniform float     hasGlowMap;
uniform float     greyscaleColor;
uniform float     paletteScale;
uniform float     envMapScale;
uniform float     normalScale;

// ---------- Debug toggles ----------
uniform float toggle_diffuse;
uniform float toggle_normal;
uniform float toggle_spec;
uniform float toggle_lighting;
uniform float toggle_vertexColor;
uniform float toggle_envMap;

// ---------- TBR debug tuning ----------
uniform float dbg_envBoost;
uniform float dbg_metalF0;
uniform float dbg_diffuseBleed;
uniform float dbg_exposure;
uniform float dbg_specBoost;
uniform float dbg_ambientBoost;

// ---------- Material uniforms ----------
uniform vec3  specColor;
uniform float specStrength;
uniform float specGlossiness;
uniform float fresnelPower;
uniform vec3  glowColor;
uniform float glowMult;

// ---------- Lighting ----------
uniform vec3 lightDir0;
uniform vec3 lightCol0;
uniform vec3 lightDir1;
uniform vec3 lightCol1;
uniform vec3 ambientCol;

// ---------- Mirror light ----------
uniform float mirrorLightEnabled;
uniform vec3  lightDir2;
uniform vec3  lightCol2;

// ---------- NIF Point Lights ----------
uniform int   numPointLights;
uniform vec3  pointLightPos[4];
uniform vec3  pointLightCol[4];
uniform float pointLightRadius[4];
uniform float pointLightConstAtten[4];
uniform float pointLightLinearAtten[4];
uniform float pointLightQuadAtten[4];

// ---------- Alpha ----------
uniform int   alphaFlags;
uniform float alphaThreshold;

// ---------- Shadow mapping ----------
uniform sampler2D shadowMap;
uniform mat4 lightSpaceMatrix;
uniform float shadowEnabled;

// ---------- MRT control ----------
uniform float mrtEnabled;

// ---------- Global opacity ----------
uniform float u_mesh_alpha;

// ---------- Camera ----------
uniform vec3 cameraPos;

// ---------- Varyings ----------
in vec2 vTexCoord;
in vec3 vNormalWorld;
in vec3 vPosWorld;
in vec3 vTangentWorld;
in vec3 vBitangentWorld;
in vec4 vVertexColor;
in vec3 vNormalView;

layout(location = 0) out vec4 fragColor;
layout(location = 1) out vec4 fragNormal;


void main()
{
    // ---- Diffuse texture ----
    vec4 diffTex = texture(diffuseMap, vTexCoord);

    // ---- Alpha handling ----
    float combinedAlpha = vVertexColor.a * diffTex.a;

    if (alphaFlags > 0) {
        int testFunc = alphaFlags & 7;
        int m = (combinedAlpha < alphaThreshold) ? 0x2B2B
              : (combinedAlpha > alphaThreshold) ? 0x7171
              : 0x4D4D;
        if ((m & (1 << testFunc)) == 0)
            discard;
    }

    float finalAlpha = 1.0;
    if ((alphaFlags & 8) != 0) {
        finalAlpha = combinedAlpha;
    }

    // ---- Albedo ----
    vec3 albedo = diffTex.rgb;

    if (toggle_vertexColor > 0.5) {
        albedo *= vVertexColor.rgb;
    }

    // Greyscale-to-palette color lookup (FO4 weapon/armor palette system)
#ifdef HAS_PALETTE
    if (greyscaleColor > 0.5) {
        vec2 palUV = vec2(diffTex.g, paletteScale * vVertexColor.r);
        albedo = textureLod(greyscaleMap, palUV, 0.0).rgb;
    }
#endif

    if (toggle_diffuse < 0.5) {
        albedo = vec3(1.0);
    }

    // ---- Unlit mode ----
    if (toggle_lighting < 0.5) {
        fragColor = vec4(albedo, finalAlpha);
        fragNormal = vec4(normalize(vNormalView) * 0.5 + 0.5, 1.0);
        return;
    }

    // ---- Normal ----
    vec3 N = decodeNormal(normalMap, vTexCoord, hasNormalMap, toggle_normal,
                          normalScale,
                          vNormalWorld, vTangentWorld, vBitangentWorld);

    vec3 V = normalize(cameraPos - vPosWorld);
    vec3 R = reflect(-V, N);

    float NdotV = max(dot(N, V), FLT_EPSILON);

    // ---- Specular parameters ----
    float smoothness = clamp(specGlossiness, 0.0, 1.0);
    float specMask = 1.0;
    float g = 1.0;
    float s = 1.0;

    if (hasSpecularMap > 0.5 && toggle_spec > 0.5) {
        vec4 specTex = texture(specMap, vTexCoord);
        g = specTex.g;
        s = specTex.r;
        smoothness = g * smoothness;
        specMask = s * specStrength;
    }

    float roughness = 1.0 - smoothness;
    float fSpecPower = exp2(smoothness * 10.0 + 1.0);

    // ---- Lighting ----
    vec3 diffuseTerm = ambientCol;

    // ---- Key light ----
    vec3 spec = vec3(0.0);
    {
        vec3  L     = normalize(lightDir0);
        vec3  H     = normalize(L + V);
        float NdotL = dot(N, L);
        float NdotL0 = max(NdotL, FLT_EPSILON);
        float NdotH = max(dot(N, H), FLT_EPSILON);
        float VdotH = max(dot(V, H), FLT_EPSILON);

        float diff = OrenNayarFull(NdotL, dot(N, V), dot(L, V), roughness);
        diffuseTerm = vec3(diff);

        if (hasSpecularMap > 0.5 && NdotL > 0.0 && toggle_spec > 0.5) {
            spec = TorranceSparrow(NdotL0, NdotH, NdotV, VdotH,
                                   vec3(specMask), fSpecPower, 0.2, fresnelPower)
                   * NdotL0 * lightCol0 * specColor;
        }
    }

    // ---- Shadow factor ----
    float shadow = calcShadow(vPosWorld, shadowMap, lightSpaceMatrix, shadowEnabled);

    // ---- Energy conservation (TBR metalness) ----
    float metalness = 0.0;
    if (hasSpecularMap > 0.5 && toggle_spec > 0.5) {
        metalness = s * smoothness;
    }
    float diffuseWeight = max(dbg_diffuseBleed, 1.0 - metalness);

    // ---- Assemble color ----
    vec3 color = diffuseTerm * albedo * lightCol0 * diffuseWeight * shadow;
    color += ambientCol * albedo * diffuseWeight * dbg_ambientBoost;
    color += spec * shadow * dbg_specBoost;

    // Ambient Fresnel
    if (toggle_spec > 0.5) {
        float VdotH = max(dot(V, normalize(normalize(lightDir0) + V)), FLT_EPSILON);
        color += ambientCol * specMask * fresnelSchlick(VdotH, 0.2, fresnelPower)
                 * (1.0 - NdotV) * lightCol0;
    }

    // ---- Fill light ----
    {
        vec3  L     = normalize(lightDir1);
        float NdotL = dot(N, L);
        float diff  = OrenNayar(L, V, N, roughness, max(NdotL, FLT_EPSILON));
        color += lightCol1 * albedo * diff * diffuseWeight;
    }

    // ---- Mirror light ----
    if (mirrorLightEnabled > 0.5) {
        vec3  L     = normalize(lightDir2);
        vec3  H     = normalize(L + V);
        float NdotL = dot(N, L);
        float NdotL0 = max(NdotL, FLT_EPSILON);
        float NdotH = max(dot(N, H), FLT_EPSILON);
        float VdotH = max(dot(V, H), FLT_EPSILON);

        float diff = OrenNayarFull(NdotL, dot(N, V), dot(L, V), roughness);
        color += vec3(diff) * albedo * lightCol2 * diffuseWeight;

        if (hasSpecularMap > 0.5 && NdotL > 0.0 && toggle_spec > 0.5) {
            vec3 mSpec = TorranceSparrow(NdotL0, NdotH, NdotV, VdotH,
                                         vec3(specMask), fSpecPower, 0.2, fresnelPower)
                         * NdotL0 * lightCol2 * specColor;
            color += mSpec * dbg_specBoost;
        }
    }

    // ---- NIF Point Lights ----
    for (int i = 0; i < numPointLights; i++) {
        vec3 lightVec = pointLightPos[i] - vPosWorld;
        float dist = length(lightVec);

        if (dist > pointLightRadius[i])
            continue;

        vec3  L     = lightVec / dist;
        float NdotL = dot(N, L);
        if (NdotL <= 0.0)
            continue;

        float atten = 1.0 / max(
            pointLightConstAtten[i]
            + pointLightLinearAtten[i] * dist
            + pointLightQuadAtten[i] * dist * dist,
            1.0
        );

        float radFade = 1.0 - smoothstep(pointLightRadius[i] * 0.8, pointLightRadius[i], dist);
        atten *= radFade;

        float NdotL0 = max(NdotL, FLT_EPSILON);

        // Diffuse — FO4's engine applies point light diffuse to ALL surface types
        float diff = OrenNayar(L, V, N, roughness, NdotL0);
        color += pointLightCol[i] * albedo * diff * atten;

        if (hasSpecularMap > 0.5 && toggle_spec > 0.5) {
            vec3  H     = normalize(L + V);
            float NdotH = max(dot(N, H), FLT_EPSILON);
            float VdotH = max(dot(V, H), FLT_EPSILON);
            vec3 pSpec = TorranceSparrow(NdotL0, NdotH, NdotV, VdotH,
                                         vec3(specMask), fSpecPower, 0.2, fresnelPower)
                         * NdotL0 * pointLightCol[i] * specColor;
            color += pSpec * atten;
        }
    }

    // ---- Environment map reflections ----
    if (hasEnvMap > 0.5 && toggle_envMap > 0.5 && toggle_spec > 0.5) {
        float envLod = 8.0 - smoothness * 8.0;
        vec2 envUV = envMapUV(R);
        vec3 cubeColor = textureLod(envMap, envUV, envLod).rgb;

        cubeColor *= envMapScale * specStrength * dbg_envBoost;

        float envMask = s;
        if (hasEnvMask > 0.5) {
            envMask = texture(envMaskMap, vTexCoord).r;
        }

        float envF0 = mix(0.04, dbg_metalF0, s);
        float fresnel = fresnelSchlick(NdotV, envF0, fresnelPower);

        color += cubeColor * envMask * fresnel;
    }

    // ---- Emissive / glow ----
    vec3 glowPost = vec3(0.0);  // glow added after tonemap for neon punch
    if (hasEmit > 0.5) {
        vec3 emissive;
        if (hasGlowMap > 0.5) {
            // Glow map IS the emissive color — brighter pixels emit more.
            // glowColor tints it (white = pass through map color as-is).
            vec3 glowSample = texture(glowMap, vTexCoord).rgb;
            vec3 tint = (dot(glowColor, glowColor) > 0.001) ? glowColor : vec3(1.0);
            emissive = glowSample * tint * glowMult * 4.0;
            // Split: part goes pre-tonemap (blends with scene), part post (neon pop)
            color += emissive * 0.15;
            glowPost = emissive;
        } else {
            emissive = glowColor * glowMult;
            color += emissive;
        }
    }

    // ---- Tone map ----
    color = tonemapUncharted2(color, dbg_exposure);

    // Add glow map contribution after tonemap so it stays vivid/neon
    color += glowPost * 0.85;

    fragColor = vec4(color, finalAlpha * u_mesh_alpha);

    if (mrtEnabled > 0.5) {
        fragNormal = vec4(normalize(vNormalView) * 0.5 + 0.5, 1.0);
    } else {
        fragNormal = vec4(0.0);
    }
}

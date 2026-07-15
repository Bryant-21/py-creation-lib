#version 330
// FO76 default fragment shader — metallic-roughness model.
// Uses Cook-Torrance specular with metallic workflow.

#include "includes/common.glsl"
#include "includes/fresnel.glsl"
#include "includes/cubemap.glsl"
#include "includes/lighting.glsl"
#include "includes/normal_decode.glsl"
#include "includes/tonemapping.glsl"
#include "materials/metalrough.glsl"

// ---------- Textures ----------
uniform sampler2D diffuseMap;       // albedo
uniform sampler2D specMap;          // smooth/spec when present
uniform sampler2D normalMap;
uniform sampler2D envMap;
uniform sampler2D   envMaskMap;
uniform sampler2D   greyscaleMap;   // grad: grayscale-to-palette lookup
uniform sampler2D   glowMap;
uniform sampler2D   lightingMap;    // _l: R=gloss, G=AO, B=scattering
uniform sampler2D   reflectivityMap; // _r: reflectance at normal incidence
uniform sampler2D   brdfLUT;        // precomputed split-sum BRDF lookup
uniform float     hasSpecularMap;
uniform float     hasNormalMap;
uniform float     hasEnvMap;
uniform float     hasEnvMask;
uniform float     hasEmit;
uniform float     hasGlowMap;
uniform float     hasLightingMap;
uniform float     hasLightingEmissive;
uniform float     hasReflectivityMap;
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

// ---------- Debug tuning ----------
uniform float dbg_envBoost;
uniform float dbg_metalF0;
uniform float dbg_diffuseBleed;
uniform float dbg_exposure;
uniform float dbg_specBoost;
uniform float dbg_ambientBoost;

// ---------- Material uniforms ----------
uniform vec3  specColor;
uniform float specStrength;
uniform float specGlossiness;       // roughness for FO76 (inverted sense)
uniform float fresnelPower;
uniform vec3  glowColor;
uniform float glowMult;
uniform float subsurfaceEnabled;
uniform vec3  subsurfaceColor;
uniform float subsurfaceScale;

// ---------- Lighting ----------
uniform vec3 lightDir0;
uniform vec3 lightCol0;
uniform vec3 lightDir1;
uniform vec3 lightCol1;
uniform vec3 ambientCol;

uniform float mirrorLightEnabled;
uniform vec3  lightDir2;
uniform vec3  lightCol2;

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
    vec4 diffTex = texture(diffuseMap, vTexCoord);

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

    vec3 albedo = diffTex.rgb;

    if (toggle_vertexColor > 0.5) {
        albedo *= vVertexColor.rgb;
    }

    if (greyscaleColor > 0.5) {
        float diffuseIntensity = dot(diffTex.rgb, vec3(0.299, 0.587, 0.114));
        float vertexIntensity = dot(vVertexColor.rgb, vec3(0.299, 0.587, 0.114));
        vec2 palUV = vec2(diffuseIntensity, paletteScale * vertexIntensity);
        albedo = textureLod(greyscaleMap, palUV, 0.0).rgb;
    }

    if (toggle_diffuse < 0.5) {
        albedo = vec3(1.0);
    }

    if (toggle_lighting < 0.5) {
        fragColor = vec4(albedo, finalAlpha);
        fragNormal = vec4(normalize(vNormalView) * 0.5 + 0.5, 1.0);
        return;
    }

    // FO76: normals have blue channel
    vec3 N = decodeNormal(normalMap, vTexCoord, hasNormalMap, toggle_normal,
                          normalScale,
                          vNormalWorld, vTangentWorld, vBitangentWorld);

    vec3 V = normalize(cameraPos - vPosWorld);
    vec3 R = reflect(-V, N);

    float NdotV = max(dot(N, V), FLT_EPSILON);

    // BGSM stores Smoothness (high=shiny), invert to roughness.
    float roughness = clamp(1.0 - specGlossiness, 0.04, 1.0);
    float metallic = 0.0;

    if (hasSpecularMap > 0.5 && toggle_spec > 0.5) {
        vec4 specTex = texture(specMap, vTexCoord);
        roughness = specTex.g;       // green = roughness
        metallic = specTex.r;        // red = metallic
    }

    vec4 lightingSample = vec4(0.0, 1.0, 0.0, 0.0);
    vec3 lightingPacked = lightingSample.rgb;
    if (hasLightingMap > 0.5) {
        lightingSample = texture(lightingMap, vTexCoord);
        lightingPacked = lightingSample.rgb;
        if (toggle_spec > 0.5) {
            roughness = clamp(1.0 - lightingPacked.r, 0.04, 1.0);
        }
    }
    float ambientOcclusion = clamp(lightingPacked.g, 0.0, 1.0);
    float scattering = (subsurfaceEnabled > 0.5)
        ? clamp(lightingPacked.b * subsurfaceScale, 0.0, 1.0)
        : 0.0;

    vec3 dielectricF0 = vec3(0.04);
    if (hasReflectivityMap > 0.5 && toggle_spec > 0.5) {
        vec3 reflectance = texture(reflectivityMap, vTexCoord).rgb;
        dielectricF0 = clamp(reflectance, vec3(0.0), vec3(1.0));
    }

    // F0: dielectric reflectance at normal incidence, metal=albedo.
    vec3 F0 = mix(dielectricF0, albedo, metallic);

    // Key light
    vec3 color = vec3(0.0);
    {
        float shadow = calcShadow(vPosWorld, shadowMap, lightSpaceMatrix, shadowEnabled);
        color += PBRDirect(N, V, normalize(lightDir0), NdotV,
                           albedo, metallic, roughness, F0,
                           lightCol0) * shadow * dbg_specBoost;
    }

    // Ambient
    {
        vec3 kS = fresnelSchlickRoughness(NdotV, F0, roughness);
        vec3 kD = (vec3(1.0) - kS) * (1.0 - metallic);
        color += kD * albedo * ambientCol * dbg_ambientBoost * ambientOcclusion;
    }

    // Fill light
    color += PBRDirect(N, V, normalize(lightDir1), NdotV,
                       albedo, metallic, roughness, F0,
                       lightCol1);

    // Mirror light
    if (mirrorLightEnabled > 0.5) {
        color += PBRDirect(N, V, normalize(lightDir2), NdotV,
                           albedo, metallic, roughness, F0,
                           lightCol2);
    }

    // Point lights
    for (int i = 0; i < numPointLights; i++) {
        vec3 lightVec = pointLightPos[i] - vPosWorld;
        float dist = length(lightVec);
        if (dist > pointLightRadius[i])
            continue;

        float atten = 1.0 / max(
            pointLightConstAtten[i]
            + pointLightLinearAtten[i] * dist
            + pointLightQuadAtten[i] * dist * dist, 1.0);
        float radFade = 1.0 - smoothstep(pointLightRadius[i] * 0.8, pointLightRadius[i], dist);
        atten *= radFade;

        color += PBRDirect(N, V, lightVec / dist, NdotV,
                           albedo, metallic, roughness, F0,
                           pointLightCol[i]) * atten;
    }

    if (scattering > 0.001) {
        float scatter0 = pow(max(dot(N, -normalize(lightDir0)), 0.0), 2.0);
        float scatter1 = pow(max(dot(N, -normalize(lightDir1)), 0.0), 2.0);
        color += albedo * subsurfaceColor * scattering
            * (lightCol0 * scatter0 + lightCol1 * scatter1) * 0.25;
    }

    // Environment reflections with split-sum BRDF LUT
    if (hasEnvMap > 0.5 && toggle_envMap > 0.5 && toggle_spec > 0.5) {
        vec3 kS = fresnelSchlickRoughness(NdotV, F0, roughness);

        float envLod = roughness * 8.0;
        vec2 envUV = envMapUV(R);
        vec3 prefilteredColor = textureLod(envMap, envUV, envLod).rgb;

        float envMask = 1.0;
        if (hasEnvMask > 0.5) {
            envMask = texture(envMaskMap, vTexCoord).r;
        }

        // Split-sum IBL with BRDF LUT
        vec2 brdf = texture(brdfLUT, vec2(NdotV, roughness)).rg;
        vec3 specularIBL = prefilteredColor * (kS * brdf.x + brdf.y);
        color += specularIBL * envMask * envMapScale * dbg_envBoost * ambientOcclusion;
    }

    // Emissive
    vec3 glowPost = vec3(0.0);
    if (hasEmit > 0.5) {
        vec3 emissive;
        if (hasGlowMap > 0.5) {
            vec3 glowSample = texture(glowMap, vTexCoord).rgb;
            vec3 tint = (dot(glowColor, glowColor) > 0.001) ? glowColor : vec3(1.0);
            emissive = glowSample * tint * glowMult * 4.0;
            color += emissive * 0.15;
            glowPost = emissive;
        } else if (hasLightingMap > 0.5 && hasLightingEmissive > 0.5) {
            vec3 tint = (dot(glowColor, glowColor) > 0.001) ? glowColor : lightingPacked;
            emissive = lightingSample.a * tint * max(glowMult, 1.0) * 4.0;
            color += emissive * 0.15;
            glowPost = emissive;
        } else {
            emissive = glowColor * glowMult;
            color += emissive;
        }
    }

    color = tonemapUncharted2(color, dbg_exposure);
    color += glowPost * 0.85;

    fragColor = vec4(color, finalAlpha);

    if (mrtEnabled > 0.5) {
        fragNormal = vec4(normalize(vNormalView) * 0.5 + 0.5, 1.0);
    } else {
        fragNormal = vec4(0.0);
    }
}

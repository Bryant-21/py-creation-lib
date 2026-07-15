#version 330
// Legacy default fragment shader for Oblivion / FO3 / FNV.
// Uses the shared viewport vertex contract and renderer material bindings.

#include "includes/common.glsl"
#include "includes/fresnel.glsl"
#include "includes/lighting.glsl"
#include "includes/normal_decode.glsl"

uniform sampler2D diffuseMap;
uniform sampler2D normalMap;
uniform sampler2D specMap;
uniform sampler2D envMap;
uniform sampler2D envMaskMap;
uniform sampler2D glowMap;

uniform float hasSpecularMap;
uniform float hasNormalMap;
uniform float hasEnvMap;
uniform float hasEnvMask;
uniform float hasEmit;
uniform float hasGlowMap;
uniform float envMapScale;
uniform float normalScale;

uniform float toggle_diffuse;
uniform float toggle_normal;
uniform float toggle_spec;
uniform float toggle_lighting;
uniform float toggle_vertexColor;
uniform float toggle_envMap;

uniform vec3 specColor;
uniform float specStrength;
uniform float specGlossiness;
uniform float fresnelPower;
uniform vec3 glowColor;
uniform float glowMult;

uniform vec3 lightDir0;
uniform vec3 lightCol0;
uniform vec3 lightDir1;
uniform vec3 lightCol1;
uniform vec3 ambientCol;

uniform float mirrorLightEnabled;
uniform vec3 lightDir2;
uniform vec3 lightCol2;

uniform int numPointLights;
uniform vec3 pointLightPos[4];
uniform vec3 pointLightCol[4];
uniform float pointLightRadius[4];
uniform float pointLightConstAtten[4];
uniform float pointLightLinearAtten[4];
uniform float pointLightQuadAtten[4];

uniform int alphaFlags;
uniform float alphaThreshold;

uniform sampler2D shadowMap;
uniform mat4 lightSpaceMatrix;
uniform float shadowEnabled;
uniform float mrtEnabled;

uniform vec3 cameraPos;

in vec2 vTexCoord;
in vec3 vNormalWorld;
in vec3 vPosWorld;
in vec3 vTangentWorld;
in vec3 vBitangentWorld;
in vec4 vVertexColor;
in vec3 vNormalView;

layout(location = 0) out vec4 fragColor;
layout(location = 1) out vec4 fragNormal;

vec3 accumulate_light(
    vec3 normal,
    vec3 view_dir,
    vec3 light_dir,
    vec3 light_color,
    vec3 albedo,
    float spec_mask,
    float spec_power
) {
    vec3 light = normalize(light_dir);
    float ndl = max(dot(normal, light), 0.0);
    vec3 diffuse = albedo * light_color * ndl;

    if (toggle_spec < 0.5 || ndl <= 0.0 || spec_mask <= 0.0) {
        return diffuse;
    }

    vec3 half_vec = normalize(light + view_dir);
    float ndh = max(dot(normal, half_vec), 0.0);
    float vdh = max(dot(view_dir, half_vec), FLT_EPSILON);
    float fresnel = fresnelSchlick(vdh, 0.2, fresnelPower);
    vec3 specular = specColor * light_color * spec_mask * fresnel * pow(ndh, spec_power);
    return diffuse + specular;
}

void main() {
    vec4 diff_tex = texture(diffuseMap, vTexCoord);
    float combined_alpha = vVertexColor.a * diff_tex.a;

    if (alphaFlags > 0) {
        int test_func = alphaFlags & 7;
        int mask = (combined_alpha < alphaThreshold) ? 0x2B2B
                : (combined_alpha > alphaThreshold) ? 0x7171
                : 0x4D4D;
        if ((mask & (1 << test_func)) == 0) {
            discard;
        }
    }

    float final_alpha = ((alphaFlags & 8) != 0) ? combined_alpha : 1.0;

    vec3 albedo = diff_tex.rgb;
    if (toggle_vertexColor > 0.5) {
        albedo *= vVertexColor.rgb;
    }
    if (toggle_diffuse < 0.5) {
        albedo = vec3(1.0);
    }

    vec3 normal = decodeNormal(
        normalMap,
        vTexCoord,
        hasNormalMap,
        toggle_normal,
        normalScale,
        vNormalWorld,
        vTangentWorld,
        vBitangentWorld
    );

    fragNormal = (mrtEnabled > 0.5)
        ? vec4(normalize(vNormalView) * 0.5 + 0.5, 1.0)
        : vec4(0.0);

    if (toggle_lighting < 0.5) {
        fragColor = vec4(albedo, final_alpha);
        return;
    }

    vec4 spec_tex = (hasSpecularMap > 0.5 && toggle_spec > 0.5)
        ? texture(specMap, vTexCoord)
        : vec4(1.0);
    float spec_mask = specStrength * spec_tex.r;
    float gloss = clamp(specGlossiness * spec_tex.g, 0.0, 1.0);
    float spec_power = mix(4.0, 128.0, gloss);

    vec3 view_dir = normalize(cameraPos - vPosWorld);
    float shadow = calcShadow(vPosWorld, shadowMap, lightSpaceMatrix, shadowEnabled);

    vec3 color = albedo * ambientCol;
    color += accumulate_light(normal, view_dir, lightDir0, lightCol0 * shadow, albedo, spec_mask, spec_power);
    color += accumulate_light(normal, view_dir, lightDir1, lightCol1, albedo, spec_mask, spec_power);

    if (mirrorLightEnabled > 0.5) {
        color += accumulate_light(normal, view_dir, lightDir2, lightCol2, albedo, spec_mask, spec_power);
    }

    for (int i = 0; i < numPointLights; i++) {
        vec3 light_vec = pointLightPos[i] - vPosWorld;
        float dist = length(light_vec);
        if (dist <= 0.0 || dist > pointLightRadius[i]) {
            continue;
        }

        float attenuation = 1.0 / (
            pointLightConstAtten[i]
            + pointLightLinearAtten[i] * dist
            + pointLightQuadAtten[i] * dist * dist
        );
        color += accumulate_light(
            normal,
            view_dir,
            light_vec / dist,
            pointLightCol[i] * attenuation,
            albedo,
            spec_mask,
            spec_power
        );
    }

    if (hasEnvMap > 0.5 && toggle_envMap > 0.5) {
        vec3 reflected = reflect(-view_dir, normal);
        vec2 env_uv = reflected.xy * 0.5 + 0.5;
        vec3 env_color = texture(envMap, env_uv).rgb;
        float env_mask = (hasEnvMask > 0.5) ? texture(envMaskMap, vTexCoord).r : 1.0;
        color += env_color * envMapScale * env_mask * max(spec_mask, 0.25);
    }

    if (hasGlowMap > 0.5) {
        color += texture(glowMap, vTexCoord).rgb * glowColor * glowMult;
    } else if (hasEmit > 0.5) {
        color += glowColor * glowMult;
    }

    fragColor = vec4(color, final_alpha);
}

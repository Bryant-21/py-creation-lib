#version 330
// Shared effect shader (BSEffectShaderProperty) — used by all games.
// Emissive-driven rendering with optional falloff, greyscale-to-palette,
// and cubemap environment reflections.

#include "includes/common.glsl"
#include "materials/effect_emissive.glsl"

// ---------- Textures ----------
uniform sampler2D BaseMap;
uniform sampler2D GreyscaleMap;
uniform sampler2D NormalMap;
uniform sampler2D CubeMap;
uniform sampler2D SpecularMap;

// ---------- Texture presence flags ----------
uniform float hasSourceTexture;
uniform float hasGreyscaleMap;
uniform float hasNormalMap;
uniform float hasCubeMap;
uniform float hasEnvMask;

// ---------- Effect properties ----------
uniform vec4  glowColor;
uniform float glowMult;
uniform vec4  falloffParams;
uniform float falloffDepth;
uniform float useFalloff;
uniform float hasRGBFalloff;
uniform float greyscaleColor;
uniform float greyscaleAlpha;
uniform float lightingInfluence;
uniform float envReflection;
uniform float doubleSided;

// ---------- Alpha ----------
uniform int   alphaFlags;
uniform float alphaThreshold;

// ---------- Lighting ----------
uniform vec3 lightDir0;
uniform vec3 lightCol0;
uniform vec3 ambientCol;

// ---------- Camera ----------
uniform vec3 cameraPos;

// ---------- Debug toggles ----------
uniform float toggle_diffuse;
uniform float toggle_normal;
uniform float toggle_spec;
uniform float toggle_lighting;
uniform float toggle_vertexColor;
uniform float toggle_envMap;

// ---------- Varyings ----------
in vec2 vTexCoord;
in vec3 vNormalWorld;
in vec3 vPosWorld;
in vec3 vTangentWorld;
in vec3 vBitangentWorld;
in vec4 vVertexColor;

layout(location = 0) out vec4 fragColor;
layout(location = 1) out vec4 fragNormal;

void main()
{
    vec4 baseMap = vec4(1.0);
    if (hasSourceTexture > 0.5) {
        baseMap = texture(BaseMap, vTexCoord);
        if (toggle_diffuse < 0.5) {
            // Keep alpha for transparency but suppress color
            baseMap.rgb = vec3(1.0);
        }
    }

    // ---- Normal ----
    vec3 normal;
    if (hasNormalMap > 0.5 && toggle_normal > 0.5) {
        vec3 nmap = texture(NormalMap, vTexCoord).rgb * 2.0 - 1.0;
        nmap.b = sqrt(max(1.0 - dot(nmap.rg, nmap.rg), 0.0));
        mat3 TBN = mat3(
            normalize(vBitangentWorld),
            normalize(vTangentWorld),
            normalize(vNormalWorld)
        );
        normal = normalize(TBN * nmap);
    } else {
        normal = normalize(vNormalWorld);
    }

    if (doubleSided > 0.5 && !gl_FrontFacing) {
        normal *= -1.0;
    }

    vec3 V = normalize(cameraPos - vPosWorld);
    vec3 R = reflect(-V, normal);

    // ---- Greyscale alpha ----
    if (greyscaleAlpha > 0.5) {
        baseMap.a = 1.0;
    }

    // ---- Base color (diffuse tint) ----
    vec4 baseColor = glowColor;
    // glowMult applied after color composition to preserve tint ratios

    // ---- Falloff ----
    float falloff = 1.0;
    if (useFalloff > 0.5 || hasRGBFalloff > 0.5) {
        float startAngle = falloffParams.x;
        float stopAngle  = falloffParams.y;
        float startOpacity = falloffParams.z;
        float stopOpacity  = falloffParams.w;

        float NdotV = abs(dot(normal, V));
        falloff = smoothstep(startAngle, stopAngle, NdotV);
        falloff = mix(max(startOpacity, 0.0), min(stopOpacity, 1.0), falloff);

        if (useFalloff > 0.5) {
            baseMap.a *= falloff;
        }
        if (hasRGBFalloff > 0.5) {
            baseMap.rgb *= falloff;
        }
    }

    // ---- Alpha multiplier ----
    float alphaMult = baseColor.a * baseColor.a;

    // ---- Compute color ----
    vec4 color;
    vec3 vc = (toggle_vertexColor > 0.5) ? vVertexColor.rgb : vec3(1.0);
    color.rgb = baseMap.rgb * vc * baseColor.rgb;
    // Apply Base Color Scale as brightness, then normalize to prevent
    // white-out when glowMult pushes channels above 1.0.
    if (greyscaleColor < 0.5) {
        color.rgb *= glowMult;
    }
    float peak = max(max(color.r, color.g), max(color.b, 1.0));
    color.rgb /= peak;
    color.a = alphaMult * vVertexColor.a * baseMap.a;

    // ---- Greyscale-to-palette color lookup ----
    if (greyscaleColor > 0.5 && hasGreyscaleMap > 0.5) {
        vec4 luG = colorLookup(GreyscaleMap, baseMap.g, baseColor.r * vVertexColor.r * falloff);
        color.rgb = luG.rgb;
    }

    // ---- Greyscale-to-palette alpha lookup ----
    if (greyscaleAlpha > 0.5 && hasGreyscaleMap > 0.5) {
        float origAlpha = texture(BaseMap, vTexCoord).a;
        vec4 luA = colorLookup(GreyscaleMap, origAlpha, color.a);
        color.a = luA.a;
    }

    // ---- Alpha test ----
    if (alphaFlags > 0) {
        float combinedAlpha = color.a;
        int testFunc = alphaFlags & 7;
        int m = (combinedAlpha < alphaThreshold) ? 0x2B2B
              : (combinedAlpha > alphaThreshold) ? 0x7171
              : 0x4D4D;
        if ((m & (1 << testFunc)) == 0)
            discard;
    }

    // ---- Lighting influence ----
    if (lightingInfluence > FLT_EPSILON && toggle_lighting > 0.5) {
        vec3 L = normalize(lightDir0);
        float NdotL = max(dot(normal, L), 0.0);
        vec3 D = lightCol0 * NdotL;
        color.rgb = mix(color.rgb, color.rgb * D, lightingInfluence);
    }

    // ---- Specular mask ----
    float g = 1.0;
    float s = 1.0;
    if (hasEnvMask > 0.5) {
        vec4 specMap = texture(SpecularMap, vTexCoord);
        g = specMap.r;
        s = specMap.g;
    }

    // ---- Environment map ----
    if (hasCubeMap > 0.5 && toggle_envMap > 0.5 && toggle_spec > 0.5) {
        float theta = atan(R.x, R.z);
        float phi = asin(clamp(R.y, -1.0, 1.0));
        vec2 envUV = vec2(theta / (2.0 * 3.14159265) + 0.5, 0.5 - phi / 3.14159265);
        vec3 cubeColor = texture(CubeMap, envUV).rgb;
        cubeColor *= envReflection * s;

        if (lightingInfluence > FLT_EPSILON && toggle_lighting > 0.5) {
            vec3 L = normalize(lightDir0);
            float NdotL = max(dot(normal, L), 0.0);
            vec3 D = lightCol0 * NdotL;
            cubeColor = mix(cubeColor, cubeColor * D, lightingInfluence);
        }

        color.rgb += cubeColor * falloff;
    }

    // ---- Final output ----
    float finalAlpha = color.a;
    if ((alphaFlags & 8) == 0) {
        finalAlpha = 1.0;
    }

    fragColor = vec4(color.rgb, finalAlpha);
    fragNormal = vec4(0.5, 0.5, 1.0, 1.0);
}

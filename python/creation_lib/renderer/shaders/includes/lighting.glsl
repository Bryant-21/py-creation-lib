// lighting.glsl — Point light attenuation + shadow sampling.
#ifndef LIGHTING_GLSL
#define LIGHTING_GLSL

// Shadow map sampling with PCF 3x3
float calcShadow(vec3 worldPos, sampler2D shadowMap, mat4 lightSpaceMatrix, float shadowEnabled)
{
    if (shadowEnabled < 0.5)
        return 1.0;

    vec4 lsPos = lightSpaceMatrix * vec4(worldPos, 1.0);
    vec3 projCoords = lsPos.xyz / lsPos.w;
    projCoords = projCoords * 0.5 + 0.5;

    if (projCoords.x < 0.0 || projCoords.x > 1.0 ||
        projCoords.y < 0.0 || projCoords.y > 1.0 ||
        projCoords.z > 1.0)
        return 1.0;

    float currentDepth = projCoords.z;
    float bias = 0.002;

    float shadow = 0.0;
    vec2 texelSize = 1.0 / vec2(textureSize(shadowMap, 0));
    for (int x = -1; x <= 1; x++) {
        for (int y = -1; y <= 1; y++) {
            float pcfDepth = texture(shadowMap, projCoords.xy + vec2(x, y) * texelSize).r;
            shadow += currentDepth - bias > pcfDepth ? 0.0 : 1.0;
        }
    }
    shadow /= 9.0;

    return mix(0.3, 1.0, shadow);
}

#endif // LIGHTING_GLSL

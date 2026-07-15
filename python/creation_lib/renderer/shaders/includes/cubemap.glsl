// cubemap.glsl — Cubemap / latlong environment map sampling.
#ifndef CUBEMAP_GLSL
#define CUBEMAP_GLSL

// Convert reflection vector to latlong (equirectangular) UV coordinates.
vec2 envMapUV(vec3 R)
{
    float theta = atan(R.x, R.z);
    float phi = asin(clamp(R.y, -1.0, 1.0));
    return vec2(theta / (2.0 * M_PI) + 0.5,
                0.5 - phi / M_PI);
}

#endif // CUBEMAP_GLSL

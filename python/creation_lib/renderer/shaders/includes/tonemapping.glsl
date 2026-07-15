// tonemapping.glsl — Tonemapping functions.
#ifndef TONEMAPPING_GLSL
#define TONEMAPPING_GLSL

// Uncharted-2 filmic tonemapping (matches NifSkope fo4_default.frag).
// Input: linear-ish color (sRGB squared * exposure).
// Output: display-ready sRGB color.
vec3 tonemapUncharted2(vec3 color, float exposure)
{
    vec3 z = color * color * exposure;
    {
        const float a = 0.15, b = 0.50, c = 0.10, d = 0.20, e = 0.02, f = 0.30;
        z = (z * (a * z + b * c) + d * e) / (z * (a * z + b) + d * f) - e / f;
    }
    return sqrt(max(z / 0.933, vec3(0.0)));
}

#endif // TONEMAPPING_GLSL

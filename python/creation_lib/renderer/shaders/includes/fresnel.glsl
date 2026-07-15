// fresnel.glsl — Schlick Fresnel approximation.
#ifndef FRESNEL_GLSL
#define FRESNEL_GLSL

float fresnelSchlick(float VdotH, float F0, float fresnelPower)
{
    float base = 1.0 - VdotH;
    float e = pow(base, fresnelPower);
    return clamp(e + F0 * (1.0 - e), 0.0, 1.0);
}

#endif // FRESNEL_GLSL

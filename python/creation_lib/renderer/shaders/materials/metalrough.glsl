// metalrough.glsl — Standard PBR metallic-roughness BRDF.
// Based on Cook-Torrance with GGX distribution, Schlick-GGX geometry,
// and Schlick Fresnel approximation.
// Reference: https://learnopengl.com/PBR/Theory
#ifndef METALROUGH_GLSL
#define METALROUGH_GLSL

// GGX/Trowbridge-Reitz Normal Distribution Function
float DistributionGGX(float NdotH, float roughness)
{
    float a = roughness * roughness;
    float a2 = a * a;
    float NdotH2 = NdotH * NdotH;
    float denom = NdotH2 * (a2 - 1.0) + 1.0;
    denom = M_PI * denom * denom;
    return a2 / max(denom, FLT_EPSILON);
}

// Schlick-GGX Geometry function
float GeometrySchlickGGX(float NdotV, float roughness)
{
    float r = roughness + 1.0;
    float k = (r * r) / 8.0;
    return NdotV / (NdotV * (1.0 - k) + k);
}

// Smith's method for combined geometry obstruction
float GeometrySmith(float NdotV, float NdotL, float roughness)
{
    return GeometrySchlickGGX(NdotV, roughness) * GeometrySchlickGGX(NdotL, roughness);
}

// Schlick Fresnel approximation (vec3, standard power=5)
vec3 fresnelSchlickVec3(float cosTheta, vec3 F0)
{
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

// Schlick Fresnel with roughness (for env map — accounts for rough surfaces
// reflecting less at grazing angles than smooth surfaces)
vec3 fresnelSchlickRoughness(float cosTheta, vec3 F0, float roughness)
{
    return F0 + (max(vec3(1.0 - roughness), F0) - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

// Cook-Torrance specular BRDF for metallic-roughness workflow.
// Returns specular radiance contribution (already includes Fresnel).
// Also outputs kS (Fresnel) so caller can compute energy-conserving diffuse.
vec3 CookTorranceSpec(float NdotL, float NdotH, float NdotV, float VdotH,
                      vec3 F0, float roughness, float fresnelPower)
{
    float D = DistributionGGX(NdotH, roughness);
    float G = GeometrySmith(NdotV, NdotL, roughness);
    vec3  F = fresnelSchlickVec3(VdotH, F0);

    vec3 numerator = D * G * F;
    float denominator = 4.0 * NdotV * NdotL;
    return numerator / max(denominator, FLT_EPSILON);
}

// Full PBR direct lighting for one light.
// Returns combined diffuse + specular with energy conservation.
vec3 PBRDirect(vec3 N, vec3 V, vec3 L, float NdotV,
               vec3 albedo, float metallic, float roughness, vec3 F0,
               vec3 lightColor)
{
    vec3  H     = normalize(L + V);
    float NdotL = max(dot(N, L), 0.0);
    float NdotH = max(dot(N, H), FLT_EPSILON);
    float VdotH = max(dot(V, H), FLT_EPSILON);

    if (NdotL <= 0.0)
        return vec3(0.0);

    float D = DistributionGGX(NdotH, roughness);
    float G = GeometrySmith(NdotV, NdotL, roughness);
    vec3  F = fresnelSchlickVec3(VdotH, F0);

    // Specular
    vec3 specular = (D * G * F) / max(4.0 * NdotV * NdotL, FLT_EPSILON);

    // Energy-conserving diffuse: (1 - F) * (1 - metallic)
    vec3 kD = (vec3(1.0) - F) * (1.0 - metallic);
    vec3 diffuse = kD * albedo / M_PI;

    return (diffuse + specular) * lightColor * NdotL;
}

#endif // METALROUGH_GLSL

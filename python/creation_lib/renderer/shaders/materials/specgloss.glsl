// specgloss.glsl — FO4/Skyrim Specular-Glossiness material evaluation.
// Oren-Nayar diffuse + Torrance-Sparrow specular BRDF.
#ifndef SPECGLOSS_GLSL
#define SPECGLOSS_GLSL

// ---- Oren-Nayar diffuse (simplified) ----------------------------------------
float OrenNayar(vec3 L, vec3 V, vec3 N, float roughness, float NdotL)
{
    float NdotV = dot(N, V);
    float LdotV = dot(L, V);

    float rough2 = roughness * roughness;

    float A = 1.0 - 0.5 * (rough2 / (rough2 + 0.57));
    float B = 0.45 * (rough2 / (rough2 + 0.09));

    float a = min(NdotV, NdotL);
    float b = max(NdotV, NdotL);
    b = (sign(b) == 0.0) ? FLT_EPSILON : sign(b) * max(0.01, abs(b));
    float C = sqrt((1.0 - a * a) * (1.0 - b * b)) / b;

    float gamma = LdotV - NdotL * NdotV;
    float L1 = A + B * max(gamma, FLT_EPSILON) * C;

    return L1 * max(NdotL, FLT_EPSILON);
}

// ---- Full Oren-Nayar (matches NifSkope OrenNayarFull) -----------------------
float OrenNayarFull(float NdotL, float NdotV, float LdotV, float roughness)
{
    float NdotL0 = clamp(NdotL, FLT_EPSILON, 1.0);
    float angleVN = acos(clamp(abs(NdotV), FLT_EPSILON, 1.0));
    float angleLN = acos(NdotL0);

    float alpha = max(angleVN, angleLN);
    float beta = min(angleVN, angleLN);
    float gamma = (LdotV - NdotL * NdotV) / sqrt(max((1.0 - NdotL * NdotL) * (1.0 - NdotV * NdotV), 0.000025));

    float roughnessSquared = roughness * roughness;
    float roughnessSquared9 = (roughnessSquared / (roughnessSquared + 0.09));

    float C1 = 1.0 - 0.5 * (roughnessSquared / (roughnessSquared + 0.33));
    float C2 = 0.45 * roughnessSquared9;

    if (gamma >= 0.0) {
        C2 *= sin(alpha);
    } else {
        C2 *= (sin(alpha) - pow((2.0 * beta) / M_PI, 3.0));
    }

    float powValue = (4.0 * alpha * beta) / (M_PI * M_PI);
    float C3 = 0.125 * roughnessSquared9 * powValue * powValue;

    float asym = M_PI / 2.0;
    float lim1 = asym + 0.005;
    float lim2 = asym - 0.005;

    float ab2 = (alpha + beta) / 2.0;

    if (beta >= asym && beta < lim1)
        beta = lim1;
    else if (beta < asym && beta >= lim2)
        beta = lim2;

    if (ab2 >= asym && ab2 < lim1)
        ab2 = lim1;
    else if (ab2 < asym && ab2 >= lim2)
        ab2 = lim2;

    float A = gamma * C2 * tan(beta);
    float B = (1.0 - abs(gamma)) * C3 * tan(ab2);

    float L1 = NdotL0 * (C1 + A + B);
    float L2 = 0.17 * NdotL0 * (roughnessSquared / (roughnessSquared + 0.13)) * (1.0 - gamma * 2.0 * beta / M_PI * 2.0 * beta / M_PI);

    return L1 + L2;
}


// ---- Cook-Torrance geometric attenuation / NdotV ----------------------------
float VisibDiv(float NdotL, float NdotV, float VdotH, float NdotH)
{
    float denom = max(VdotH, FLT_EPSILON);
    float numL = min(NdotV, NdotL);
    float numR = 2.0 * NdotH;
    if (denom >= (numL * numR)) {
        numL = (numL == NdotV) ? 1.0 : (NdotL / NdotV);
        return (numL * numR) / denom;
    }
    return 1.0 / NdotV;
}


// ---- Torrance-Sparrow specular BRDF ----------------------------------------
vec3 TorranceSparrow(float NdotL, float NdotH, float NdotV, float VdotH,
                     vec3 color, float power, float F0, float fresnelPower)
{
    float D = ((power + 2.0) / (2.0 * M_PI)) * pow(NdotH, power);
    float G_NdotV = VisibDiv(NdotL, NdotV, VdotH, NdotH);
    float F = fresnelSchlick(VdotH, F0, fresnelPower);
    float spec = (F * G_NdotV * D) / 4.0;
    return color * spec * M_PI;
}

#endif // SPECGLOSS_GLSL

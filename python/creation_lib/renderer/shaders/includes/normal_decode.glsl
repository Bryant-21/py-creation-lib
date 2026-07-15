// normal_decode.glsl — TBN matrix construction and normal map unpacking.
// All games: reconstruct Z from RG channels in shader (matches NifSkope).
// normalScale: intensity multiplier from .mat floatParam (default 1.0).
#ifndef NORMAL_DECODE_GLSL
#define NORMAL_DECODE_GLSL

vec3 decodeNormal(sampler2D normalMap, vec2 texCoord, float hasNormalMap,
                  float toggleNormal, float normalScale,
                  vec3 vNormalWorld, vec3 vTangentWorld, vec3 vBitangentWorld)
{
    if (hasNormalMap > 0.5 && toggleNormal > 0.5) {
        vec3 nmap;
        nmap.rg = texture(normalMap, texCoord).rg * 2.0 - 1.0;
        nmap.rg *= normalScale;
        nmap.b = sqrt(max(1.0 - dot(nmap.rg, nmap.rg), 0.0));
        // TBN in [Bitangent, Tangent, Normal] order — matches NifSkope and
        // FO4's DirectX tangent-space convention without needing a Y-flip.
        mat3 TBN = mat3(
            normalize(vBitangentWorld),
            normalize(vTangentWorld),
            normalize(vNormalWorld)
        );
        return normalize(TBN * nmap);
    }
    return normalize(vNormalWorld);
}

#endif // NORMAL_DECODE_GLSL

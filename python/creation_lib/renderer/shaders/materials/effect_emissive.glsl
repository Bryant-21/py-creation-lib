// effect_emissive.glsl — Shared effect shader material evaluation.
// Used by BSEffectShaderProperty across all games.
#ifndef EFFECT_EMISSIVE_GLSL
#define EFFECT_EMISSIVE_GLSL

vec4 colorLookup(sampler2D greyscaleMap, float x, float y) {
    return textureLod(greyscaleMap, vec2(clamp(x, 0.0, 1.0), clamp(y, 0.0, 1.0)), 0.0);
}

#endif // EFFECT_EMISSIVE_GLSL

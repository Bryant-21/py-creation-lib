// starfield_layered.glsl — Starfield multi-layer material blending.
// Supports up to 4 GPU-rendered layers with 3 blenders.
// Blend modes: linear, additive, position_contrast, multiply, screen.
#ifndef STARFIELD_LAYERED_GLSL
#define STARFIELD_LAYERED_GLSL

// Maximum layers rendered on GPU (texture unit budget: 14 of 16 used)
#define MAX_SF_LAYERS 3
#define MAX_SF_BLENDERS 2

// Blend mode constants (matched to CPU-side encoding)
#define BLEND_LINEAR           0
#define BLEND_ADDITIVE         1
#define BLEND_POSITION_CONTRAST 2
#define BLEND_MULTIPLY         3
#define BLEND_SCREEN           4

// Per-layer material: albedo, roughness/metallic, tint, opacity
// Normal maps are NOT per-layer (texture unit budget) — single normalMap used.
struct SFLayer {
    vec3  albedo;
    float roughness;
    float metallic;
    vec3  tint;
    float opacity;
};

// Blending helpers --------------------------------------------------------

vec3 blendLinear(vec3 base, vec3 top, float alpha) {
    return mix(base, top, alpha);
}

vec3 blendAdditive(vec3 base, vec3 top, float alpha) {
    return base + top * alpha;
}

vec3 blendMultiply(vec3 base, vec3 top, float alpha) {
    return mix(base, base * top, alpha);
}

vec3 blendScreen(vec3 base, vec3 top, float alpha) {
    vec3 screened = 1.0 - (1.0 - base) * (1.0 - top);
    return mix(base, screened, alpha);
}

vec3 blendPositionContrast(vec3 base, vec3 top, float alpha, float threshold, float factor) {
    // Height-based blending: sharpen the transition around the threshold
    float adjusted = clamp((alpha - threshold) * factor + 0.5, 0.0, 1.0);
    return mix(base, top, adjusted);
}

// Blend a single float value (roughness, metallic) with the same mode
float blendScalar(float base, float top, float alpha, int mode, float threshold, float factor) {
    if (mode == BLEND_ADDITIVE) return base + top * alpha;
    if (mode == BLEND_MULTIPLY) return mix(base, base * top, alpha);
    if (mode == BLEND_POSITION_CONTRAST) {
        float adjusted = clamp((alpha - threshold) * factor + 0.5, 0.0, 1.0);
        return mix(base, top, adjusted);
    }
    // Linear and screen (screen doesn't apply well to scalars, use linear)
    return mix(base, top, alpha);
}

// Main blending: evaluate one blend step
SFLayer blendLayers(SFLayer base, SFLayer top, float blendAlpha,
                     int blendMode, float threshold, float factor) {
    SFLayer result;
    float alpha = blendAlpha * top.opacity;

    if (blendMode == BLEND_POSITION_CONTRAST) {
        result.albedo = blendPositionContrast(base.albedo, top.albedo * top.tint,
                                               alpha, threshold, factor);
    } else if (blendMode == BLEND_ADDITIVE) {
        result.albedo = blendAdditive(base.albedo, top.albedo * top.tint, alpha);
    } else if (blendMode == BLEND_MULTIPLY) {
        result.albedo = blendMultiply(base.albedo, top.albedo * top.tint, alpha);
    } else if (blendMode == BLEND_SCREEN) {
        result.albedo = blendScreen(base.albedo, top.albedo * top.tint, alpha);
    } else {
        // Linear (default)
        result.albedo = blendLinear(base.albedo, top.albedo * top.tint, alpha);
    }

    result.roughness = blendScalar(base.roughness, top.roughness, alpha, blendMode, threshold, factor);
    result.metallic = blendScalar(base.metallic, top.metallic, alpha, blendMode, threshold, factor);
    result.tint = vec3(1.0);
    result.opacity = 1.0;

    return result;
}

#endif // STARFIELD_LAYERED_GLSL

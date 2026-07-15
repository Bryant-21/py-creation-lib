#version 330

// --- Inputs ---
in vec3 v_world_pos;
in vec3 v_normal;
in vec2 v_texcoord;
in vec3 v_tangent;
in vec4 v_light_space_pos;

out vec4 fragColor;

// --- Material textures ---
uniform sampler2D albedoMap;      // unit 0
uniform sampler2D normalMap;      // unit 1
uniform sampler2D mrMap;          // unit 2 (G=roughness, B=metallic)
uniform sampler2D aoMap;          // unit 3
uniform sampler2D emissiveMap;    // unit 4

// --- Material scalars (fallbacks when no texture) ---
uniform vec4  u_base_color;
uniform float u_metallic;
uniform float u_roughness;
uniform vec3  u_emissive_factor;

// --- Texture presence flags ---
uniform float has_albedo;
uniform float has_normal;
uniform float has_mr;
uniform float has_ao;
uniform float has_emissive;

// --- Lighting ---
uniform vec3 u_camera_pos;
uniform vec3 u_key_dir;
uniform vec3 u_key_color;
uniform vec3 u_fill_dir;
uniform vec3 u_fill_color;
uniform vec3 u_ambient_color;

// --- Shadow ---
uniform sampler2D shadowMap;      // unit 5
uniform float shadowEnabled;

const float PI = 3.14159265359;

// --- PBR functions ---
float DistributionGGX(vec3 N, vec3 H, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float NdotH = max(dot(N, H), 0.0);
    float NdotH2 = NdotH * NdotH;
    float denom = NdotH2 * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

float GeometrySchlickGGX(float NdotV, float roughness) {
    float r = roughness + 1.0;
    float k = (r * r) / 8.0;
    return NdotV / (NdotV * (1.0 - k) + k);
}

float GeometrySmith(vec3 N, vec3 V, vec3 L, float roughness) {
    return GeometrySchlickGGX(max(dot(N, V), 0.0), roughness)
         * GeometrySchlickGGX(max(dot(N, L), 0.0), roughness);
}

vec3 FresnelSchlick(float cosTheta, vec3 F0) {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

float ShadowCalculation(vec4 fragPosLightSpace) {
    vec3 projCoords = fragPosLightSpace.xyz / fragPosLightSpace.w;
    projCoords = projCoords * 0.5 + 0.5;
    if (projCoords.z > 1.0) return 0.0;

    float currentDepth = projCoords.z;
    float bias = 0.005;

    // PCF 3x3
    float shadow = 0.0;
    vec2 texelSize = 1.0 / textureSize(shadowMap, 0);
    for (int x = -1; x <= 1; ++x) {
        for (int y = -1; y <= 1; ++y) {
            float pcfDepth = texture(shadowMap, projCoords.xy + vec2(x, y) * texelSize).r;
            shadow += currentDepth - bias > pcfDepth ? 1.0 : 0.0;
        }
    }
    return shadow / 9.0;
}

vec3 CalcLight(vec3 lightDir, vec3 lightColor, vec3 N, vec3 V,
               vec3 albedo, float metallic, float roughness, vec3 F0) {
    vec3 L = normalize(-lightDir);
    vec3 H = normalize(V + L);

    float NDF = DistributionGGX(N, H, roughness);
    float G = GeometrySmith(N, V, L, roughness);
    vec3 F = FresnelSchlick(max(dot(H, V), 0.0), F0);

    vec3 numerator = NDF * G * F;
    float denominator = 4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.0001;
    vec3 specular = numerator / denominator;

    vec3 kD = (1.0 - F) * (1.0 - metallic);
    float NdotL = max(dot(N, L), 0.0);

    return (kD * albedo / PI + specular) * lightColor * NdotL;
}

void main() {
    // Sample textures
    vec4 albedo4 = has_albedo > 0.5 ? texture(albedoMap, v_texcoord) : u_base_color;
    vec3 albedo = albedo4.rgb;
    float alpha = albedo4.a;

    float metallic = u_metallic;
    float roughness = u_roughness;
    if (has_mr > 0.5) {
        vec3 mr = texture(mrMap, v_texcoord).rgb;
        metallic = mr.b;
        roughness = mr.g;
    }
    roughness = clamp(roughness, 0.04, 1.0);

    float ao = has_ao > 0.5 ? texture(aoMap, v_texcoord).r : 1.0;

    // Normal mapping
    vec3 N = normalize(v_normal);
    if (has_normal > 0.5) {
        vec3 T = normalize(v_tangent - dot(v_tangent, N) * N);
        vec3 B = cross(N, T);
        mat3 TBN = mat3(T, B, N);
        vec3 tangentNormal = texture(normalMap, v_texcoord).rgb * 2.0 - 1.0;
        N = normalize(TBN * tangentNormal);
    }

    vec3 V = normalize(u_camera_pos - v_world_pos);
    vec3 F0 = mix(vec3(0.04), albedo, metallic);

    // Lighting — compute key and fill separately for shadow application
    vec3 keyContrib = CalcLight(u_key_dir, u_key_color, N, V, albedo, metallic, roughness, F0);
    vec3 fillContrib = CalcLight(u_fill_dir, u_fill_color, N, V, albedo, metallic, roughness, F0);

    // Shadow (key light only)
    float shadow = 0.0;
    if (shadowEnabled > 0.5) {
        shadow = ShadowCalculation(v_light_space_pos);
    }

    vec3 Lo = keyContrib * (1.0 - shadow) + fillContrib;

    // Ambient
    vec3 ambient = u_ambient_color * albedo * ao;

    // Emissive
    vec3 emissive = u_emissive_factor;
    if (has_emissive > 0.5) {
        emissive *= texture(emissiveMap, v_texcoord).rgb;
    }

    vec3 color = ambient + Lo + emissive;

    // Tone mapping (Reinhard) + gamma
    color = color / (color + vec3(1.0));
    color = pow(color, vec3(1.0 / 2.2));

    fragColor = vec4(color, alpha);
}

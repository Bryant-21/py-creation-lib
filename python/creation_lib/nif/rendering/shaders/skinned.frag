#version 330 core

in vec3 v_normal;
in vec2 v_uv;
in vec3 v_world_pos;
in float v_bone_weight;
in vec3 v_segment_color;
in vec4 v_vertex_color;
in float v_mask;

uniform vec3 u_light_dir;
uniform vec3 u_light_color;
uniform vec3 u_ambient;
uniform sampler2D u_diffuse_tex;
uniform bool u_has_texture;

// Weight visualization
uniform bool u_weight_mode;

// Segment visualization
uniform bool u_segment_mode;
uniform bool u_use_submesh_color;  // When true, use u_submesh_color instead of per-vertex
uniform vec3 u_submesh_color;      // Flat segment color for submesh rendering

// Vertex color visualization
uniform bool u_vertex_color_mode;

// Alpha for transparency (reference mesh overlay)
uniform float u_alpha;

// Mask visualization
uniform bool u_show_mask;

out vec4 frag_color;

vec3 weight_to_heatmap(float w) {
    // Classic heatmap: blue(0) -> cyan(0.25) -> green(0.5) -> yellow(0.75) -> red(1.0)
    vec3 color;
    if (w <= 0.0) {
        color = vec3(0.15, 0.15, 0.15);  // Dark gray for unweighted
    } else if (w < 0.25) {
        float t = w / 0.25;
        color = mix(vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), t);
    } else if (w < 0.5) {
        float t = (w - 0.25) / 0.25;
        color = mix(vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0), t);
    } else if (w < 0.75) {
        float t = (w - 0.5) / 0.25;
        color = mix(vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0), t);
    } else {
        float t = (w - 0.75) / 0.25;
        color = mix(vec3(1.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), t);
    }
    return color;
}

void main() {
    vec3 N = normalize(v_normal);
    vec3 L = normalize(u_light_dir);
    float diff = max(dot(N, L), 0.0);

    if (u_vertex_color_mode) {
        // Vertex color mode — show per-vertex RGBA from NIF
        float lighting = 0.5 + 0.5 * diff;
        vec3 color = v_vertex_color.rgb * lighting;
        frag_color = vec4(color, v_vertex_color.a * u_alpha);
    } else if (u_segment_mode) {
        // Segment color mode — show dismemberment sections
        float lighting = 0.5 + 0.5 * diff;
        vec3 color;
        if (u_use_submesh_color) {
            // Submesh-based: flat color uniform per draw call (hard boundaries)
            color = u_submesh_color;
        } else {
            // Per-vertex fallback (used for all_weights mode)
            color = v_segment_color;
        }
        if (length(color) < 0.01) {
            // Unassigned segments: dark red warning color
            color = vec3(0.4, 0.1, 0.1);
        }
        frag_color = vec4(color * lighting, u_alpha);
    } else if (u_weight_mode) {
        // Weight heatmap mode -- apply subtle lighting for depth perception
        vec3 heatmap = weight_to_heatmap(v_bone_weight);
        float lighting = 0.6 + 0.4 * diff;  // Subtle directional shading
        frag_color = vec4(heatmap * lighting, u_alpha);
    } else {
        // Normal rendering mode
        vec3 base_color;
        if (u_has_texture) {
            base_color = texture(u_diffuse_tex, v_uv).rgb;
        } else {
            base_color = vec3(0.7, 0.7, 0.7);
        }
        vec3 color = base_color * (u_ambient + u_light_color * diff);
        frag_color = vec4(color, u_alpha);
    }

    // Darken masked vertices when mask visualization is enabled
    if (u_show_mask && v_mask > 0.0) {
        float darken = mix(1.0, 0.3, v_mask);
        frag_color.rgb *= darken;
    }
}

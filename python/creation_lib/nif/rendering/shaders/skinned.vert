#version 330 core

// Standard mesh attributes
in vec3 in_position;
in vec3 in_normal;
in vec2 in_uv;

// Skinning attributes
in vec4 in_bone_weights;   // 4 bone weights
in ivec4 in_bone_indices;  // 4 bone indices

// Segment visualization
in vec3 in_segment_color; // Per-vertex segment color (RGB)

// Vertex color from NIF
in vec4 in_vertex_color;   // Per-vertex RGBA color

// Vertex mask (0.0=editable, 1.0=locked)
in float in_mask;

// Uniforms
uniform mat4 u_mvp;
uniform mat4 u_model;
uniform mat3 u_normal_matrix;
uniform mat4 u_bone_matrices[128];  // Max 128 bones
uniform bool u_skinned;             // Toggle skinning on/off

// Weight visualization
uniform bool u_weight_mode;
uniform int u_selected_bone;  // -1 = show total weight sum

out vec3 v_normal;
out vec2 v_uv;
out vec3 v_world_pos;
out float v_bone_weight;       // Weight for selected bone at this vertex
out vec3 v_segment_color;    // Segment color pass-through
out vec4 v_vertex_color;       // Vertex color pass-through
out float v_mask;              // Mask value pass-through

void main() {
    vec4 pos = vec4(in_position, 1.0);
    vec3 norm = in_normal;

    // Compute weight for selected bone (or total weight sum)
    v_bone_weight = 0.0;
    if (u_weight_mode) {
        if (u_selected_bone >= 0) {
            // Show weight for a specific bone
            for (int i = 0; i < 4; i++) {
                if (in_bone_indices[i] == u_selected_bone) {
                    v_bone_weight = in_bone_weights[i];
                    break;
                }
            }
        } else {
            // Show total weight sum (highlights unweighted vertices)
            v_bone_weight = in_bone_weights[0] + in_bone_weights[1]
                          + in_bone_weights[2] + in_bone_weights[3];
            v_bone_weight = clamp(v_bone_weight, 0.0, 1.0);
        }
    }

    // Pass segment color and mask through
    v_segment_color = in_segment_color;
    v_vertex_color = in_vertex_color;
    v_mask = in_mask;

    if (u_skinned) {
        mat4 skin_matrix = mat4(0.0);
        for (int i = 0; i < 4; i++) {
            if (in_bone_weights[i] > 0.0) {
                skin_matrix += u_bone_matrices[in_bone_indices[i]] * in_bone_weights[i];
            }
        }
        pos = skin_matrix * pos;
        norm = mat3(skin_matrix) * norm;
    }

    v_world_pos = (u_model * pos).xyz;
    v_normal = normalize(u_normal_matrix * norm);
    v_uv = in_uv;
    gl_Position = u_mvp * pos;
}

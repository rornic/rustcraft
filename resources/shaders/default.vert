#version 150

in vec3 position;
in vec3 normal;
in vec2 uv;

out vec3 v_pos;
out vec3 v_normal;
out vec2 v_uv;
out float camera_dist;

uniform GlobalUniforms {
    mat4 model_matrix;
    mat4 projection_matrix;
    mat4 view_matrix;
    vec4 camera_pos;
    vec4 light;
    float fog_start;
    float fog_end;
};


void main() {
    v_normal = transpose(inverse(mat3(model_matrix))) * normal;
    v_uv = uv;
    v_pos = position;
    camera_dist = distance(vec3(camera_pos), v_pos);
    gl_Position = projection_matrix * view_matrix * model_matrix * vec4(position, 1.0);
}
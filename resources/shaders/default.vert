#version 150

in vec3 position;
in vec3 normal;
in vec2 uv;

out vec3 v_normal;
out vec2 v_uv;

uniform global_render_uniforms {
    mat4 projection_matrix;
    mat4 view_matrix;
    vec3 light;
};

uniform mat4 model_matrix;

void main() {
    v_normal = transpose(inverse(mat3(model_matrix))) * normal;
    v_uv = uv;
    gl_Position = projection_matrix * view_matrix * model_matrix * vec4(position, 1.0);
}
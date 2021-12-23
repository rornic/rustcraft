#version 140

in vec3 v_normal;
out vec4 color;

uniform global_render_uniforms {
    mat4 projection_matrix;
    mat4 view_matrix;
    vec3 light;
};


void main() {
    float brightness = dot(normalize(v_normal), normalize(light));
    vec3 dark = vec3(0.5, 0.0, 0.25);
    vec3 regular = vec3(1.0, 0.0, 0.5);
    color = vec4(mix(dark, regular, brightness), 1.0);
}
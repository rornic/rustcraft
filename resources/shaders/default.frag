#version 140

in vec3 v_normal;
in vec2 v_uv;

out vec4 color;

uniform GlobalUniforms {
    mat4 model_matrix;
    mat4 projection_matrix;
    mat4 view_matrix;
    vec3 light;
};
uniform sampler2D tex;


void main() {
    float brightness = dot(normalize(v_normal), normalize(light));
    vec4 tex_color = texture(tex, vec2(v_uv.x, v_uv.y));
    vec4 dark = tex_color * 0.5;
    color = mix(dark, tex_color, brightness);
}
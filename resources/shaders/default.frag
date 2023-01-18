#version 140

in vec3 v_pos;
in vec3 v_normal;
in vec2 v_uv;
in float camera_dist;

out vec4 color;

uniform GlobalUniforms {
    mat4 model_matrix;
    mat4 projection_matrix;
    mat4 view_matrix;
    vec4 camera_pos;
    vec4 light;
    float fog_start;
    float fog_end;
};
uniform sampler2D tex;

float getFog(float d)
{
    if (d>=fog_end) return 1;
    if (d<=fog_start) return 0;

    return 1 - (fog_end - d) / (fog_end - fog_start);
}

void main() {
    float brightness = dot(normalize(v_normal), normalize(vec3(light)));
    vec4 tex_color = texture(tex, vec2(v_uv.x, v_uv.y));
    vec4 dark = tex_color * 0.5;

    vec4 tex_color_lit = mix(dark, tex_color, brightness);
    tex_color_lit.a = 1.0;

    float alpha = getFog(camera_dist);
    color = mix(tex_color_lit, vec4(0.0, 0.0, 0.0, 0.0), alpha);
}
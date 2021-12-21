pub const VERTEX_SHADER_SRC: &str = r#"
#version 150

in vec3 position;
in vec3 normal;

out vec3 v_normal;

uniform mat4 projection_matrix;
uniform mat4 view_matrix;
uniform mat4 model_matrix;

void main() {
    v_normal = transpose(inverse(mat3(model_matrix))) * normal;
    gl_Position = projection_matrix * view_matrix * model_matrix * vec4(position, 1.0);
}
"#;

pub const FRAGMENT_SHADER_SRC: &str = r#"
#version 140

in vec3 v_normal;
out vec4 color;

uniform vec3 u_light;

void main() {
    float brightness = dot(normalize(v_normal), normalize(u_light));
    vec3 dark = vec3(0.6, 0.0, 0.0);
    vec3 regular = vec3(1.0, 0.0, 0.0);
    color = vec4(mix(dark, regular, brightness), 1.0);
}
"#;

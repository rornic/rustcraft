use glium::{Display, Program, ProgramCreationError};

pub const VERTEX_SHADER_SRC: &str = r#"
#version 150

in vec3 position;
in vec3 normal;

out vec3 v_normal;

uniform global_render_uniforms {
    mat4 projection_matrix;
    mat4 view_matrix;
    vec3 light;
};

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

uniform global_render_uniforms {
    mat4 projection_matrix;
    mat4 view_matrix;
    vec3 light;
};

void main() {
    float brightness = dot(normalize(v_normal), normalize(light));
    vec3 dark = vec3(0.6, 0.0, 0.0);
    vec3 regular = vec3(1.0, 0.0, 0.0);
    color = vec4(mix(dark, regular, brightness), 1.0);
}
"#;

pub fn create_shader_program(
    display: &Display,
    vertex_shader_src: &str,
    fragment_shader_src: &str,
) -> Result<Program, ProgramCreationError> {
    Program::from_source(display, vertex_shader_src, fragment_shader_src, None)
}

use glium::{Display, Program, ProgramCreationError};

/// Represents an error that can occur when loading a shader
#[derive(Debug)]
pub enum ShaderLoadError {
    UnknownVertexShader(std::io::Error),
    UnknownFragmentShader(std::io::Error),
    ProgramCreationError(ProgramCreationError),
}

/// Loads a vertex and fragment shader from `resources/shaders` using `shader_name` as the file name followed by the appropriate `.vert` or `.frag` file extension.
///
/// Then creates and returns a `Program` for this shader.
pub fn load_shader(display: &Display, shader_name: &str) -> Result<Program, ShaderLoadError> {
    let vertex_src = std::fs::read_to_string(format!("resources/shaders/{}.vert", shader_name))
        .map_err(|e| ShaderLoadError::UnknownVertexShader(e))?;

    let fragment_src = std::fs::read_to_string(format!("resources/shaders/{}.frag", shader_name))
        .map_err(|e| ShaderLoadError::UnknownFragmentShader(e))?;

    create_shader_program(&display, &vertex_src, &fragment_src)
        .map_err(|e| ShaderLoadError::ProgramCreationError(e))
}

/// Creates a shader `Program` from vertex and fragment shader strings.
fn create_shader_program(
    display: &Display,
    vertex_shader_src: &str,
    fragment_shader_src: &str,
) -> Result<Program, ProgramCreationError> {
    Program::from_source(display, vertex_shader_src, fragment_shader_src, None)
}

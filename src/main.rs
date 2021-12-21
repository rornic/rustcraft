#[macro_use]
extern crate glium;
use glium::{glutin::event::VirtualKeyCode, Surface};

use crate::input::KeyboardMap;

mod input;
mod shapes;

const VERTEX_SHADER_SRC: &str = r#"
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

const FRAGMENT_SHADER_SRC: &str = r#"
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

fn main() {
    use glium::glutin;

    let event_loop = glutin::event_loop::EventLoop::new();
    let wb = glutin::window::WindowBuilder::new();
    let cb = glutin::ContextBuilder::new()
        .with_depth_buffer(24)
        .with_vsync(true);
    let display = glium::Display::new(wb, cb, &event_loop).unwrap();

    let mut elapsed_time: f32 = 0.0;
    let mut delta_time: f32 = 0.0;
    let mut camera_x: f32 = 0.0;

    let mut keyboard = KeyboardMap::new();

    // Set up cube for rendering
    let shape = shapes::cube();
    let (vertex_buffer, normal_buffer, index_buffer) = (
        glium::VertexBuffer::new(&display, &shape.vertices).unwrap(),
        glium::VertexBuffer::new(&display, &shape.normals).unwrap(),
        glium::IndexBuffer::new(
            &display,
            glium::index::PrimitiveType::TrianglesList,
            &shape.indices,
        )
        .unwrap(),
    );

    let program =
        glium::Program::from_source(&display, VERTEX_SHADER_SRC, FRAGMENT_SHADER_SRC, None)
            .unwrap();

    event_loop.run(move |ev, _, control_flow| {
        let frame_start = std::time::Instant::now();

        // Handle window events
        match ev {
            glutin::event::Event::WindowEvent { event, .. } => match event {
                glutin::event::WindowEvent::CloseRequested => {
                    *control_flow = glutin::event_loop::ControlFlow::Exit;
                    return;
                }
                _ => return,
            },
            glutin::event::Event::DeviceEvent { event, .. } => match event {
                glutin::event::DeviceEvent::Key(input) => keyboard.process_event(input),
                _ => (),
            },
            _ => (),
        };

        if keyboard.is_pressed(VirtualKeyCode::A) {
            camera_x -= 3.0 * delta_time;
        } else if keyboard.is_pressed(VirtualKeyCode::D) {
            camera_x += 3.0 * delta_time;
        }

        // Start drawing on window
        let mut target = display.draw();
        target.clear_color_and_depth((0.01, 0.01, 0.01, 1.0), 1.0);

        let projection_matrix = {
            let (width, height) = target.get_dimensions();
            let aspect_ratio = height as f32 / width as f32;

            let fov: f32 = 3.141592 / 3.0;
            let zfar = 1024.0;
            let znear = 0.1;

            let f = 1.0 / (fov / 2.0).tan();

            [
                [f * aspect_ratio, 0.0, 0.0, 0.0],
                [0.0, f, 0.0, 0.0],
                [0.0, 0.0, (zfar + znear) / (zfar - znear), 1.0],
                [0.0, 0.0, -(2.0 * zfar * znear) / (zfar - znear), 0.0],
            ]
        };

        let view_matrix = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-camera_x, 0.0, 0.0, 1.0],
        ];

        let uniforms = uniform! {
            model_matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, elapsed_time.cos(), -elapsed_time.sin(), 0.0],
                [0.0, elapsed_time.sin(), elapsed_time.cos(), 0.0],
                [0.0, 0.0, 15.0, 1.0f32],
            ],
            view_matrix: view_matrix,
            projection_matrix: projection_matrix,
            u_light: [-1.0, 0.4, 0.9f32],
        };

        let params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::draw_parameters::DepthTest::IfLess,
                write: true,
                ..Default::default()
            },
            ..Default::default()
        };

        target
            .draw(
                (&vertex_buffer, &normal_buffer),
                &index_buffer,
                &program,
                &uniforms,
                &params,
            )
            .unwrap();
        target.finish().unwrap();

        // Update delta_time with the time of this frame
        delta_time = (std::time::Instant::now() - frame_start).as_secs_f32();
        elapsed_time += delta_time;
    });
}

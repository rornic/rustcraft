#[macro_use]
extern crate glium;
use glium::glutin::event::Event;
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::uniforms::UniformBuffer;
use glium::Display;
use glium::{glutin::event::VirtualKeyCode, Surface};

use crate::input::KeyboardMap;
use crate::render::model::{GlobalRenderUniforms, Renderable};

mod input;
mod render;
mod world;

use world::Vector3;

use render::model::primitives::cube;
use render::shader::{FRAGMENT_SHADER_SRC, VERTEX_SHADER_SRC};

/// Prepares a `Display` and `EventLoop` for rendering and updating.
fn init_display() -> (EventLoop<()>, Display) {
    use glium::glutin;

    // Set up window
    let event_loop = glutin::event_loop::EventLoop::new();
    let wb = glutin::window::WindowBuilder::new();
    let cb = glutin::ContextBuilder::new()
        .with_depth_buffer(24)
        .with_vsync(true);
    let display = glium::Display::new(wb, cb, &event_loop).unwrap();
    (event_loop, display)
}

struct Input {
    keyboard: KeyboardMap,
}

fn init_input() -> Input {
    Input {
        keyboard: KeyboardMap::new(),
    }
}

fn process_window_event(input: &mut Input, ev: Event<()>, control_flow: &mut ControlFlow) {
    use glium::glutin;

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
            glutin::event::DeviceEvent::Key(ki) => input.keyboard.process_event(ki),
            _ => (),
        },
        _ => (),
    };
}

fn main() {
    let (event_loop, display) = init_display();

    let mut elapsed_time: f32 = 0.0;
    let mut delta_time: f32 = 0.0;

    let mut camera_pos = Vector3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    // Prepare keyboard for input
    let mut input = init_input();

    // Set up cube for rendering
    let model = cube().load(&display).unwrap();

    // Create the shader program
    let program =
        render::shader::create_shader_program(&display, VERTEX_SHADER_SRC, FRAGMENT_SHADER_SRC)
            .expect("Failed to create shader program.");

    // Create a buffer for global uniforms
    let global_uniform_buffer = UniformBuffer::empty(&display).unwrap();

    // Set up draw parameters
    let params = glium::DrawParameters {
        depth: glium::Depth {
            test: glium::draw_parameters::DepthTest::IfLess,
            write: true,
            ..Default::default()
        },
        ..Default::default()
    };
    event_loop.run(move |ev, _, control_flow| {
        let frame_start = std::time::Instant::now();

        process_window_event(&mut input, ev, control_flow);

        if input.keyboard.is_pressed(VirtualKeyCode::A) {
            camera_pos.x -= 3.0 * delta_time;
        } else if input.keyboard.is_pressed(VirtualKeyCode::D) {
            camera_pos.x += 3.0 * delta_time;
        }

        if input.keyboard.is_pressed(VirtualKeyCode::W) {
            camera_pos.z += 3.0 * delta_time;
        } else if input.keyboard.is_pressed(VirtualKeyCode::S) {
            camera_pos.z -= 3.0 * delta_time;
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
            [-camera_pos.x, -camera_pos.y, -camera_pos.z, 1.0],
        ];

        // Update global_uniform_buffer with updated projection and view matrices
        let global_render_uniforms = GlobalRenderUniforms {
            projection_matrix: projection_matrix,
            view_matrix: view_matrix,
            light: [-1.0, 0.4, 0.9f32],
        };
        global_uniform_buffer.write(&global_render_uniforms);

        let uniforms = uniform! {
            model_matrix:  [
                 [1.0, 0.0, 0.0, 0.0],
                 [0.0, elapsed_time.cos(), -elapsed_time.sin(), 0.0],
                 [0.0, elapsed_time.sin(), elapsed_time.cos(), 0.0],
                 [0.0, 0.0, 15.0, 1.0f32],
            ],
            global_render_uniforms: &global_uniform_buffer,
        };

        model
            .render(&mut target, &program, &uniforms, &params)
            .unwrap();

        target.finish().unwrap();

        // Update delta_time with the time of this frame
        delta_time = (std::time::Instant::now() - frame_start).as_secs_f32();
        elapsed_time += delta_time;
    });
}

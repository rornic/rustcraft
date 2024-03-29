use std::collections::HashMap;

use glium::glutin::event::{ElementState, KeyboardInput, MouseButton, VirtualKeyCode};

/// Represents the state of all input peripherals.
///
/// Currently just represents the keyboard.
#[derive(Default)]
pub struct Input {
    pub keyboard: KeyboardMap,
    pub mouse: Mouse,
}

#[derive(Debug)]
pub enum InputEvent {
    Keyboard(KeyboardInput),
    MouseMotion {
        delta: (f64, f64),
    },
    MouseButton {
        button: MouseButton,
        state: ElementState,
    },
}

impl Input {
    pub fn update(&mut self) {
        // TODO: refactor this awful input system
        self.mouse.delta = (0.0, 0.0);
        self.mouse.left_press = false;
    }

    pub fn process_event(&mut self, event: &InputEvent) {
        match event {
            InputEvent::Keyboard(KeyboardInput {
                virtual_keycode: Some(code),
                state,
                ..
            }) => match state {
                ElementState::Pressed => self.keyboard.press(*code),
                ElementState::Released => self.keyboard.release(*code),
            },
            InputEvent::MouseMotion { delta } => {
                self.mouse.move_mouse((delta.0 as f32, delta.1 as f32))
            }
            InputEvent::MouseButton { button, state } => {
                self.mouse.left_press =
                    *button == MouseButton::Left && *state == ElementState::Pressed;
            }
            _ => (),
        }
    }
}

/// Represents the state of the keyboard.
///
/// Maintains a `HashMap` of `VirtualKeyCode` variants and the current `ElementState` they are observed to be in.
/// Processes `KeyboardInput` events as they are received and updates the state accordingly.
#[derive(Default)]
pub struct KeyboardMap {
    map: HashMap<VirtualKeyCode, ElementState>,
}

impl KeyboardMap {
    pub fn is_pressed(&self, virtual_keycode: VirtualKeyCode) -> bool {
        match self.map.get(&virtual_keycode) {
            Some(ElementState::Pressed) => true,
            _ => false,
        }
    }

    fn press(&mut self, key: VirtualKeyCode) {
        self.map.insert(key, ElementState::Pressed);
    }

    fn release(&mut self, key: VirtualKeyCode) {
        self.map.remove(&key);
    }
}

#[derive(Default)]
pub struct Mouse {
    delta: (f32, f32),
    left_press: bool,
}
impl Mouse {
    pub fn is_left_pressed(&self) -> bool {
        self.left_press
    }

    fn move_mouse(&mut self, delta: (f32, f32)) {
        self.delta = delta;
    }

    pub fn horizontal_motion(&self) -> f32 {
        self.delta.0
    }

    pub fn vertical_motion(&self) -> f32 {
        self.delta.1
    }
}

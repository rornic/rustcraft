use std::collections::HashMap;

use glium::glutin::event::{DeviceEvent, ElementState, KeyboardInput, VirtualKeyCode};

/// Represents the state of all input peripherals.
///
/// Currently just represents the keyboard.
#[derive(Default)]
pub struct Input {
    pub keyboard: KeyboardMap,
    pub mouse: Mouse,
}

impl Input {
    /// Updates the input, resetting any values if they should only be set on a per-frame basis.
    pub fn update(&mut self) {
        self.mouse.motion = (0.0, 0.0);
    }

    pub fn process_event(&mut self, event: DeviceEvent) {
        match event {
            DeviceEvent::Key(KeyboardInput {
                virtual_keycode: Some(code),
                state,
                ..
            }) => match state {
                ElementState::Pressed => self.keyboard.press(code),
                ElementState::Released => self.keyboard.release(code),
            },
            DeviceEvent::MouseMotion { delta: motion } => self.mouse.motion(motion),
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
    /// Checks whether a key is currently pressed.
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
    motion: (f32, f32),
}
impl Mouse {
    fn motion(&mut self, motion: (f64, f64)) {
        self.motion = (motion.0 as f32, motion.1 as f32);
    }

    pub fn horizontal_motion(&self) -> f32 {
        self.motion.0
    }

    pub fn vertical_motion(&self) -> f32 {
        self.motion.1
    }
}

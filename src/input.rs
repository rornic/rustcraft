use std::collections::HashMap;

use glium::glutin::event::{ElementState, KeyboardInput, VirtualKeyCode};

/// Represents the state of the keyboard.
///
/// Maintains a `HashMap` of `VirtualKeyCode` variants and the current `ElementState` they are observed to be in.
/// Processes `KeyboardInput` events as they are received and updates the state accordingly.
pub struct KeyboardMap {
    map: HashMap<VirtualKeyCode, ElementState>,
}

impl KeyboardMap {
    pub fn new() -> Self {
        KeyboardMap {
            map: HashMap::new(),
        }
    }

    /// Checks whether a key is currently pressed.
    pub fn is_pressed(&self, virtual_keycode: VirtualKeyCode) -> bool {
        match self.map.get(&virtual_keycode) {
            Some(ElementState::Pressed) => true,
            _ => false,
        }
    }

    /// Processes a `KeyboardInput` event and updates the `KeyboardMap` state accordingly.
    pub fn process_event(&mut self, event: KeyboardInput) {
        match event {
            KeyboardInput {
                virtual_keycode: Some(code),
                state,
                ..
            } => match state {
                ElementState::Pressed => self.map.insert(code, state),
                ElementState::Released => self.map.remove(&code),
            },
            _ => return,
        };
    }
}

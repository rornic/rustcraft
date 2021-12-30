#[macro_export]
macro_rules! vector3 {
    ( $x:expr,$y:expr,$z:expr ) => {
        cgmath::Vector3 {
            x: $x,
            y: $y,
            z: $z,
        }
    };
}

#[macro_export]
macro_rules! vector2 {
    ( $x:expr,$y:expr ) => {
        cgmath::Vector2 { x: $x, y: $y }
    };
}

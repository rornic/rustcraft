//! What the gameplay pass finds and places on the map, stored with it so every machine and every
//! system (spawning, the map export, later the game's own encounters) reads the same labels.

/// A labelled place, in world coordinates.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Feature {
    pub kind: FeatureKind,
    pub x: i32,
    pub z: i32,
    /// What the kind measures: a pass's height, a crossing's river width in blocks, a spawn
    /// region's radius, and nothing for the rest.
    pub value: f32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FeatureKind {
    /// The lowest saddle through a range, kept walkable.
    Pass = 0,
    /// A shallow, wide stretch of a river that can be waded.
    Ford = 1,
    /// A river too wide and deep to wade, with flat banks to build across.
    BridgeSite = 2,
    /// Mild, sheltered lowland to start in.
    Spawn = 3,
    Iron = 4,
    Coal = 5,
    Clay = 6,
    Salt = 7,
    /// A cave mouth in a steep face.
    Cave = 8,
    /// Somewhere dangerous, to meet something.
    Hotspot = 9,
}

impl FeatureKind {
    pub const ALL: [FeatureKind; 10] = [
        Self::Pass,
        Self::Ford,
        Self::BridgeSite,
        Self::Spawn,
        Self::Iron,
        Self::Coal,
        Self::Clay,
        Self::Salt,
        Self::Cave,
        Self::Hotspot,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Ford => "ford",
            Self::BridgeSite => "bridge site",
            Self::Spawn => "spawn",
            Self::Iron => "iron",
            Self::Coal => "coal",
            Self::Clay => "clay",
            Self::Salt => "salt",
            Self::Cave => "cave",
            Self::Hotspot => "hotspot",
        }
    }
}

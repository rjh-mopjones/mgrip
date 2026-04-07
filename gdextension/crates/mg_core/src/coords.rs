/// Grid position of a chunk in chunk-space coordinates.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Global tile position in tile-space coordinates.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
}

impl TileCoord {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Continuous world-space position using f64 for precision.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldPos {
    pub x: f64,
    pub y: f64,
}

impl WorldPos {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Detail level for the fractal noise hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DetailLevel {
    #[default]
    Macro = 0,
    Meso = 1,
    Micro = 2,
}

impl DetailLevel {
    pub const fn as_u32(&self) -> u32 {
        *self as u32
    }

    pub const fn octave_offset(&self) -> u32 {
        match self {
            DetailLevel::Macro => 1,
            DetailLevel::Meso => 2,
            DetailLevel::Micro => 3,
        }
    }
}

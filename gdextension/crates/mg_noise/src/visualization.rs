/// Layer identifiers for debug PNG export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoiseLayer {
    Biome,
    Heightmap,
    Temperature,
    Humidity,
    Continentalness,
    Tectonic,
    RockHardness,
    LightLevel,
    PeaksValleys,
    Erosion,
    Rivers,
    Aridity,
    PrecipitationType,
    Snowpack,
    WaterTable,
    VegetationDensity,
    SoilType,
    ResourceRichness,
    WindSpeed,
    Volcanism,
}

impl NoiseLayer {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Biome => "biome",
            Self::Heightmap => "Heightmap",
            Self::Temperature => "temperature",
            Self::Humidity => "humidity",
            Self::Continentalness => "continentalness",
            Self::Tectonic => "tectonic",
            Self::RockHardness => "rock_hardness",
            Self::LightLevel => "light_level",
            Self::PeaksValleys => "peaks_valleys",
            Self::Erosion => "erosion",
            Self::Rivers => "rivers",
            Self::Aridity => "aridity",
            Self::PrecipitationType => "precipitation_type",
            Self::Snowpack => "snowpack",
            Self::WaterTable => "water_table",
            Self::VegetationDensity => "vegetation_density",
            Self::SoilType => "soil_type",
            Self::ResourceRichness => "resource_richness",
            Self::WindSpeed => "wind_speed",
            Self::Volcanism => "volcanism",
        }
    }

    pub fn all() -> &'static [NoiseLayer] {
        &[
            Self::Biome,
            Self::Heightmap,
            Self::Temperature,
            Self::Humidity,
            Self::Continentalness,
            Self::Tectonic,
            Self::RockHardness,
            Self::LightLevel,
            Self::PeaksValleys,
            Self::Erosion,
            Self::Rivers,
            Self::Aridity,
            Self::PrecipitationType,
            Self::Snowpack,
            Self::WaterTable,
            Self::VegetationDensity,
            Self::SoilType,
            Self::ResourceRichness,
        ]
    }
}

/// Map [0,1] float to grayscale RGBA.
pub fn grayscale_to_rgba(v: f64) -> [u8; 4] {
    let b = (v.clamp(0.0, 1.0) * 255.0) as u8;
    [b, b, b, 255]
}

/// Map heightmap [-1,1] to colored RGBA (ocean blue → land green → mountain gray).
pub fn heightmap_to_rgba(v: f64) -> [u8; 4] {
    if v < -0.01 {
        let t = ((v + 1.0) / 0.99).clamp(0.0, 1.0);
        [0, (t * 80.0) as u8, (100.0 + t * 155.0) as u8, 255]
    } else {
        let t = (v / 1.0).clamp(0.0, 1.0);
        let r = (100.0 + t * 100.0) as u8;
        let g = (120.0 - t * 80.0) as u8;
        let b = (80.0 - t * 60.0) as u8;
        [r, g, b, 255]
    }
}

/// Map temperature (°C) to a blue→green→red heat gradient.
pub fn temperature_to_rgba(temp: f64) -> [u8; 4] {
    let t = ((temp + 80.0) / 200.0).clamp(0.0, 1.0);
    if t < 0.5 {
        let s = t * 2.0;
        [
            (s * 80.0) as u8,
            (s * 200.0) as u8,
            (255.0 * (1.0 - s)) as u8,
            255,
        ]
    } else {
        let s = (t - 0.5) * 2.0;
        [(80.0 + s * 175.0) as u8, (200.0 * (1.0 - s)) as u8, 0, 255]
    }
}

pub fn humidity_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [
        (40.0 * (1.0 - t)) as u8,
        (100.0 * t) as u8,
        (200.0 * t + 55.0 * (1.0 - t)) as u8,
        255,
    ]
}

pub fn tectonic_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [
        (200.0 * (1.0 - t)) as u8,
        (100.0 * t) as u8,
        (80.0 * t) as u8,
        255,
    ]
}

pub fn light_level_to_rgba(v: f64) -> [u8; 4] {
    let t = (v.clamp(0.0, 1.0) * 255.0) as u8;
    [t, (t as f64 * 0.8) as u8, (t as f64 * 0.4) as u8, 255]
}

pub fn peaks_to_rgba(v: f64) -> [u8; 4] {
    let t = ((v + 1.0) * 0.5).clamp(0.0, 1.0);
    grayscale_to_rgba(t)
}

pub fn erosion_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [(180.0 * t) as u8, (120.0 * t) as u8, (60.0 * t) as u8, 255]
}

pub fn river_to_rgba(v: f64) -> [u8; 4] {
    if v > 0.0 {
        let t = (v / 2000.0).clamp(0.0, 1.0);
        [0, (100.0 + t * 155.0) as u8, 255, 255]
    } else {
        [20, 20, 20, 255]
    }
}

pub fn aridity_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [
        (200.0 * t + 55.0 * (1.0 - t)) as u8,
        (180.0 * (1.0 - t)) as u8,
        (60.0 * (1.0 - t)) as u8,
        255,
    ]
}

pub fn precipitation_type_to_rgba(v: f64) -> [u8; 4] {
    if v < 0.0 {
        let t = (-v).clamp(0.0, 1.0);
        [
            (200.0 * (1.0 - t) + 55.0) as u8,
            (220.0 * (1.0 - t) + 35.0) as u8,
            255,
            255,
        ]
    } else {
        let t = v.clamp(0.0, 1.0);
        [
            (200.0 * t + 55.0 * (1.0 - t)) as u8,
            (80.0 * t) as u8,
            (20.0 * t) as u8,
            255,
        ]
    }
}

pub fn snowpack_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [(200.0 + 55.0 * t) as u8, (200.0 + 55.0 * t) as u8, 255, 255]
}

pub fn water_table_to_rgba(v: f64) -> [u8; 4] {
    humidity_to_rgba(v)
}

pub fn rock_hardness_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [
        (120.0 + 80.0 * t) as u8,
        (100.0 + 60.0 * t) as u8,
        (80.0 + 40.0 * t) as u8,
        255,
    ]
}

pub fn vegetation_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [
        (20.0 * (1.0 - t)) as u8,
        (180.0 * t + 40.0) as u8,
        (20.0 * (1.0 - t)) as u8,
        255,
    ]
}

pub fn soil_type_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [
        (150.0 * t + 60.0) as u8,
        (100.0 * t + 40.0) as u8,
        (30.0 * t) as u8,
        255,
    ]
}

pub fn resources_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [(255.0 * t) as u8, (200.0 * t) as u8, 0, 255]
}

pub fn wind_speed_to_rgba(v: f64) -> [u8; 4] {
    grayscale_to_rgba(v)
}

pub fn volcanism_to_rgba(v: f64) -> [u8; 4] {
    let t = v.clamp(0.0, 1.0);
    [
        (200.0 * t + 55.0) as u8,
        (30.0 * t) as u8,
        (20.0 * t) as u8,
        255,
    ]
}

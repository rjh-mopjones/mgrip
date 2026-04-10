use mg_core::TileType;
use noise::{NoiseFn, OpenSimplex};

pub fn derive_temperature(
    light_level: f64,
    elevation: f64,
    humidity: f64,
    continentalness: f64,
) -> f64 {
    let base_temp = if light_level < 0.28 {
        -80.0 + (light_level / 0.28) * 80.0
    } else if light_level < 0.62 {
        ((light_level - 0.28) / 0.34) * 45.0
    } else {
        45.0 + ((light_level - 0.62) / 0.38) * 75.0
    };

    let max_lapse = if base_temp > 0.0 {
        base_temp * 0.30
    } else {
        25.0
    };
    let lapse_rate = (elevation.max(0.0) * 55.0).min(max_lapse);
    let humidity_buffer = humidity * 5.0;
    let raw = base_temp - lapse_rate + humidity_buffer;

    let inland_factor = ((continentalness + 0.01).max(0.0) * 5.0).clamp(0.0, 1.0);
    let heat_damper = ((55.0 - raw) / 10.0).clamp(0.0, 1.0);
    let coastal_moderation = (15.0 - raw) * (1.0 - inland_factor) * 0.25 * heat_damper;

    let extremity = if inland_factor > 0.7 {
        (raw - 15.0) * ((inland_factor - 0.7) / 0.3) * 0.12
    } else {
        0.0
    };

    raw + coastal_moderation + extremity
}

pub fn derive_heightmap(continentalness: f64, _tectonic: f64, peaks_valleys: f64) -> f64 {
    let continental_base = continentalness * 0.95;
    let relief = peaks_valleys * 0.85;
    let coastal_taper = if continentalness < -0.05 {
        0.3
    } else if continentalness < 0.1 {
        ((continentalness + 0.05) / 0.15).clamp(0.0, 1.0).sqrt()
    } else {
        1.0
    };
    (continental_base + relief * coastal_taper).clamp(-1.0, 1.0)
}

/// Spec #49 corrected micro heightmap. Starts at octave 8 (not 12) with larger budgets.
///
/// The `detail_noise` must be created ONCE outside the pixel loop:
///   `OpenSimplex::new(seed.wrapping_add(50))`
pub fn derive_micro_heightmap(
    base_heightmap: f64,
    wx: f64,
    wy: f64,
    detail_noise: &OpenSimplex,
) -> f64 {
    // Start at octave 8: freq 0.01 * 2^8 = 2.56. Produces ~2.5 major landforms per chunk.
    let start_freq = 0.01 * 2.0_f64.powi(8);

    let mut value = 0.0;
    let mut amp = 1.0;
    let mut freq = start_freq;
    let mut max_amp = 0.0;

    for _ in 0..8 {
        value += detail_noise.get([wx * freq, wy * freq]) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    let detail = value / max_amp; // independently normalized [-1, 1]

    // Amplitude budgets by terrain type (spec #49):
    //   mountains: 0.40 → ~102 blocks at HEIGHT_SCALE=256
    //   hills/coast: 0.20 → ~51 blocks
    //   plains/forest: 0.08 → ~20 blocks
    //   ocean: 0.04 → ~10 blocks
    let budget = if base_heightmap > 0.03 {
        0.40
    } else if base_heightmap > -0.01 {
        0.20
    } else if base_heightmap > -0.025 {
        0.08
    } else {
        0.04
    };

    (base_heightmap + detail * budget).clamp(-1.0, 1.0)
}

pub fn derive_peaks_valleys(base_pv: f64, tectonic: f64, rock_hardness: f64) -> f64 {
    // Keep tectonic boundaries important without letting them dominate the
    // whole macro relief field as broad wedge-shaped ridges.
    let stress = 1.0 - tectonic;
    let stress_envelope = stress * stress * stress;
    let amplitude = 0.14 + stress_envelope * 0.58;
    let hardness_factor = 0.7 + rock_hardness * 0.3;
    (base_pv * amplitude * hardness_factor).clamp(-1.0, 1.0)
}

pub fn derive_erosion(heightmap: f64, rock_hardness: f64, humidity: f64) -> f64 {
    let raw = (heightmap.max(0.0) * 2.5 + humidity * 0.8) * (1.0 - rock_hardness * 0.3);
    raw.sqrt().clamp(0.0, 1.0)
}

pub fn derive_aridity(temperature: f64, humidity: f64) -> f64 {
    let temp_factor = ((temperature - 10.0) / 45.0).clamp(0.0, 1.0);
    (temp_factor * 0.65 + (1.0 - humidity) * 0.35).clamp(0.0, 1.0)
}

pub fn derive_precipitation_type(temperature: f64, humidity: f64, heightmap: f64) -> f64 {
    if humidity < 0.15 {
        return 1.0;
    }
    let snow_factor = ((-temperature + 5.0) / 25.0).clamp(0.0, 1.0);
    let altitude_bonus = (heightmap - 0.1).max(0.0) * 2.0;
    let snow = (snow_factor + altitude_bonus).min(1.0);
    let humid_capped = humidity.min(0.8);
    -snow * humid_capped + (1.0 - humid_capped) * (1.0 - snow)
}

pub fn derive_snowpack(
    precipitation_type: f64,
    temperature: f64,
    heightmap: f64,
    light_level: f64,
) -> f64 {
    let cold_factor = ((3.0 - temperature) / 40.0).clamp(0.0, 1.0);
    let snow_precip = (-precipitation_type).max(0.0);
    let temperature_snow = cold_factor * snow_precip;

    let temp_gate = ((30.0 - temperature) / 20.0).clamp(0.0, 1.0);
    let snow_altitude = if light_level < 0.5 {
        light_level * 0.1
    } else {
        0.05 + (light_level - 0.5) * 0.2
    };
    let moisture_availability = (light_level * 3.0).clamp(0.2, 1.0);
    let altitude_snow = if heightmap > snow_altitude {
        ((heightmap - snow_altitude) * 12.0).min(1.0) * temp_gate * moisture_availability
    } else {
        0.0
    };

    temperature_snow.max(altitude_snow).clamp(0.0, 1.0)
}

pub fn derive_water_table(
    river_flow: f64,
    humidity: f64,
    heightmap: f64,
    precipitation_type: f64,
    continentalness: f64,
) -> f64 {
    let humidity_base = humidity * 0.3;
    let river_boost = (river_flow * 4.0).min(1.0) * 0.45;
    let elevation_boost = (1.0 - heightmap.max(0.0) * 2.0).max(0.0) * 0.2;
    let precip_boost = (-precipitation_type).max(0.0) * 0.1;
    let coastal_boost = (1.0 - (continentalness - (-0.01_f64)).max(0.0) * 10.0).max(0.0) * 0.1;
    (humidity_base + river_boost + elevation_boost + precip_boost + coastal_boost).clamp(0.0, 1.0)
}

pub fn derive_resource_richness(tectonic: f64, rock_hardness: f64, erosion: f64) -> f64 {
    let boundary = (1.0 - tectonic).powf(1.5);
    (boundary * 0.5 + rock_hardness * 0.3 + erosion * 0.2).clamp(0.0, 1.0)
}

pub fn derive_vegetation_density(biome: TileType, water_table: f64) -> f64 {
    let base = match biome {
        TileType::Jungle => 0.95,
        TileType::TemperateRainforest | TileType::SubtropicalForest => 0.85,
        TileType::Forest | TileType::DeciduousForest | TileType::CloudForest => 0.8,
        TileType::Marsh => 0.7,
        TileType::Woodland | TileType::DryWoodland => 0.65,
        TileType::Taiga | TileType::Mangrove => 0.6,
        TileType::Oasis => 0.55,
        TileType::Plains | TileType::Meadow => 0.5,
        TileType::Savanna | TileType::HighlandSavanna => 0.4,
        TileType::AlpineMeadow => 0.35,
        TileType::Steppe | TileType::Thornland => 0.25,
        TileType::Scrubland | TileType::Plateau => 0.2,
        TileType::Mountain => 0.15,
        TileType::Tundra | TileType::FrozenBog => 0.1,
        TileType::Beach | TileType::SeaCliff => 0.1,
        TileType::Badlands | TileType::Hamada => 0.08,
        TileType::Desert | TileType::Sahara | TileType::Erg => 0.05,
        TileType::RockyCoast => 0.05,
        TileType::SaltFlat | TileType::ScorchedRock => 0.02,
        TileType::Volcanic | TileType::LavaField | TileType::MoltenWaste => 0.02,
        TileType::Snow | TileType::Glacier | TileType::White | TileType::IceSheet => 0.0,
        _ => 0.0,
    };
    (base + water_table * 0.3).clamp(0.0, 1.0)
}

pub fn derive_soil_type(biome: TileType, erosion: f64, rock_hardness: f64) -> f64 {
    let base = match biome {
        TileType::Forest
        | TileType::Jungle
        | TileType::DeciduousForest
        | TileType::TemperateRainforest
        | TileType::SubtropicalForest
        | TileType::CloudForest => 0.8,
        TileType::Plains | TileType::Marsh | TileType::Meadow | TileType::Oasis => 0.7,
        TileType::Woodland | TileType::DryWoodland => 0.6,
        TileType::Savanna | TileType::HighlandSavanna => 0.5,
        TileType::Taiga | TileType::Mangrove => 0.4,
        TileType::AlpineMeadow => 0.35,
        TileType::Steppe | TileType::Thornland | TileType::Scrubland => 0.3,
        TileType::FrozenBog | TileType::Tundra => 0.2,
        TileType::Beach => 0.15,
        TileType::Desert
        | TileType::Sahara
        | TileType::Badlands
        | TileType::Erg
        | TileType::Hamada => 0.1,
        TileType::SaltFlat | TileType::ScorchedRock | TileType::RockyCoast | TileType::SeaCliff => {
            0.05
        }
        TileType::Mountain | TileType::Glacier | TileType::Plateau => 0.05,
        TileType::Volcanic | TileType::LavaField | TileType::MoltenWaste => 0.08,
        TileType::Snow | TileType::White | TileType::IceSheet => 0.03,
        _ => 0.0,
    };
    (base + erosion * 0.3 - rock_hardness * 0.2).clamp(0.0, 1.0)
}

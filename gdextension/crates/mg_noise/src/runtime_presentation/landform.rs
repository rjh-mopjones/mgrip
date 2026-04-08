use super::{
    coastalness, is_frozen_biome, is_volcanic_biome, score_high, score_low, PlanetZone,
    RuntimePresentationSample, SurfaceWaterState,
};
use mg_core::TileType;
use serde::{Deserialize, Serialize};

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandformClass {
    FlatPlain = 0,
    Basin = 1,
    Plateau = 2,
    Ridge = 3,
    Escarpment = 4,
    BrokenHighland = 5,
    AlpineMassif = 6,
    CoastShelf = 7,
    CliffCoast = 8,
    FrozenShelf = 9,
    DuneWaste = 10,
    Badlands = 11,
    FractureBelt = 12,
    RiverCutLowland = 13,
    VolcanicField = 14,
}

impl LandformClass {
    pub const COUNT: usize = 15;
    pub const ALL: [Self; Self::COUNT] = [
        Self::FlatPlain,
        Self::Basin,
        Self::Plateau,
        Self::Ridge,
        Self::Escarpment,
        Self::BrokenHighland,
        Self::AlpineMassif,
        Self::CoastShelf,
        Self::CliffCoast,
        Self::FrozenShelf,
        Self::DuneWaste,
        Self::Badlands,
        Self::FractureBelt,
        Self::RiverCutLowland,
        Self::VolcanicField,
    ];

    pub(super) fn classify(
        sample: &RuntimePresentationSample,
        zone: PlanetZone,
        water_state: SurfaceWaterState,
    ) -> Self {
        let coast = coastalness(sample.continentalness);
        let frozen_biome = f64::from(is_frozen_biome(sample.biome));
        let volcanic_biome = f64::from(is_volcanic_biome(sample.biome));
        let sandy_biome = f64::from(is_sandy_biome(sample.biome));
        let rocky_biome = f64::from(is_rocky_biome(sample.biome));
        let basin_curvature = score_high(sample.curvature, 0.012, 0.055);
        let ridge_curvature = score_high(-sample.curvature, 0.010, 0.050);
        let frozen_water = f64::from(matches!(
            water_state,
            SurfaceWaterState::FrozenSea | SurfaceWaterState::IceSheet
        ));

        let flat_plain = score_low(sample.local_relief, 0.05, 0.18) * 2.8
            + score_low(sample.slope, 0.04, 0.16) * 2.2
            + score_low(sample.heightmap, 0.10, 0.34) * 0.8
            + score_low(sample.rivers, 0.08, 0.24) * 0.4;

        let basin = basin_curvature * 2.8
            + score_low(sample.heightmap, 0.02, 0.22) * 1.4
            + score_high(sample.water_table, 0.20, 0.60) * 1.0
            + score_high(sample.local_relief, 0.08, 0.26) * 0.8
            + f64::from(zone.is_dayside()) * f64::from(matches!(
                water_state,
                SurfaceWaterState::EvaporiteBasin | SurfaceWaterState::BrineFlat
            )) * 2.6
            + f64::from(matches!(
                water_state,
                SurfaceWaterState::EvaporiteBasin
                    | SurfaceWaterState::BrineFlat
                    | SurfaceWaterState::MarshWater
            )) * 1.2;

        let plateau = score_high(sample.heightmap, 0.10, 0.40) * 2.0
            + score_low(sample.slope, 0.08, 0.24) * 1.8
            + score_high(sample.local_relief, 0.06, 0.18) * 0.6
            + f64::from(sample.biome == TileType::Plateau) * 1.4;

        let ridge = ridge_curvature * 2.2
            + score_high(sample.slope, 0.10, 0.28) * 2.0
            + score_high(sample.local_relief, 0.10, 0.28) * 1.0
            + score_high(sample.peaks_valleys, 0.08, 0.32) * 0.8;

        let escarpment = score_high(sample.slope, 0.16, 0.36) * 2.8
            + score_high(sample.local_relief, 0.16, 0.34) * 1.8
            + ridge_curvature * 0.8
            + rocky_biome * 0.4;

        let broken_highland = score_high(sample.heightmap, 0.12, 0.44) * 1.6
            + score_high(sample.local_relief, 0.16, 0.36) * 2.2
            + score_high(sample.slope, 0.10, 0.24) * 1.2;

        let alpine_massif = score_high(sample.heightmap, 0.24, 0.58) * 2.4
            + score_high(sample.local_relief, 0.24, 0.46) * 2.2
            + score_high(sample.slope, 0.16, 0.34) * 1.8
            + frozen_biome * 0.6;

        let coast_shelf = score_high(coast, 0.42, 0.90) * 2.8
            + score_low(sample.local_relief, 0.06, 0.22) * 1.8
            + score_low(sample.slope, 0.05, 0.18) * 1.4
            + score_high(sample.water_table, 0.14, 0.52) * 0.6;

        let cliff_coast = score_high(coast, 0.42, 0.92) * 2.4
            + score_high(sample.local_relief, 0.18, 0.38) * 2.0
            + score_high(sample.slope, 0.16, 0.34) * 2.0
            + rocky_biome * 0.8;

        let frozen_shelf = score_high(coast, 0.42, 0.92) * 2.4
            + score_low(sample.local_relief, 0.06, 0.24) * 1.6
            + score_low(sample.temperature, -4.0, 8.0) * 1.2
            + (frozen_biome + frozen_water + f64::from(zone.is_nightside())) * 1.2;

        let dune_waste = f64::from(zone.is_dayside()) * 1.4
            + score_high(sample.aridity, 0.56, 0.94) * 2.8
            + score_low(sample.local_relief, 0.04, 0.16) * 1.2
            + score_low(sample.water_table, 0.10, 0.34) * 1.0
            - f64::from(matches!(
                water_state,
                SurfaceWaterState::EvaporiteBasin | SurfaceWaterState::BrineFlat
            )) * 1.8
            + sandy_biome * 1.4;

        let badlands = score_high(sample.aridity, 0.48, 0.88) * 1.8
            + score_high(sample.erosion, 0.28, 0.70) * 2.0
            + score_high(sample.local_relief, 0.14, 0.32) * 1.4
            + rocky_biome * 1.0
            + f64::from(sample.biome == TileType::Badlands) * 1.2;

        let fracture_belt = score_low(sample.tectonic, 0.30, 0.68) * 2.6
            + score_high(sample.local_relief, 0.16, 0.34) * 1.6
            + score_high(sample.slope, 0.10, 0.26) * 1.0
            + rocky_biome * 0.8;

        let river_cut_lowland = score_high(sample.rivers, 0.06, 0.18) * 2.4
            + score_high(sample.water_table, 0.20, 0.64) * 1.6
            + score_low(sample.heightmap, 0.10, 0.30) * 1.2
            + score_low(sample.local_relief, 0.10, 0.26) * 0.8
            + f64::from(matches!(
                water_state,
                SurfaceWaterState::LiquidRiver
                    | SurfaceWaterState::FrozenRiver
                    | SurfaceWaterState::MeltwaterChannel
            )) * 1.8;

        let volcanic_field = volcanic_biome * 3.4
            + score_low(sample.tectonic, 0.34, 0.76) * 1.2
            + score_high(sample.local_relief, 0.10, 0.28) * 0.8
            + score_high(sample.rock_hardness, 0.38, 0.80) * 0.6;

        let scores = [
            (Self::FlatPlain, flat_plain),
            (Self::Basin, basin),
            (Self::Plateau, plateau),
            (Self::Ridge, ridge),
            (Self::Escarpment, escarpment),
            (Self::BrokenHighland, broken_highland),
            (Self::AlpineMassif, alpine_massif),
            (Self::CoastShelf, coast_shelf),
            (Self::CliffCoast, cliff_coast),
            (Self::FrozenShelf, frozen_shelf),
            (Self::DuneWaste, dune_waste),
            (Self::Badlands, badlands),
            (Self::FractureBelt, fracture_belt),
            (Self::RiverCutLowland, river_cut_lowland),
            (Self::VolcanicField, volcanic_field),
        ];

        scores
            .into_iter()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(landform, _)| landform)
            .unwrap_or(Self::FlatPlain)
    }

    pub fn as_index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlatPlain => "FlatPlain",
            Self::Basin => "Basin",
            Self::Plateau => "Plateau",
            Self::Ridge => "Ridge",
            Self::Escarpment => "Escarpment",
            Self::BrokenHighland => "BrokenHighland",
            Self::AlpineMassif => "AlpineMassif",
            Self::CoastShelf => "CoastShelf",
            Self::CliffCoast => "CliffCoast",
            Self::FrozenShelf => "FrozenShelf",
            Self::DuneWaste => "DuneWaste",
            Self::Badlands => "Badlands",
            Self::FractureBelt => "FractureBelt",
            Self::RiverCutLowland => "RiverCutLowland",
            Self::VolcanicField => "VolcanicField",
        }
    }
}

fn is_sandy_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::Desert
            | TileType::Sahara
            | TileType::Erg
            | TileType::SaltFlat
            | TileType::Beach
            | TileType::Savanna
    )
}

fn is_rocky_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::Mountain
            | TileType::Plateau
            | TileType::RockyCoast
            | TileType::SeaCliff
            | TileType::ScorchedRock
            | TileType::Badlands
            | TileType::Hamada
            | TileType::Volcanic
            | TileType::LavaField
            | TileType::MoltenWaste
            | TileType::Glacier
            | TileType::IceSheet
            | TileType::White
    )
}

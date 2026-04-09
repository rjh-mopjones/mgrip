use super::{
    band_score, coastalness, is_frozen_biome, is_volcanic_biome, score_high, score_low, PlanetZone,
    RuntimePresentationSample, SurfaceWaterState,
};
use mg_core::TileType;
use serde::{Deserialize, Serialize};

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfacePaletteClass {
    ScorchedStone = 0,
    AshDust = 1,
    DarkTerminusSoil = 2,
    WetTerminusGround = 3,
    FungalLowland = 4,
    CoastalSediment = 5,
    SaltCrust = 6,
    SnowCover = 7,
    BlueIce = 8,
    BlackIceRock = 9,
    ExposedStone = 10,
    IronOxideHighland = 11,
    VegetatedDarkCanopyFloor = 12,
}

impl SurfacePaletteClass {
    pub const COUNT: usize = 13;
    pub const ALL: [Self; Self::COUNT] = [
        Self::ScorchedStone,
        Self::AshDust,
        Self::DarkTerminusSoil,
        Self::WetTerminusGround,
        Self::FungalLowland,
        Self::CoastalSediment,
        Self::SaltCrust,
        Self::SnowCover,
        Self::BlueIce,
        Self::BlackIceRock,
        Self::ExposedStone,
        Self::IronOxideHighland,
        Self::VegetatedDarkCanopyFloor,
    ];

    pub(super) fn classify(
        sample: &RuntimePresentationSample,
        zone: PlanetZone,
        water_state: SurfaceWaterState,
    ) -> Self {
        let coast = coastalness(sample.continentalness);
        let frozen_biome = f64::from(is_frozen_biome(sample.biome));
        let volcanic_biome = f64::from(is_volcanic_biome(sample.biome));
        let canopy_biome = f64::from(is_dark_canopy_biome(sample.biome));
        let wet_biome = f64::from(is_wet_biome(sample.biome));
        let coastal_biome = f64::from(is_coastal_biome(sample.biome));
        let rocky_biome = f64::from(is_rocky_biome(sample.biome));
        let ice_biome = f64::from(is_blue_ice_biome(sample.biome));
        let salt_water = f64::from(matches!(
            water_state,
            SurfaceWaterState::BrineFlat | SurfaceWaterState::EvaporiteBasin
        ));
        let frozen_water = f64::from(matches!(
            water_state,
            SurfaceWaterState::FrozenSea | SurfaceWaterState::IceSheet
        ));

        let scorched_stone = f64::from(zone.is_dayside()) * 1.8
            + score_high(sample.temperature, 52.0, 110.0) * 3.2
            + score_high(sample.aridity, 0.55, 0.95) * 2.5
            + score_low(sample.soil_type, 0.12, 0.45) * 1.2
            + rocky_biome * 0.8
            + score_low(sample.water_table, 0.18, 0.50) * 1.0;

        let ash_dust = volcanic_biome * 3.5
            + f64::from(zone.is_dayside()) * 1.1
            + score_high(sample.temperature, 45.0, 105.0) * 1.6
            + score_high(sample.aridity, 0.45, 0.90) * 1.4
            + score_low(sample.soil_type, 0.18, 0.48) * 0.8;

        let dark_terminus_soil = f64::from(zone.is_terminus()) * 2.8
            + band_score(sample.temperature, -10.0, 2.0, 24.0, 38.0) * 1.4
            + band_score(sample.water_table, 0.08, 0.18, 0.42, 0.62) * 1.5
            + band_score(sample.vegetation_density, 0.05, 0.12, 0.42, 0.60) * 1.0
            + score_high(sample.soil_type, 0.18, 0.62) * 0.8;

        let wet_terminus_ground = f64::from(zone.is_terminus()) * 2.2
            + score_high(sample.humidity, 0.48, 0.88) * 2.0
            + score_high(sample.water_table, 0.36, 0.82) * 2.4
            + wet_biome * 0.9
            + score_low(sample.aridity, 0.18, 0.55) * 0.8;

        let fungal_lowland = f64::from(zone.is_terminus()) * 2.0
            + score_high(sample.humidity, 0.58, 0.94) * 2.2
            + score_high(sample.water_table, 0.42, 0.88) * 2.0
            + wet_biome * 1.2
            + canopy_biome * 0.8
            + score_high(sample.vegetation_density, 0.45, 0.90) * 0.8;

        let coastal_sediment = score_high(coast, 0.38, 0.92) * 3.0
            + coastal_biome * 1.6
            + band_score(sample.water_table, 0.08, 0.18, 0.52, 0.72) * 0.9
            + score_low(sample.snowpack, 0.15, 0.45) * 0.5;

        let salt_crust = salt_water * 3.8
            + score_high(sample.aridity, 0.62, 0.96) * 2.3
            + score_low(sample.vegetation_density, 0.08, 0.30) * 1.0
            + score_low(sample.soil_type, 0.18, 0.48) * 0.6;

        let snow_cover = score_high(sample.snowpack, 0.40, 0.92) * 3.2
            + frozen_biome * 1.4
            + score_low(sample.temperature, -4.0, -42.0) * 1.3
            + f64::from(matches!(sample.biome, TileType::Snow)) * 1.2;

        let blue_ice = frozen_water * 3.5
            + ice_biome * 1.8
            + score_low(sample.temperature, -16.0, -70.0) * 2.0
            + score_high(sample.water_table, 0.18, 0.60) * 0.8
            + f64::from(zone.is_nightside()) * 0.6;

        let black_ice_rock = f64::from(zone.is_nightside()) * 2.0
            + rocky_biome * 1.5
            + score_low(sample.temperature, -18.0, -65.0) * 1.8
            + score_low(sample.snowpack, 0.12, 0.40) * 1.1
            + score_high(sample.rock_hardness, 0.45, 0.88) * 0.9;

        let exposed_stone = rocky_biome * 1.6
            + score_high(sample.rock_hardness, 0.48, 0.92) * 2.0
            + score_high(sample.heightmap, 0.12, 0.55) * 1.4
            + score_low(sample.soil_type, 0.16, 0.48) * 1.0;

        let iron_oxide_highland = f64::from(zone.is_dayside()) * 1.4
            + rocky_biome * 1.2
            + score_high(sample.rock_hardness, 0.45, 0.88) * 1.5
            + score_high(sample.aridity, 0.45, 0.90) * 2.0
            + score_high(sample.heightmap, 0.08, 0.45) * 1.4;

        let vegetated_dark_canopy_floor = canopy_biome * 2.4
            + score_high(sample.vegetation_density, 0.52, 0.92) * 2.2
            + score_high(sample.soil_type, 0.32, 0.78) * 1.0
            + f64::from(zone.is_terminus()) * 0.9
            + score_low(sample.aridity, 0.18, 0.58) * 0.8;

        let scores = [
            (Self::ScorchedStone, scorched_stone),
            (Self::AshDust, ash_dust),
            (Self::DarkTerminusSoil, dark_terminus_soil),
            (Self::WetTerminusGround, wet_terminus_ground),
            (Self::FungalLowland, fungal_lowland),
            (Self::CoastalSediment, coastal_sediment),
            (Self::SaltCrust, salt_crust),
            (Self::SnowCover, snow_cover),
            (Self::BlueIce, blue_ice),
            (Self::BlackIceRock, black_ice_rock),
            (Self::ExposedStone, exposed_stone),
            (Self::IronOxideHighland, iron_oxide_highland),
            (Self::VegetatedDarkCanopyFloor, vegetated_dark_canopy_floor),
        ];

        scores
            .into_iter()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(palette, _)| palette)
            .unwrap_or(Self::ExposedStone)
    }

    pub fn as_index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScorchedStone => "ScorchedStone",
            Self::AshDust => "AshDust",
            Self::DarkTerminusSoil => "DarkTerminusSoil",
            Self::WetTerminusGround => "WetTerminusGround",
            Self::FungalLowland => "FungalLowland",
            Self::CoastalSediment => "CoastalSediment",
            Self::SaltCrust => "SaltCrust",
            Self::SnowCover => "SnowCover",
            Self::BlueIce => "BlueIce",
            Self::BlackIceRock => "BlackIceRock",
            Self::ExposedStone => "ExposedStone",
            Self::IronOxideHighland => "IronOxideHighland",
            Self::VegetatedDarkCanopyFloor => "VegetatedDarkCanopyFloor",
        }
    }
}

fn is_dark_canopy_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::Forest
            | TileType::DeciduousForest
            | TileType::TemperateRainforest
            | TileType::SubtropicalForest
            | TileType::CloudForest
            | TileType::Jungle
            | TileType::Woodland
            | TileType::DryWoodland
            | TileType::Taiga
            | TileType::Oasis
    )
}

fn is_wet_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::Marsh
            | TileType::Mangrove
            | TileType::FrozenBog
            | TileType::TemperateRainforest
            | TileType::CloudForest
            | TileType::Jungle
            | TileType::River
    )
}

fn is_coastal_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::Beach
            | TileType::Mangrove
            | TileType::RockyCoast
            | TileType::SeaCliff
            | TileType::ContinentalShelf
            | TileType::ShallowSea
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

fn is_blue_ice_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::Glacier | TileType::IceSheet | TileType::White
    )
}

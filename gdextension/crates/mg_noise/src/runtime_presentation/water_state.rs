use super::{
    coastalness, is_frozen_biome, score_high, score_low, PlanetZone, RuntimePresentationSample,
};
use serde::{Deserialize, Serialize};

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceWaterState {
    None = 0,
    LiquidSea = 1,
    LiquidCoast = 2,
    FrozenSea = 3,
    IceSheet = 4,
    BrineFlat = 5,
    EvaporiteBasin = 6,
    MeltwaterChannel = 7,
    LiquidRiver = 8,
    FrozenRiver = 9,
    MarshWater = 10,
}

impl SurfaceWaterState {
    pub const COUNT: usize = 11;
    pub const ALL: [Self; Self::COUNT] = [
        Self::None,
        Self::LiquidSea,
        Self::LiquidCoast,
        Self::FrozenSea,
        Self::IceSheet,
        Self::BrineFlat,
        Self::EvaporiteBasin,
        Self::MeltwaterChannel,
        Self::LiquidRiver,
        Self::FrozenRiver,
        Self::MarshWater,
    ];

    pub(super) fn classify(sample: &RuntimePresentationSample, zone: PlanetZone) -> Self {
        let coast = coastalness(sample.continentalness);
        let frozen_biome = is_frozen_biome(sample.biome);
        let has_river = sample.rivers > 0.06;
        let has_surface_water = sample.water_table > 0.42;

        if sample.is_ocean {
            if zone.is_dayside() || sample.temperature > 38.0 {
                return if sample.aridity > 0.72 || score_high(coast, 0.35, 0.80) > 0.5 {
                    Self::EvaporiteBasin
                } else {
                    Self::BrineFlat
                };
            }
            if zone.is_nightside()
                || sample.temperature < -18.0
                || sample.snowpack > 0.35
                || frozen_biome
            {
                return if coast > 0.55 {
                    Self::IceSheet
                } else {
                    Self::FrozenSea
                };
            }
            return if coast > 0.60 {
                Self::LiquidCoast
            } else {
                Self::LiquidSea
            };
        }

        let hot_closed_basin = zone.is_dayside()
            && sample.continentalness < 0.0
            && sample.temperature > 35.0
            && sample.aridity > 0.72;
        if hot_closed_basin {
            return if sample.water_table > 0.52 {
                Self::BrineFlat
            } else {
                Self::EvaporiteBasin
            };
        }

        if has_river {
            if sample.temperature < -8.0 || sample.snowpack > 0.28 || frozen_biome {
                return Self::FrozenRiver;
            }
            return Self::LiquidRiver;
        }

        if has_surface_water && sample.aridity > 0.78 {
            return if sample.water_table > 0.65 {
                Self::BrineFlat
            } else {
                Self::EvaporiteBasin
            };
        }

        if has_surface_water
            && sample.humidity > 0.62
            && sample.temperature > -4.0
            && sample.temperature < 18.0
        {
            return Self::MarshWater;
        }

        if sample.snowpack > 0.48
            && sample.water_table > 0.25
            && score_low(sample.temperature, -2.0, -18.0) < 0.7
        {
            return Self::MeltwaterChannel;
        }

        Self::None
    }

    pub fn as_index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::LiquidSea => "LiquidSea",
            Self::LiquidCoast => "LiquidCoast",
            Self::FrozenSea => "FrozenSea",
            Self::IceSheet => "IceSheet",
            Self::BrineFlat => "BrineFlat",
            Self::EvaporiteBasin => "EvaporiteBasin",
            Self::MeltwaterChannel => "MeltwaterChannel",
            Self::LiquidRiver => "LiquidRiver",
            Self::FrozenRiver => "FrozenRiver",
            Self::MarshWater => "MarshWater",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SurfaceWaterState;
    use crate::runtime_presentation::{PlanetZone, RuntimePresentationSample};
    use mg_core::TileType;

    #[test]
    fn hot_closed_dayside_basins_stay_evaporite_without_ocean_flag() {
        let sample = RuntimePresentationSample {
            biome: TileType::SaltFlat,
            continentalness: -0.08,
            heightmap: -0.05,
            temperature: 58.0,
            humidity: 0.18,
            light_level: 0.68,
            rivers: 0.0,
            aridity: 0.91,
            snowpack: 0.0,
            water_table: 0.38,
            vegetation_density: 0.0,
            soil_type: 0.12,
            rock_hardness: 0.42,
            tectonic: 0.35,
            erosion: 0.22,
            peaks_valleys: 0.04,
            slope: 0.06,
            local_relief: 0.08,
            curvature: 0.02,
            is_ocean: false,
        };

        assert_eq!(
            SurfaceWaterState::classify(&sample, PlanetZone::DryDaysideMargin),
            SurfaceWaterState::EvaporiteBasin
        );
    }
}

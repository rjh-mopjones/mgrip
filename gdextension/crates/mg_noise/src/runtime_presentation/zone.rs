use super::{
    band_score, coastalness, is_frozen_biome, score_high, score_low, RuntimePresentationSample,
};
use serde::{Deserialize, Serialize};

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanetZone {
    SubstellarInferno = 0,
    ScorchBelt = 1,
    DryDaysideMargin = 2,
    InnerTerminus = 3,
    OuterTerminus = 4,
    ColdTerminus = 5,
    FrostMargin = 6,
    FrozenCoast = 7,
    DeepNightIce = 8,
    AbyssalNight = 9,
}

impl PlanetZone {
    pub const COUNT: usize = 10;
    pub const ALL: [Self; Self::COUNT] = [
        Self::SubstellarInferno,
        Self::ScorchBelt,
        Self::DryDaysideMargin,
        Self::InnerTerminus,
        Self::OuterTerminus,
        Self::ColdTerminus,
        Self::FrostMargin,
        Self::FrozenCoast,
        Self::DeepNightIce,
        Self::AbyssalNight,
    ];

    pub(super) fn classify(sample: &RuntimePresentationSample) -> Self {
        let light = sample.light_level;
        let temp = sample.temperature;
        let humidity = sample.humidity;
        let aridity = sample.aridity;
        let snowpack = sample.snowpack;
        let water_table = sample.water_table;
        let coast = coastalness(sample.continentalness);
        let frozen_biome = f64::from(is_frozen_biome(sample.biome));

        let inferno = score_high(light, 0.78, 0.96) * 4.0
            + score_high(temp, 55.0, 110.0) * 4.0
            + score_high(aridity, 0.65, 0.95) * 2.0
            + score_low(humidity, 0.15, 0.45) * 1.0
            + score_low(snowpack, 0.05, 0.20) * 1.0;

        let scorch = band_score(light, 0.62, 0.72, 0.88, 0.98) * 4.0
            + score_high(temp, 30.0, 75.0) * 3.0
            + score_high(aridity, 0.45, 0.85) * 2.0
            + score_low(humidity, 0.20, 0.55) * 1.0;

        let dry_dayside = band_score(light, 0.42, 0.54, 0.74, 0.88) * 3.0
            + score_high(aridity, 0.38, 0.80) * 2.5
            + band_score(temp, 5.0, 15.0, 55.0, 75.0) * 2.0
            + score_low(humidity, 0.20, 0.60) * 1.0;

        let inner_terminus = band_score(light, 0.28, 0.36, 0.52, 0.62) * 4.0
            + band_score(temp, -5.0, 5.0, 28.0, 40.0) * 2.0
            + score_high(humidity, 0.30, 0.75) * 1.5
            + score_high(water_table, 0.20, 0.65) * 1.0;

        let outer_terminus = band_score(light, 0.18, 0.26, 0.42, 0.52) * 4.0
            + band_score(temp, -18.0, -5.0, 14.0, 26.0) * 2.0
            + band_score(humidity, 0.18, 0.28, 0.62, 0.78) * 1.0
            + score_high(water_table, 0.18, 0.55) * 0.8;

        let cold_terminus = band_score(light, 0.10, 0.16, 0.30, 0.40) * 3.0
            + band_score(temp, -42.0, -26.0, -2.0, 10.0) * 3.0
            + score_high(snowpack, 0.18, 0.62) * 2.0
            + frozen_biome * 1.0;

        let frost_margin = band_score(light, 0.05, 0.08, 0.18, 0.26) * 2.5
            + score_low(temp, -18.0, -58.0) * 3.0
            + score_high(snowpack, 0.30, 0.85) * 2.0
            + frozen_biome * 1.0;

        let frozen_coast = score_low(light, 0.10, 0.24) * 2.0
            + score_low(temp, -10.0, -55.0) * 2.5
            + score_high(coast, 0.45, 0.95) * 3.0
            + score_high(water_table, 0.18, 0.55) * 1.0
            + frozen_biome * 1.0;

        let deep_night_ice = score_low(light, 0.05, 0.14) * 4.0
            + score_low(temp, -22.0, -70.0) * 3.0
            + score_high(snowpack, 0.40, 0.95) * 2.0
            + score_high(water_table, 0.15, 0.55) * 1.0
            + frozen_biome * 1.0;

        let abyssal_night = score_low(light, 0.02, 0.08) * 4.0
            + score_low(temp, -35.0, -85.0) * 3.0
            + score_low(water_table, 0.20, 0.60) * 1.0
            + score_low(coast, 0.25, 0.70) * 1.0
            + score_high(aridity, 0.20, 0.65) * 1.0;

        let scores = [
            (Self::SubstellarInferno, inferno),
            (Self::ScorchBelt, scorch),
            (Self::DryDaysideMargin, dry_dayside),
            (Self::InnerTerminus, inner_terminus),
            (Self::OuterTerminus, outer_terminus),
            (Self::ColdTerminus, cold_terminus),
            (Self::FrostMargin, frost_margin),
            (Self::FrozenCoast, frozen_coast),
            (Self::DeepNightIce, deep_night_ice),
            (Self::AbyssalNight, abyssal_night),
        ];

        scores
            .into_iter()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(zone, _)| zone)
            .unwrap_or(Self::InnerTerminus)
    }

    pub fn as_index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SubstellarInferno => "SubstellarInferno",
            Self::ScorchBelt => "ScorchBelt",
            Self::DryDaysideMargin => "DryDaysideMargin",
            Self::InnerTerminus => "InnerTerminus",
            Self::OuterTerminus => "OuterTerminus",
            Self::ColdTerminus => "ColdTerminus",
            Self::FrostMargin => "FrostMargin",
            Self::FrozenCoast => "FrozenCoast",
            Self::DeepNightIce => "DeepNightIce",
            Self::AbyssalNight => "AbyssalNight",
        }
    }

    pub fn is_dayside(self) -> bool {
        matches!(
            self,
            Self::SubstellarInferno | Self::ScorchBelt | Self::DryDaysideMargin
        )
    }

    pub fn is_nightside(self) -> bool {
        matches!(
            self,
            Self::FrostMargin | Self::FrozenCoast | Self::DeepNightIce | Self::AbyssalNight
        )
    }

    pub fn is_terminus(self) -> bool {
        matches!(
            self,
            Self::InnerTerminus | Self::OuterTerminus | Self::ColdTerminus
        )
    }
}

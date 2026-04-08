use super::{
    band_score, is_frozen_biome, is_volcanic_biome, score_high, score_low, PlanetZone,
    RuntimePresentationSample,
};
use serde::{Deserialize, Serialize};

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtmosphereClass {
    BlastedRadiance = 0,
    HarshAmberHaze = 1,
    DryTwilight = 2,
    TemperateTwilight = 3,
    WetTwilight = 4,
    FrostTwilight = 5,
    PolarGlow = 6,
    BlackIceDark = 7,
    GeothermalNight = 8,
}

impl AtmosphereClass {
    pub const COUNT: usize = 9;
    pub const ALL: [Self; Self::COUNT] = [
        Self::BlastedRadiance,
        Self::HarshAmberHaze,
        Self::DryTwilight,
        Self::TemperateTwilight,
        Self::WetTwilight,
        Self::FrostTwilight,
        Self::PolarGlow,
        Self::BlackIceDark,
        Self::GeothermalNight,
    ];

    pub(super) fn classify(sample: &RuntimePresentationSample, zone: PlanetZone) -> Self {
        let light = sample.light_level;
        let temp = sample.temperature;
        let humidity = sample.humidity;
        let water_table = sample.water_table;
        let snowpack = sample.snowpack;
        let frozen_biome = f64::from(is_frozen_biome(sample.biome));
        let volcanic_biome = f64::from(is_volcanic_biome(sample.biome));
        let nightside = f64::from(zone.is_nightside());

        let blasted = score_high(light, 0.75, 0.96) * 4.0
            + score_high(temp, 50.0, 105.0) * 3.0
            + score_low(humidity, 0.18, 0.42) * 1.5
            + score_low(snowpack, 0.05, 0.20) * 1.0
            + f64::from(zone == PlanetZone::SubstellarInferno) * 1.5;

        let amber = band_score(light, 0.55, 0.68, 0.88, 0.98) * 3.0
            + score_high(temp, 22.0, 70.0) * 2.0
            + score_high(sample.aridity, 0.35, 0.80) * 2.0
            + f64::from(zone == PlanetZone::ScorchBelt || zone == PlanetZone::DryDaysideMargin)
                * 1.0;

        let dry_twilight = band_score(light, 0.22, 0.30, 0.52, 0.64) * 3.0
            + score_low(humidity, 0.25, 0.55) * 1.5
            + score_high(sample.aridity, 0.28, 0.70) * 1.5
            + band_score(temp, -4.0, 4.0, 26.0, 40.0) * 1.0;

        let temperate = band_score(light, 0.20, 0.28, 0.48, 0.60) * 3.0
            + band_score(temp, -6.0, 2.0, 22.0, 34.0) * 2.0
            + band_score(humidity, 0.22, 0.35, 0.62, 0.78) * 1.5
            + score_high(water_table, 0.12, 0.48) * 0.8;

        let wet = band_score(light, 0.22, 0.28, 0.46, 0.58) * 2.5
            + score_high(humidity, 0.45, 0.88) * 2.5
            + score_high(water_table, 0.35, 0.80) * 1.8
            + score_low(sample.aridity, 0.20, 0.60) * 1.0;

        let frost = band_score(light, 0.08, 0.14, 0.28, 0.40) * 2.5
            + band_score(temp, -46.0, -24.0, -2.0, 10.0) * 2.5
            + score_high(snowpack, 0.18, 0.70) * 2.0
            + frozen_biome * 1.0
            + f64::from(zone == PlanetZone::ColdTerminus || zone == PlanetZone::FrostMargin) * 1.0;

        let polar = score_low(light, 0.08, 0.18) * 2.5
            + score_low(temp, -16.0, -58.0) * 2.0
            + score_high(snowpack, 0.35, 0.90) * 2.5
            + score_high(water_table, 0.15, 0.55) * 0.8
            + f64::from(zone == PlanetZone::FrozenCoast || zone == PlanetZone::DeepNightIce) * 1.0;

        let black_ice = score_low(light, 0.03, 0.10) * 4.0
            + score_low(temp, -28.0, -80.0) * 2.0
            + score_high(nightside, 0.5, 1.0) * 1.5
            + score_low(water_table, 0.15, 0.55) * 1.0;

        let geothermal = score_low(light, 0.03, 0.10) * 3.0
            + band_score(temp, -18.0, -6.0, 16.0, 30.0) * 2.0
            + score_high(water_table, 0.20, 0.65) * 1.0
            + volcanic_biome * 2.0
            + nightside * 1.0;

        let scores = [
            (Self::BlastedRadiance, blasted),
            (Self::HarshAmberHaze, amber),
            (Self::DryTwilight, dry_twilight),
            (Self::TemperateTwilight, temperate),
            (Self::WetTwilight, wet),
            (Self::FrostTwilight, frost),
            (Self::PolarGlow, polar),
            (Self::BlackIceDark, black_ice),
            (Self::GeothermalNight, geothermal),
        ];

        scores
            .into_iter()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(atmosphere, _)| atmosphere)
            .unwrap_or(Self::TemperateTwilight)
    }

    pub fn as_index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BlastedRadiance => "BlastedRadiance",
            Self::HarshAmberHaze => "HarshAmberHaze",
            Self::DryTwilight => "DryTwilight",
            Self::TemperateTwilight => "TemperateTwilight",
            Self::WetTwilight => "WetTwilight",
            Self::FrostTwilight => "FrostTwilight",
            Self::PolarGlow => "PolarGlow",
            Self::BlackIceDark => "BlackIceDark",
            Self::GeothermalNight => "GeothermalNight",
        }
    }
}

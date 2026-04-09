use mg_core::TileType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimateClass {
    Frozen,
    Cold,
    Temperate,
    Warm,
    Hot,
    Scorching,
}

impl ClimateClass {
    pub fn from_temperature(temp: f64) -> Self {
        if temp < -20.0 {
            Self::Frozen
        } else if temp < 3.0 {
            Self::Cold
        } else if temp < 35.0 {
            Self::Temperate
        } else if temp < 55.0 {
            Self::Warm
        } else if temp < 80.0 {
            Self::Hot
        } else {
            Self::Scorching
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoistureClass {
    Arid,
    Dry,
    Moderate,
    Humid,
    Saturated,
}

impl MoistureClass {
    pub fn from_humidity(h: f64) -> Self {
        if h < 0.2 {
            Self::Arid
        } else if h < 0.4 {
            Self::Dry
        } else if h < 0.6 {
            Self::Moderate
        } else if h < 0.8 {
            Self::Humid
        } else {
            Self::Saturated
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElevationClass {
    Coastal,
    Lowland,
    Upland,
    Highland,
    Alpine,
}

impl ElevationClass {
    pub fn from_elevation(above_sea: f64) -> Self {
        if above_sea < 0.04 {
            Self::Coastal
        } else if above_sea < 0.12 {
            Self::Lowland
        } else if above_sea < 0.25 {
            Self::Upland
        } else if above_sea < 0.38 {
            Self::Highland
        } else {
            Self::Alpine
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainClass {
    Flat,
    Rolling,
    Rugged,
}

impl TerrainClass {
    /// High erosion = flat; low erosion = rugged peaks preserved.
    pub fn from_erosion(erosion: f64) -> Self {
        if erosion < 0.3 {
            Self::Rugged
        } else if erosion < 0.7 {
            Self::Rolling
        } else {
            Self::Flat
        }
    }
}

pub struct BiomeSplines {
    sea_level: f64,
}

impl BiomeSplines {
    pub fn new(sea_level: f64) -> Self {
        Self { sea_level }
    }

    pub fn evaluate_with_light(
        &self,
        continentalness: f64,
        temperature: f64,
        tectonic: f64,
        erosion: f64,
        peaks_valleys: f64,
        humidity: f64,
        aridity: f64,
        rock_hardness: f64,
        light_level: f64,
    ) -> TileType {
        let raw_elevation =
            self.compute_elevation(continentalness, peaks_valleys, erosion, tectonic);
        let coast_perturb = (rock_hardness - 0.5) * 0.10 + peaks_valleys * 0.05;
        let dist_to_coast = (raw_elevation - self.sea_level).abs();
        let coast_fade = (1.0 - dist_to_coast * 10.0).clamp(0.0, 1.0);
        let elevation = raw_elevation + coast_perturb * coast_fade;

        if elevation < self.sea_level {
            return self.below_sea_biome(elevation, temperature, tectonic, light_level);
        }

        let adjusted_humidity = self.adjust_humidity(humidity, elevation);

        let climate = ClimateClass::from_temperature(temperature);
        let mut moisture = MoistureClass::from_humidity(adjusted_humidity);
        let above_sea = elevation - self.sea_level;
        let elev_class = ElevationClass::from_elevation(above_sea);
        let terrain = TerrainClass::from_erosion(erosion);

        // Temperature gate — above 45°C (fuzzy ±3°C via rock_hardness), no vegetation
        let gate_temp = 45.0 + (rock_hardness - 0.5) * 6.0;
        if temperature > gate_temp {
            moisture = MoistureClass::Arid;
        } else if aridity > 0.75 {
            moisture = MoistureClass::Arid;
        } else if aridity > 0.6 {
            if matches!(
                moisture,
                MoistureClass::Moderate | MoistureClass::Humid | MoistureClass::Saturated
            ) {
                moisture = MoistureClass::Dry;
            }
        }

        // Coastal zone with rock-hardness-modulated width
        let coast_width = 0.01 + (1.0 - rock_hardness) * 0.02 + peaks_valleys.abs() * 0.005;
        if above_sea < coast_width {
            return match climate {
                ClimateClass::Frozen => TileType::Glacier,
                ClimateClass::Cold => TileType::Snow,
                ClimateClass::Scorching => TileType::SaltFlat,
                ClimateClass::Warm | ClimateClass::Hot
                    if matches!(moisture, MoistureClass::Humid | MoistureClass::Saturated) =>
                {
                    TileType::Mangrove
                }
                _ if rock_hardness > 0.7 && peaks_valleys.abs() > 0.2 => TileType::SeaCliff,
                _ if peaks_valleys.abs() > 0.3 || rock_hardness > 0.6 => TileType::RockyCoast,
                _ => TileType::Beach,
            };
        }

        if above_sea >= coast_width
            && above_sea < coast_width + 0.03
            && terrain == TerrainClass::Rugged
        {
            return TileType::SeaCliff;
        }

        self.land_biome(climate, moisture, elev_class, terrain, rock_hardness)
    }

    fn compute_elevation(&self, cont: f64, pv: f64, erosion: f64, _tectonic: f64) -> f64 {
        let is_land = cont >= self.sea_level;
        let erosion_damp = 1.0 - erosion * 0.7;
        let peak_height = if is_land {
            pv.max(0.0) * 0.25 * erosion_damp
        } else {
            0.0
        };
        let valley_depth = if is_land {
            pv.min(0.0).abs() * 0.12
        } else {
            0.0
        };
        cont + peak_height - valley_depth
    }

    fn ocean_biome(&self, elevation: f64, temp: f64, tectonic: f64) -> TileType {
        if temp < -15.0 {
            return TileType::White;
        }
        if temp > 80.0 {
            return TileType::SaltFlat;
        }
        let depth = self.sea_level - elevation;
        if tectonic < 0.2 && depth > 0.3 {
            return TileType::OceanTrench;
        }
        if tectonic < 0.3 && depth > 0.1 {
            return TileType::OceanRidge;
        }
        if depth < 0.05 {
            TileType::ShallowSea
        } else if depth < 0.15 {
            TileType::ContinentalShelf
        } else if depth > 0.25 {
            TileType::DeepOcean
        } else {
            TileType::Sea
        }
    }

    fn below_sea_biome(&self, elevation: f64, temp: f64, tectonic: f64, light_level: f64) -> TileType {
        let depth = self.sea_level - elevation;

        if light_level < 0.18 || temp < -12.0 {
            return if depth > 0.12 || temp < -35.0 {
                TileType::White
            } else {
                TileType::IceSheet
            };
        }

        if light_level > 0.58 || temp > 35.0 {
            return if temp > 95.0 || depth > 0.16 {
                TileType::ScorchedRock
            } else {
                TileType::SaltFlat
            };
        }

        self.ocean_biome(elevation, temp, tectonic)
    }

    fn adjust_humidity(&self, humidity: f64, elevation: f64) -> f64 {
        let above_sea = (elevation - self.sea_level).max(0.0);
        let rain_shadow = if above_sea > 0.15 {
            ((above_sea - 0.15) * 2.5).min(0.4)
        } else {
            0.0
        };
        (humidity - rain_shadow).clamp(0.0, 1.0)
    }

    fn land_biome(
        &self,
        climate: ClimateClass,
        moisture: MoistureClass,
        elevation: ElevationClass,
        terrain: TerrainClass,
        rock_hardness: f64,
    ) -> TileType {
        use ClimateClass::*;
        use ElevationClass::*;
        use MoistureClass::*;
        use TerrainClass::*;

        match climate {
            Frozen => match (moisture, elevation) {
                (_, Alpine) => TileType::Glacier,
                (Arid | Dry, Highland) => TileType::Glacier,
                (Arid | Dry, _) => TileType::IceSheet,
                (Saturated, Lowland | Coastal) => TileType::FrozenBog,
                _ => TileType::Snow,
            },
            Cold => match (moisture, elevation, terrain) {
                (_, Alpine, _) => TileType::Snow,
                (_, Highland, Rugged) => TileType::Mountain,
                (_, Highland, _) => TileType::AlpineMeadow,
                (Arid | Dry, _, _) => TileType::Tundra,
                (Saturated, Lowland | Coastal, _) => TileType::FrozenBog,
                (Humid | Saturated, _, _) => TileType::Taiga,
                (Moderate, _, _) => TileType::Tundra,
            },
            Temperate => match (moisture, elevation, terrain) {
                (_, Alpine, _) => TileType::Mountain,
                (_, Highland, Rugged) => TileType::Mountain,
                (Humid | Saturated, Highland, _) => TileType::Plateau,
                (_, Highland, _) => TileType::Plateau,
                (Arid, _, Rugged) => TileType::Scrubland,
                (Arid, _, _) => TileType::Steppe,
                (Dry, _, Rugged) => TileType::Scrubland,
                (Dry, Upland, _) => TileType::Woodland,
                (Dry, _, _) => TileType::Steppe,
                (Saturated, Lowland | Coastal, _) => TileType::Marsh,
                (Saturated, _, _) => TileType::TemperateRainforest,
                (Humid, Lowland, Flat) => TileType::Meadow,
                (Humid, Lowland | Coastal, _) => TileType::DeciduousForest,
                (Humid, _, _) => TileType::DeciduousForest,
                (Moderate, Lowland, Flat) => TileType::Meadow,
                (Moderate, Lowland, _) => TileType::Plains,
                (Moderate, Upland, _) => TileType::Woodland,
                (Moderate, _, _) => TileType::Plains,
            },
            Warm => match (moisture, elevation, terrain) {
                (_, Alpine, _) => TileType::Mountain,
                (_, Highland, Rugged) => TileType::Mountain,
                (Humid | Saturated, Highland, _) => TileType::CloudForest,
                (Moderate | Dry, Highland, _) => TileType::HighlandSavanna,
                (Arid, Highland, _) => TileType::Badlands,
                (Arid, _, Rugged) => TileType::Badlands,
                (Arid, _, _) => TileType::Desert,
                (Dry, _, Rugged) => TileType::Thornland,
                (Dry, Upland, _) => TileType::DryWoodland,
                (Dry, _, _) => TileType::Savanna,
                (Saturated, Lowland | Coastal, _) => TileType::Marsh,
                (Saturated, _, _) => TileType::SubtropicalForest,
                (Humid, Lowland | Coastal, _) => TileType::SubtropicalForest,
                (Humid, _, _) => TileType::SubtropicalForest,
                (Moderate, Upland, _) => TileType::DryWoodland,
                (Moderate, _, _) => TileType::Savanna,
            },
            Hot => match (moisture, elevation, terrain) {
                (Humid | Saturated, Highland | Alpine, _) => TileType::CloudForest,
                (Moderate, Highland | Alpine, _) => TileType::HighlandSavanna,
                (_, Alpine, _) => TileType::ScorchedRock,
                (Arid, Highland, Rugged) => TileType::Badlands,
                (Arid, _, Rugged) => {
                    if rock_hardness > 0.6 {
                        TileType::ScorchedRock
                    } else {
                        TileType::Badlands
                    }
                }
                (Arid, _, Flat) => {
                    if rock_hardness > 0.6 {
                        TileType::Hamada
                    } else {
                        TileType::Erg
                    }
                }
                (Arid, _, _) => {
                    if rock_hardness > 0.6 {
                        TileType::Hamada
                    } else {
                        TileType::Sahara
                    }
                }
                (Dry, _, Rugged) => TileType::Hamada,
                (Dry, _, _) => TileType::Desert,
                (Moderate, _, _) => TileType::Savanna,
                (Humid | Saturated, _, _) => TileType::Jungle,
            },
            Scorching => match (moisture, elevation, terrain) {
                (_, Alpine | Highland, _) => TileType::ScorchedRock,
                (Arid, _, Flat) => {
                    if rock_hardness < 0.4 {
                        TileType::Erg
                    } else {
                        TileType::SaltFlat
                    }
                }
                (Arid, _, Rugged) => {
                    if rock_hardness > 0.6 {
                        TileType::ScorchedRock
                    } else {
                        TileType::MoltenWaste
                    }
                }
                (Arid, _, _) => {
                    if rock_hardness > 0.6 {
                        TileType::Hamada
                    } else {
                        TileType::Sahara
                    }
                }
                (Dry, _, Rugged) => TileType::Hamada,
                (Dry, _, _) => TileType::Erg,
                _ => TileType::Desert,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BiomeSplines;
    use mg_core::TileType;

    #[test]
    fn below_sea_extremities_do_not_default_to_marine_tiles() {
        let splines = BiomeSplines::new(0.0);

        let nightside = splines.evaluate_with_light(-0.18, -42.0, 0.2, 0.0, 0.0, 0.4, 0.2, 0.5, 0.04);
        let dayside = splines.evaluate_with_light(-0.08, 58.0, 0.2, 0.0, 0.0, 0.1, 0.9, 0.5, 0.72);

        assert_eq!(nightside, TileType::White);
        assert_eq!(dayside, TileType::SaltFlat);
    }

    #[test]
    fn below_sea_terminus_can_still_emit_marine_tiles() {
        let splines = BiomeSplines::new(0.0);
        let terminus = splines.evaluate_with_light(-0.08, 12.0, 0.4, 0.0, 0.0, 0.5, 0.3, 0.5, 0.34);

        assert!(matches!(
            terminus,
            TileType::ShallowSea
                | TileType::ContinentalShelf
                | TileType::Sea
                | TileType::DeepOcean
                | TileType::OceanTrench
                | TileType::OceanRidge
        ));
    }
}

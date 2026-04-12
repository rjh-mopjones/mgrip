//! Composited terrain renderer (ported from Randlebrot).
//!
//! Replaces flat biome colors with a multi-layer composited image using
//! heightmap, erosion, snowpack, rivers, vegetation, and lighting data.

use mg_core::TileType;
use crate::biome_map::{BiomeMap, SEA_LEVEL, compute_slope_grid};

/// Global heightmap statistics for consistent normalization across tiles.
#[derive(Clone, Debug)]
pub struct NormalizationHints {
    pub heightmap_min: f64,
    pub heightmap_max: f64,
}

/// Render a fully composited terrain image from all BiomeMap layers.
/// Returns RGBA bytes (width * height * 4).
pub fn render_terrain(map: &BiomeMap, hints: Option<&NormalizationHints>) -> Vec<u8> {
    let w = map.width;
    let h = map.height;
    let mut data = Vec::with_capacity(w * h * 4);

    let light_dir = normalize([1.0, -1.0, 2.0]);

    let norm_height = {
        let (hmin, hmax) = if let Some(hints) = hints {
            (hints.heightmap_min, hints.heightmap_max)
        } else {
            let mut hmin = f64::MAX;
            let mut hmax = f64::MIN;
            for &v in &map.heightmap {
                if v < hmin { hmin = v; }
                if v > hmax { hmax = v; }
            }
            (hmin, hmax)
        };
        let range = if hmax > hmin { hmax - hmin } else { 1.0 };
        map.heightmap.iter().map(|&v| ((v - hmin) / range).clamp(0.0, 1.0)).collect::<Vec<f64>>()
    };

    let slope = compute_slope_grid(&norm_height, w, h);

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let cont = map.continentalness[idx];
            let biome = map.biomes[idx];

            let [r, g, b] = if is_water_biome(biome) && cont < SEA_LEVEL {
                let mut ocean_pixel = render_ocean(
                    biome,
                    cont,
                    map.light_level[idx],
                    map.temperature[idx],
                );
                // River-into-ocean confluence: where a river segment runs into
                // an ocean cell, blend the river corridor color over the ocean
                // pixel so the river visibly meets the sea instead of stopping
                // at the shoreline. Strength scales with the river field so
                // only true mouths show through.
                let river_here = map.rivers[idx];
                if river_here > 0.02 {
                    let river_col = solid_river_color(
                        map.temperature[idx],
                        map.light_level[idx],
                        map.aridity[idx],
                    );
                    let blend = (river_here / 0.5).clamp(0.0, 1.0) * 0.7;
                    ocean_pixel = lerp_rgb(ocean_pixel, river_col, blend);
                }
                ocean_pixel
            } else if biome == TileType::River {
                // River biome cells are surface water — render the corridor
                // colour solid instead of blending it over land tinting. This
                // is the difference between a clear blue river and a tan-tinted
                // smear at low river strength.
                solid_river_color(
                    map.temperature[idx],
                    map.light_level[idx],
                    map.aridity[idx],
                )
            } else {
                let mut pixel = biome.rgb();

                // Sub-biome tinting for highland biomes
                if matches!(biome,
                    TileType::Mountain | TileType::Plateau | TileType::Badlands
                    | TileType::Hamada | TileType::ScorchedRock | TileType::AlpineMeadow
                ) {
                    let rock = map.rock_hardness[idx];
                    let eros = map.erosion[idx];
                    let temp = map.temperature[idx];
                    let soil = map.soil_type[idx];
                    let sl = slope[idx];
                    let hn = norm_height[idx];
                    let tect = map.tectonic[idx];

                    pixel = lerp_rgb(pixel, [160, 130, 100], (1.0 - rock) * 0.25);
                    if eros > 0.1 {
                        pixel = lerp_rgb(pixel, [180, 150, 110], (eros * 0.4).min(0.3));
                    }
                    if temp > 50.0 {
                        let heat = ((temp - 50.0) / 40.0).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [170, 100, 70], heat * 0.25);
                    } else if temp < 10.0 {
                        let cold = ((10.0 - temp) / 30.0).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [130, 140, 160], cold * 0.2);
                    }
                    if soil > 0.2 {
                        pixel = lerp_rgb(pixel, [126, 126, 94], (soil - 0.2) * 0.3);
                    }
                    if sl > 0.02 {
                        let ridge = ((sl - 0.02) / 0.08).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [60, 55, 50], ridge * 0.3);
                    }
                    if hn > 0.7 {
                        let alt = ((hn - 0.7) / 0.3).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, [190, 195, 200], alt * 0.2);
                    }
                    let stress = 1.0 - tect;
                    if stress > 0.5 {
                        pixel = lerp_rgb(pixel, [150, 100, 80], ((stress - 0.5) / 0.5).clamp(0.0, 1.0) * 0.15);
                    } else if stress < 0.2 {
                        pixel = lerp_rgb(pixel, [140, 150, 165], ((0.2 - stress) / 0.2).clamp(0.0, 1.0) * 0.15);
                    }
                }

                // Sub-biome tinting for desert biomes
                if is_desert_biome(biome) {
                    let rock = map.rock_hardness[idx];
                    let temp = map.temperature[idx];
                    let eros = map.erosion[idx];
                    let arid = map.aridity[idx];

                    if rock > 0.6 { pixel = lerp_rgb(pixel, [140, 90, 60], (rock - 0.6) * 0.5); }
                    else if rock < 0.4 { pixel = lerp_rgb(pixel, [230, 210, 170], (0.4 - rock) * 0.4); }
                    if temp > 80.0 {
                        pixel = lerp_rgb(pixel, [200, 120, 60], ((temp - 80.0) / 40.0).clamp(0.0, 1.0) * 0.2);
                    }
                    if eros > 0.4 {
                        pixel = lerp_rgb(pixel, [185, 110, 75], ((eros - 0.4) / 0.4).clamp(0.0, 1.0) * 0.2);
                    }
                    if !map.drainage_area.is_empty() {
                        let drain = (map.drainage_area[idx] as f64 / 500.0).clamp(0.0, 1.0);
                        if drain > 0.1 { pixel = lerp_rgb(pixel, [220, 210, 190], drain * 0.25); }
                    }
                    if arid > 0.85 {
                        pixel = lerp_rgb(pixel, [120, 80, 55], ((arid - 0.85) / 0.15).clamp(0.0, 1.0) * 0.2);
                    } else if arid > 0.6 {
                        pixel = lerp_rgb(pixel, [240, 230, 200], ((arid - 0.6) / 0.25).clamp(0.0, 1.0) * 0.15);
                    } else if arid > 0.3 {
                        pixel = lerp_rgb(pixel, [210, 185, 120], ((arid - 0.3) / 0.3).clamp(0.0, 1.0) * 0.15);
                    }
                }

                // Frozen biome tinting
                if is_frozen_biome(biome) {
                    let rock = map.rock_hardness[idx];
                    let pv = map.peaks_valleys[idx];
                    let snow_depth = map.snowpack[idx];
                    let rock_show = pv.abs() * rock;
                    if rock_show > 0.1 {
                        pixel = lerp_rgb(pixel, [100, 105, 115], ((rock_show - 0.1) / 0.4).clamp(0.0, 1.0) * 0.25);
                    }
                    if snow_depth < 0.3 {
                        pixel = lerp_rgb(pixel, [180, 210, 235], ((0.3 - snow_depth) / 0.3).clamp(0.0, 1.0) * 0.2);
                    }
                }

                // Slope-based cliff tinting
                {
                    let sl = slope[idx];
                    if sl > 0.03 && !is_water_biome(biome) {
                        pixel = lerp_rgb(pixel, [90, 85, 80], ((sl - 0.03) / 0.07).clamp(0.0, 1.0) * 0.2);
                    }
                }

                // Height-based brightness modulation
                let hn = norm_height[idx];
                let brightness = 0.85 + hn * 0.30;
                pixel = [
                    (pixel[0] as f64 * brightness).clamp(0.0, 255.0) as u8,
                    (pixel[1] as f64 * brightness).clamp(0.0, 255.0) as u8,
                    (pixel[2] as f64 * brightness).clamp(0.0, 255.0) as u8,
                ];

                // Coastal fringing
                if !matches!(biome, TileType::Beach | TileType::Mangrove | TileType::RockyCoast | TileType::SeaCliff) {
                    let coast = coastal_fringe(cont);
                    if coast > 0.0 {
                        let temp = map.temperature[idx];
                        let rock = map.rock_hardness[idx];
                        let humid = map.humidity[idx];
                        let coast_color = if temp < 0.0 { [210, 220, 235] }
                        else if rock > 0.6 { [130, 115, 100] }
                        else if humid > 0.6 && temp > 20.0 { [144, 146, 110] }
                        else { [210, 190, 150] };
                        pixel = lerp_rgb(pixel, coast_color, coast * 0.4);
                    }
                }

                // River corridors. `rasterise_smooth_line` puts a value floor of
                // ~0.15 at every channel center pixel, so anything above the
                // ~0.05 noise threshold is "really" a river surface and should
                // paint solid water. Below that, fall back to a soft blend so
                // sub-pixel edge contributions still anti-alias against land.
                let river = map.rivers[idx];
                let temp_here = map.temperature[idx];
                if river > 0.005 {
                    let arid = map.aridity[idx];
                    let light = map.light_level[idx];
                    let corridor_color =
                        solid_river_color(temp_here, light, arid);
                    if river >= 0.05 {
                        pixel = corridor_color;
                    } else {
                        // Far edge of the rasterised channel — soft blend.
                        let edge_strength = (river / 0.05).clamp(0.0, 1.0);
                        pixel = lerp_rgb(pixel, corridor_color, edge_strength);
                    }
                }
                if !map.sediment.is_empty() && river > 0.01 {
                    let sed = map.sediment[idx];
                    if sed > 0.2 {
                        pixel = lerp_rgb(pixel, [100, 80, 50], ((sed - 0.2) / 0.5).clamp(0.0, 1.0) * 0.2);
                    }
                }
                let rmoist = map.water_table[idx];
                if rmoist > 0.05 && river <= 0.02 && temp_here < 45.0 {
                    let base_strength = if is_desert_biome(biome) { 0.35 } else { 0.25 };
                    let riparian_tint = ((rmoist - 0.05) * base_strength).clamp(0.0, 0.2);
                    pixel = lerp_rgb(pixel, [96, 122, 92], riparian_tint);
                }

                // Snowpack overlay
                let snow = map.snowpack[idx];
                if snow > 0.01 {
                    pixel = lerp_rgb(pixel, [245, 248, 255], snow.powf(0.7));
                }

                // Volcanism overlay
                let volc = map.volcanism[idx];
                let is_emissive = volc > 0.85;
                if volc > 0.5 {
                    let factor = ((volc - 0.5) / 0.5).clamp(0.0, 1.0);
                    if is_emissive { pixel = lerp_rgb(pixel, [255, 120, 30], factor); }
                    else { pixel = lerp_rgb(pixel, [120, 40, 20], factor); }
                }

                // Vegetation tint — category-aware
                let veg = map.vegetation_density[idx];
                if veg > 0.05 && temp_here < 45.0 {
                    if is_forest_biome(biome) {
                        let humid = map.humidity[idx];
                        let base_g = (veg * 100.0 + 80.0).min(255.0) as u8;
                        let canopy_target = if humid > 0.6 {
                            [66, (base_g as f64 * 0.78) as u8, 58]
                        } else {
                            [96, (base_g as f64 * 0.68) as u8, 72]
                        };
                        pixel = lerp_rgb(pixel, canopy_target, veg * 0.42);
                    } else if is_grassland_biome(biome) {
                        let arid = map.aridity[idx];
                        let steppe_target = if arid > 0.4 {
                            lerp_rgb([126, 142, 82], [184, 168, 104], ((arid - 0.4) / 0.4).clamp(0.0, 1.0))
                        } else {
                            [118, (veg * 58.0 + 108.0).min(255.0) as u8, 84]
                        };
                        pixel = lerp_rgb(pixel, steppe_target, veg * 0.38);
                    } else if is_wetland_biome(biome) {
                        let wt = map.water_table[idx];
                        let teal = ((wt - 0.2) / 0.6).clamp(0.0, 1.0);
                        let wet_target = lerp_rgb([88, 112, 72], [62, 104, 96], teal);
                        pixel = lerp_rgb(pixel, wet_target, veg * 0.42);
                    } else {
                        let surface_target = [94, (veg * 52.0 + 94.0).min(255.0) as u8, 72];
                        let strength = if is_desert_biome(biome) { 0.3 } else { 0.5 };
                        pixel = lerp_rgb(pixel, surface_target, veg * strength * 0.75);
                    }
                }

                // Hillshading on normalized heightmap
                if !is_emissive {
                    let shade = compute_hillshade(&norm_height, x, y, w, h, light_dir);
                    let ao = compute_ao(&norm_height, x, y, w, h);
                    let lighting = shade * ao;
                    pixel = [
                        (pixel[0] as f64 * lighting).clamp(0.0, 255.0) as u8,
                        (pixel[1] as f64 * lighting).clamp(0.0, 255.0) as u8,
                        (pixel[2] as f64 * lighting).clamp(0.0, 255.0) as u8,
                    ];
                }

                // Aerial perspective
                let ll = map.light_level[idx];
                let haze_strength = if is_polar_ice(biome, ll) { 0.05 } else { 0.15 };
                let haze_amount = (1.0 - ll) * haze_strength;
                if haze_amount > 0.001 {
                    let haze_color = lerp_rgb([180, 200, 220], [220, 200, 170], ll);
                    pixel = lerp_rgb(pixel, haze_color, haze_amount);
                }

                pixel
            };

            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    data
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    [
        (a[0] as f64 * inv + b[0] as f64 * t) as u8,
        (a[1] as f64 * inv + b[1] as f64 * t) as u8,
        (a[2] as f64 * inv + b[2] as f64 * t) as u8,
    ]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-10 { return [0.0, 0.0, 1.0]; }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn is_water_biome(b: TileType) -> bool {
    matches!(b,
        TileType::Sea | TileType::ShallowSea | TileType::ContinentalShelf
        | TileType::DeepOcean | TileType::OceanTrench | TileType::OceanRidge
        | TileType::White
    )
}

fn is_polar_ice(b: TileType, light_level: f64) -> bool {
    if light_level < 0.12 {
        return matches!(b,
            TileType::White | TileType::IceSheet | TileType::Snow
            | TileType::Glacier | TileType::FrozenBog | TileType::Mountain | TileType::Tundra
        );
    }
    if light_level < 0.20 {
        return matches!(b, TileType::White | TileType::IceSheet | TileType::Snow | TileType::Glacier);
    }
    false
}

fn is_desert_biome(b: TileType) -> bool {
    matches!(b,
        TileType::Desert | TileType::Sahara | TileType::Erg
        | TileType::Hamada | TileType::SaltFlat | TileType::Badlands | TileType::ScorchedRock
    )
}

fn is_forest_biome(b: TileType) -> bool {
    matches!(b,
        TileType::Forest | TileType::DeciduousForest | TileType::TemperateRainforest
        | TileType::SubtropicalForest | TileType::CloudForest | TileType::Jungle
        | TileType::Taiga | TileType::Woodland | TileType::DryWoodland
    )
}

fn is_grassland_biome(b: TileType) -> bool {
    matches!(b,
        TileType::Plains | TileType::Meadow | TileType::Steppe
        | TileType::Savanna | TileType::HighlandSavanna | TileType::Scrubland
        | TileType::Thornland | TileType::AlpineMeadow
    )
}

fn is_wetland_biome(b: TileType) -> bool {
    matches!(b, TileType::Marsh | TileType::FrozenBog | TileType::Mangrove)
}

fn is_frozen_biome(b: TileType) -> bool {
    matches!(b,
        TileType::Snow | TileType::IceSheet | TileType::Glacier
        | TileType::FrozenBog | TileType::Tundra | TileType::White
    )
}

/// Pick a solid river surface colour from local climate context.
///
/// Cold/dim cells render as pale ice-water. Hot/bright/arid cells render as
/// muddy/dust corridors. Default is the standard blue river. Used both for
/// `TileType::River` cells (solid pixels) and the high-strength branch of the
/// river corridor blend.
fn solid_river_color(temperature: f64, light_level: f64, _aridity: f64) -> [u8; 3] {
    // Visible rivers are always surface water — no more muddy/grey corridor
    // color for arid regions (those segments are DryWadi which is_visible_channel
    // now filters out). Remaining channels are Permanent/SeasonalFlow/Frozen
    // and should look like water everywhere. Cold/dim regions get pale
    // ice-water; everything else gets standard blue.
    if temperature < -1.0 || light_level < 0.12 {
        [160, 190, 210]
    } else {
        [80, 130, 180]
    }
}

fn coastal_fringe(continentalness: f64) -> f64 {
    if continentalness < SEA_LEVEL { return 0.0; }
    1.0 - ((continentalness - SEA_LEVEL) / 0.03).clamp(0.0, 1.0)
}

fn compute_hillshade(nh: &[f64], x: usize, y: usize, w: usize, h: usize, light: [f64; 3]) -> f64 {
    let r = 2;
    let get = |xi: usize, yi: usize| nh[yi.min(h - 1) * w + xi.min(w - 1)];
    let z_factor = 30.0;
    let x_lo = x.saturating_sub(r);
    let x_hi = (x + r).min(w - 1);
    let y_lo = y.saturating_sub(r);
    let y_hi = (y + r).min(h - 1);
    let dx = (get(x_hi, y) - get(x_lo, y)) / (x_hi - x_lo).max(1) as f64;
    let dy = (get(x, y_hi) - get(x, y_lo)) / (y_hi - y_lo).max(1) as f64;
    let normal = normalize([-dx * z_factor, -dy * z_factor, 1.0]);
    let dot = normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2];
    dot.clamp(0.25, 1.0)
}

fn compute_ao(nh: &[f64], x: usize, y: usize, w: usize, h: usize) -> f64 {
    let r = 2;
    let get = |xi: usize, yi: usize| nh[yi.min(h - 1) * w + xi.min(w - 1)];
    let center = get(x, y);
    let neighbors = get(x.saturating_sub(r), y)
        + get((x + r).min(w - 1), y)
        + get(x, y.saturating_sub(r))
        + get(x, (y + r).min(h - 1));
    let laplacian = neighbors / 4.0 - center;
    1.0 - (-laplacian * 8.0).clamp(0.0, 0.3)
}

fn render_ocean(biome: TileType, continentalness: f64, light_level: f64, temperature: f64) -> [u8; 3] {
    let depth = (SEA_LEVEL - continentalness).clamp(0.0, 0.5);
    let depth_norm = depth / 0.5;

    if biome == TileType::White {
        let base_ice = lerp_rgb([220, 235, 250], [235, 245, 255], (1.0 - depth_norm).clamp(0.0, 1.0));
        let brightness = 0.85 + light_level * 0.15;
        return [
            (base_ice[0] as f64 * brightness) as u8,
            (base_ice[1] as f64 * brightness) as u8,
            (base_ice[2] as f64 * brightness) as u8,
        ];
    }

    let deep = [10u8, 30, 80];
    let shallow = [60u8, 140, 200];
    let mut pixel = lerp_rgb(shallow, deep, depth_norm);

    if depth_norm < 0.3 {
        let shallow_factor = 1.0 - depth_norm / 0.3;
        if temperature > 25.0 {
            let warmth = ((temperature - 25.0) / 15.0).clamp(0.0, 1.0);
            pixel = lerp_rgb(pixel, [70, 190, 185], warmth * shallow_factor * 0.3);
        } else if temperature < 0.0 && temperature >= -10.0 {
            let cold = ((-temperature) / 10.0).clamp(0.0, 1.0);
            pixel = lerp_rgb(pixel, [80, 110, 150], cold * shallow_factor * 0.3);
        }
    }
    if temperature < -10.0 {
        let ice_hint = ((-10.0 - temperature) / 15.0).clamp(0.0, 0.3);
        pixel = lerp_rgb(pixel, [200, 220, 240], ice_hint);
    }

    let brightness = if temperature < -10.0 { 0.7 + light_level * 0.3 } else { 0.5 + light_level * 0.5 };
    [
        (pixel[0] as f64 * brightness) as u8,
        (pixel[1] as f64 * brightness) as u8,
        (pixel[2] as f64 * brightness) as u8,
    ]
}

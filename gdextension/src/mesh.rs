use godot::prelude::*;
use mg_noise::{BiomeMap, NoiseLayer, SEA_LEVEL};

const NORMAL_Y_WEIGHT: f32 = 2.0;
const SMOOTH_CENTER_WEIGHT: f32 = 2.0;

pub fn build_chunk_mesh_data(map: &BiomeMap, height_scale: f64, sub_size: i64) -> Dictionary {
    let mut result = Dictionary::new();
    if sub_size <= 0 || map.width == 0 || map.height == 0 {
        return result;
    }

    let width = map.width;
    let height = map.height;
    let sub_size = sub_size as usize;

    let heights: Vec<i32> = map
        .heightmap
        .iter()
        .map(|&sample| (sample * height_scale).floor() as i32)
        .collect();
    let ocean_mask: Vec<u8> = map
        .heightmap
        .iter()
        .map(|&sample| u8::from(sample < SEA_LEVEL))
        .collect();
    let biome_rgba = map.layer_to_rgba(NoiseLayer::Biome);

    let smoothed_heights = build_smoothed_heights(&heights, width, height);
    let normals = build_normals(&smoothed_heights, width, height);
    let colors = build_colors(&heights, &biome_rgba, width, height);
    let sea_level_y = (SEA_LEVEL * height_scale).floor() as f32;

    let land_surfaces = build_land_surfaces(
        &smoothed_heights,
        &normals,
        &colors,
        &ocean_mask,
        width,
        height,
        sub_size,
    );
    let water_surfaces = build_water_surfaces(&ocean_mask, width, height, sub_size, sea_level_y);

    result.set("heights", PackedInt32Array::from(heights.as_slice()));
    result.set("ocean_mask", PackedByteArray::from(ocean_mask.as_slice()));
    result.set("land_surfaces", land_surfaces);
    result.set("water_surfaces", water_surfaces);
    result
}

fn build_smoothed_heights(heights: &[i32], width: usize, height: usize) -> Vec<f32> {
    let mut smoothed = vec![0.0; width * height];
    for z in 0..height {
        for x in 0..width {
            let mut sum = 0.0_f32;
            let mut weight_sum = 0.0_f32;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let sx = clamp_index(x as isize + dx, width);
                    let sz = clamp_index(z as isize + dz, height);
                    let weight = if dx == 0 && dz == 0 {
                        SMOOTH_CENTER_WEIGHT
                    } else {
                        1.0
                    };
                    sum += (heights[sz * width + sx] as f32 + 1.0) * weight;
                    weight_sum += weight;
                }
            }
            smoothed[z * width + x] = sum / weight_sum;
        }
    }
    smoothed
}

fn build_normals(smoothed_heights: &[f32], width: usize, height: usize) -> Vec<Vector3> {
    let mut normals = vec![Vector3::UP; width * height];
    for z in 0..height {
        for x in 0..width {
            let left = smoothed_heights[z * width + clamp_index(x as isize - 1, width)];
            let right = smoothed_heights[z * width + clamp_index(x as isize + 1, width)];
            let back = smoothed_heights[clamp_index(z as isize - 1, height) * width + x];
            let forward = smoothed_heights[clamp_index(z as isize + 1, height) * width + x];
            normals[z * width + x] =
                Vector3::new(left - right, NORMAL_Y_WEIGHT, back - forward).normalized();
        }
    }
    normals
}

fn build_colors(
    heights: &[i32],
    biome_rgba: &[u8],
    width: usize,
    height: usize,
) -> Vec<Color> {
    let mut colors = Vec::with_capacity(width * height);
    for idx in 0..(width * height) {
        let rgba_index = idx * 4;
        let base = Color::from_rgba(
            biome_rgba[rgba_index] as f32 / 255.0,
            biome_rgba[rgba_index + 1] as f32 / 255.0,
            biome_rgba[rgba_index + 2] as f32 / 255.0,
            1.0,
        );
        colors.push(surface_color(base, heights[idx]));
    }
    colors
}

fn build_land_surfaces(
    smoothed_heights: &[f32],
    normals: &[Vector3],
    colors: &[Color],
    ocean_mask: &[u8],
    width: usize,
    height: usize,
    sub_size: usize,
) -> VariantArray {
    let mut surfaces = VariantArray::new();

    for oz in (0..height).step_by(sub_size) {
        for ox in (0..width).step_by(sub_size) {
            let max_x = (ox + sub_size - 1).min(width.saturating_sub(2));
            let max_z = (oz + sub_size - 1).min(height.saturating_sub(2));
            if max_x < ox || max_z < oz {
                continue;
            }

            let verts_w = max_x - ox + 2;
            let verts_h = max_z - oz + 2;
            let mut vertices = Vec::with_capacity(verts_w * verts_h);
            let mut surface_normals = Vec::with_capacity(verts_w * verts_h);
            let mut surface_colors = Vec::with_capacity(verts_w * verts_h);
            for z in oz..=(max_z + 1) {
                for x in ox..=(max_x + 1) {
                    let idx = z * width + x;
                    vertices.push(Vector3::new(
                        x as f32,
                        smoothed_heights[idx],
                        z as f32,
                    ));
                    surface_normals.push(normals[idx]);
                    surface_colors.push(colors[idx]);
                }
            }

            let mut indices = Vec::with_capacity((max_x - ox + 1) * (max_z - oz + 1) * 6);
            for z in oz..=max_z {
                for x in ox..=max_x {
                    if cell_is_ocean(ocean_mask, width, x, z) {
                        continue;
                    }

                    let lx = x - ox;
                    let lz = z - oz;
                    let i00 = (lz * verts_w + lx) as i32;
                    let i10 = i00 + 1;
                    let i01 = i00 + verts_w as i32;
                    let i11 = i01 + 1;
                    indices.extend_from_slice(&[i00, i10, i01, i10, i11, i01]);
                }
            }

            if indices.is_empty() {
                continue;
            }

            let mut surface = Dictionary::new();
            surface.set("vertices", PackedVector3Array::from(vertices));
            surface.set("normals", PackedVector3Array::from(surface_normals));
            surface.set("colors", PackedColorArray::from(surface_colors));
            surface.set("indices", PackedInt32Array::from(indices));
            surfaces.push(&surface.to_variant());
        }
    }

    surfaces
}

fn build_water_surfaces(
    ocean_mask: &[u8],
    width: usize,
    height: usize,
    sub_size: usize,
    sea_level_y: f32,
) -> VariantArray {
    let mut surfaces = VariantArray::new();

    for oz in (0..height).step_by(sub_size) {
        for ox in (0..width).step_by(sub_size) {
            let max_x = (ox + sub_size).min(width);
            let max_z = (oz + sub_size).min(height);
            let mut vertices = Vec::new();
            let mut normals = Vec::new();
            let mut indices = Vec::new();

            for z in oz..max_z {
                for x in ox..max_x {
                    let idx = z * width + x;
                    if ocean_mask[idx] == 0 {
                        continue;
                    }

                    let base_index = vertices.len() as i32;
                    let x0 = x as f32;
                    let x1 = x0 + 1.0;
                    let z0 = z as f32;
                    let z1 = z0 + 1.0;
                    vertices.extend_from_slice(&[
                        Vector3::new(x0, sea_level_y, z0),
                        Vector3::new(x1, sea_level_y, z0),
                        Vector3::new(x1, sea_level_y, z1),
                        Vector3::new(x0, sea_level_y, z1),
                    ]);
                    normals.extend_from_slice(&[
                        Vector3::UP,
                        Vector3::UP,
                        Vector3::UP,
                        Vector3::UP,
                    ]);
                    indices.extend_from_slice(&[
                        base_index,
                        base_index + 1,
                        base_index + 2,
                        base_index,
                        base_index + 2,
                        base_index + 3,
                    ]);
                }
            }

            if indices.is_empty() {
                continue;
            }

            let mut surface = Dictionary::new();
            surface.set("vertices", PackedVector3Array::from(vertices));
            surface.set("normals", PackedVector3Array::from(normals));
            surface.set("indices", PackedInt32Array::from(indices));
            surfaces.push(&surface.to_variant());
        }
    }

    surfaces
}

fn cell_is_ocean(ocean_mask: &[u8], width: usize, x: usize, z: usize) -> bool {
    let i00 = z * width + x;
    let i10 = z * width + (x + 1);
    let i01 = (z + 1) * width + x;
    let i11 = (z + 1) * width + (x + 1);
    ocean_mask[i00] != 0 && ocean_mask[i10] != 0 && ocean_mask[i01] != 0 && ocean_mask[i11] != 0
}

fn clamp_index(value: isize, upper_bound: usize) -> usize {
    value.clamp(0, upper_bound.saturating_sub(1) as isize) as usize
}

fn surface_color(base: Color, height: i32) -> Color {
    let ridge = (((height as f32) - 18.0) / 64.0).clamp(0.0, 1.0);
    let haze = (((height as f32) + 8.0) / 96.0).clamp(0.0, 1.0);
    let mut color = base.lerp(Color::from_rgb(0.63, 0.46, 0.39), f64::from(ridge * 0.10));
    color = color.lerp(
        Color::from_rgb(0.75, 0.54, 0.37),
        f64::from(0.04 + haze * 0.06),
    );
    Color::from_rgba(
        (color.r * 0.97).clamp(0.0, 1.0),
        (color.g * 0.95).clamp(0.0, 1.0),
        (color.b * 0.94).clamp(0.0, 1.0),
        1.0,
    )
}

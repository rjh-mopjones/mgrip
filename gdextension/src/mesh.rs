use godot::prelude::*;
use mg_noise::{BiomeMap, NoiseLayer, SEA_LEVEL};

const NORMAL_Y_WEIGHT: f32 = 2.0;
const SMOOTH_CENTER_WEIGHT: f32 = 2.0;
const CHUNK_BLOCK_SPAN: f32 = 512.0;
const SKIRT_DEPTH: f32 = 24.0;

pub struct ChunkSurfaceBuffers {
    pub vertices: Vec<Vector3>,
    pub normals: Vec<Vector3>,
    pub colors: Option<Vec<Color>>,
    pub indices: Vec<i32>,
}

pub struct ChunkMeshBuffers {
    pub heights: Vec<i32>,
    pub ocean_mask: Vec<u8>,
    pub land_surfaces: Vec<ChunkSurfaceBuffers>,
    pub water_surfaces: Vec<ChunkSurfaceBuffers>,
}

pub fn build_chunk_mesh_data(
    map: &BiomeMap,
    height_scale: f64,
    sub_size: i64,
    use_edge_skirts: bool,
) -> Dictionary {
    chunk_mesh_buffers_into_dictionary(build_chunk_mesh_buffers(
        map,
        height_scale,
        sub_size,
        use_edge_skirts,
    ))
}

pub fn build_chunk_mesh_buffers(
    map: &BiomeMap,
    height_scale: f64,
    sub_size: i64,
    use_edge_skirts: bool,
) -> ChunkMeshBuffers {
    if sub_size <= 0 || map.width == 0 || map.height == 0 {
        return ChunkMeshBuffers {
            heights: Vec::new(),
            ocean_mask: Vec::new(),
            land_surfaces: Vec::new(),
            water_surfaces: Vec::new(),
        };
    }

    let width = map.width;
    let height = map.height;
    let sub_size = sub_size as usize;
    let x_scale = sample_axis_scale(width);
    let z_scale = sample_axis_scale(height);

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

    ChunkMeshBuffers {
        heights,
        ocean_mask: ocean_mask.clone(),
        land_surfaces: build_land_surfaces(
            &smoothed_heights,
            &normals,
            &colors,
            &ocean_mask,
            width,
            height,
            sub_size,
            x_scale,
            z_scale,
            use_edge_skirts,
        ),
        water_surfaces: build_water_surfaces(
            &ocean_mask,
            width,
            height,
            sub_size,
            sea_level_y,
            x_scale,
            z_scale,
        ),
    }
}

pub fn chunk_mesh_buffers_into_dictionary(mesh_buffers: ChunkMeshBuffers) -> Dictionary {
    let mut result = Dictionary::new();
    result.set("heights", PackedInt32Array::from(mesh_buffers.heights));
    result.set("ocean_mask", PackedByteArray::from(mesh_buffers.ocean_mask));
    result.set(
        "land_surfaces",
        surface_buffers_vec_into_variant_array(mesh_buffers.land_surfaces),
    );
    result.set(
        "water_surfaces",
        surface_buffers_vec_into_variant_array(mesh_buffers.water_surfaces),
    );
    result
}

fn build_smoothed_heights(heights: &[i32], width: usize, height: usize) -> Vec<f32> {
    let mut smoothed = vec![0.0; width * height];
    for z in 0..height {
        for x in 0..width {
            let idx = z * width + x;
            if x == 0 || z == 0 || x + 1 == width || z + 1 == height {
                smoothed[idx] = heights[idx] as f32 + 1.0;
                continue;
            }

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
            smoothed[idx] = sum / weight_sum;
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
    x_scale: f32,
    z_scale: f32,
    use_edge_skirts: bool,
) -> Vec<ChunkSurfaceBuffers> {
    let mut surfaces = Vec::new();

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
                        x as f32 * x_scale,
                        smoothed_heights[idx],
                        z as f32 * z_scale,
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

            if use_edge_skirts && ox == 0 {
                append_edge_skirt(
                    &mut vertices,
                    &mut surface_normals,
                    &mut surface_colors,
                    &mut indices,
                    collect_edge_indices(verts_w, verts_h, EdgeKind::West),
                    Vector3::LEFT,
                );
            }
            if use_edge_skirts && max_x == width.saturating_sub(2) {
                append_edge_skirt(
                    &mut vertices,
                    &mut surface_normals,
                    &mut surface_colors,
                    &mut indices,
                    collect_edge_indices(verts_w, verts_h, EdgeKind::East),
                    Vector3::RIGHT,
                );
            }
            if use_edge_skirts && oz == 0 {
                append_edge_skirt(
                    &mut vertices,
                    &mut surface_normals,
                    &mut surface_colors,
                    &mut indices,
                    collect_edge_indices(verts_w, verts_h, EdgeKind::North),
                    -Vector3::BACK,
                );
            }
            if use_edge_skirts && max_z == height.saturating_sub(2) {
                append_edge_skirt(
                    &mut vertices,
                    &mut surface_normals,
                    &mut surface_colors,
                    &mut indices,
                    collect_edge_indices(verts_w, verts_h, EdgeKind::South),
                    Vector3::BACK,
                );
            }

            surfaces.push(ChunkSurfaceBuffers {
                vertices,
                normals: surface_normals,
                colors: Some(surface_colors),
                indices,
            });
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
    x_scale: f32,
    z_scale: f32,
) -> Vec<ChunkSurfaceBuffers> {
    let mut surfaces = Vec::new();

    for oz in (0..height).step_by(sub_size) {
        for ox in (0..width).step_by(sub_size) {
            let max_x = (ox + sub_size - 1).min(width.saturating_sub(2));
            let max_z = (oz + sub_size - 1).min(height.saturating_sub(2));
            if max_x < ox || max_z < oz {
                continue;
            }
            let mut vertices = Vec::new();
            let mut normals = Vec::new();
            let mut indices = Vec::new();

            for z in oz..=max_z {
                for x in ox..=max_x {
                    if !cell_is_ocean(ocean_mask, width, x, z) {
                        continue;
                    }

                    let base_index = vertices.len() as i32;
                    let x0 = x as f32 * x_scale;
                    let x1 = (x + 1) as f32 * x_scale;
                    let z0 = z as f32 * z_scale;
                    let z1 = (z + 1) as f32 * z_scale;
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

            surfaces.push(ChunkSurfaceBuffers {
                vertices,
                normals,
                colors: None,
                indices,
            });
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

#[derive(Clone, Copy)]
enum EdgeKind {
    West,
    East,
    North,
    South,
}

fn collect_edge_indices(verts_w: usize, verts_h: usize, edge_kind: EdgeKind) -> Vec<usize> {
    let mut edge = Vec::new();
    match edge_kind {
        EdgeKind::West => {
            for lz in 0..verts_h {
                edge.push(lz * verts_w);
            }
        }
        EdgeKind::East => {
            for lz in 0..verts_h {
                edge.push(lz * verts_w + (verts_w - 1));
            }
        }
        EdgeKind::North => {
            for lx in 0..verts_w {
                edge.push(lx);
            }
        }
        EdgeKind::South => {
            let row_start = (verts_h - 1) * verts_w;
            for lx in 0..verts_w {
                edge.push(row_start + lx);
            }
        }
    }
    edge
}

fn append_edge_skirt(
    vertices: &mut Vec<Vector3>,
    normals: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    indices: &mut Vec<i32>,
    edge: Vec<usize>,
    edge_normal: Vector3,
) {
    if edge.len() < 2 {
        return;
    }

    let mut top_indices = Vec::with_capacity(edge.len());
    let mut bottom_indices = Vec::with_capacity(edge.len());

    for edge_index in edge {
        let top_vertex = vertices[edge_index];
        let color = colors[edge_index];

        top_indices.push(vertices.len() as i32);
        vertices.push(top_vertex);
        normals.push(edge_normal);
        colors.push(color);

        bottom_indices.push(vertices.len() as i32);
        vertices.push(Vector3::new(
            top_vertex.x,
            top_vertex.y - SKIRT_DEPTH,
            top_vertex.z,
        ));
        normals.push(edge_normal);
        colors.push(color);
    }

    for i in 0..(top_indices.len() - 1) {
        let t0 = top_indices[i];
        let b0 = bottom_indices[i];
        let t1 = top_indices[i + 1];
        let b1 = bottom_indices[i + 1];
        indices.extend_from_slice(&[t0, b0, b1, t0, b1, t1]);
        indices.extend_from_slice(&[t0, b1, b0, t0, t1, b1]);
    }
}

fn surface_buffers_vec_into_variant_array(
    surfaces: Vec<ChunkSurfaceBuffers>,
) -> VariantArray {
    let mut array = VariantArray::new();
    for surface in surfaces {
        let mut dict = Dictionary::new();
        dict.set("vertices", PackedVector3Array::from(surface.vertices));
        dict.set("normals", PackedVector3Array::from(surface.normals));
        dict.set("indices", PackedInt32Array::from(surface.indices));
        if let Some(colors) = surface.colors {
            dict.set("colors", PackedColorArray::from(colors));
        }
        array.push(&dict.to_variant());
    }
    array
}

fn sample_axis_scale(sample_count: usize) -> f32 {
    if sample_count <= 1 {
        return 1.0;
    }
    CHUNK_BLOCK_SPAN / (sample_count.saturating_sub(1)) as f32
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

#[cfg(test)]
mod tests {
    use super::build_smoothed_heights;

    #[test]
    fn border_vertices_stay_locked_to_raw_heights() {
        let heights = vec![
            0, 1, 2,
            3, 4, 5,
            6, 7, 8,
        ];

        let smoothed = build_smoothed_heights(&heights, 3, 3);

        assert_eq!(smoothed[0], 1.0);
        assert_eq!(smoothed[2], 3.0);
        assert_eq!(smoothed[6], 7.0);
        assert_eq!(smoothed[8], 9.0);
        assert!(smoothed[4] > 4.0);
    }
}

use godot::prelude::*;
use mg_noise::{tile_has_fluid_surface, BiomeMap, NoiseLayer, SEA_LEVEL};

const NORMAL_Y_WEIGHT: f32 = 2.0;
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
    pub collision_heights: Vec<f32>,
    pub fluid_surface_mask: Vec<u8>,
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
            collision_heights: Vec::new(),
            fluid_surface_mask: Vec::new(),
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
    let fluid_surface_mask: Vec<u8> = map
        .biomes
        .iter()
        .map(|&biome| u8::from(tile_has_fluid_surface(biome)))
        .collect();
    let biome_rgba = map.layer_to_rgba(NoiseLayer::Biome);

    let render_heights = build_render_heights(&heights);
    let normals = build_normals(&render_heights, width, height);
    let colors = build_colors(&heights, &biome_rgba, width, height);
    let sea_level_y = (SEA_LEVEL * height_scale).floor() as f32;

    ChunkMeshBuffers {
        heights,
        collision_heights: render_heights.clone(),
        fluid_surface_mask: fluid_surface_mask.clone(),
        land_surfaces: build_land_surfaces(
            &render_heights,
            &normals,
            &colors,
            width,
            height,
            sub_size,
            x_scale,
            z_scale,
            use_edge_skirts,
        ),
        water_surfaces: build_water_surfaces(
            &fluid_surface_mask,
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
    result.set(
        "collision_heights",
        PackedFloat32Array::from(mesh_buffers.collision_heights),
    );
    result.set(
        "fluid_surface_mask",
        PackedByteArray::from(mesh_buffers.fluid_surface_mask),
    );
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

fn build_render_heights(heights: &[i32]) -> Vec<f32> {
    heights.iter().map(|&height| height as f32 + 1.0).collect()
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

fn build_colors(heights: &[i32], biome_rgba: &[u8], width: usize, height: usize) -> Vec<Color> {
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
    fluid_surface_mask: &[u8],
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
                    if !cell_has_fluid_surface(fluid_surface_mask, width, x, z) {
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

fn cell_has_fluid_surface(fluid_surface_mask: &[u8], width: usize, x: usize, z: usize) -> bool {
    let i00 = z * width + x;
    let i10 = z * width + (x + 1);
    let i01 = (z + 1) * width + x;
    let i11 = (z + 1) * width + (x + 1);
    fluid_surface_mask[i00] != 0
        && fluid_surface_mask[i10] != 0
        && fluid_surface_mask[i01] != 0
        && fluid_surface_mask[i11] != 0
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

fn surface_buffers_vec_into_variant_array(surfaces: Vec<ChunkSurfaceBuffers>) -> VariantArray {
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
    let luma = (base.r * 0.299) + (base.g * 0.587) + (base.b * 0.114);
    let ridge = (((height as f32) - 18.0) / 64.0).clamp(0.0, 1.0);
    let haze = (((height as f32) + 8.0) / 96.0).clamp(0.0, 1.0);
    let mineral = 0.82 + ridge * 0.10 + haze * 0.05;
    let cool_shift = (0.03 + (1.0 - ridge) * 0.02) * (0.40 + haze * 0.60);
    let neutral = Color::from_rgba(
        (luma * mineral).clamp(0.0, 1.0),
        (luma * (mineral - cool_shift * 0.40)).clamp(0.0, 1.0),
        (luma * (mineral + cool_shift * 0.85)).clamp(0.0, 1.0),
        1.0,
    );
    let retain_base = 0.10;
    let color = Color::from_rgba(
        (neutral.r * (1.0 - retain_base) + base.r * retain_base).clamp(0.0, 1.0),
        (neutral.g * (1.0 - retain_base) + base.g * retain_base).clamp(0.0, 1.0),
        (neutral.b * (1.0 - retain_base) + base.b * retain_base).clamp(0.0, 1.0),
        1.0,
    );
    Color::from_rgba(
        (color.r * 0.99).clamp(0.0, 1.0),
        (color.g * 0.98).clamp(0.0, 1.0),
        (color.b * 1.01).clamp(0.0, 1.0),
        1.0,
    )
}

#[cfg(test)]
mod tests {
    use super::{build_chunk_mesh_buffers, build_render_heights};
    use mg_core::TileType;
    use mg_noise::BiomeMap;

    #[test]
    fn render_heights_preserve_raw_steps() {
        let heights = vec![0, 1, 2, 3, 4, 5, 6, 7, 8];

        let render_heights = build_render_heights(&heights);

        assert_eq!(render_heights[0], 1.0);
        assert_eq!(render_heights[2], 3.0);
        assert_eq!(render_heights[6], 7.0);
        assert_eq!(render_heights[8], 9.0);
        assert_eq!(render_heights[4], 5.0);
    }

    #[test]
    fn low_frozen_chunks_keep_land_and_do_not_emit_fluid_surface() {
        let map = simple_map(TileType::IceSheet, -0.30);
        let buffers = build_chunk_mesh_buffers(&map, 200.0, 2, false);

        assert!(buffers.fluid_surface_mask.iter().all(|&value| value == 0));
        assert!(!buffers.land_surfaces.is_empty());
        assert!(buffers.water_surfaces.is_empty());
    }

    #[test]
    fn fluid_chunks_keep_terrain_bed_under_surface_overlay() {
        let map = simple_map(TileType::Sea, -0.30);
        let buffers = build_chunk_mesh_buffers(&map, 200.0, 2, false);

        assert!(buffers.fluid_surface_mask.iter().all(|&value| value == 1));
        assert!(!buffers.land_surfaces.is_empty());
        assert!(!buffers.water_surfaces.is_empty());
    }

    fn simple_map(tile: TileType, height: f64) -> BiomeMap {
        let width = 3;
        let height_samples = 3;
        let len = width * height_samples;
        BiomeMap {
            width,
            height: height_samples,
            continentalness: vec![0.0; len],
            tectonic: vec![0.0; len],
            tectonic_plate_ids: vec![0.0; len],
            humidity: vec![0.0; len],
            rock_hardness: vec![0.0; len],
            light_level: vec![0.0; len],
            peaks_valleys: vec![0.0; len],
            volcanism: vec![0.0; len],
            heightmap: vec![height; len],
            temperature: vec![0.0; len],
            erosion: vec![0.0; len],
            rivers: vec![0.0; len],
            aridity: vec![0.0; len],
            precipitation_type: vec![0.0; len],
            water_table: vec![0.0; len],
            wind_speed: vec![0.0; len],
            resource_richness: vec![0.0; len],
            snowpack: vec![0.0; len],
            biomes: vec![tile; len],
            vegetation_density: vec![0.0; len],
            soil_type: vec![0.0; len],
            drainage_area: vec![0; len],
            sediment: vec![0.0; len],
            river_network: None,
            world_width: 1024.0,
            world_height: 512.0,
        }
    }
}

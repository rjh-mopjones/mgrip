//! WGPU compute pipelines for all base noise layers.
//! WGSL shaders use exact OpenSimplex2D matching the `noise` crate for CPU/GPU parity.

use wgpu::{BindGroupLayout, ComputePipeline, Device};

pub struct NoisePipelines {
    pub continentalness:       ComputePipeline,
    pub continentalness_layout: BindGroupLayout,
    pub light_level:           ComputePipeline,
    pub light_level_layout:    BindGroupLayout,
    pub rock_hardness:         ComputePipeline,
    pub rock_hardness_layout:  BindGroupLayout,
    pub peaks_valleys:         ComputePipeline,
    pub peaks_valleys_layout:  BindGroupLayout,
    pub humidity:              ComputePipeline,
    pub humidity_layout:       BindGroupLayout,
}

impl NoisePipelines {
    pub fn new(device: &Device) -> Self {
        let cont_src   = format!("{}{}{}", INDEPENDENT_BINDINGS, OPEN_SIMPLEX_FUNCS, CONTINENTALNESS_MAIN);
        let light_src  = format!("{}{}{}", INDEPENDENT_BINDINGS, OPEN_SIMPLEX_FUNCS, LIGHT_LEVEL_MAIN);
        let rock_src   = format!("{}{}{}", INDEPENDENT_BINDINGS, OPEN_SIMPLEX_FUNCS, ROCK_HARDNESS_MAIN);
        let peaks_src  = format!("{}{}{}", INDEPENDENT_BINDINGS, OPEN_SIMPLEX_FUNCS, PEAKS_VALLEYS_MAIN);
        let humid_src  = format!("{}{}{}", DEPENDENT_BINDINGS,   OPEN_SIMPLEX_FUNCS, HUMIDITY_MAIN);

        let (continentalness, continentalness_layout) = independent(device, "Continentalness", &cont_src);
        let (light_level,     light_level_layout)     = independent(device, "LightLevel",      &light_src);
        let (rock_hardness,   rock_hardness_layout)   = independent(device, "RockHardness",    &rock_src);
        let (peaks_valleys,   peaks_valleys_layout)   = independent(device, "PeaksValleys",    &peaks_src);
        let (humidity,        humidity_layout)        = dependent  (device, "Humidity",        &humid_src);

        Self {
            continentalness, continentalness_layout,
            light_level, light_level_layout,
            rock_hardness, rock_hardness_layout,
            peaks_valleys, peaks_valleys_layout,
            humidity, humidity_layout,
        }
    }
}

// ─── Pipeline factory helpers ─────────────────────────────────────────────────

fn independent(device: &Device, name: &str, src: &str) -> (ComputePipeline, BindGroupLayout) {
    make_pipeline(device, name, src, &[
        entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform,                      has_dynamic_offset: false, min_binding_size: None }),
        entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true  }, has_dynamic_offset: false, min_binding_size: None }),
        entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
    ])
}

fn dependent(device: &Device, name: &str, src: &str) -> (ComputePipeline, BindGroupLayout) {
    make_pipeline(device, name, src, &[
        entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform,                      has_dynamic_offset: false, min_binding_size: None }),
        entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true  }, has_dynamic_offset: false, min_binding_size: None }),
        entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true  }, has_dynamic_offset: false, min_binding_size: None }),
        entry(3, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
    ])
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}

fn make_pipeline(device: &Device, name: &str, src: &str, entries: &[wgpu::BindGroupLayoutEntry]) -> (ComputePipeline, BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("{name} shader")),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("{name} bind group layout")),
        entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{name} pipeline layout")),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&format!("{name} pipeline")),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (pipeline, layout)
}

// ─── WGSL shaders ─────────────────────────────────────────────────────────────

/// Struct + bindings for shaders that only need params + perm_table
const INDEPENDENT_BINDINGS: &str = r#"
struct Params {
    seed: u32, width: u32, height: u32, octaves: u32,
    frequency: f32, persistence: f32, lacunarity: f32, scale: f32,
    world_x: f32, world_y: f32, world_height: f32, _padding: f32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> perm_table: array<u32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;
"#;

/// Bindings for humidity which additionally reads continentalness
const DEPENDENT_BINDINGS: &str = r#"
struct Params {
    seed: u32, width: u32, height: u32, octaves: u32,
    frequency: f32, persistence: f32, lacunarity: f32, scale: f32,
    world_x: f32, world_y: f32, world_height: f32, _padding: f32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> perm_table: array<u32>;
@group(0) @binding(2) var<storage, read> continentalness: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
"#;

/// OpenSimplex 2D + fBm + ridged multifractal — exact port of the `noise` crate.
const OPEN_SIMPLEX_FUNCS: &str = r#"
const STRETCH_2D: f32 = -0.211324865405187;
const SQUISH_2D:  f32 =  0.366025403784439;
const NORM_2D:    f32 =  1.0 / 14.0;
const DIAG:       f32 =  0.7071067811865476;

fn grad2(index: u32) -> vec2<f32> {
    switch (index % 8u) {
        case 0u: { return vec2<f32>( 1.0,  0.0); }
        case 1u: { return vec2<f32>(-1.0,  0.0); }
        case 2u: { return vec2<f32>( 0.0,  1.0); }
        case 3u: { return vec2<f32>( 0.0, -1.0); }
        case 4u: { return vec2<f32>( DIAG,  DIAG); }
        case 5u: { return vec2<f32>(-DIAG,  DIAG); }
        case 6u: { return vec2<f32>( DIAG, -DIAG); }
        default: { return vec2<f32>(-DIAG, -DIAG); }
    }
}

fn perm_hash(x: i32, y: i32) -> u32 {
    let a = perm_table[u32(x & 255)];
    let b = a ^ u32(y & 255);
    return perm_table[b & 255u];
}

fn surflet(index: u32, point: vec2<f32>) -> f32 {
    let t = 2.0 - dot(point, point);
    if (t > 0.0) { let t2 = t * t; return t2 * t2 * dot(point, grad2(index)); }
    return 0.0;
}

fn open_simplex_2d(x: f32, y: f32) -> f32 {
    let stretch_offset = (x + y) * STRETCH_2D;
    let xs = x + stretch_offset;
    let ys = y + stretch_offset;
    let xsb = i32(floor(xs));
    let ysb = i32(floor(ys));
    let squish_offset = f32(xsb + ysb) * SQUISH_2D;
    let xb = f32(xsb) + squish_offset;
    let yb = f32(ysb) + squish_offset;
    let xins = xs - f32(xsb);
    let yins = ys - f32(ysb);
    let in_sum = xins + yins;
    let dx0 = x - xb;
    let dy0 = y - yb;
    var value = 0.0;
    let dx1 = dx0 - 1.0 - SQUISH_2D;
    let dy1 = dy0 - SQUISH_2D;
    value += surflet(perm_hash(xsb + 1, ysb),     vec2<f32>(dx1, dy1));
    let dx2 = dx0 - SQUISH_2D;
    let dy2 = dy0 - 1.0 - SQUISH_2D;
    value += surflet(perm_hash(xsb, ysb + 1),     vec2<f32>(dx2, dy2));
    if (in_sum > 1.0) {
        let dx3 = dx0 - 1.0 - 2.0 * SQUISH_2D;
        let dy3 = dy0 - 1.0 - 2.0 * SQUISH_2D;
        value += surflet(perm_hash(xsb + 1, ysb + 1), vec2<f32>(dx3, dy3));
    } else {
        value += surflet(perm_hash(xsb, ysb), vec2<f32>(dx0, dy0));
    }
    return value * NORM_2D;
}

fn fbm(x: f32, y: f32, octaves: u32, freq: f32, persistence: f32, lacunarity: f32) -> f32 {
    var v = 0.0; var amp = 1.0; var f = freq; var maxamp = 0.0;
    for (var i = 0u; i < octaves; i++) {
        v += open_simplex_2d(x * f, y * f) * amp;
        maxamp += amp; amp *= persistence; f *= lacunarity;
    }
    return v / maxamp;
}

fn ridged(x: f32, y: f32, octaves: u32, freq: f32, persistence: f32, lacunarity: f32) -> f32 {
    var v = 0.0; var amp = 1.0; var f = freq; var maxamp = 0.0;
    for (var i = 0u; i < octaves; i++) {
        v += (1.0 - abs(open_simplex_2d(x * f, y * f))) * amp;
        maxamp += amp; amp *= persistence; f *= lacunarity;
    }
    return (v / maxamp) * 2.0 - 1.0;
}
"#;

/// 16-octave fBm continentalness, scale 0.01 (matching CPU).
const CONTINENTALNESS_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }
    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;
    output[idx] = fbm(wx * 0.01, wy * 0.01, params.octaves, params.frequency, params.persistence, params.lacunarity);
}
"#;

/// Cosine angular distance from sub-stellar point (0.5, 1.0) normalised, with domain warp + scatter.
const LIGHT_LEVEL_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }
    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;
    let map_width = params.world_height * 2.0;
    let nx = wx / map_width;
    let ny = wy / params.world_height;
    // Two-pass domain warp matching CPU
    let warp1_x = open_simplex_2d(wx * 0.0015,        wy * 0.0015 + 50.0)  * 0.12;
    let warp1_y = open_simplex_2d(wx * 0.0015 + 150.0, wy * 0.0015)         * 0.12;
    let warp2_x = open_simplex_2d(wx * 0.005,          wy * 0.005 + 100.0) * 0.06;
    let warp2_y = open_simplex_2d(wx * 0.005 + 200.0,  wy * 0.005)         * 0.06;
    var raw_dx = nx - 0.5 + warp1_x + warp2_x;
    if (raw_dx >  0.5) { raw_dx = raw_dx - 1.0; }
    if (raw_dx < -0.5) { raw_dx = raw_dx + 1.0; }
    let dy = ny - 1.0 + warp1_y + warp2_y;
    let dist = min(sqrt(raw_dx * raw_dx + dy * dy), 1.0);
    let far_dist = max((dist - 0.5) / 0.5, 0.0);
    let darkening = 1.0 + 1.5 * far_dist * far_dist;
    let base_light = pow(cos(dist * 1.5707963), darkening);
    let scatter = fbm(wx * 0.005, wy * 0.005, params.octaves, params.frequency, params.persistence, params.lacunarity) * 0.05;
    output[idx] = clamp(base_light + scatter, 0.0, 1.0);
}
"#;

/// 3-octave fBm rock hardness, scale 0.0125, output [0,1] (matching CPU).
const ROCK_HARDNESS_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }
    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;
    let raw = fbm(wx * 0.0125, wy * 0.0125, params.octaves, params.frequency, params.persistence, params.lacunarity);
    output[idx] = clamp((raw + 1.0) * 0.5, 0.0, 1.0);
}
"#;

/// 6-octave ridged multifractal peaks & valleys, scale 0.015 (matching CPU).
const PEAKS_VALLEYS_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }
    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;
    output[idx] = ridged(wx * 0.015, wy * 0.015, params.octaves, params.frequency, params.persistence, params.lacunarity);
}
"#;

/// Terminator-ring humidity model (tidally locked planet).
/// Reads continentalness; computes light_level inline (no scatter, saves a round-trip).
/// Gaussian peak at light≈0.2, day-side drying, night-side cold trap.
const HUMIDITY_MAIN: &str = r#"
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }
    let idx = gid.y * params.width + gid.x;
    let wx = params.world_x + f32(gid.x) * params.scale;
    let wy = params.world_y + f32(gid.y) * params.scale;
    let cont = continentalness[idx];

    // Light level (simplified — no scatter for GPU efficiency)
    let map_width = params.world_height * 2.0;
    var raw_dx = wx / map_width - 0.5;
    if (raw_dx >  0.5) { raw_dx -= 1.0; }
    if (raw_dx < -0.5) { raw_dx += 1.0; }
    let dy = wy / params.world_height - 1.0;
    let dist = min(sqrt(raw_dx * raw_dx + dy * dy), 1.0);
    let far  = max((dist - 0.5) / 0.5, 0.0);
    let light = clamp(pow(cos(dist * 1.5707963), 1.0 + 1.5 * far * far), 0.0, 1.0);

    // Terminator humidity model (matching CPU generate_terminator_model)
    let terminator_peak = exp(-(light - 0.2) * (light - 0.2) / (2.0 * 0.22 * 0.22));
    let day_drying = select(1.0, 1.0 - pow((light - 0.4) / 0.6, 2.0) * 0.8, light > 0.4);
    let night_trap = select(0.15 + (light / 0.15) * 0.85, 1.0, light >= 0.15);

    var moisture_source = 0.0;
    let sea_level = -0.01;
    if (cont < sea_level) {
        moisture_source = 1.0;
    } else if (cont < 0.05) {
        moisture_source = 1.0 - ((cont + 0.01) / 0.06) * 0.5;
    } else if (cont < 0.2) {
        moisture_source = 0.5 - ((cont - 0.05) / 0.15) * 0.3;
    } else {
        moisture_source = 0.2 - min((cont - 0.2) / 0.3, 1.0) * 0.1;
    }

    let base_noise = (fbm(wx * 0.003, wy * 0.003, params.octaves, params.frequency, params.persistence, params.lacunarity) + 1.0) * 0.5;
    let atmospheric = terminator_peak * day_drying * night_trap;
    let scaled_moisture = moisture_source * (0.3 + terminator_peak * 0.7);
    output[idx] = clamp(base_noise * 0.2 + scaled_moisture * 0.3 + atmospheric * 0.5, 0.0, 1.0);
}
"#;

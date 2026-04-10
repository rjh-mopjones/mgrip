//! GPU context — lazily-initialised wgpu device, dispatches all base noise layers.

use bytemuck::{Pod, Zeroable};
use std::sync::OnceLock;
use wgpu::util::DeviceExt;

use super::perm_table::permutation_table_to_u32;
use super::{generate_permutation_table, GpuNoiseResult, NoisePipelines};

static GPU_CONTEXT: OnceLock<Option<GpuNoiseContext>> = OnceLock::new();

/// Uniform struct passed to every noise compute shader.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct NoiseParams {
    pub seed: u32,
    pub width: u32,
    pub height: u32,
    pub octaves: u32,
    pub frequency: f32,
    pub persistence: f32,
    pub lacunarity: f32,
    pub scale: f32, // pixel-to-world: (world_size_x / tile_w) as f32
    pub world_x: f32,
    pub world_y: f32,
    pub world_height: f32,
    pub _padding: f32,
}

pub struct GpuNoiseContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: NoisePipelines,
}

impl GpuNoiseContext {
    /// Get (or lazily initialise) the global GPU context. Returns `None` if no GPU.
    pub fn global() -> Option<&'static GpuNoiseContext> {
        if std::env::var_os("MG_NOISE_FORCE_CPU").is_some() {
            return None;
        }
        GPU_CONTEXT
            .get_or_init(|| {
                let result = pollster::block_on(Self::new());
                if let Err(ref e) = result {
                    eprintln!("[mg_noise] GPU init failed: {e}");
                }
                result.ok()
            })
            .as_ref()
    }

    pub fn is_available() -> bool {
        Self::global().is_some()
    }

    async fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| "No GPU adapter".to_string())?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("mg_noise GPU"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                experimental_features: Default::default(),
            })
            .await
            .map_err(|e| e.to_string())?;

        let pipelines = NoisePipelines::new(&device);
        Ok(Self {
            device,
            queue,
            pipelines,
        })
    }

    // ── Public entry point ────────────────────────────────────────────────────

    /// Generate all 5 base noise layers on the GPU for one tile.
    ///
    /// `scale` = world units per pixel = `world_size_x / tile_w`.
    /// Seed offsets match `biome_map.rs` constants (CONTINENTALNESS=0, HUMIDITY=2,
    /// ROCK_HARDNESS=3, LIGHT_LEVEL=4, PEAKS_VALLEYS=7).
    pub fn generate_layers(
        &self,
        seed: u32,
        tile_w: usize,
        tile_h: usize,
        world_x: f64,
        world_y: f64,
        scale: f64,
        world_height: f64,
        detail_level: u32,
    ) -> GpuNoiseResult {
        let mk_perm = |s: u32, lbl: &str| {
            let tbl = generate_permutation_table(s);
            let u32s = permutation_table_to_u32(&tbl);
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(lbl),
                    contents: bytemuck::cast_slice(&u32s),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };

        // Seed offsets must match biome_map.rs constants
        let cont_perm = mk_perm(seed, "cont perm");
        let humi_perm = mk_perm(seed.wrapping_add(2), "humi perm");
        let rock_perm = mk_perm(seed.wrapping_add(3), "rock perm");
        let light_perm = mk_perm(seed.wrapping_add(4), "light perm");
        let peak_perm = mk_perm(seed.wrapping_add(7), "peak perm");

        let continentalness = self.dispatch_independent(
            &self.pipelines.continentalness,
            &self.pipelines.continentalness_layout,
            NoiseParams {
                seed,
                width: tile_w as u32,
                height: tile_h as u32,
                octaves: 16 + detail_level,
                frequency: 1.0,
                persistence: 0.59,
                lacunarity: 2.0,
                scale: scale as f32,
                world_x: world_x as f32,
                world_y: world_y as f32,
                world_height: world_height as f32,
                _padding: 0.0,
            },
            &cont_perm,
            tile_w,
            tile_h,
        );

        let light_level = self.dispatch_independent(
            &self.pipelines.light_level,
            &self.pipelines.light_level_layout,
            NoiseParams {
                seed: seed.wrapping_add(4),
                width: tile_w as u32,
                height: tile_h as u32,
                octaves: 3 + detail_level,
                frequency: 1.0,
                persistence: 0.5,
                lacunarity: 2.0,
                scale: scale as f32,
                world_x: world_x as f32,
                world_y: world_y as f32,
                world_height: world_height as f32,
                _padding: 0.0,
            },
            &light_perm,
            tile_w,
            tile_h,
        );

        let rock_hardness = self.dispatch_independent(
            &self.pipelines.rock_hardness,
            &self.pipelines.rock_hardness_layout,
            NoiseParams {
                seed: seed.wrapping_add(3),
                width: tile_w as u32,
                height: tile_h as u32,
                octaves: 3 + detail_level,
                frequency: 1.0,
                persistence: 0.6,
                lacunarity: 2.0,
                scale: scale as f32,
                world_x: world_x as f32,
                world_y: world_y as f32,
                world_height: 0.0,
                _padding: 0.0,
            },
            &rock_perm,
            tile_w,
            tile_h,
        );

        let peaks_valleys = self.dispatch_independent(
            &self.pipelines.peaks_valleys,
            &self.pipelines.peaks_valleys_layout,
            NoiseParams {
                seed: seed.wrapping_add(7),
                width: tile_w as u32,
                height: tile_h as u32,
                octaves: 6 + detail_level,
                frequency: 1.0,
                persistence: 0.5,
                lacunarity: 2.0,
                scale: scale as f32,
                world_x: world_x as f32,
                world_y: world_y as f32,
                world_height: 0.0,
                _padding: 0.0,
            },
            &peak_perm,
            tile_w,
            tile_h,
        );

        let humidity = self.dispatch_dependent(
            &self.pipelines.humidity,
            &self.pipelines.humidity_layout,
            NoiseParams {
                seed: seed.wrapping_add(2),
                width: tile_w as u32,
                height: tile_h as u32,
                octaves: 5 + detail_level,
                frequency: 1.0,
                persistence: 0.5,
                lacunarity: 2.0,
                scale: scale as f32,
                world_x: world_x as f32,
                world_y: world_y as f32,
                world_height: world_height as f32,
                _padding: 0.0,
            },
            &continentalness,
            &humi_perm,
            tile_w,
            tile_h,
        );

        GpuNoiseResult {
            continentalness,
            peaks_valleys,
            humidity,
            light_level,
            rock_hardness,
        }
    }

    // ── Dispatch helpers ──────────────────────────────────────────────────────

    fn dispatch_independent(
        &self,
        pipeline: &wgpu::ComputePipeline,
        layout: &wgpu::BindGroupLayout,
        params: NoiseParams,
        perm: &wgpu::Buffer,
        w: usize,
        h: usize,
    ) -> Vec<f32> {
        let buf_size = (w * h * std::mem::size_of::<f32>()) as u64;
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: buf_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let stage_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: buf_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });
        self.run_compute(pipeline, &bind_group, &out_buf, &stage_buf, w, h, buf_size)
    }

    fn dispatch_dependent(
        &self,
        pipeline: &wgpu::ComputePipeline,
        layout: &wgpu::BindGroupLayout,
        params: NoiseParams,
        cont: &[f32],
        perm: &wgpu::Buffer,
        w: usize,
        h: usize,
    ) -> Vec<f32> {
        let buf_size = (w * h * std::mem::size_of::<f32>()) as u64;
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let cont_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(cont),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: buf_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let stage_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: buf_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: perm.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: cont_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: out_buf.as_entire_binding(),
                },
            ],
        });
        self.run_compute(pipeline, &bind_group, &out_buf, &stage_buf, w, h, buf_size)
    }

    fn run_compute(
        &self,
        pipeline: &wgpu::ComputePipeline,
        bind_group: &wgpu::BindGroup,
        out_buf: &wgpu::Buffer,
        stage_buf: &wgpu::Buffer,
        w: usize,
        h: usize,
        buf_size: u64,
    ) -> Vec<f32> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups((w as u32 + 15) / 16, (h as u32 + 15) / 16, 1);
        }
        encoder.copy_buffer_to_buffer(out_buf, 0, stage_buf, 0, buf_size);
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = stage_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device
            .poll(wgpu::PollType::Wait {
                timeout: None,
                submission_index: None,
            })
            .ok();
        rx.recv().unwrap().unwrap();

        let data = slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        stage_buf.unmap();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_availability() {
        let available = GpuNoiseContext::is_available();
        println!("GPU available: {available}");
        // Print more info regardless
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });
            match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
            {
                Ok(a) => {
                    let info = a.get_info();
                    println!(
                        "Adapter: {} / {:?} / {:?}",
                        info.name, info.backend, info.device_type
                    );
                }
                Err(e) => println!("request_adapter failed: {e:?}"),
            }
        });
    }
}

//! GPU side: pipelines, per-view offscreen targets, background mesh scheduling, culling, LOD.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::{Vec3, Vec4};
use meadow_core::tile::{TILE_SHIFT, TILE_SIZE};
use meadow_core::{DimensionKind, World};
use rustc_hash::{FxHashMap, FxHashSet};
use wgpu::util::DeviceExt;

use crate::camera::{Camera, Frustum};
use crate::mesh::{MeshData, MeshOptions, TileMesh, Vertex, build_tile};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    sun: [f32; 4],
    fog: [f32; 4],
    params: [f32; 4],
}

/// Shared GPU state (device, queue, pipelines). Cheap to clone.
#[derive(Clone)]
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    inner: Arc<parking_lot::Mutex<Pipelines>>,
    layout: Arc<wgpu::BindGroupLayout>,
}

struct Pipelines {
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    by_samples: FxHashMap<u32, (wgpu::RenderPipeline, wgpu::RenderPipeline)>,
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meadow terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meadow globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meadow"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Renderer {
            device,
            queue,
            inner: Arc::new(parking_lot::Mutex::new(Pipelines { shader, pipeline_layout, by_samples: FxHashMap::default() })),
            layout: Arc::new(layout),
        }
    }

    /// Create a renderer without a window (software or hardware adapter), e.g. for screenshots.
    ///
    /// Tries the primary backends (Vulkan, Metal, DX12) first and only then OpenGL, whose
    /// software drivers are the least robust. `WGPU_BACKEND` overrides the choice.
    pub fn headless() -> anyhow::Result<Renderer> {
        let attempts: Vec<wgpu::Backends> = match wgpu::Backends::from_env() {
            Some(b) => vec![b],
            None => vec![wgpu::Backends::PRIMARY, wgpu::Backends::all()],
        };
        let mut last_err = None;
        for backends in attempts {
            let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
            desc.backends = backends;
            let instance = wgpu::Instance::new(desc);
            let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(e) => {
                    last_err = Some(anyhow::Error::from(e));
                    continue;
                }
            };
            log::info!("headless adapter: {:?}", adapter.get_info());
            match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("meadow headless"),
                required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })) {
                Ok((device, queue)) => return Ok(Renderer::new(device, queue)),
                Err(e) => last_err = Some(e.into()),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no GPU adapter found")))
    }

    fn pipelines(&self, samples: u32) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
        let mut p = self.inner.lock();
        if let Some(x) = p.by_samples.get(&samples) {
            return x.clone();
        }
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute { format: wgpu::VertexFormat::Sint16x4, offset: 0, shader_location: 0 },
                    wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 8, shader_location: 1 },
                ],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 2 }],
            }),
        ];
        let make = |fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool| {
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&p.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &p.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &p.shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = make("fs_main", None, true);
        let water = make("fs_water", Some(wgpu::BlendState::ALPHA_BLENDING), false);
        p.by_samples.insert(samples, (opaque.clone(), water.clone()));
        (opaque, water)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SceneSettings {
    pub msaa: u32,
    /// Maximum distance (blocks) at which tiles are drawn.
    pub view_distance: f32,
    /// Distance below which full detail (step 1) is used; LOD doubles every multiple.
    pub detail_distance: f32,
    /// Trees/objects/plants are meshed within this distance.
    pub decoration_distance: f32,
    pub mesh: MeshOptions,
    pub fog: bool,
    pub sky: [f32; 3],
    /// Sun direction (towards the sun).
    pub sun: [f32; 3],
    pub ambient: f32,
    /// Concurrent background mesh builds.
    pub max_inflight: usize,
}

impl Default for SceneSettings {
    fn default() -> Self {
        SceneSettings {
            msaa: 4,
            view_distance: 6000.0,
            detail_distance: 384.0,
            decoration_distance: 320.0,
            mesh: MeshOptions::default(),
            fog: true,
            sky: [0.62, 0.76, 0.93],
            sun: [0.45, 0.8, 0.35],
            ambient: 0.45,
            max_inflight: 8,
        }
    }
}

struct GpuMesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
}

fn upload(device: &wgpu::Device, m: &MeshData) -> Option<GpuMesh> {
    if m.is_empty() {
        return None;
    }
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tile vb"),
        contents: bytemuck::cast_slice(&m.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tile ib"),
        contents: bytemuck::cast_slice(&m.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    Some(GpuMesh { vb, ib, count: m.indices.len() as u32 })
}

struct GpuTile {
    step: i32,
    decor: bool,
    dep: u64,
    generation: u64,
    opaque: Option<GpuMesh>,
    water: Option<GpuMesh>,
    origin: wgpu::Buffer,
    min: Vec3,
    max: Vec3,
}

struct Target {
    size: (u32, u32),
    samples: u32,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    /// Same texture viewed as plain `Rgba8Unorm` (gamma-space), which is what egui expects.
    egui_view: wgpu::TextureView,
    msaa_view: Option<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SceneStats {
    pub tiles_drawn: usize,
    pub triangles: u64,
    pub pending: usize,
    pub meshes: usize,
}

type Built = (u64, u64, TileMesh);

pub struct Scene {
    pub camera: Camera,
    pub settings: SceneSettings,
    world: Option<Arc<World>>,
    kind: DimensionKind,
    tiles: FxHashMap<(i32, i32), GpuTile>,
    inflight: FxHashSet<(i32, i32)>,
    tx: Sender<Built>,
    rx: Receiver<Built>,
    generation: u64,
    target: Option<Target>,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    height_cache: FxHashMap<(i32, i32), (u64, f32, f32)>,
    pub stats: SceneStats,
}

impl Scene {
    pub fn new(r: &Renderer) -> Scene {
        let globals = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = r.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &r.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let (tx, rx) = channel();
        Scene {
            camera: Camera::default(),
            settings: SceneSettings::default(),
            world: None,
            kind: DimensionKind::Overworld,
            tiles: FxHashMap::default(),
            inflight: FxHashSet::default(),
            tx,
            rx,
            generation: 1,
            target: None,
            globals,
            bind_group,
            height_cache: FxHashMap::default(),
            stats: SceneStats::default(),
        }
    }

    /// Give the scene a (snapshot of the) world to draw. Changed tiles are detected by revision.
    pub fn set_world(&mut self, world: Arc<World>, kind: DimensionKind) {
        if self.kind != kind {
            self.tiles.clear();
            self.kind = kind;
        }
        self.world = Some(world);
    }

    pub fn world(&self) -> Option<&Arc<World>> {
        self.world.as_ref()
    }

    /// Rebuild everything (e.g. after layer or terrain settings changed). Old meshes stay
    /// visible until replaced.
    pub fn invalidate_all(&mut self) {
        self.generation += 1;
    }

    /// Point the camera at the middle of the world.
    pub fn frame_world(&mut self) {
        let Some(w) = &self.world else { return };
        let Some(d) = w.dimension(self.kind) else { return };
        if let Some((x0, z0, x1, z1)) = d.block_bounds() {
            let cx = (x0 + x1) / 2;
            let cz = (z0 + z1) / 2;
            let h = d.height(cx, cz).unwrap_or(64.0);
            self.camera.target = Vec3::new(cx as f32, h, cz as f32);
            self.camera.distance = ((x1 - x0).max(z1 - z0) as f32 * 0.7).clamp(64.0, 4000.0);
        }
    }

    fn dep_rev(d: &meadow_core::Dimension, tx: i32, ty: i32) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let r = d.tile(tx + dx, ty + dy).map(|t| t.rev).unwrap_or(0);
                h = (h ^ r).wrapping_mul(0x100000001b3);
            }
        }
        h
    }

    /// Receive finished meshes and schedule new builds. Call once per frame.
    pub fn update(&mut self, r: &Renderer, aspect: f32) {
        while let Ok((generation, dep, m)) = self.rx.try_recv() {
            self.inflight.remove(&(m.tx, m.ty));
            let origin = r.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tile origin"),
                contents: bytemuck::cast_slice(&[(m.tx << TILE_SHIFT) as f32, 0.0, (m.ty << TILE_SHIFT) as f32, 0.0]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let (ox, oz) = ((m.tx << TILE_SHIFT) as f32, (m.ty << TILE_SHIFT) as f32);
            let t = GpuTile {
                step: m.step,
                decor: m.with_decorations,
                dep,
                generation,
                opaque: upload(&r.device, &m.opaque),
                water: upload(&r.device, &m.water),
                origin,
                min: Vec3::new(ox - 40.0, m.min_y, oz - 40.0),
                max: Vec3::new(ox + TILE_SIZE as f32 + 40.0, m.max_y, oz + TILE_SIZE as f32 + 40.0),
            };
            self.tiles.insert((m.tx, m.ty), t);
        }
        let Some(world) = self.world.clone() else { return };
        let Some(dim) = world.dimension(self.kind) else {
            self.tiles.clear();
            return;
        };
        self.tiles.retain(|k, _| dim.tile(k.0, k.1).is_some());
        let eye = self.camera.eye();
        let frustum = Frustum::from_view_proj(self.camera.view_proj(aspect));
        let s = self.settings;
        let mut wanted: Vec<(f32, (i32, i32), i32, bool, u64)> = Vec::new();
        for t in dim.tiles() {
            let key = (t.tx, t.ty);
            let (lo, hi) = match self.height_cache.get(&key) {
                Some((rev, lo, hi)) if *rev == t.rev => (*lo, *hi),
                _ => {
                    let (lo, hi) = (t.min_height(), t.max_height());
                    self.height_cache.insert(key, (t.rev, lo, hi));
                    (lo, hi)
                }
            };
            let (ox, oz) = ((t.tx << TILE_SHIFT) as f32, (t.ty << TILE_SHIFT) as f32);
            let min = Vec3::new(ox - 40.0, lo - 32.0, oz - 40.0);
            let max = Vec3::new(ox + TILE_SIZE as f32 + 40.0, hi + 48.0, oz + TILE_SIZE as f32 + 40.0);
            let closest = eye.clamp(min, max);
            let d = closest.distance(eye);
            if d > s.view_distance || !frustum.intersects_aabb(min, max) {
                continue;
            }
            let lod = ((d / s.detail_distance.max(16.0)).max(1.0).log2().floor() as i32).clamp(0, 4);
            let step = 1 << lod;
            let decor = step == 1 && d < s.decoration_distance && s.mesh.decorations;
            let dep = Self::dep_rev(dim, t.tx, t.ty);
            let up_to_date = self
                .tiles
                .get(&key)
                .is_some_and(|g| g.step == step && g.decor == decor && g.dep == dep && g.generation == self.generation);
            if !up_to_date && !self.inflight.contains(&key) {
                wanted.push((d, key, step, decor, dep));
            }
        }
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, key, step, decor, dep) in wanted {
            if self.inflight.len() >= s.max_inflight {
                break;
            }
            self.inflight.insert(key);
            let tx = self.tx.clone();
            let world = world.clone();
            let kind = self.kind;
            let generation = self.generation;
            let mut opts = s.mesh;
            opts.decorations = decor;
            rayon::spawn(move || {
                if let Some(d) = world.dimension(kind)
                    && let Some(m) = build_tile(&world, d, key.0, key.1, step, &opts)
                {
                    let _ = tx.send((generation, dep, m));
                    return;
                }
                // Tile vanished; still report so the slot frees up.
                let _ = tx.send((
                    generation,
                    dep,
                    TileMesh { tx: key.0, ty: key.1, step, with_decorations: decor, opaque: MeshData::default(), water: MeshData::default(), min_y: 0.0, max_y: 0.0 },
                ));
            });
        }
        self.stats.pending = self.inflight.len();
        self.stats.meshes = self.tiles.len();
    }

    /// True while background meshes are still being built.
    pub fn is_busy(&self) -> bool {
        !self.inflight.is_empty()
    }

    fn ensure_target(&mut self, r: &Renderer, w: u32, h: u32) {
        let samples = if self.settings.msaa >= 4 { 4 } else { 1 };
        if let Some(t) = &self.target
            && t.size == (w, h)
            && t.samples == samples
        {
            return;
        }
        let tex = |label: &str, format, samples: u32, usage| {
            let view_formats: &[wgpu::TextureFormat] = if format == COLOR_FORMAT && samples == 1 { &[wgpu::TextureFormat::Rgba8Unorm] } else { &[] };
            r.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let color = tex(
            "scene color",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let msaa_view = (samples > 1)
            .then(|| tex("scene msaa", COLOR_FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default()));
        let depth = tex("scene depth", DEPTH_FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT);
        self.target = Some(Target {
            size: (w, h),
            samples,
            color_view: color.create_view(&Default::default()),
            egui_view: color.create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Rgba8Unorm), ..Default::default() }),
            color,
            msaa_view,
            depth_view: depth.create_view(&Default::default()),
        });
    }

    /// Render into the scene's offscreen texture and return a view of it for egui (`Rgba8Unorm`,
    /// gamma space). Use `read_pixels` for CPU readback.
    pub fn render(&mut self, r: &Renderer, width: u32, height: u32) -> &wgpu::TextureView {
        let (w, h) = (width.max(1), height.max(1));
        self.ensure_target(r, w, h);
        let aspect = w as f32 / h as f32;
        self.update(r, aspect);
        let s = self.settings;
        let eye = self.camera.eye();
        let vp = self.camera.view_proj(aspect);
        let fog_density = if s.fog { 1.0 / (s.view_distance * 0.75).max(100.0) } else { 0.0 };
        let globals = Globals {
            view_proj: vp.to_cols_array_2d(),
            camera: [eye.x, eye.y, eye.z, 1.0],
            sun: [s.sun[0], s.sun[1], s.sun[2], 0.0],
            fog: [s.sky[0], s.sky[1], s.sky[2], fog_density],
            params: [s.ambient, 0.0, 0.0, 0.0],
        };
        r.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let samples = self.target.as_ref().unwrap().samples;
        let (opaque_pipe, water_pipe) = r.pipelines(samples);
        let frustum = Frustum::from_view_proj(vp);
        let visible: Vec<&GpuTile> = self
            .tiles
            .values()
            .filter(|t| frustum.intersects_aabb(t.min, t.max) && t.min.clamp(eye, t.max).distance(eye) <= s.view_distance + 256.0)
            .collect();
        let mut stats = SceneStats { pending: self.inflight.len(), meshes: self.tiles.len(), ..Default::default() };
        let target = self.target.as_ref().unwrap();
        let mut enc = r.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene") });
        {
            let (view, resolve) = match &target.msaa_view {
                Some(m) => (m, Some(&target.color_view)),
                None => (&target.color_view, None),
            };
            let sky = s.sky.map(|c| (c as f64).powf(2.2));
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: sky[0], g: sky[1], b: sky[2], a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&opaque_pipe);
            for t in &visible {
                if let Some(m) = &t.opaque {
                    pass.set_vertex_buffer(0, m.vb.slice(..));
                    pass.set_vertex_buffer(1, t.origin.slice(..));
                    pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                    stats.triangles += m.count as u64 / 3;
                    stats.tiles_drawn += 1;
                }
            }
            pass.set_pipeline(&water_pipe);
            for t in &visible {
                if let Some(m) = &t.water {
                    pass.set_vertex_buffer(0, m.vb.slice(..));
                    pass.set_vertex_buffer(1, t.origin.slice(..));
                    pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                    stats.triangles += m.count as u64 / 3;
                }
            }
        }
        r.queue.submit([enc.finish()]);
        self.stats = stats;
        &self.target.as_ref().unwrap().egui_view
    }

    /// LOD steps of the meshes currently loaded (diagnostics).
    pub fn tiles_steps(&self) -> std::collections::BTreeSet<i32> {
        self.tiles.values().map(|t| t.step).collect()
    }

    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.color_view)
    }

    /// Block until all meshes for the current view are built (used for screenshots).
    pub fn finish_loading(&mut self, r: &Renderer, aspect: f32) {
        for _ in 0..10_000 {
            self.update(r, aspect);
            if !self.is_busy() {
                // One more pass picks up anything scheduled by the last results.
                self.update(r, aspect);
                if !self.is_busy() {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Read the last rendered frame back to the CPU.
    pub fn read_pixels(&self, r: &Renderer) -> Option<image::RgbaImage> {
        let t = self.target.as_ref()?;
        let (w, h) = t.size;
        let padded = (w * 4).div_ceil(256) * 256;
        let buf = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = r.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &t.color, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        r.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        r.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            let row = &data[(y * padded) as usize..(y * padded + w * 4) as usize];
            img.as_mut()[(y * w * 4) as usize..((y + 1) * w * 4) as usize].copy_from_slice(row);
        }
        drop(data);
        buf.unmap();
        Some(img)
    }

    /// Cast a ray from normalised device coordinates onto the terrain; returns the hit column.
    pub fn pick(&self, ndc_x: f32, ndc_y: f32, aspect: f32) -> Option<(i32, i32, f32)> {
        let world = self.world.as_ref()?;
        let d = world.dimension(self.kind)?;
        let (o, dir) = self.camera.ray(ndc_x, ndc_y, aspect);
        let mut t = 0.0f32;
        let max = self.settings.view_distance;
        let mut step = 1.0f32;
        while t < max {
            let p = o + dir * t;
            if let Some(h) = d.height(p.x.floor() as i32, p.z.floor() as i32)
                && p.y <= h + 1.0
            {
                return Some((p.x.floor() as i32, p.z.floor() as i32, h));
            }
            t += step;
            step = (t * 0.004).clamp(0.5, 4.0);
        }
        None
    }

    pub fn eye_and_forward(&self) -> (Vec3, Vec3) {
        (self.camera.eye(), self.camera.forward())
    }

    pub fn sun_vec(&self) -> Vec4 {
        let s = self.settings.sun;
        Vec4::new(s[0], s[1], s[2], 0.0)
    }
}

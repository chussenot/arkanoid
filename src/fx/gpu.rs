//! The effects on the GPU, shared by both renderers: the HDR scene target,
//! the effect geometry pipeline, and the post-processing that turns the
//! scene into the frame (docs/design/kinetic-fx.md, "Post-processing").
//!
//! A renderer draws its scene (the HUD's backdrop, then its world) into
//! [`FxGpu::scene_view`], calls [`FxGpu::draw_geometry`] in the same pass,
//! then [`FxGpu::finish`] composites the scene onto the surface, and only
//! then draws the HUD overlay there. So:
//! - **The scene is HDR** (`Rgba16Float`). Palette colors stay at or below
//!   1.0 (the scene pipelines clamp), and only effect light goes above: the
//!   bloom's bright-pass at 1.0 selects it by construction, with no mask.
//! - **The HUD is never post-processed.** Its text and chrome are drawn
//!   after the composite, sharp, at full contrast.
//! - **Six passes at most**: a bright-pass to quarter size, two
//!   downsamples, two additive upsamples, the composite.
//! - **None when nothing needs them.** A frame with no effect light and no
//!   shockwave (most frames of ordinary play, and every frame under
//!   `BRYX_FX=geometry` or `off`) is drawn straight to the surface
//!   ([`FxGpu::direct`]), exactly as before the effects existed: the
//!   composite of such a frame would be the same pixels, since its spill
//!   leaves everything at or below 1.0 untouched.
//!
//! The shaders are WESL modules under `src/shaders/`, compiled to WGSL by
//! build.rs.

use std::cell::Cell;
use std::mem::size_of;

use super::{FxFrame, Pulse, Quality, MAX_PULSES};
use crate::fx::geometry::{FxVertex, MAX_INDICES, MAX_VERTICES};

/// The scene's format: half floats, for light above 1.0.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

const GEOMETRY_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/fx_geometry.wgsl"));
const BLOOM_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/bloom.wgsl"));
const COMPOSITE_WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/composite.wgsl"));

/// Bloom only above this, eased in over `BLOOM_KNEE`; added back at
/// `BLOOM_STRENGTH`. Tuned so a flash glows a brick's width or so, never
/// a haze over the field.
const BLOOM_THRESHOLD: f32 = 1.0;
const BLOOM_KNEE: f32 = 0.25;
const BLOOM_STRENGTH: f32 = 0.55;

/// How effect geometry is projected: field pixels flat (2D), or onto the
/// board through the camera (3D), depth-tested against the scene so a wall
/// hides what is behind it.
pub enum Projection {
    Field,
    World { depth: wgpu::TextureFormat },
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniform {
    view_proj: [f32; 16],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct PulseUniform {
    ring: [f32; 4],
    gain: [f32; 4],
}

/// Mirrors `Post` in `src/shaders/fx/params.wesl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    field: [f32; 4],
    bloom: [f32; 4],
    misc: [f32; 4],
    pulses: [PulseUniform; MAX_PULSES],
}

/// Field pixels (0..800 across, 0..600 down) to clip space, for the 2D
/// renderer, whose pass viewport is already the field's rect.
pub fn field_view_proj() -> [f32; 16] {
    [
        2.0 / 800.0,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / 600.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        -1.0,
        1.0,
        0.0,
        1.0,
    ]
}

/// Where the frame's pulses are on the surface: a renderer's own
/// projection of a field point (at a height) to surface pixels, and the
/// surface pixels one field pixel spans there.
pub struct ScreenMap<'a> {
    pub surface: (u32, u32),
    /// The field's rect on the surface: x, y, width, height in pixels.
    pub field: (f32, f32, f32, f32),
    pub project: &'a dyn Fn([f32; 2], f32) -> Option<[f32; 2]>,
    /// Surface pixels per field pixel around `point` (perspective shrinks
    /// it with distance in 3D).
    pub scale: &'a dyn Fn([f32; 2], f32) -> f32,
}

/// One line naming the adapter a renderer got, printed to stderr at start:
/// a frame time means nothing without the GPU and driver it ran on.
pub fn adapter_line(info: &wgpu::AdapterInfo) -> String {
    format!(
        "gpu: {} ({:?}, {:?}, driver {} {})",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    )
}

/// The frame's stages a [`GpuTimer`] times, each with a begin and an end
/// timestamp. The scene is one pass in 2D and two (backdrop, world) in 3D;
/// the bloom chain is five passes timed as one.
#[derive(Debug, Clone, Copy)]
pub enum Stage {
    Scene = 0,
    Bloom = 1,
    Composite = 2,
    Hud = 3,
}

#[allow(dead_code)] // read by src/bin/bench_render3d.rs
pub const STAGE_NAMES: [&str; 4] = ["scene", "bloom", "composite", "hud"];
const STAGES: usize = 4;
/// Bytes per stage in the resolve buffer: a resolve's offset alignment.
const STAGE_STRIDE: wgpu::BufferAddress = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;

/// Which end of its stage a pass marks: both for a single-pass stage, or
/// the begin and the end when the stage spans several passes.
#[derive(Debug, Clone, Copy)]
pub enum Span {
    Whole,
    Begin,
    End,
}

/// Each stage's GPU time in one frame, in milliseconds; `None` for a stage
/// that did not run (bloom and composite on a direct frame, the HUD when
/// it shares the scene's pass).
pub type StageTimes = [Option<f32>; STAGES];

/// Opt-in GPU stage timing (`BRYX_GPU_TIMING=1`) with render-pass
/// timestamps, for `src/bin/bench_render3d.rs`. Off, it costs nothing: no
/// query set, no extra device feature, nothing per frame.
///
/// A renderer requests [`GpuTimer::features`] at device creation, marks
/// its passes with [`FxGpu::timestamps`] (`finish` marks its own), calls
/// [`FxGpu::resolve_timestamps`] before it submits, and, once the GPU has
/// finished the frame, reads the stage times with
/// [`FxGpu::read_timestamps`]. Only the stages a frame wrote are resolved
/// and reported.
pub struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period: f32,
    /// Bit `s` set when stage `s` wrote a timestamp this frame.
    written: Cell<u8>,
}

/// Whether `BRYX_GPU_TIMING`'s value (if set) asks for timing: only `1`.
/// Everything the timer costs hangs on this: without the feature request
/// no device has `TIMESTAMP_QUERY`, so `GpuTimer::new` makes no query set
/// and nothing is written, resolved or read back.
fn timing_requested(var: Option<&str>) -> bool {
    var == Some("1")
}

impl GpuTimer {
    /// The device features timing needs: `TIMESTAMP_QUERY` when asked for
    /// and available, none otherwise.
    pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
        if !timing_requested(std::env::var("BRYX_GPU_TIMING").ok().as_deref()) {
            return wgpu::Features::empty();
        }
        if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            eprintln!("gpu timing: this adapter has no timestamp queries; stage times unavailable");
            wgpu::Features::empty()
        }
    }

    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("fx gpu timer"),
            ty: wgpu::QueryType::Timestamp,
            count: 2 * STAGES as u32,
        });
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: STAGE_STRIDE * STAGES as wgpu::BufferAddress,
                usage,
                mapped_at_creation: false,
            })
        };
        Some(Self {
            set,
            resolve: buffer(
                "fx gpu timer resolve",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            readback: buffer(
                "fx gpu timer readback",
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            ),
            period: queue.get_timestamp_period(),
            written: Cell::new(0),
        })
    }
}

struct Target {
    // Kept alive for its view.
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn target(device: &wgpu::Device, label: &str, w: u32, h: u32) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SCENE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Target {
        _texture: texture,
        view,
    }
}

/// The size-dependent half: the scene and the bloom chain, with the bind
/// groups that read them. Rebuilt on resize.
struct Targets {
    scene: Target,
    /// Quarter, eighth and sixteenth size.
    levels: [Target; 3],
    prefilter: wgpu::BindGroup,
    /// `down[i]` reads level i (to draw level i + 1).
    down: [wgpu::BindGroup; 2],
    /// `up[i]` reads level i + 1 (to add into level i).
    up: [wgpu::BindGroup; 2],
    composite: wgpu::BindGroup,
}

pub struct FxGpu {
    quality: Quality,
    geometry: wgpu::RenderPipeline,
    /// The same, drawing straight to the surface.
    geometry_direct: wgpu::RenderPipeline,
    view_buffer: wgpu::Buffer,
    view_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    post_buffer: wgpu::Buffer,
    pulse_count: usize,
    /// Whether this frame runs the bloom chain: post-processing is on and
    /// something in the frame emits light.
    bloom: bool,
    /// This frame skips post-processing and draws on the surface.
    direct: bool,
    sampler: wgpu::Sampler,
    bloom_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    targets: Targets,
    timer: Option<GpuTimer>,
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const PREMULTIPLIED: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};

#[allow(clippy::too_many_arguments)]
fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    entry: &str,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl FxGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        size: (u32, u32),
        projection: Projection,
        quality: Quality,
    ) -> Self {
        let geometry_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx geometry"),
            source: wgpu::ShaderSource::Wgsl(GEOMETRY_WGSL.into()),
        });
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx view"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let geometry_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx geometry"),
            bind_group_layouts: &[Some(&view_layout)],
            immediate_size: 0,
        });
        let (entry, depth_stencil) = match projection {
            Projection::Field => ("vs_field", None),
            Projection::World { depth } => (
                "vs_world",
                Some(wgpu::DepthStencilState {
                    format: depth,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
            ),
        };
        const ATTRS: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x4];
        let geometry_for = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("fx geometry"),
                layout: Some(&geometry_layout),
                vertex: wgpu::VertexState {
                    module: &geometry_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<FxVertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &ATTRS,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: depth_stencil.clone(),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &geometry_module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(PREMULTIPLIED),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let geometry = geometry_for(SCENE_FORMAT);
        let geometry_direct = geometry_for(surface_format);
        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx view"),
            size: size_of::<ViewUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx view"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx vertices"),
            size: (size_of::<FxVertex>() * MAX_VERTICES) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx indices"),
            size: (size_of::<u16>() * MAX_INDICES) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let post_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx post"),
            size: size_of::<PostUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fx linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx bloom"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx composite"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                sampler_entry(2),
                uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let bloom_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx bloom"),
            source: wgpu::ShaderSource::Wgsl(BLOOM_WGSL.into()),
        });
        let composite_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx composite"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()),
        });
        let prefilter = fullscreen_pipeline(
            device,
            "fx bloom prefilter",
            &bloom_module,
            "fs_prefilter",
            &bloom_layout,
            SCENE_FORMAT,
            None,
        );
        let down = fullscreen_pipeline(
            device,
            "fx bloom down",
            &bloom_module,
            "fs_down",
            &bloom_layout,
            SCENE_FORMAT,
            None,
        );
        let up = fullscreen_pipeline(
            device,
            "fx bloom up",
            &bloom_module,
            "fs_up",
            &bloom_layout,
            SCENE_FORMAT,
            Some(ADDITIVE),
        );
        let composite = fullscreen_pipeline(
            device,
            "fx composite",
            &composite_module,
            "fs_main",
            &composite_layout,
            surface_format,
            None,
        );
        let targets = Self::targets(
            device,
            size,
            &bloom_layout,
            &composite_layout,
            &sampler,
            &post_buffer,
        );
        Self {
            quality,
            geometry,
            geometry_direct,
            view_buffer,
            view_group,
            vertices,
            indices,
            index_count: 0,
            post_buffer,
            pulse_count: 0,
            bloom: false,
            direct: true,
            sampler,
            bloom_layout,
            composite_layout,
            prefilter,
            down,
            up,
            composite,
            targets,
            timer: GpuTimer::new(device, queue),
        }
    }

    fn targets(
        device: &wgpu::Device,
        (w, h): (u32, u32),
        bloom_layout: &wgpu::BindGroupLayout,
        composite_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        post: &wgpu::Buffer,
    ) -> Targets {
        let scene = target(device, "fx scene", w, h);
        let levels = [
            target(device, "fx bloom 1/4", w / 4, h / 4),
            target(device, "fx bloom 1/8", w / 8, h / 8),
            target(device, "fx bloom 1/16", w / 16, h / 16),
        ];
        let bloom_group = |source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx bloom"),
                layout: bloom_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: post.as_entire_binding(),
                    },
                ],
            })
        };
        let prefilter = bloom_group(&scene.view);
        let down = [bloom_group(&levels[0].view), bloom_group(&levels[1].view)];
        let up = [bloom_group(&levels[1].view), bloom_group(&levels[2].view)];
        let composite = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx composite"),
            layout: composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&scene.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&levels[0].view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: post.as_entire_binding(),
                },
            ],
        });
        Targets {
            scene,
            levels,
            prefilter,
            down,
            up,
            composite,
        }
    }

    /// The scene and bloom targets follow the surface: call on every
    /// resize (not for a zero size).
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        self.targets = Self::targets(
            device,
            size,
            &self.bloom_layout,
            &self.composite_layout,
            &self.sampler,
            &self.post_buffer,
        );
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// Whether this frame (as last prepared) skips post-processing: the
    /// renderer draws its scene straight onto the surface, with its
    /// surface-format pipelines, and `finish` does nothing.
    pub fn direct(&self) -> bool {
        self.direct
    }

    /// Where a renderer draws its scene this frame: the HDR target, or
    /// `surface` itself when the frame is [`direct`](Self::direct).
    pub fn scene_view<'a>(&'a self, surface: &'a wgpu::TextureView) -> &'a wgpu::TextureView {
        if self.direct {
            surface
        } else {
            &self.targets.scene.view
        }
    }

    /// Uploads this frame's effect geometry, its projection, and its
    /// shockwaves placed on the surface by `map`.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        frame: &FxFrame,
        view_proj: [f32; 16],
        map: &ScreenMap,
    ) {
        let canvas = &frame.canvas;
        self.index_count = canvas.indices.len() as u32;
        queue.write_buffer(
            &self.view_buffer,
            0,
            bytemuck::bytes_of(&ViewUniform { view_proj }),
        );
        if !canvas.vertices.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&canvas.vertices));
            // A buffer write must be a multiple of 4 bytes: pad an odd
            // count of 16-bit indices with one unused zero.
            let indices = &canvas.indices;
            if indices.len().is_multiple_of(2) {
                queue.write_buffer(&self.indices, 0, bytemuck::cast_slice(indices));
            } else {
                let even = indices.len() - 1;
                queue.write_buffer(&self.indices, 0, bytemuck::cast_slice(&indices[..even]));
                queue.write_buffer(
                    &self.indices,
                    (even * size_of::<u16>()) as wgpu::BufferAddress,
                    bytemuck::cast_slice(&[indices[even], 0]),
                );
            }
        }
        self.bloom = self.quality.post() && frame.glows;
        let uniform = post_uniform(self.quality, self.bloom, &frame.pulses, map);
        self.pulse_count = uniform.misc[0] as usize;
        self.direct = !self.bloom && self.pulse_count == 0;
        queue.write_buffer(&self.post_buffer, 0, bytemuck::bytes_of(&uniform));
        if let Some(timer) = &self.timer {
            timer.written.set(0);
        }
    }

    /// Draws the effect geometry into the scene pass (its viewport set to
    /// the field's rect).
    pub fn draw_geometry(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.index_count == 0 {
            return;
        }
        pass.set_pipeline(if self.direct {
            &self.geometry_direct
        } else {
            &self.geometry
        });
        pass.set_bind_group(0, &self.view_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    /// Bloom (when on) and the composite onto `surface`. The caller draws
    /// the HUD overlay on `surface` afterward.
    pub fn finish(&self, encoder: &mut wgpu::CommandEncoder, surface: &wgpu::TextureView) {
        if self.direct {
            return;
        }
        if self.bloom {
            let t = &self.targets;
            let passes: [(
                &wgpu::RenderPipeline,
                &wgpu::BindGroup,
                &wgpu::TextureView,
                bool,
            ); 5] = [
                (&self.prefilter, &t.prefilter, &t.levels[0].view, false),
                (&self.down, &t.down[0], &t.levels[1].view, false),
                (&self.down, &t.down[1], &t.levels[2].view, false),
                (&self.up, &t.up[1], &t.levels[1].view, true),
                (&self.up, &t.up[0], &t.levels[0].view, true),
            ];
            for (i, (pipeline, group, view, add)) in passes.into_iter().enumerate() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fx bloom"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if add {
                                wgpu::LoadOp::Load
                            } else {
                                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: match i {
                        0 => self.timestamps(Stage::Bloom, Span::Begin),
                        4 => self.timestamps(Stage::Bloom, Span::End),
                        _ => None,
                    },
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fx composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: self.timestamps(Stage::Composite, Span::Whole),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &self.targets.composite, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The timestamp writes for a pass of `stage`; `None` when timing is
    /// off, so a pass descriptor can take it directly.
    pub fn timestamps(
        &self,
        stage: Stage,
        span: Span,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let timer = self.timer.as_ref()?;
        let s = stage as u32;
        timer.written.set(timer.written.get() | 1 << s);
        let (begin, end) = match span {
            Span::Whole => (true, true),
            Span::Begin => (true, false),
            Span::End => (false, true),
        };
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &timer.set,
            beginning_of_pass_write_index: begin.then_some(2 * s),
            end_of_pass_write_index: end.then_some(2 * s + 1),
        })
    }

    /// Resolves the stages this frame wrote, after its last pass and
    /// before the submit. Nothing when timing is off.
    pub fn resolve_timestamps(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(timer) = &self.timer else {
            return;
        };
        let written = timer.written.get();
        for s in 0..STAGES as u32 {
            if written & (1 << s) != 0 {
                encoder.resolve_query_set(
                    &timer.set,
                    2 * s..2 * s + 2,
                    &timer.resolve,
                    wgpu::BufferAddress::from(s) * STAGE_STRIDE,
                );
            }
        }
        if written != 0 {
            encoder.copy_buffer_to_buffer(
                &timer.resolve,
                0,
                &timer.readback,
                0,
                STAGE_STRIDE * STAGES as wgpu::BufferAddress,
            );
        }
    }

    /// The stage times of the frame last resolved, once the GPU has
    /// finished it (the caller polled the device). Blocks on the readback;
    /// `None` when timing is off.
    #[allow(dead_code)] // read by src/bin/bench_render3d.rs
    pub fn read_timestamps(&self, device: &wgpu::Device) -> Option<StageTimes> {
        let timer = self.timer.as_ref()?;
        let written = timer.written.get();
        let slice = timer.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("gpu timer readback failed")
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");
        let mut times = [None; STAGES];
        {
            let data = slice
                .get_mapped_range()
                .expect("gpu timer readback is not mapped");
            let tick = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
            for (s, time) in times.iter_mut().enumerate() {
                if written & (1 << s) != 0 {
                    let at = s * STAGE_STRIDE as usize;
                    let ticks = tick(at + 8).wrapping_sub(tick(at));
                    *time = Some(ticks as f32 * timer.period * 1e-6);
                }
            }
        }
        timer.readback.unmap();
        Some(times)
    }

    /// The HDR scene target's size (for the bench's header: proof of what
    /// the fullscreen passes ran over).
    #[allow(dead_code)] // read by src/bin/bench_render3d.rs
    pub fn scene_size(&self) -> (u32, u32) {
        let t = &self.targets.scene._texture;
        (t.width(), t.height())
    }

    /// Passes `finish` records this frame (for the performance report).
    #[allow(dead_code)] // read by src/bin/bench_render3d.rs
    pub fn post_passes(&self) -> usize {
        match (self.direct, self.bloom) {
            (true, _) => 0,
            (false, true) => 6,
            (false, false) => 1,
        }
    }
}

/// The composite's uniform: the field's rect, the bloom's settings, and
/// each pulse moved onto the surface. Pure, for the tests.
fn post_uniform(quality: Quality, bloom: bool, pulses: &[Pulse], map: &ScreenMap) -> PostUniform {
    let (sw, sh) = (map.surface.0.max(1) as f32, map.surface.1.max(1) as f32);
    let (fx, fy, fw, fh) = map.field;
    let mut uniform = PostUniform {
        field: [fx / sw, fy / sh, (fx + fw) / sw, (fy + fh) / sh],
        bloom: [
            BLOOM_THRESHOLD,
            BLOOM_KNEE,
            BLOOM_STRENGTH,
            if bloom { 1.0 } else { 0.0 },
        ],
        misc: [0.0, sw / sh, 0.0, 0.0],
        pulses: [PulseUniform::default(); MAX_PULSES],
    };
    if !quality.post() {
        return uniform;
    }
    let mut n = 0;
    for p in pulses.iter().take(MAX_PULSES) {
        let Some(center) = (map.project)(p.center, p.height) else {
            continue;
        };
        let k = (map.scale)(p.center, p.height) / sh;
        uniform.pulses[n] = PulseUniform {
            ring: [center[0] / sw, center[1] / sh, p.radius * k, p.width * k],
            gain: [p.strength * k, p.light, 0.0, 0.0],
        };
        n += 1;
    }
    uniform.misc[0] = n as f32;
    uniform
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every compiled shader parses and validates as WGSL, headless: the
    /// same front end and validator wgpu runs on them at start.
    #[test]
    fn the_compiled_wesl_shaders_are_valid_wgsl() {
        use wgpu::naga;
        for (name, source) in [
            ("fx_geometry", GEOMETRY_WGSL),
            ("bloom", BLOOM_WGSL),
            ("composite", COMPOSITE_WGSL),
        ] {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }

    #[test]
    fn gpu_timing_is_off_unless_asked_for() {
        for off in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(!timing_requested(off), "{off:?} must request nothing");
        }
        assert!(timing_requested(Some("1")));
    }

    #[test]
    fn the_uniforms_match_their_wgsl_layouts() {
        // Post: three vec4s, then MAX_PULSES pulses of two vec4s each.
        assert_eq!(size_of::<PostUniform>(), 16 * 3 + MAX_PULSES * 32);
        assert_eq!(size_of::<ViewUniform>(), 64);
        assert_eq!(size_of::<FxVertex>(), 28);
        assert!(GEOMETRY_WGSL.contains("vs_field") && GEOMETRY_WGSL.contains("vs_world"));
    }

    fn map_2d<'a>(
        project: &'a dyn Fn([f32; 2], f32) -> Option<[f32; 2]>,
        scale: &'a dyn Fn([f32; 2], f32) -> f32,
    ) -> ScreenMap<'a> {
        ScreenMap {
            surface: (1600, 1296),
            field: (0.0, 96.0, 1600.0, 1200.0),
            project,
            scale,
        }
    }

    #[test]
    fn pulses_land_on_the_surface_where_their_events_did() {
        let project = |p: [f32; 2], _h: f32| Some([p[0] * 2.0, 96.0 + p[1] * 2.0]);
        let scale = |_p: [f32; 2], _h: f32| 2.0;
        let pulse = Pulse {
            center: [400.0, 300.0],
            height: 0.0,
            radius: 50.0,
            width: 10.0,
            strength: 4.0,
            light: 0.1,
        };
        let u = post_uniform(Quality::Full, true, &[pulse], &map_2d(&project, &scale));
        assert_eq!(u.misc[0], 1.0);
        let ring = u.pulses[0].ring;
        assert!((ring[0] - 0.5).abs() < 1e-6 && (ring[1] - (96.0 + 600.0) / 1296.0).abs() < 1e-6);
        assert!(
            (ring[2] - 100.0 / 1296.0).abs() < 1e-6,
            "radius in height units"
        );
        // Field rect, for the mask: below the band.
        assert!((u.field[1] - 96.0 / 1296.0).abs() < 1e-6);
        // Without post-processing, nothing distorts or blooms.
        let off = post_uniform(
            Quality::Geometry,
            false,
            &[pulse],
            &map_2d(&project, &scale),
        );
        assert_eq!((off.misc[0], off.bloom[3]), (0.0, 0.0));
    }

    #[test]
    fn the_field_projection_maps_its_corners_to_clip_space() {
        let m = glam::Mat4::from_cols_array(&field_view_proj());
        let clip = |x: f32, y: f32| m * glam::Vec4::new(x, y, 0.0, 1.0);
        assert_eq!(
            clip(0.0, 0.0).truncate().truncate(),
            glam::Vec2::new(-1.0, 1.0)
        );
        assert_eq!(
            clip(800.0, 600.0).truncate().truncate(),
            glam::Vec2::new(1.0, -1.0)
        );
    }
}

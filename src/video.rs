//! GPU video presentation. Decoded YUV planes are uploaded as textures and
//! converted to RGB in a fragment shader, so no per-pixel work happens on the CPU.

use crate::ffi;
use crate::remote::VideoFrame;
use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use parking_lot::Mutex;
use std::sync::Arc;

const SHADER: &str = r#"
struct Params {
    mode: u32, // 0 = NV12, 1 = I420
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var tex_y: texture_2d<f32>;
@group(0) @binding(1) var tex_u: texture_2d<f32>;
@group(0) @binding(2) var tex_v: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One oversized triangle that covers the viewport.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: VsOut;
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let y = textureSample(tex_y, samp, in.uv).r;
    var cb: f32;
    var cr: f32;
    if (params.mode == 0u) {
        let c = textureSample(tex_u, samp, in.uv).rg;
        cb = c.r;
        cr = c.g;
    } else {
        cb = textureSample(tex_u, samp, in.uv).r;
        cr = textureSample(tex_v, samp, in.uv).r;
    }
    // BT.709, limited range (what the console sends).
    let yl = (y - 16.0 / 255.0) * (255.0 / 219.0);
    let u = (cb - 128.0 / 255.0) * (255.0 / 224.0);
    let v = (cr - 128.0 / 255.0) * (255.0 / 224.0);
    let rgb = vec3<f32>(
        yl + 1.5748 * v,
        yl - 0.1873 * u - 0.4681 * v,
        yl + 1.8556 * u,
    );
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

struct Planes {
    width: u32,
    height: u32,
    format: i32,
    y: wgpu::Texture,
    u: wgpu::Texture,
    v: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    planes: Option<Planes>,
}

/// Owns the GPU resources for the video surface. Cheap to clone.
#[derive(Clone)]
pub struct VideoRenderer {
    gpu: Arc<Mutex<Gpu>>,
}

impl VideoRenderer {
    pub fn new(rs: &egui_wgpu::RenderState) -> Self {
        let device = rs.device.clone();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv_layout"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("yuv_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: rs.target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yuv_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yuv_params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            gpu: Arc::new(Mutex::new(Gpu {
                device,
                queue: rs.queue.clone(),
                pipeline,
                layout,
                sampler,
                params,
                planes: None,
            })),
        }
    }

    /// Size of the last uploaded frame.
    pub fn frame_size(&self) -> Option<[u32; 2]> {
        self.gpu.lock().planes.as_ref().map(|p| [p.width, p.height])
    }

    pub fn clear(&self) {
        self.gpu.lock().planes = None;
    }

    /// Uploads a decoded frame. Returns false for pixel formats we can't show.
    pub fn upload(&self, frame: &VideoFrame) -> bool {
        let fmt = frame.format();
        if fmt != ffi::PIXFMT_NV12 && fmt != ffi::PIXFMT_YUV420P {
            return false;
        }
        let (w, h) = (frame.width(), frame.height());
        if w == 0 || h == 0 {
            return false;
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut gpu = self.gpu.lock();
        let needs_new = gpu
            .planes
            .as_ref()
            .is_none_or(|p| p.width != w || p.height != h || p.format != fmt);
        if needs_new {
            let planes = gpu.create_planes(w, h, fmt);
            let mode: [u32; 4] = [if fmt == ffi::PIXFMT_NV12 { 0 } else { 1 }, 0, 0, 0];
            gpu.queue
                .write_buffer(&gpu.params, 0, bytemuck::cast_slice(&mode));
            gpu.planes = Some(planes);
        }
        let gpu = &*gpu;
        let planes = gpu.planes.as_ref().unwrap();
        let write = |tex: &wgpu::Texture, plane: usize, width: u32, rows: u32| {
            let (data, stride) = frame.plane(plane, rows);
            if data.is_empty() {
                return;
            }
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(rows),
                },
                wgpu::Extent3d {
                    width,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
        };
        write(&planes.y, 0, w, h);
        write(&planes.u, 1, cw, ch);
        if fmt == ffi::PIXFMT_YUV420P {
            write(&planes.v, 2, cw, ch);
        }
        true
    }

    /// Paints the video into `rect`, letterboxed to the stream's aspect ratio.
    pub fn paint(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        let Some([w, h]) = self.frame_size() else {
            return;
        };
        let aspect = w as f32 / h as f32;
        let mut size = rect.size();
        if size.x / size.y > aspect {
            size.x = size.y * aspect;
        } else {
            size.y = size.x / aspect;
        }
        let target = egui::Rect::from_center_size(rect.center(), size);
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            target,
            VideoCallback {
                gpu: self.gpu.clone(),
            },
        ));
    }
}

impl Gpu {
    fn create_planes(&self, w: u32, h: u32, fmt: i32) -> Planes {
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let tex = |label, width, height, format| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let y = tex("video_y", w, h, wgpu::TextureFormat::R8Unorm);
        let (u, v) = if fmt == ffi::PIXFMT_NV12 {
            (
                tex("video_uv", cw, ch, wgpu::TextureFormat::Rg8Unorm),
                tex("video_unused", 1, 1, wgpu::TextureFormat::R8Unorm),
            )
        } else {
            (
                tex("video_u", cw, ch, wgpu::TextureFormat::R8Unorm),
                tex("video_v", cw, ch, wgpu::TextureFormat::R8Unorm),
            )
        };
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let (vy, vu, vv) = (view(&y), view(&u), view(&v));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv_bind_group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&vy),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&vu),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&vv),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        Planes {
            width: w,
            height: h,
            format: fmt,
            y,
            u,
            v,
            bind_group,
        }
    }
}

struct VideoCallback {
    gpu: Arc<Mutex<Gpu>>,
}

impl egui_wgpu::CallbackTrait for VideoCallback {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        let gpu = self.gpu.lock();
        let Some(planes) = gpu.planes.as_ref() else {
            return;
        };
        render_pass.set_pipeline(&gpu.pipeline);
        render_pass.set_bind_group(0, &planes.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

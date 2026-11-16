//! Compositeur sur la carte graphique (wgpu).
//!
//! Chaque image : les sources visibles sont dessinées dans une texture à la
//! taille du canevas (affichée telle quelle dans l'aperçu), puis convertie sur
//! la carte graphique en NV12 (le format natif des encodeurs NVENC, Quick Sync
//! et AMF). Seul le NV12 (1,5 octet par pixel au lieu de 4) est relu vers le
//! processeur pour FFmpeg, sans aucune conversion côté processeur.

use crate::capture::Frame;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const SHADER: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_quad(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs_quad(i: VOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, i.uv);
}

// Triangle couvrant tout l'écran, pour la conversion en NV12.
@vertex
fn vs_full(@builtin(vertex_index) idx: u32) -> VOut {
    let x = f32((idx << 1u) & 2u);
    let y = f32(idx & 2u);
    var o: VOut;
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

// BT.709, plage limitée (16-235), comme OBS par défaut.
@fragment
fn fs_y(i: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv).rgb;
    let y = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(16.0 / 255.0 + y * 219.0 / 255.0, 0.0, 0.0, 1.0);
}

// Demi-résolution : l'échantillonnage bilinéaire au centre de chaque bloc 2×2 en fait la moyenne.
@fragment
fn fs_uv(i: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv).rgb;
    let u = dot(c, vec3<f32>(-0.1146, -0.3854, 0.5));
    let v = dot(c, vec3<f32>(0.5, -0.4542, -0.0458));
    return vec4<f32>(0.5 + u * 224.0 / 255.0, 0.5 + v * 224.0 / 255.0, 0.0, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

/// Source placée sur le canevas (en pixels du canevas).
pub struct DrawItem {
    pub source: u64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

struct SourceTexture {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    width: u32,
    height: u32,
    seq: u64,
}

const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;

/// Tampon de relecture du NV12 (Y puis UV, lignes alignées sur 256 octets).
struct Readback {
    buffer: Arc<wgpu::Buffer>,
    state: Arc<AtomicU8>,
}

pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub width: u32,
    pub height: u32,
    _output: wgpu::Texture,
    output_view: wgpu::TextureView,
    y_tex: wgpu::Texture,
    uv_tex: wgpu::Texture,
    y_view: wgpu::TextureView,
    uv_view: wgpu::TextureView,
    quad_pipeline: wgpu::RenderPipeline,
    y_pipeline: wgpu::RenderPipeline,
    uv_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    output_bind: wgpu::BindGroup,
    sources: HashMap<u64, SourceTexture>,
    vertices: wgpu::Buffer,
    readbacks: Vec<Readback>,
    in_flight: VecDeque<usize>,
    y_stride: u32,
    uv_stride: u32,
}

fn align256(n: u32) -> u32 {
    n.div_ceil(256) * 256
}

const MAX_ITEMS: usize = 64;

impl Compositor {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, width: u32, height: u32) -> Self {
        // NV12 : dimensions paires obligatoires.
        let width = width.max(2) & !1;
        let height = height.max(2) & !1;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compositeur"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |vs: &str,
                        fs: &str,
                        format: wgpu::TextureFormat,
                        blend: Option<wgpu::BlendState>,
                        quad: bool| {
            let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];
            let buffers = [Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attrs,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: if quad { &buffers } else { &[] },
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
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
        };
        let quad_pipeline = pipeline(
            "vs_quad",
            "fs_quad",
            wgpu::TextureFormat::Rgba8Unorm,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            true,
        );
        let y_pipeline = pipeline("vs_full", "fs_y", wgpu::TextureFormat::R8Unorm, None, false);
        let uv_pipeline = pipeline(
            "vs_full",
            "fs_uv",
            wgpu::TextureFormat::Rg8Unorm,
            None,
            false,
        );

        let texture = |label: &str,
                       w: u32,
                       h: u32,
                       format: wgpu::TextureFormat,
                       usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let output = texture(
            "canevas",
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let target = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let y_tex = texture("Y", width, height, wgpu::TextureFormat::R8Unorm, target);
        let uv_tex = texture(
            "UV",
            width / 2,
            height / 2,
            wgpu::TextureFormat::Rg8Unorm,
            target,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bilinéaire"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let output_view = output.create_view(&Default::default());
        let output_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canevas"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sommets"),
            size: (MAX_ITEMS * 6 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let y_stride = align256(width);
        let uv_stride = align256(width); // largeur/2 pixels × 2 octets
        let size = (y_stride * height + uv_stride * (height / 2)) as u64;
        let readbacks = (0..3)
            .map(|_| Readback {
                buffer: Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("relecture NV12"),
                    size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Self {
            y_view: y_tex.create_view(&Default::default()),
            uv_view: uv_tex.create_view(&Default::default()),
            device,
            queue,
            width,
            height,
            _output: output,
            output_view,
            y_tex,
            uv_tex,
            quad_pipeline,
            y_pipeline,
            uv_pipeline,
            layout,
            sampler,
            output_bind,
            sources: HashMap::new(),
            vertices,
            readbacks,
            in_flight: VecDeque::new(),
            y_stride,
            uv_stride,
        }
    }

    /// Texture du canevas, affichée dans l'aperçu.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.output_view
    }

    /// Envoie la dernière image d'une source à la carte graphique, si elle a changé.
    fn upload(&mut self, id: u64, frame: &Frame) {
        let stale = self
            .sources
            .get(&id)
            .map(|s| s.width != frame.width || s.height != frame.height)
            .unwrap_or(true);
        if stale {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("source"),
                size: wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("source"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.sources.insert(
                id,
                SourceTexture {
                    texture,
                    bind,
                    width: frame.width,
                    height: frame.height,
                    seq: 0,
                },
            );
        }
        let src = self.sources.get_mut(&id).unwrap();
        if src.seq == frame.seq {
            return;
        }
        src.seq = frame.seq;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Oublie les textures des sources qui ne sont plus utilisées.
    pub fn retain(&mut self, alive: impl Fn(u64) -> bool) {
        self.sources.retain(|id, _| alive(*id));
    }

    /// Compose une image. Avec `readback`, la convertit en NV12 et lance sa
    /// relecture ; renvoie l'image NV12 la plus récente déjà relue, s'il y en a une.
    pub fn render(
        &mut self,
        items: &[DrawItem],
        frames: &HashMap<u64, Arc<Frame>>,
        readback: bool,
    ) -> Option<Vec<u8>> {
        for item in items {
            if let Some(f) = frames.get(&item.source) {
                self.upload(item.source, f);
            }
        }
        // Sommets : du dernier élément (arrière-plan) au premier (premier plan).
        let (w, h) = (self.width as f32, self.height as f32);
        let mut verts: Vec<Vertex> = Vec::new();
        let mut draws: Vec<u64> = Vec::new();
        for item in items.iter().rev().take(MAX_ITEMS) {
            if !self.sources.contains_key(&item.source) {
                continue;
            }
            let x0 = item.x / w * 2.0 - 1.0;
            let x1 = (item.x + item.w) / w * 2.0 - 1.0;
            let y0 = 1.0 - item.y / h * 2.0;
            let y1 = 1.0 - (item.y + item.h) / h * 2.0;
            let quad = [
                Vertex {
                    pos: [x0, y0],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    pos: [x1, y0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    pos: [x0, y1],
                    uv: [0.0, 1.0],
                },
                Vertex {
                    pos: [x0, y1],
                    uv: [0.0, 1.0],
                },
                Vertex {
                    pos: [x1, y0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    pos: [x1, y1],
                    uv: [1.0, 1.0],
                },
            ];
            verts.extend_from_slice(&quad);
            draws.push(item.source);
        }
        if !verts.is_empty() {
            self.queue
                .write_buffer(&self.vertices, 0, bytemuck::cast_slice(&verts));
        }

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("image"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composition"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.quad_pipeline);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            for (i, id) in draws.iter().enumerate() {
                pass.set_bind_group(0, &self.sources[id].bind, &[]);
                let first = (i * 6) as u32;
                pass.draw(first..first + 6, 0..1);
            }
        }

        // Relecture : un tampon libre parmi trois (l'image arrive avec 1 à 2 images de retard,
        // sans jamais attendre la carte graphique).
        let mut submitted = None;
        if readback {
            if let Some(slot) = self
                .readbacks
                .iter()
                .position(|r| r.state.load(Ordering::Acquire) == FREE)
            {
                for (view, pipeline) in [
                    (&self.y_view, &self.y_pipeline),
                    (&self.uv_view, &self.uv_pipeline),
                ] {
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("NV12"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &self.output_bind, &[]);
                    pass.draw(0..3, 0..1);
                }
                let buffer = &self.readbacks[slot].buffer;
                let copy = |enc: &mut wgpu::CommandEncoder,
                            tex: &wgpu::Texture,
                            offset: u64,
                            stride: u32,
                            w: u32,
                            h: u32| {
                    enc.copy_texture_to_buffer(
                        wgpu::TexelCopyTextureInfo {
                            texture: tex,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyBufferInfo {
                            buffer,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset,
                                bytes_per_row: Some(stride),
                                rows_per_image: Some(h),
                            },
                        },
                        wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                    );
                };
                copy(
                    &mut enc,
                    &self.y_tex,
                    0,
                    self.y_stride,
                    self.width,
                    self.height,
                );
                copy(
                    &mut enc,
                    &self.uv_tex,
                    (self.y_stride * self.height) as u64,
                    self.uv_stride,
                    self.width / 2,
                    self.height / 2,
                );
                submitted = Some(slot);
            }
        }
        self.queue.submit([enc.finish()]);
        if let Some(slot) = submitted {
            let r = &self.readbacks[slot];
            r.state.store(PENDING, Ordering::Release);
            let state = r.state.clone();
            r.buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |res| {
                    state.store(if res.is_ok() { READY } else { FREE }, Ordering::Release)
                });
            self.in_flight.push_back(slot);
        }
        let _ = self.device.poll(wgpu::PollType::Poll);

        // Image relue la plus récente (les plus anciennes, s'il y en a plusieurs, sont libérées).
        let mut out = None;
        while let Some(&slot) = self.in_flight.front() {
            let r = &self.readbacks[slot];
            match r.state.load(Ordering::Acquire) {
                READY => {
                    self.in_flight.pop_front();
                    out = Some(self.extract(slot));
                }
                FREE => {
                    self.in_flight.pop_front();
                }
                _ => break,
            }
        }
        out
    }

    /// Copie le NV12 sans les marges d'alignement des lignes, puis libère le tampon.
    fn extract(&self, slot: usize) -> Vec<u8> {
        let r = &self.readbacks[slot];
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = Vec::with_capacity(w * h * 3 / 2);
        if let Ok(data) = r.buffer.slice(..).get_mapped_range() {
            for row in 0..h {
                let start = row * self.y_stride as usize;
                out.extend_from_slice(&data[start..start + w]);
            }
            let base = self.y_stride as usize * h;
            for row in 0..h / 2 {
                let start = base + row * self.uv_stride as usize;
                out.extend_from_slice(&data[start..start + w]);
            }
        }
        r.buffer.unmap();
        r.state.store(FREE, Ordering::Release);
        out
    }
}

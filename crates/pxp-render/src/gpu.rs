// SPDX-License-Identifier: LGPL-2.1-or-later
//! The wgpu renderer: offscreen viewport textures, shaded faces, B-rep
//! edges, silhouettes, a section plane and GPU picking.
//!
//! ## Frame
//!
//! ```text
//! pass 1 (colour + depth): background gradient ─► shaded faces ─► edges (pulled toward the viewer)
//! pass 2 (colour only):    silhouettes from depth discontinuities (depth texture sampled)
//! pick (on demand):        face ids into an R32Uint texture ─► copy the texel under the cursor ─► map
//! ```
//!
//! The viewport renders into its own textures, never into a window
//! surface, so the same code serves the desktop shell (which shows the
//! colour texture as an image), screenshots, tests and the orbit benchmark
//! (all headless). Depth is reversed (cleared to 0, test `Greater`).
//!
//! ## Precision
//!
//! Positions are uploaded as `f32` relative to the scene origin (the first
//! body's bounding-box centre), so a part 10 km from the model origin still
//! renders without jitter; the camera matrices use the same origin.

use crate::camera::{Camera, V3};
use std::sync::mpsc;
use wgpu::util::DeviceExt as _;

/// Why rendering could not start or finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// No GPU adapter at all, not even a software one.
    NoAdapter(String),
    /// The device could not be created.
    Device(String),
    /// Reading back from the GPU failed.
    Readback(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter(m) => write!(f, "no graphics adapter: {m}"),
            Self::Device(m) => write!(f, "graphics device: {m}"),
            Self::Readback(m) => write!(f, "reading back from the GPU: {m}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A device and queue, with what kind of adapter they run on.
#[derive(Debug, Clone)]
pub struct GpuContext {
    /// The device.
    pub device: wgpu::Device,
    /// Its queue.
    pub queue: wgpu::Queue,
    /// Adapter name, backend and type.
    pub adapter: wgpu::AdapterInfo,
}

impl GpuContext {
    /// Wraps a device someone else created (the desktop shell shares
    /// eframe's device so the viewport texture can be shown directly).
    #[must_use]
    pub fn from_parts(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: wgpu::AdapterInfo,
    ) -> Self {
        Self {
            device,
            queue,
            adapter,
        }
    }

    /// A headless context: the best hardware adapter, or, when there is
    /// none, the OS software rasteriser (llvmpipe/lavapipe on Linux, WARP on
    /// Windows), which [`GpuContext::software_warning`] then reports.
    ///
    /// # Errors
    /// [`RenderError::NoAdapter`] or [`RenderError::Device`].
    pub fn headless() -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let request = |fallback: bool| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: fallback,
                compatible_surface: None,
                ..Default::default()
            }))
        };
        let adapter = request(false)
            .or_else(|_| request(true))
            .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("pxp-render"),
                required_features: wgpu::Features::empty(),
                // Large meshes need big buffers: take what the adapter offers.
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits())
                    .using_alignment(adapter.limits()),
                ..Default::default()
            }),
        )
        .map_err(|e| RenderError::Device(e.to_string()))?;
        Ok(Self {
            device,
            queue,
            adapter: info,
        })
    }

    /// `Some(message)` when the adapter is a software rasteriser: the
    /// master prompt's "fall back to the OS software rasteriser with a
    /// warning".
    #[must_use]
    pub fn software_warning(&self) -> Option<String> {
        (self.adapter.device_type == wgpu::DeviceType::Cpu).then(|| {
            format!(
                "No usable GPU: drawing with the software rasteriser ({}, {:?}). Large models will be slow.",
                self.adapter.name, self.adapter.backend
            )
        })
    }
}

/// A body's display mesh as the renderer takes it: model-space `f64`
/// positions (made relative to the scene origin on upload).
#[derive(Debug, Clone, Default)]
pub struct MeshInput {
    /// Vertex positions (mm).
    pub positions: Vec<[f64; 3]>,
    /// Unit normals, one per position.
    pub normals: Vec<[f32; 3]>,
    /// Triangles (counter-clockwise from outside).
    pub triangles: Vec<[u32; 3]>,
    /// Face index per triangle (picking reports it).
    pub tri_face: Vec<u32>,
    /// Edge polylines to draw (seams already left out).
    pub edges: Vec<Vec<[f64; 3]>>,
}

/// What a pick found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picked {
    /// Index of the body in the scene (upload order).
    pub body: usize,
    /// Face index within the body (its `tri_face` value).
    pub face: u32,
}

/// A pick: the face under the cursor and the model point on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Which body and face.
    pub picked: Picked,
    /// The point on the face under the cursor's pixel centre (mm), from
    /// the depth buffer: as exact as the display mesh (within the chord
    /// tolerance of the true surface).
    pub point: V3,
}

/// Picking packs (body, face) into one `u32`: 0 is the background, ids
/// start at 1; bodies get consecutive ranges.
#[derive(Debug, Clone, Copy)]
struct IdRange {
    first: u32,
    faces: u32,
}

/// How the viewport looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplaySettings {
    /// Draw B-rep edges.
    pub edges: bool,
    /// Draw silhouettes.
    pub silhouettes: bool,
    /// Section plane `n·p + d = 0` in model space; the side where
    /// `n·p + d > 0` is cut away.
    pub section: Option<([f64; 3], f64)>,
    /// Background gradient, top and bottom (linear RGBA).
    pub background: [[f32; 4]; 2],
    /// Edge and silhouette colour.
    pub edge_color: [f32; 4],
    /// Colour of the cut seen through the section.
    pub cap_color: [f32; 4],
    /// Selected body, and face within it (`None`: the whole body), drawn in
    /// the selection colour.
    pub highlight: Option<(usize, Option<u32>)>,
}

impl DisplaySettings {
    /// Light theme: a pale blue-grey gradient (our own colours).
    pub const LIGHT: Self = Self {
        edges: true,
        silhouettes: true,
        section: None,
        background: [[0.62, 0.68, 0.78, 1.0], [0.93, 0.94, 0.96, 1.0]],
        edge_color: [0.02, 0.02, 0.03, 1.0],
        cap_color: [0.75, 0.25, 0.22, 1.0],
        highlight: None,
    };
    /// Dark theme.
    pub const DARK: Self = Self {
        edges: true,
        silhouettes: true,
        section: None,
        background: [[0.10, 0.11, 0.14, 1.0], [0.24, 0.26, 0.31, 1.0]],
        edge_color: [0.0, 0.0, 0.0, 1.0],
        cap_color: [0.80, 0.30, 0.25, 1.0],
        highlight: None,
    };
}

/// Uniform block (matches `struct Frame` in `shaders.wgsl`).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    section: [f32; 4],
    flags: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    edge_color: [f32; 4],
    cap_color: [f32; 4],
    viewport: [f32; 4],
    highlight: [u32; 4],
}

/// One vertex of a face mesh.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    id: u32,
    color: [f32; 4],
}

struct GpuBody {
    /// Display edges kept on the CPU for snapping.
    edge_lines: Vec<Vec<V3>>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    edges: wgpu::Buffer,
    edge_vertex_count: u32,
    ids: IdRange,
}

/// The bodies on screen, uploaded to the GPU.
pub struct Scene {
    bodies: Vec<GpuBody>,
    origin: V3,
    min: V3,
    max: V3,
    next_id: u32,
}

/// Narrows to the GPU's `f32` after moving to the scene origin.
#[allow(clippy::cast_possible_truncation)]
fn rel(p: [f64; 3], o: V3) -> [f32; 3] {
    [
        (p[0] - o[0]) as f32,
        (p[1] - o[1]) as f32,
        (p[2] - o[2]) as f32,
    ]
}

impl Scene {
    /// An empty scene.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            origin: [0.0; 3],
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
            next_id: 1,
        }
    }

    /// Bodies in the scene.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    /// No bodies.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    /// Bounding box of everything uploaded (model space), if anything.
    #[must_use]
    pub fn bounds(&self) -> Option<(V3, V3)> {
        (!self.bodies.is_empty()).then_some((self.min, self.max))
    }

    /// Radius of the bounding sphere (at least 1 mm).
    #[must_use]
    pub fn radius(&self) -> f64 {
        self.bounds().map_or(100.0, |(a, b)| {
            (0.5 * crate::camera::v3::len(crate::camera::v3::sub(b, a))).max(1.0)
        })
    }

    /// Uploads a body with a display colour (linear RGBA). Returns its index.
    pub fn add(&mut self, ctx: &GpuContext, mesh: &MeshInput, color: [f32; 4]) -> usize {
        if self.bodies.is_empty()
            && let Some(first) = mesh.positions.first()
        {
            // The scene origin: near the first body, for f32 precision.
            self.origin = *first;
        }
        for p in &mesh.positions {
            for ((lo, hi), &x) in self.min.iter_mut().zip(self.max.iter_mut()).zip(p.iter()) {
                *lo = lo.min(x);
                *hi = hi.max(x);
            }
        }
        // Picking needs a face id per vertex, but neighbouring faces share
        // boundary vertices: duplicate each vertex once per face using it.
        let faces = mesh.tri_face.iter().copied().max().map_or(0, |m| m + 1);
        let ids = IdRange {
            first: self.next_id,
            faces,
        };
        self.next_id = self.next_id.saturating_add(faces.max(1));
        let mut remap: std::collections::HashMap<(u32, u32), u32> =
            std::collections::HashMap::new();
        let mut vertices: Vec<Vertex> = Vec::with_capacity(mesh.positions.len());
        let mut indices: Vec<u32> = Vec::with_capacity(mesh.triangles.len() * 3);
        for (t, tri) in mesh.triangles.iter().enumerate() {
            let face = mesh.tri_face.get(t).copied().unwrap_or(0);
            for &v in tri {
                let next = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
                let k = *remap.entry((v, face)).or_insert_with(|| {
                    let i = v as usize;
                    vertices.push(Vertex {
                        pos: rel(
                            mesh.positions.get(i).copied().unwrap_or([0.0; 3]),
                            self.origin,
                        ),
                        normal: mesh.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
                        id: ids.first + face,
                        color,
                    });
                    next
                });
                indices.push(k);
            }
        }
        let mut edge_points: Vec<[f32; 3]> = Vec::new();
        for line in &mesh.edges {
            for pair in line.windows(2) {
                if let [a, b] = pair {
                    edge_points.push(rel(*a, self.origin));
                    edge_points.push(rel(*b, self.origin));
                }
            }
        }
        let d = &ctx.device;
        let buffer = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            // wgpu refuses zero-sized vertex buffers in some backends: keep 16 bytes.
            let padded: Vec<u8> = if bytes.is_empty() {
                vec![0; 16]
            } else {
                bytes.to_vec()
            };
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &padded,
                usage,
            })
        };
        self.bodies.push(GpuBody {
            edge_lines: mesh.edges.clone(),
            vertices: buffer(
                "faces",
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: buffer(
                "indices",
                bytemuck::cast_slice(&indices),
                wgpu::BufferUsages::INDEX,
            ),
            index_count: u32::try_from(indices.len()).unwrap_or(u32::MAX),
            edges: buffer(
                "edges",
                bytemuck::cast_slice(&edge_points),
                wgpu::BufferUsages::VERTEX,
            ),
            edge_vertex_count: u32::try_from(edge_points.len()).unwrap_or(u32::MAX),
            ids,
        });
        self.bodies.len() - 1
    }

    /// Pick ids `[first, end)` of a body, or of one face of it.
    #[must_use]
    pub fn id_range(&self, body: usize, face: Option<u32>) -> Option<(u32, u32)> {
        let b = self.bodies.get(body)?;
        Some(match face {
            Some(f) if f < b.ids.faces => (b.ids.first + f, b.ids.first + f + 1),
            Some(_) => return None,
            None => (b.ids.first, b.ids.first + b.ids.faces.max(1)),
        })
    }

    /// A body's display edges (for snapping), model space.
    #[must_use]
    pub fn edges(&self, body: usize) -> &[Vec<V3>] {
        self.bodies
            .get(body)
            .map_or(&[], |b| b.edge_lines.as_slice())
    }

    /// Which body and face a pick id names.
    fn resolve(&self, id: u32) -> Option<Picked> {
        self.bodies.iter().enumerate().find_map(|(body, b)| {
            (id >= b.ids.first && id < b.ids.first + b.ids.faces.max(1)).then(|| Picked {
                body,
                face: id - b.ids.first,
            })
        })
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

/// Colour format of the viewport texture (the shell displays it as is).
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const PICK_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

struct Targets {
    size: [u32; 2],
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    pick_depth: wgpu::Texture,
    pick_depth_view: wgpu::TextureView,
    id: wgpu::Texture,
    id_view: wgpu::TextureView,
    sil_bind: wgpu::BindGroup,
}

/// A viewport: pipelines plus colour, depth and id textures of one size.
pub struct Viewport {
    uniform: wgpu::Buffer,
    bind_layout: wgpu::BindGroupLayout,
    sil_layout: wgpu::BindGroupLayout,
    frame_bind: wgpu::BindGroup,
    bg: wgpu::RenderPipeline,
    mesh: wgpu::RenderPipeline,
    pick: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    sil: wgpu::RenderPipeline,
    targets: Option<Targets>,
}

impl Viewport {
    /// Builds the pipelines.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "one straight list of GPU objects; splitting it hides nothing"
    )]
    pub fn new(ctx: &GpuContext) -> Self {
        let d = &ctx.device;
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pxp viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let uniform = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[uniform_entry],
        });
        let sil_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("silhouette"),
            entries: &[
                uniform_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let frame_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let sil_pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("silhouette"),
            bind_group_layouts: &[Some(&sil_layout)],
            immediate_size: 0,
        });
        let depth = |write: bool| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        };
        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32, 3 => Float32x4],
        })];
        let edge_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
        })];
        let color_target = [Some(wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let id_target = [
            Some(wgpu::ColorTargetState {
                format: ID_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
            Some(wgpu::ColorTargetState {
                format: PICK_DEPTH_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
        ];
        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology: wgpu::PrimitiveTopology,
                        depth_stencil: Option<wgpu::DepthStencilState>,
                        targets: &[Option<wgpu::ColorTargetState>]| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    // No culling: back faces show the section cut.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let tri = wgpu::PrimitiveTopology::TriangleList;
        Self {
            bg: pipeline(
                "background",
                &layout,
                "bg_vs",
                "bg_fs",
                &[],
                tri,
                depth(false),
                &color_target,
            ),
            mesh: pipeline(
                "faces",
                &layout,
                "mesh_vs",
                "mesh_fs",
                &mesh_buffers,
                tri,
                depth(true),
                &color_target,
            ),
            pick: pipeline(
                "pick",
                &layout,
                "mesh_vs",
                "pick_fs",
                &mesh_buffers,
                tri,
                depth(true),
                &id_target,
            ),
            edges: pipeline(
                "edges",
                &layout,
                "edge_vs",
                "edge_fs",
                &edge_buffers,
                wgpu::PrimitiveTopology::LineList,
                depth(false),
                &color_target,
            ),
            sil: pipeline(
                "silhouettes",
                &sil_pipeline_layout,
                "sil_vs",
                "sil_fs",
                &[],
                tri,
                None,
                &color_target,
            ),
            uniform,
            bind_layout,
            sil_layout,
            frame_bind,
            targets: None,
        }
    }

    /// (Re)creates the textures for a new size. Cheap when the size is
    /// unchanged.
    fn ensure_size(&mut self, ctx: &GpuContext, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let d = &ctx.device;
        let tex = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
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
        let color = tex(
            "viewport colour",
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = tex(
            "viewport depth",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let pick_depth = tex(
            "viewport pick depth",
            PICK_DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let id = tex(
            "viewport ids",
            ID_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let sil_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("silhouette"),
            layout: &self.sil_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
            ],
        });
        self.targets = Some(Targets {
            size,
            color_view: color.create_view(&wgpu::TextureViewDescriptor::default()),
            color,
            depth_view,
            pick_depth_view: pick_depth.create_view(&wgpu::TextureViewDescriptor::default()),
            pick_depth,
            id_view: id.create_view(&wgpu::TextureViewDescriptor::default()),
            id,
            sil_bind,
        });
    }

    /// The colour texture of the last frame (for the shell to display).
    #[must_use]
    pub fn color_texture(&self) -> Option<&wgpu::Texture> {
        self.targets.as_ref().map(|t| &t.color)
    }

    /// The colour texture's view.
    #[must_use]
    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.color_view)
    }

    fn write_uniform(
        &self,
        ctx: &GpuContext,
        scene: &Scene,
        camera: &Camera,
        s: &DisplaySettings,
        size: [u32; 2],
    ) {
        let aspect = f64::from(size[0]) / f64::from(size[1].max(1));
        let radius = scene.radius();
        let view = camera.view_matrix(radius, scene.origin);
        let proj = camera.projection_matrix(aspect, radius);
        let (section, on) = match s.section {
            Some((n, d)) => {
                // Move the plane to the scene origin: n·(p' + o) + d = n·p' + (d + n·o).
                let d2 = d + crate::camera::v3::dot(n, scene.origin);
                #[allow(clippy::cast_possible_truncation)]
                let v = [n[0] as f32, n[1] as f32, n[2] as f32, d2 as f32];
                (v, 1.0)
            }
            None => ([0.0; 4], 0.0),
        };
        #[allow(clippy::cast_precision_loss)]
        let u = FrameUniform {
            view_proj: mul4(proj, view),
            view,
            section,
            flags: [on, if s.silhouettes { 1.0 } else { 0.0 }, 2e-5, 0.0],
            bg_top: s.background[0],
            bg_bottom: s.background[1],
            edge_color: s.edge_color,
            cap_color: s.cap_color,
            viewport: [size[0] as f32, size[1] as f32, 0.0, 0.0],
            highlight: s
                .highlight
                .and_then(|(body, face)| scene.id_range(body, face))
                .map_or([0; 4], |(a, b)| [a, b, 0, 0]),
        };
        ctx.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    /// Renders one frame into the viewport's colour texture.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        scene: &Scene,
        camera: &Camera,
        settings: &DisplaySettings,
        size: [u32; 2],
    ) {
        self.ensure_size(ctx, size);
        let Some(t) = &self.targets else { return };
        self.write_uniform(ctx, scene, camera, settings, t.size);
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shaded"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.set_pipeline(&self.bg);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.mesh);
            for b in &scene.bodies {
                pass.set_vertex_buffer(0, b.vertices.slice(..));
                pass.set_index_buffer(b.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.index_count, 0, 0..1);
            }
            if settings.edges {
                pass.set_pipeline(&self.edges);
                for b in scene.bodies.iter().filter(|b| b.edge_vertex_count > 0) {
                    pass.set_vertex_buffer(0, b.edges.slice(..));
                    pass.draw(0..b.edge_vertex_count, 0..1);
                }
            }
        }
        if settings.silhouettes {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("silhouettes"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &t.sil_bind, &[]);
            pass.set_pipeline(&self.sil);
            pass.draw(0..3, 0..1);
        }
        ctx.queue.submit([enc.finish()]);
    }

    /// GPU picking: the face under viewport pixel `at` (from the top-left)
    /// in the last rendered size, or `None` for background.
    ///
    /// # Errors
    /// [`RenderError::Readback`] if the GPU read-back fails.
    pub fn pick(
        &mut self,
        ctx: &GpuContext,
        scene: &Scene,
        camera: &Camera,
        settings: &DisplaySettings,
        at: [u32; 2],
    ) -> Result<Option<Hit>, RenderError> {
        let Some(t) = &self.targets else {
            return Ok(None);
        };
        if at[0] >= t.size[0] || at[1] >= t.size[1] {
            return Ok(None);
        }
        self.write_uniform(ctx, scene, camera, settings, t.size);
        let mut enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pick"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pick"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &t.id_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &t.pick_depth_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // Only the pixel under the cursor matters.
            pass.set_scissor_rect(at[0], at[1], 1, 1);
            pass.set_bind_group(0, &self.frame_bind, &[]);
            pass.set_pipeline(&self.pick);
            for b in &scene.bodies {
                pass.set_vertex_buffer(0, b.vertices.slice(..));
                pass.set_index_buffer(b.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.index_count, 0, 0..1);
            }
        }
        let word = |b: &[u8]| b.get(..4).and_then(|b| <[u8; 4]>::try_from(b).ok());
        let id = word(&read_texture(
            ctx,
            enc,
            &t.id,
            wgpu::TextureAspect::All,
            at,
            [1, 1],
            4,
        )?)
        .map_or(0, u32::from_le_bytes);
        let Some(picked) = (id != 0).then(|| scene.resolve(id)).flatten() else {
            return Ok(None);
        };
        // The pick pass also wrote each fragment's depth to a float target:
        // read the same texel and turn it back into a model point.
        let enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pick depth"),
            });
        let depth = word(&read_texture(
            ctx,
            enc,
            &t.pick_depth,
            wgpu::TextureAspect::All,
            at,
            [1, 1],
            4,
        )?)
        .map_or(0.0, f32::from_le_bytes);
        let size = [f64::from(t.size[0]), f64::from(t.size[1])];
        let centre = [f64::from(at[0]) + 0.5, f64::from(at[1]) + 0.5];
        let point = camera.unproject(centre, size, f64::from(depth), scene.radius());
        Ok(Some(Hit { picked, point }))
    }

    /// The last frame as tightly packed RGBA8 (sRGB) rows, top row first.
    ///
    /// # Errors
    /// [`RenderError::Readback`].
    pub fn read_pixels(&self, ctx: &GpuContext) -> Result<(Vec<u8>, [u32; 2]), RenderError> {
        let Some(t) = &self.targets else {
            return Ok((Vec::new(), [0, 0]));
        };
        let enc = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("read"),
            });
        Ok((
            read_texture(
                ctx,
                enc,
                &t.color,
                wgpu::TextureAspect::All,
                [0, 0],
                t.size,
                4,
            )?,
            t.size,
        ))
    }

    /// The bind group layout of the frame uniform (exposed for tests).
    #[must_use]
    pub fn frame_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_layout
    }
}

/// Copies a texture region to a buffer, waits, and returns tightly packed
/// rows (`texel` bytes per texel).
fn read_texture(
    ctx: &GpuContext,
    mut enc: wgpu::CommandEncoder,
    tex: &wgpu::Texture,
    aspect: wgpu::TextureAspect,
    origin: [u32; 2],
    size: [u32; 2],
    texel: u32,
) -> Result<Vec<u8>, RenderError> {
    let row = size[0] * texel;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin[0],
                y: origin[1],
                z: 0,
            },
            aspect,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    rx.recv()
        .map_err(|e| RenderError::Readback(e.to_string()))?
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    let data = slice
        .get_mapped_range()
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    let mut out = Vec::with_capacity((row * size[1]) as usize);
    for r in 0..size[1] {
        let start = (r * padded) as usize;
        out.extend_from_slice(data.get(start..start + row as usize).unwrap_or(&[]));
    }
    drop(data);
    buffer.unmap();
    Ok(out)
}

/// `a · b` for column-major 4×4 matrices.
fn mul4(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (j, col) in b.iter().enumerate() {
        for (i, o) in out
            .get_mut(j)
            .into_iter()
            .flat_map(|c| c.iter_mut())
            .enumerate()
        {
            *o = (0..4)
                .map(|k| {
                    a.get(k).and_then(|ak| ak.get(i)).copied().unwrap_or(0.0)
                        * col.get(k).copied().unwrap_or(0.0)
                })
                .sum();
        }
    }
    out
}

//! GPU-instanced scene renderer, material inheritance, and opacity compositing.
//!
//! [`SceneRenderer`] manages per-definition shared [`GeometryBuffers`] and
//! dynamic GPU instance buffers for highly repeated components. A single
//! definition can be placed thousands of times across the scene while sharing
//! the exact same vertex buffers and drawing in a single instanced draw call.
//!
//! # Normal Transformation
//! Normal vectors are transformed using column-major inverse-transpose matrices
//! computed in [`InstanceData::new`], correctly preserving surface perpendicularity
//! under non-uniform scaling and reflection/mirroring.
//!
//! # Material Hierarchy & Opacity
//! Faces with explicit material assignments (`explicit_material = true`) take precedence
//! over inherited container materials. When a face lacks explicit material, it inherits
//! the container's appearance, multiplying face opacity and inherited container opacity.
//! Inactive context geometry is dimmed by the instance dim factor (e.g. 0.25).
//!
//! # Two-Pass Transparency & Sorting Limitations
//! Geometry is drawn into a transparent offscreen target in two passes:
//! 1. **Opaque pass**: Depth-write enabled (`LessEqual`), fragments with `alpha < 0.999` are discarded.
//! 2. **Edge pass**: Line geometry drawn with depth writes enabled.
//! 3. **Transparent pass**: Depth-write disabled (`LessEqual`), fragments with `alpha >= 0.999` are discarded,
//!    blending with premultiplied alpha over the opaque pass.
//!
//! *Sorting limitations*: Overlapping transparent surfaces are submitted in definition order
//! without per-triangle back-to-front depth sorting.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use bytemuck::{Pod, Zeroable};
use cindral_core::DefinitionId;
use egui_wgpu::{wgpu, CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::mesh::{EdgeVertex, FaceVertex, GeometryBuffers, MeshUpdate};

/// Offscreen color target format (transparent RGBA).
const OFFSCREEN_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Offscreen depth target format.
const OFFSCREEN_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Minimum byte capacity allocated for a GPU buffer.
const MIN_BUFFER_BYTES: u64 = 256;

/// Byte size of a single [`FaceVertex`].
const FACE_VERTEX_BYTES: u64 = std::mem::size_of::<FaceVertex>() as u64;

/// Byte size of a single [`EdgeVertex`].
const EDGE_VERTEX_BYTES: u64 = std::mem::size_of::<EdgeVertex>() as u64;

/// Byte size of a single [`InstanceData`].
const INSTANCE_DATA_BYTES: u64 = std::mem::size_of::<InstanceData>() as u64;

/// Per-instance GPU transformation and material parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InstanceData {
    /// Column-major local-to-world model transform.
    pub model: [[f32; 4]; 4],
    /// Column-major inverse-transpose normal matrix for lighting.
    pub normal: [[f32; 4]; 4],
    /// Inherited linear RGBA container material.
    pub material: [f32; 4],
    /// Parameters:
    /// - `[0]`: 1.0 if inherited container material is enabled, 0.0 otherwise.
    /// - `[1]`: dim factor (1.0 for active context, 0.25 for outside/inactive context).
    /// - `[2..]`: reserved for future use.
    pub parameters: [f32; 4],
}

impl InstanceData {
    /// Creates instance data with computed inverse-transpose normal matrix.
    pub fn new(model: glam::Mat4, material: Option<[f32; 4]>, dim: f32) -> Self {
        let normal_mat =
            if model.is_finite() && model.determinant() != 0.0 && model.inverse().is_finite() {
                model.inverse().transpose()
            } else {
                glam::Mat4::IDENTITY
            };
        let (mat_arr, inherited_enabled) = match material {
            Some(m) => (m, 1.0_f32),
            None => ([0.0, 0.0, 0.0, 0.0], 0.0_f32),
        };
        Self {
            model: model.to_cols_array_2d(),
            normal: normal_mat.to_cols_array_2d(),
            material: mat_arr,
            parameters: [inherited_enabled, dim, 0.0, 0.0],
        }
    }
}

/// Definition geometry delta or full rebuild.
#[derive(Clone, Debug)]
pub struct DefinitionUpdate {
    /// Target definition identity.
    pub definition: DefinitionId,
    /// Mesh vertex delta.
    pub mesh: Arc<MeshUpdate>,
}

/// Batch of instance placements for a single definition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceBatch {
    /// Target definition identity.
    pub definition: DefinitionId,
    /// All instance transforms and appearance parameters for this definition.
    pub instances: Vec<InstanceData>,
}

/// Complete scene delta for geometry and instance placement.
#[derive(Clone, Debug, Default)]
pub struct SceneUpdate {
    /// Geometry updates for definitions whose meshes changed.
    pub geometry: Vec<DefinitionUpdate>,
    /// Updated instance batches. When `None`, existing instance batches are retained.
    pub batches: Option<Vec<InstanceBatch>>,
    /// Definitions removed from the document, whose GPU buffers should be freed.
    pub removed: Vec<DefinitionId>,
}

fn align_capacity(size: u64) -> u64 {
    let align = wgpu::COPY_BUFFER_ALIGNMENT;
    size.div_ceil(align) * align
}

/// Persistent GPU buffer for instanced component occurrences.
struct InstanceBuffer {
    buffer: wgpu::Buffer,
    capacity_bytes: u64,
    instance_count: u32,
    previous: Vec<InstanceData>,
}

impl InstanceBuffer {
    fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cindral scene instance buffer"),
            size: MIN_BUFFER_BYTES,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            capacity_bytes: MIN_BUFFER_BYTES,
            instance_count: 0,
            previous: Vec::new(),
        }
    }

    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, instances: &[InstanceData]) {
        self.instance_count = instances.len() as u32;
        if instances.is_empty() {
            self.previous.clear();
            return;
        }
        let needed_bytes = (instances.len() as u64) * INSTANCE_DATA_BYTES;
        if needed_bytes > self.capacity_bytes {
            let new_cap = align_capacity(
                self.capacity_bytes
                    .saturating_mul(2)
                    .max(needed_bytes.saturating_mul(2))
                    .max(MIN_BUFFER_BYTES),
            );
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Cindral scene instance buffer"),
                size: new_cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.capacity_bytes = new_cap;
            self.previous.clear();
        }
        for range in changed_instance_ranges(&self.previous, instances) {
            queue.write_buffer(
                &self.buffer,
                range.start as u64 * INSTANCE_DATA_BYTES,
                bytemuck::cast_slice(&instances[range]),
            );
        }
        self.previous.clear();
        self.previous.extend_from_slice(instances);
    }
}

fn changed_instance_ranges(
    before: &[InstanceData],
    after: &[InstanceData],
) -> Vec<std::ops::Range<usize>> {
    if before.len() != after.len() {
        return if after.is_empty() {
            Vec::new()
        } else {
            std::iter::once(0..after.len()).collect()
        };
    }
    let mut ranges = Vec::new();
    let mut start = None;
    for i in 0..=after.len() {
        let changed = i < after.len() && before[i] != after[i];
        if changed {
            start.get_or_insert(i);
        } else if let Some(first) = start.take() {
            ranges.push(first..i);
        }
    }
    ranges
}

/// GPU resources allocated for a single geometry definition.
struct DefinitionState {
    geometry: GeometryBuffers,
    instances: InstanceBuffer,
}

struct OffscreenTarget {
    size: [u32; 2],
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    composite_bind_group: wgpu::BindGroup,
}

/// Persistent GPU instanced renderer for scene definitions and occurrences.
pub struct SceneRenderer {
    definitions: BTreeMap<DefinitionId, DefinitionState>,
    uniform: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    opaque_face_pipeline: wgpu::RenderPipeline,
    transparent_face_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,
    offscreen: OffscreenTarget,
    last_update: Option<Weak<SceneUpdate>>,
    draw_calls: usize,
}

impl SceneRenderer {
    /// Creates pipelines, shared shaders, and initial offscreen targets for the
    /// viewport's color `format` and MSAA `samples`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cindral scene camera uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cindral scene camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cindral scene camera binding"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });

        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cindral scene shader module"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });

        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Cindral scene pipeline layout"),
                bind_group_layouts: &[&camera_layout],
                push_constant_ranges: &[],
            });

        let face_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: FACE_VERTEX_BYTES,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // location 0: position
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                // location 1: normal
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
                // location 2: color
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 24,
                    shader_location: 2,
                },
                // location 3: material
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 40,
                    shader_location: 3,
                },
            ],
        };

        let edge_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: EDGE_VERTEX_BYTES,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
            ],
        };

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: INSTANCE_DATA_BYTES,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // model matrix: locations 4..8
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 7,
                },
                // normal matrix: locations 8..12
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 8,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 80,
                    shader_location: 9,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 96,
                    shader_location: 10,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 112,
                    shader_location: 11,
                },
                // inherited material: location 12
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 128,
                    shader_location: 12,
                },
                // parameters: location 13
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 144,
                    shader_location: 13,
                },
            ],
        };

        let opaque_face_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cindral scene opaque face pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_face"),
                compilation_options: Default::default(),
                buffers: &[face_vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_face_opaque"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: OFFSCREEN_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let transparent_face_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Cindral scene transparent face pipeline"),
                layout: Some(&scene_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &scene_shader,
                    entry_point: Some("vs_face"),
                    compilation_options: Default::default(),
                    buffers: &[face_vertex_layout, instance_layout.clone()],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &scene_shader,
                    entry_point: Some("fs_face_transparent"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: OFFSCREEN_COLOR_FORMAT,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: OFFSCREEN_DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 1.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cindral scene edge pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_edge"),
                compilation_options: Default::default(),
                buffers: &[edge_vertex_layout, instance_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_edge"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: OFFSCREEN_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cindral scene composite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cindral scene composite layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Cindral scene composite pipeline layout"),
                bind_group_layouts: &[&composite_layout],
                push_constant_ranges: &[],
            });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cindral scene composite pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples.max(1),
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });

        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Cindral scene composite sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let offscreen =
            create_offscreen_target(device, &composite_layout, &composite_sampler, [1, 1]);

        Self {
            definitions: BTreeMap::new(),
            uniform,
            camera_bind_group,
            opaque_face_pipeline,
            transparent_face_pipeline,
            edge_pipeline,
            composite_pipeline,
            composite_layout,
            composite_sampler,
            offscreen,
            last_update: None,
            draw_calls: 0,
        }
    }

    /// Applies geometry updates, instance batches, and removals to persistent GPU buffers.
    ///
    /// - Removed definitions have their GPU geometry and instance buffers deallocated.
    /// - Dirty writes affect only the supplied definitions.
    /// - Instance-only batch updates (`geometry` empty) update instance buffers without
    ///   reuploading geometry.
    pub fn apply_update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        update: &SceneUpdate,
    ) {
        for removed_id in &update.removed {
            self.definitions.remove(removed_id);
        }

        for geom_update in &update.geometry {
            let state = self
                .definitions
                .entry(geom_update.definition)
                .or_insert_with(|| DefinitionState {
                    geometry: GeometryBuffers::new(device),
                    instances: InstanceBuffer::new(device),
                });
            state.geometry.apply(device, queue, &geom_update.mesh);
        }

        if let Some(batches) = &update.batches {
            for batch in batches {
                let state =
                    self.definitions
                        .entry(batch.definition)
                        .or_insert_with(|| DefinitionState {
                            geometry: GeometryBuffers::new(device),
                            instances: InstanceBuffer::new(device),
                        });
                state.instances.update(device, queue, &batch.instances);
            }
        }
    }

    /// Total capacity in bytes currently allocated across per-definition geometry and instance GPU buffers.
    pub fn allocated_bytes(&self) -> u64 {
        self.definitions
            .values()
            .map(|d| d.geometry.allocated_bytes() + d.instances.capacity_bytes)
            .sum()
    }

    /// Number of GPU draw calls executed during the last scene render pass.
    pub fn draw_calls(&self) -> usize {
        self.draw_calls
    }

    /// Current offscreen target dimensions in physical pixels.
    pub fn target_size(&self) -> [u32; 2] {
        self.offscreen.size
    }

    /// Returns a reference to the [`GeometryBuffers`] for `definition`, if allocated.
    pub fn geometry(&self, definition: DefinitionId) -> Option<&GeometryBuffers> {
        self.definitions.get(&definition).map(|d| &d.geometry)
    }

    /// Returns the active instance count for `definition`, if allocated.
    pub fn instance_count(&self, definition: DefinitionId) -> Option<u32> {
        self.definitions
            .get(&definition)
            .map(|d| d.instances.instance_count)
    }

    fn apply_update_once(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        update: &Arc<SceneUpdate>,
    ) {
        let already_applied = self
            .last_update
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|prev| Arc::ptr_eq(&prev, update));
        if !already_applied {
            self.apply_update(device, queue, update);
            self.last_update = Some(Arc::downgrade(update));
        }
    }

    fn ensure_target_size(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        if size[0] == 0 || size[1] == 0 {
            return false;
        }
        let max_dim = device.limits().max_texture_dimension_2d.max(1);
        let clamped = [size[0].min(max_dim), size[1].min(max_dim)];
        if self.offscreen.size != clamped {
            self.offscreen = create_offscreen_target(
                device,
                &self.composite_layout,
                &self.composite_sampler,
                clamped,
            );
        }
        true
    }

    /// Encode the offscreen scene pass independently of egui (also used by benchmarks).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_projection: [[f32; 4]; 4],
        size: [u32; 2],
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&view_projection));
        if self.ensure_target_size(device, size) {
            self.render_offscreen(encoder);
        }
    }

    fn render_offscreen(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let mut draw_calls = 0;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Cindral scene offscreen pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.offscreen.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.offscreen.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_bind_group(0, &self.camera_bind_group, &[]);

            // 1. Opaque face pass (depth writes enabled)
            pass.set_pipeline(&self.opaque_face_pipeline);
            for state in self.definitions.values() {
                if state.geometry.face_vertices() > 0 && state.instances.instance_count > 0 {
                    pass.set_vertex_buffer(0, state.geometry.face_buffer().slice(..));
                    pass.set_vertex_buffer(1, state.instances.buffer.slice(..));
                    pass.draw(
                        0..state.geometry.face_vertices(),
                        0..state.instances.instance_count,
                    );
                    draw_calls += 1;
                }
            }

            // 2. Edge pass (depth writes enabled)
            pass.set_pipeline(&self.edge_pipeline);
            for state in self.definitions.values() {
                if state.geometry.edge_vertices() > 0 && state.instances.instance_count > 0 {
                    pass.set_vertex_buffer(0, state.geometry.edge_buffer().slice(..));
                    pass.set_vertex_buffer(1, state.instances.buffer.slice(..));
                    pass.draw(
                        0..state.geometry.edge_vertices(),
                        0..state.instances.instance_count,
                    );
                    draw_calls += 1;
                }
            }

            // 3. Transparent face pass (depth writes disabled, premultiplied alpha blending)
            pass.set_pipeline(&self.transparent_face_pipeline);
            for state in self.definitions.values() {
                if state.geometry.face_vertices() > 0 && state.instances.instance_count > 0 {
                    pass.set_vertex_buffer(0, state.geometry.face_buffer().slice(..));
                    pass.set_vertex_buffer(1, state.instances.buffer.slice(..));
                    pass.draw(
                        0..state.geometry.face_vertices(),
                        0..state.instances.instance_count,
                    );
                    draw_calls += 1;
                }
            }
        }
        self.draw_calls = draw_calls;
    }
}

fn create_offscreen_target(
    device: &wgpu::Device,
    composite_layout: &wgpu::BindGroupLayout,
    composite_sampler: &wgpu::Sampler,
    size: [u32; 2],
) -> OffscreenTarget {
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Cindral scene offscreen color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Cindral scene offscreen depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Cindral scene composite bind group"),
        layout: composite_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(composite_sampler),
            },
        ],
    });
    OffscreenTarget {
        size,
        color_view,
        depth_view,
        composite_bind_group,
    }
}

/// Per-frame `egui-wgpu` callback payload for rendering cached instanced scene geometry.
#[derive(Clone, Debug)]
pub struct SceneCallback {
    /// Column-major view-projection matrix using wgpu's `0..1` depth range.
    pub view_projection: [[f32; 4]; 4],
    /// Viewport dimensions in physical pixels (`[width, height]`).
    pub size: [u32; 2],
    /// Optional pending scene update to upload during `prepare`.
    pub update: Option<Arc<SceneUpdate>>,
}

impl CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(renderer) = resources.get_mut::<SceneRenderer>() else {
            return Vec::new();
        };
        if let Some(update) = &self.update {
            renderer.apply_update_once(device, queue, update);
        }
        queue.write_buffer(
            &renderer.uniform,
            0,
            bytemuck::cast_slice(&self.view_projection),
        );
        if renderer.ensure_target_size(device, self.size) {
            renderer.render_offscreen(encoder);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if self.size[0] == 0 || self.size[1] == 0 {
            return;
        }
        if let Some(renderer) = resources.get::<SceneRenderer>() {
            pass.set_pipeline(&renderer.composite_pipeline);
            pass.set_bind_group(0, &renderer.offscreen.composite_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3, Vec4};

    #[test]
    fn instance_data_memory_layout_and_pod() {
        assert_eq!(std::mem::size_of::<InstanceData>(), 160);
        assert_eq!(INSTANCE_DATA_BYTES, 160);

        let zeroed = InstanceData::zeroed();
        assert_eq!(zeroed.model, [[0.0; 4]; 4]);
        assert_eq!(zeroed.normal, [[0.0; 4]; 4]);
        assert_eq!(zeroed.material, [0.0; 4]);
        assert_eq!(zeroed.parameters, [0.0; 4]);

        let slice = [zeroed];
        let bytes: &[u8] = bytemuck::cast_slice(&slice);
        assert_eq!(bytes.len(), 160);
        let decoded: &[InstanceData] = bytemuck::cast_slice(bytes);
        assert_eq!(decoded[0], zeroed);
    }

    #[test]
    fn instance_data_identity_and_dim_factors() {
        let inst = InstanceData::new(Mat4::IDENTITY, None, 1.0);
        assert_eq!(inst.model, Mat4::IDENTITY.to_cols_array_2d());
        assert_eq!(inst.normal, Mat4::IDENTITY.to_cols_array_2d());
        assert_eq!(inst.material, [0.0; 4]);
        assert_eq!(inst.parameters[0], 0.0); // inherited material disabled
        assert_eq!(inst.parameters[1], 1.0); // active dim = 1.0

        let outside = InstanceData::new(Mat4::IDENTITY, None, 0.25);
        assert_eq!(outside.parameters[1], 0.25); // outside dim = 0.25
    }

    #[test]
    fn instance_data_inherited_material_activation() {
        let mat = [0.8, 0.4, 0.2, 0.6];
        let inst = InstanceData::new(Mat4::IDENTITY, Some(mat), 0.25);
        assert_eq!(inst.material, mat);
        assert_eq!(inst.parameters[0], 1.0); // inherited material enabled
        assert_eq!(inst.parameters[1], 0.25);
    }

    #[test]
    fn instance_data_nonuniform_scale_inverse_transpose_normal() {
        // Non-uniform scale: X * 2.0, Y * 1.0, Z * 0.5
        let scale = Vec3::new(2.0, 1.0, 0.5);
        let model = Mat4::from_scale(scale);
        let inst = InstanceData::new(model, None, 1.0);

        let normal_mat = Mat4::from_cols_array_2d(&inst.normal);
        // Inverse transpose of diag(2, 1, 0.5) is diag(0.5, 1, 2)
        let expected_normal_mat = Mat4::from_scale(Vec3::new(0.5, 1.0, 2.0));
        for c in 0..4 {
            for r in 0..4 {
                assert!(
                    (normal_mat.col(c)[r] - expected_normal_mat.col(c)[r]).abs() < 1e-5,
                    "mismatch at col {c} row {r}"
                );
            }
        }

        // Test that a transformed normal stays perpendicular to transformed tangents:
        // Original surface on XY plane: normal = (0, 0, 1), tangent1 = (1, 0, 0), tangent2 = (0, 1, 0)
        let n = Vec4::new(0.0, 0.0, 1.0, 0.0);
        let t1 = Vec4::new(1.0, 0.0, 0.0, 0.0);
        let t2 = Vec4::new(0.0, 1.0, 0.0, 0.0);

        let transformed_n = (normal_mat * n).truncate().normalize();
        let transformed_t1 = (model * t1).truncate();
        let transformed_t2 = (model * t2).truncate();

        assert!(transformed_n.dot(transformed_t1).abs() < 1e-5);
        assert!(transformed_n.dot(transformed_t2).abs() < 1e-5);
    }

    #[test]
    fn instance_data_mirror_reflection() {
        // Mirror along X: scale (-1, 1, 1)
        let model = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let inst = InstanceData::new(model, None, 1.0);
        let normal_mat = Mat4::from_cols_array_2d(&inst.normal);

        let n = Vec4::new(1.0, 0.0, 0.0, 0.0);
        let transformed_n = (normal_mat * n).truncate().normalize();
        assert_eq!(transformed_n, Vec3::new(-1.0, 0.0, 0.0));
    }

    #[test]
    fn instance_data_translation_invariance() {
        let model = Mat4::from_translation(Vec3::new(100.0, -50.0, 25.0));
        let inst = InstanceData::new(model, None, 1.0);
        let normal_mat = Mat4::from_cols_array_2d(&inst.normal);

        let n = Vec4::new(0.0, 1.0, 0.0, 0.0);
        let transformed_n = (normal_mat * n).truncate();
        assert_eq!(transformed_n, Vec3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn instance_data_singular_fallback() {
        let singular = Mat4::from_scale(Vec3::new(0.0, 0.0, 0.0));
        let inst = InstanceData::new(singular, None, 1.0);
        for col in inst.normal {
            for val in col {
                assert!(val.is_finite());
            }
        }
        assert_eq!(inst.normal, Mat4::IDENTITY.to_cols_array_2d());
    }

    #[test]
    fn simulated_material_hierarchy_and_opacity_multiplication() {
        fn evaluate_fragment(
            face_color: [f32; 4],
            explicit_material: bool,
            inherited_color: [f32; 4],
            inherited_enabled: bool,
            dim: f32,
        ) -> (Vec3, f32) {
            let (rgb, alpha) = if explicit_material {
                (Vec3::from_slice(&face_color[0..3]), face_color[3])
            } else if inherited_enabled {
                (
                    Vec3::from_slice(&inherited_color[0..3]),
                    face_color[3] * inherited_color[3],
                )
            } else {
                (Vec3::from_slice(&face_color[0..3]), face_color[3])
            };
            (rgb * dim, alpha)
        }

        let face_red = [1.0, 0.0, 0.0, 0.8];
        let container_blue = [0.0, 0.0, 1.0, 0.5];

        // Case 1: Explicit face material wins over container
        let (rgb1, a1) = evaluate_fragment(face_red, true, container_blue, true, 1.0);
        assert_eq!(rgb1, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(a1, 0.8);

        // Case 2: Inherited container material used when not explicit; opacities multiply
        let (rgb2, a2) = evaluate_fragment(face_red, false, container_blue, true, 1.0);
        assert_eq!(rgb2, Vec3::new(0.0, 0.0, 1.0));
        assert!((a2 - 0.4).abs() < 1e-6); // 0.8 * 0.5 = 0.4

        // Case 3: Inherited container disabled -> face color and face opacity
        let (rgb3, a3) = evaluate_fragment(face_red, false, container_blue, false, 1.0);
        assert_eq!(rgb3, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(a3, 0.8);

        // Case 4: Dimming outside context (0.25)
        let (rgb4, a4) = evaluate_fragment(face_red, false, container_blue, true, 0.25);
        assert_eq!(rgb4, Vec3::new(0.0, 0.0, 0.25));
        assert!((a4 - 0.4).abs() < 1e-6);
    }

    #[test]
    fn instance_batch_5000_instances_layout() {
        let def_id = DefinitionId(42);
        let count = 5000;
        let instances: Vec<InstanceData> = (0..count)
            .map(|i| {
                let x = (i as f32) * 2.0;
                InstanceData::new(
                    Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
                    Some([0.5, 0.6, 0.7, 1.0]),
                    1.0,
                )
            })
            .collect();

        let batch = InstanceBatch {
            definition: def_id,
            instances,
        };
        assert_eq!(batch.instances.len(), 5000);
        let raw_bytes: &[u8] = bytemuck::cast_slice(&batch.instances);
        assert_eq!(raw_bytes.len(), 5000 * 160);
    }

    #[test]
    fn scene_update_structure_and_defaults() {
        let update = SceneUpdate::default();
        assert!(update.geometry.is_empty());
        assert!(update.batches.is_none());
        assert!(update.removed.is_empty());

        let def_update = DefinitionUpdate {
            definition: DefinitionId(1),
            mesh: Arc::new(MeshUpdate::default()),
        };
        let batch = InstanceBatch {
            definition: DefinitionId(1),
            instances: vec![InstanceData::new(Mat4::IDENTITY, None, 1.0)],
        };
        let custom_update = SceneUpdate {
            geometry: vec![def_update],
            batches: Some(vec![batch]),
            removed: vec![DefinitionId(99)],
        };
        assert_eq!(custom_update.geometry.len(), 1);
        assert_eq!(custom_update.batches.as_ref().unwrap().len(), 1);
        assert_eq!(custom_update.removed, vec![DefinitionId(99)]);
    }
}

#[cfg(test)]
mod scale_regression {
    use super::*;
    #[test]
    fn tiny_mirrored_scale_retains_inverse_transpose_normals() {
        let model = glam::Mat4::from_scale(glam::Vec3::new(-1e-5, 2e-5, 3e-5));
        let instance = InstanceData::new(model, None, 1.);
        let normal = glam::Mat4::from_cols_array_2d(&instance.normal);
        assert!((normal.x_axis.x + 100000.).abs() < 0.1);
        assert!((normal.y_axis.y - 50000.).abs() < 0.1);
    }
    #[test]
    fn changed_placements_upload_only_coalesced_matrix_spans() {
        let original = vec![InstanceData::new(glam::Mat4::IDENTITY, None, 1.); 5000];
        let mut edited = original.clone();
        edited[123].parameters[1] = 0.25;
        edited[124].parameters[1] = 0.25;
        edited[4999].parameters[1] = 0.25;
        assert_eq!(
            changed_instance_ranges(&original, &edited),
            vec![123..125, 4999..5000]
        );
        assert!(changed_instance_ranges(&edited, &edited).is_empty());
        assert_eq!(changed_instance_ranges(&[], &edited), vec![0..5000]);
    }
}

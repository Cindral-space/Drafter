//! Cached directional shadow map, independent of document geometry buffers.
use cindral_core::{SectionPlane, ShadowQuality, ShadowSettings};
use egui_wgpu::wgpu;
use glam::{Mat4, Vec3};
pub(crate) struct ShadowMap {
    pub depth_binding: wgpu::BindGroup,
    depth_layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
    pub edge_pipeline: wgpu::RenderPipeline,
    pub from_edges: bool,
    pub view: wgpu::TextureView,
    pub ground_pipeline: Option<wgpu::RenderPipeline>,
    pub uniform: wgpu::Buffer,
    pub sampler: wgpu::Sampler,
    pub resolution: u32,
    pub dirty: bool,
    pub enabled: bool,
    pub ground: bool,
    pub renders: u64,
    key: Vec<u32>,
}
fn depth_texture(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Sun depth map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
fn depth_binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    params: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Sun depth material binding"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(params),
            },
        ],
    })
}
/// Tight orthographic fit to all eight bounds corners in the sun coordinate frame.
pub fn light_matrix(bounds: [Vec3; 2], sun: Vec3) -> Mat4 {
    let center = (bounds[0] + bounds[1]) * 0.5;
    let radius = (bounds[1] - bounds[0]).length().max(1.);
    let up = if sun.dot(Vec3::Z).abs() > 0.98 {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let view = Mat4::look_at_rh(center + sun * radius * 2., center, up);
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for x in [bounds[0].x, bounds[1].x] {
        for y in [bounds[0].y, bounds[1].y] {
            for z in [bounds[0].z.min(0.), bounds[1].z.max(0.)] {
                let p = view.transform_point3(Vec3::new(x, y, z));
                min = min.min(p);
                max = max.max(p);
            }
        }
    }
    let padding = radius * 0.005 + 0.01;
    Mat4::orthographic_rh(
        min.x - padding,
        max.x + padding,
        min.y - padding,
        max.y + padding,
        (-max.z - padding).max(0.01),
        -min.z + padding,
    ) * view
}
impl ShadowMap {
    pub fn new(
        device: &wgpu::Device,
        face: wgpu::VertexBufferLayout<'_>,
        edge: wgpu::VertexBufferLayout<'_>,
        instance: wgpu::VertexBufferLayout<'_>,
        materials: &wgpu::TextureView,
        material_sampler: &wgpu::Sampler,
        material_params: &wgpu::TextureView,
    ) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sun uniform"),
            size: 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shadow depth layout"),
            entries: &[
                entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let depth_binding = depth_binding(
            device,
            &depth_layout,
            &uniform,
            materials,
            material_sampler,
            material_params,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PCF comparison sampler"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let view = depth_texture(device, 2048);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sun depth shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Sun depth pipeline layout"),
            bind_group_layouts: &[&depth_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Sun shadow depth pass"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &[face, instance.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shadow"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.,
                    clamp: 0.,
                },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Optional edge shadow pass"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow_edge"),
                compilation_options: Default::default(),
                buffers: &[edge, instance],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shadow"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.,
                    clamp: 0.,
                },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Self {
            depth_layout,
            depth_binding,
            pipeline,
            edge_pipeline,
            from_edges: false,
            view,
            ground_pipeline: None,
            uniform,
            sampler,
            resolution: 2048,
            dirty: true,
            enabled: false,
            ground: false,
            renders: 0,
            key: Vec::new(),
        }
    }
    pub fn set_materials(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        params: &wgpu::TextureView,
    ) {
        self.depth_binding = depth_binding(
            device,
            &self.depth_layout,
            &self.uniform,
            view,
            sampler,
            params,
        );
        self.dirty = true;
    }
    pub fn configure(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: &ShadowSettings,
        bounds: Option<[Vec3; 2]>,
        section: Option<&SectionPlane>,
    ) {
        let resolution = match settings.quality {
            ShadowQuality::Low => 1024,
            ShadowQuality::Medium => 2048,
            ShadowQuality::High => 4096,
        };
        let resolution = resolution.min(device.limits().max_texture_dimension_2d);
        if resolution != self.resolution {
            self.resolution = resolution;
            self.view = depth_texture(device, resolution);
            self.dirty = true;
        }
        let sun = cindral_core::sun_direction(settings).as_vec3();
        if self.from_edges != settings.from_edges {
            self.from_edges = settings.from_edges;
            self.dirty = true;
        }
        self.enabled = settings.enabled && sun.z > 0. && bounds.is_some();
        self.ground = settings.enabled && settings.on_ground && bounds.is_some();
        let bounds = bounds.unwrap_or([Vec3::splat(-1.), Vec3::ONE]);
        let matrix = light_matrix(bounds, sun);
        let clip = section.map_or([0.; 4], |p| {
            [
                p.normal[0] as f32,
                p.normal[1] as f32,
                p.normal[2] as f32,
                -p.distance as f32,
            ]
        });
        let mut key = matrix.to_cols_array().map(f32::to_bits).to_vec();
        key.extend(clip.map(f32::to_bits));
        key.push(resolution);
        if key != self.key {
            self.key = key;
            self.dirty = true;
        }
        let mut values = matrix.to_cols_array().to_vec();
        values.extend([sun.x, sun.y, sun.z, settings.light]);
        values.extend([
            f32::from(self.enabled),
            settings.dark,
            f32::from(settings.on_faces),
            f32::from(settings.enabled),
        ]);
        values.extend(clip);
        let pad = (bounds[1] - bounds[0]).length() * 0.1 + 1.;
        values.extend([
            bounds[0].x - pad,
            bounds[0].y - pad,
            bounds[1].x + pad,
            bounds[1].y + pad,
        ]);
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&values));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_contains_all_corners_and_ground() {
        let bounds = [Vec3::new(-2., -4., 1.), Vec3::new(3., 2., 7.)];
        let m = light_matrix(bounds, Vec3::new(0.4, -0.3, 0.8).normalize());
        for x in [-2., 3.] {
            for y in [-4., 2.] {
                for z in [0., 1., 7.] {
                    let p = m.project_point3(Vec3::new(x, y, z));
                    assert!(p.x.abs() <= 1. && p.y.abs() <= 1. && p.z >= 0. && p.z <= 1.);
                }
            }
        }
    }
}

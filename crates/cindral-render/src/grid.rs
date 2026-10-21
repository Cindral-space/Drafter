//! Persistent GPU line geometry for the construction grid and world axes.

use bytemuck::{Pod, Zeroable};
use egui_wgpu::{wgpu, CallbackResources, CallbackTrait, ScreenDescriptor};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

/// Cached GPU resources. Geometry is uploaded once, camera uniforms per frame.
pub struct GridRenderer {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    count: u32,
}

impl GridRenderer {
    /// Create resources for the viewport's color target and sample count.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let vertices = grid_vertices();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cindral grid vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cindral camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cindral camera layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cindral camera binding"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cindral grid shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("grid.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cindral grid pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cindral grid pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            vertices: buffer,
            uniform,
            bind_group,
            count: vertices.len() as u32,
        }
    }
}

fn grid_vertices() -> Vec<Vertex> {
    let mut vertices = Vec::new();
    let mut line = |a, b, color| {
        vertices.push(Vertex { position: a, color });
        vertices.push(Vertex { position: b, color });
    };
    for i in -50..=50 {
        if i == 0 {
            continue;
        }
        let value = i as f32;
        let color = if i % 5 == 0 {
            [0.72, 0.74, 0.68]
        } else {
            [0.83, 0.84, 0.78]
        };
        line([value, -50.0, 0.0], [value, 50.0, 0.0], color);
        line([-50.0, value, 0.0], [50.0, value, 0.0], color);
    }
    line([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0], [0.8, 0.12, 0.15]);
    line([0.0, -50.0, 0.0], [0.0, 50.0, 0.0], [0.14, 0.62, 0.34]);
    line([0.0, 0.0, -50.0], [0.0, 0.0, 50.0], [0.15, 0.4, 0.95]);
    vertices
}

/// Camera snapshot passed to the GPU callback for one viewport.
pub struct GridCallback {
    /// Column-major view-projection matrix using wgpu's 0..1 depth range.
    pub view_projection: [[f32; 4]; 4],
}

impl CallbackTrait for GridCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(grid) = resources.get::<GridRenderer>() {
            queue.write_buffer(
                &grid.uniform,
                0,
                bytemuck::cast_slice(&self.view_projection),
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(grid) = resources.get::<GridRenderer>() {
            pass.set_pipeline(&grid.pipeline);
            pass.set_bind_group(0, &grid.bind_group, &[]);
            pass.set_vertex_buffer(0, grid.vertices.slice(..));
            pass.draw(0..grid.count, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn grid_is_finite_line_pairs_with_three_axes() {
        let vertices = super::grid_vertices();
        assert_eq!(vertices.len(), 406);
        assert!(vertices
            .iter()
            .all(|v| v.position.iter().all(|p| p.is_finite())));
        assert_eq!(vertices[vertices.len() - 1].position, [0.0, 0.0, 50.0]);
    }
}

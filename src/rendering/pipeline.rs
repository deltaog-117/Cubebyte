//! The `wgpu` pipeline: surface, depth buffer and one draw per chunk mesh.
//!
//! This is the only module that talks to the GPU. It takes any window handle through
//! `wgpu::SurfaceTarget`, so it does not depend on `winit`.

use std::collections::HashMap;

use glam::Mat4;
use thiserror::Error;
use wgpu::util::DeviceExt;

use crate::rendering::camera::Camera;
use crate::rendering::fog::Fog;
use crate::rendering::vertex::GpuVertex;
use crate::shared::voxel::ChunkPos;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SKY: wgpu::Color = wgpu::Color {
    r: 0.53,
    g: 0.75,
    b: 0.95,
    a: 1.0,
};

/// What the shaders read each frame. Padded to vec4s, which is how a uniform block lays out.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    /// The camera's position in `xyz`.
    eye: [f32; 4],
    /// Fog start in `x` and end in `y`.
    fog: [f32; 4],
    /// The sky colour fog fades into.
    sky: [f32; 4],
    /// How strongly shut-in corners darken, in `x`: 0.0 is not at all, 1.0 is black.
    shading: [f32; 4],
}

/// How strongly the shader darkens a fully shut-in corner. Terrain is not re-meshed to change
/// it.
const OCCLUSION_STRENGTH: f32 = 0.5;

const SHADER: &str = "
struct Camera {
    view_projection: mat4x4<f32>,
    eye: vec4<f32>,
    fog: vec4<f32>,
    sky: vec4<f32>,
    shading: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) openness: f32,
};

struct Fragment {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) openness: f32,
};

@vertex
fn vs_main(vertex: Vertex) -> Fragment {
    return Fragment(
        camera.view_projection * vec4<f32>(vertex.position, 1.0),
        vertex.color,
        vertex.position,
        vertex.openness,
    );
}

// The interface is already in clip space, so it skips the camera.
@vertex
fn vs_overlay(vertex: Vertex) -> Fragment {
    return Fragment(vec4<f32>(vertex.position, 1.0), vertex.color, vertex.position, 1.0);
}

// Same blend as `Fog::factor`: clear up to the start, all sky from the end.
@fragment
fn fs_main(fragment: Fragment) -> @location(0) vec4<f32> {
    let distance = length(fragment.world - camera.eye.xyz);
    let amount = clamp((distance - camera.fog.x) / (camera.fog.y - camera.fog.x), 0.0, 1.0);
    let lit = fragment.color * (1.0 - camera.shading.x * (1.0 - fragment.openness));
    return vec4<f32>(mix(lit, camera.sky.rgb, amount), 1.0);
}

// The interface is never fogged.
@fragment
fn fs_overlay(fragment: Fragment) -> @location(0) vec4<f32> {
    return vec4<f32>(fragment.color, 1.0);
}
";

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("cannot create a surface for the window")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no graphics adapter can draw to this window")]
    NoAdapter(#[from] wgpu::RequestAdapterError),
    #[error("cannot open the graphics device")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("the surface does not support this adapter")]
    UnsupportedSurface,
    #[error("cannot draw a frame")]
    Frame(#[from] wgpu::SurfaceError),
}

/// A mesh that lives on the GPU.
struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    /// Draws the interface over everything else, ignoring depth.
    overlay_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    fog: Fog,
    meshes: HashMap<ChunkPos, GpuMesh>,
    /// Everything that moves, rebuilt every frame by [`Renderer::set_entities`].
    entities: Option<GpuMesh>,
    /// The interface drawn on top, replaced by [`Renderer::set_overlay`].
    overlay: Option<GpuMesh>,
}

impl Renderer {
    /// # Errors
    ///
    /// Fails when no adapter or device is available for the window.
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        (width, height): (u32, u32),
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(target)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..wgpu::RequestAdapterOptions::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;
        let config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or(RenderError::UnsupportedSurface)?;
        surface.configure(&device, &config);

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[&camera_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &GpuVertex::ATTRIBUTES,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(config.format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_overlay"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &GpuVertex::ATTRIBUTES,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_overlay"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(config.format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let depth_view = depth_view(&device, &config);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            overlay_pipeline,
            camera_buffer,
            camera_group,
            depth_view,
            fog: Fog::OFF,
            meshes: HashMap::new(),
            entities: None,
            overlay: None,
        })
    }

    /// Shows `chunk` with this mesh, replacing whatever it showed before. An empty mesh
    /// removes the chunk from the scene.
    pub fn set_mesh(&mut self, chunk: ChunkPos, vertices: &[GpuVertex], indices: &[u32]) {
        match self.upload(vertices, indices) {
            Some(mesh) => {
                self.meshes.insert(chunk, mesh);
            }
            None => {
                self.meshes.remove(&chunk);
            }
        }
    }

    /// Shows the moving things with this mesh, replacing the previous frame's. An empty mesh
    /// shows none.
    pub fn set_entities(&mut self, vertices: &[GpuVertex], indices: &[u32]) {
        self.entities = self.upload(vertices, indices);
    }

    /// Shows the interface with this mesh, in clip-space positions, over the scene and
    /// replacing the previous one. An empty mesh shows none.
    pub fn set_overlay(&mut self, vertices: &[GpuVertex], indices: &[u32]) {
        self.overlay = self.upload(vertices, indices);
    }

    /// Sets where fog starts and ends, so far terrain fades into the sky.
    pub fn set_fog(&mut self, fog: Fog) {
        self.fog = fog;
    }

    /// The window's drawable size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Copies a mesh to the GPU. `None` for an empty mesh, and for one too large to draw.
    fn upload(&self, vertices: &[GpuVertex], indices: &[u32]) -> Option<GpuMesh> {
        let Ok(index_count) = u32::try_from(indices.len()) else {
            tracing::error!(len = indices.len(), "mesh has too many indices to draw");
            return None;
        };
        if index_count == 0 {
            return None;
        }
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        Some(GpuMesh {
            vertices: vertex_buffer,
            indices: index_buffer,
            index_count,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = depth_view(&self.device, &self.config);
    }

    /// Draws every mesh from `camera`'s point of view.
    ///
    /// A lost or outdated surface is reconfigured and the frame skipped.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Frame`] for failures a new surface cannot fix, such as running
    /// out of memory.
    pub fn render(&mut self, camera: &Camera) -> Result<(), RenderError> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let aspect = self.config.width as f32 / self.config.height as f32;
        let view_projection: Mat4 = camera.view_projection(aspect);
        let eye = camera.position;
        let uniform = CameraUniform {
            view_projection: view_projection.to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 0.0],
            fog: [self.fog.start(), self.fog.end(), 0.0, 0.0],
            sky: [SKY.r as f32, SKY.g as f32, SKY.b as f32, 1.0],
            shading: [OCCLUSION_STRENGTH, 0.0, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SKY),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_group, &[]);
            for mesh in self.meshes.values().chain(&self.entities) {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            if let Some(mesh) = &self.overlay {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }
}

fn depth_view(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

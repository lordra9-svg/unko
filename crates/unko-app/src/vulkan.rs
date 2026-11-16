//! Offscreen wgpu/Vulkan renderer scaffold.
//!
//! This is the first backend slice: it creates a Vulkan device, uploads the
//! emulator's RGBA frame into a GPU texture and renders a fullscreen triangle
//! into an offscreen target that can be read back. It is deliberately not
//! wired into the egui shell yet; the existing glow path remains the default.
//!
//! Enable with `--features vulkan`. On systems without a Vulkan adapter
//! `VulkanRenderer::new` returns an error and callers should fall back to glow.

use std::sync::mpsc;

/// RGBA output of one offscreen render.
pub struct VulkanRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    _sampler: wgpu::Sampler,
    frame_texture: wgpu::Texture,
    _frame_view: wgpu::TextureView,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row_bytes: u32,
    adapter_name: String,
}

impl VulkanRenderer {
    /// Creates a Vulkan device and the offscreen pipeline.
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("frame size must be non-zero".into());
        }
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "no Vulkan adapter available".to_string())?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("unko-vulkan"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|err| format!("Vulkan device ({}): {err}", info.name))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("unko-vulkan-fullscreen"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("unko-vulkan-frame-layout"),
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
            label: Some("unko-vulkan-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("unko-vulkan-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let frame_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("unko-vulkan-frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let frame_view = frame_texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("unko-vulkan-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("unko-vulkan-frame-bind"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&frame_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("unko-vulkan-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());
        let padded_row_bytes = width * 4
            + (wgpu::COPY_BYTES_PER_ROW_ALIGNMENT - width * 4 % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unko-vulkan-readback"),
            size: u64::from(padded_row_bytes) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group,
            _sampler: sampler,
            frame_texture,
            _frame_view: frame_view,
            target,
            target_view,
            readback,
            width,
            height,
            padded_row_bytes,
            adapter_name: info.name,
        })
    }

    /// Adapter name reported by wgpu.
    #[must_use]
    pub fn adapter_name(&self) -> String {
        self.adapter_name.clone()
    }

    /// Renders one RGBA frame and returns the RGBA readback.
    pub fn render(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        let expected = self.width as usize * self.height as usize * 4;
        if frame.len() != expected {
            return Err(format!(
                "frame is {} bytes, expected {expected}",
                frame.len()
            ));
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("unko-vulkan-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("unko-vulkan-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row_bytes),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = self.readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .map_err(|err| format!("readback channel: {err}"))?
            .map_err(|err| format!("readback map: {err}"))?;
        let mapped = slice.get_mapped_range();
        let row_bytes = self.width as usize * 4;
        let mut out = Vec::with_capacity(row_bytes * self.height as usize);
        for row in 0..self.height as usize {
            let start = row * self.padded_row_bytes as usize;
            out.extend_from_slice(&mapped[start..start + row_bytes]);
        }
        drop(mapped);
        self.readback.unmap();
        Ok(out)
    }
}

const SHADER: &str = r"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VsOut;
    out.position = vec4<f32>(positions[index], 0.0, 1.0);
    out.uv = uvs[index];
    return out;
}

@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    return textureSample(frame_texture, frame_sampler, input.uv);
}
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offscreen_vulkan_round_trips_a_frame() {
        let mut renderer = match VulkanRenderer::new(256, 240) {
            Ok(renderer) => renderer,
            Err(err) => {
                eprintln!("skipping Vulkan test: {err}");
                return;
            }
        };
        let mut frame = vec![0u8; 256 * 240 * 4];
        for (index, pixel) in frame.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pixel[0] = (index % 256) as u8;
            pixel[1] = (index / 256) as u8;
            pixel[2] = 0x40;
            pixel[3] = 0xff;
        }
        let out = renderer.render(&frame).expect("render");
        assert_eq!(out.len(), frame.len());
        assert_eq!(&out[..4], &frame[..4]);
        let last = (256 * 240 - 1) * 4;
        assert_eq!(&out[last..last + 4], &frame[last..last + 4]);
    }
}

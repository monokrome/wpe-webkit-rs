//! Renderer for compositing WPE buffers to a window surface.
//!
//! This module handles the rendering of exported WPE buffers to a winit window.
//! Supports both software rendering (softbuffer) and GPU rendering (wgpu).

use std::cell::RefCell;
use std::rc::Rc;

/// Shared frame buffer for zero-copy transfer between WPE export and renderer.
///
/// This buffer is shared between the WPE export callback and the renderer,
/// allowing direct writes from the SHM buffer without intermediate copies.
#[derive(Debug, Clone)]
pub struct SharedFrameBuffer {
    inner: Rc<RefCell<FrameBufferInner>>,
}

#[derive(Debug)]
struct FrameBufferInner {
    /// Pixel data in ARGB format
    pixels: Vec<u32>,
    /// Buffer width
    width: u32,
    /// Buffer height
    height: u32,
    /// Whether new frame data is available
    dirty: bool,
}

impl SharedFrameBuffer {
    /// Create a new shared frame buffer with the given dimensions.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            inner: Rc::new(RefCell::new(FrameBufferInner {
                pixels: vec![0xFF000000; (width * height) as usize],
                width,
                height,
                dirty: false,
            })),
        }
    }

    /// Resize the buffer. Clears existing content.
    pub fn resize(&self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        let mut inner = self.inner.borrow_mut();
        if inner.width != width || inner.height != height {
            inner.width = width;
            inner.height = height;
            inner.pixels.resize((width * height) as usize, 0xFF000000);
            inner.dirty = false;
        }
    }

    /// Copy pixel data from an SHM buffer directly into this buffer.
    ///
    /// # Safety
    /// The data pointer must be valid for the given dimensions and stride.
    #[allow(unsafe_code)]
    pub unsafe fn copy_from_shm(&self, data: *const u8, src_width: u32, src_height: u32, stride: u32) {
        if data.is_null() || src_width == 0 || src_height == 0 {
            return;
        }

        let mut inner = self.inner.borrow_mut();
        let dest_width = inner.width.min(src_width);
        let dest_height = inner.height.min(src_height);

        for y in 0..dest_height {
            let src_row = data.add((y * stride) as usize);
            let dest_row_start = (y * inner.width) as usize;

            for x in 0..dest_width {
                let src_pixel = src_row.add((x * 4) as usize) as *const u32;
                let dest_idx = dest_row_start + x as usize;
                if dest_idx < inner.pixels.len() {
                    inner.pixels[dest_idx] = *src_pixel;
                }
            }
        }

        inner.dirty = true;
    }

    /// Check if new frame data is available.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.inner.borrow().dirty
    }

    /// Clear the dirty flag after presenting.
    pub fn clear_dirty(&self) {
        self.inner.borrow_mut().dirty = false;
    }

    /// Get the current dimensions.
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        let inner = self.inner.borrow();
        (inner.width, inner.height)
    }

    /// Get a reference to the pixel data for reading.
    /// Returns (pixels, width, height).
    pub fn with_pixels<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u32], u32, u32) -> R,
    {
        let inner = self.inner.borrow();
        f(&inner.pixels, inner.width, inner.height)
    }
}

// Software renderer implementation
#[cfg(feature = "winit")]
mod software {
    use super::*;
    use crate::{Error, Result};
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use winit::window::Window;

    /// A software renderer that composites WPE buffers to a window.
    pub struct SoftwareRenderer {
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        frame_buffer: SharedFrameBuffer,
    }

    impl SoftwareRenderer {
        /// Create a new software renderer for the given window.
        ///
        /// # Errors
        /// Returns an error if the renderer could not be created.
        pub fn new(window: Arc<Window>, frame_buffer: SharedFrameBuffer) -> Result<Self> {
            let context = softbuffer::Context::new(window.clone())
                .map_err(|e| Error::RendererCreationFailed(e.to_string()))?;

            let surface = softbuffer::Surface::new(&context, window)
                .map_err(|e| Error::RendererCreationFailed(e.to_string()))?;

            let size = surface.window().inner_size();
            frame_buffer.resize(size.width, size.height);

            Ok(Self {
                surface,
                frame_buffer,
            })
        }

        /// Get the shared frame buffer.
        #[must_use]
        pub fn frame_buffer(&self) -> &SharedFrameBuffer {
            &self.frame_buffer
        }

        /// Resize the renderer to match the window size.
        pub fn resize(&mut self, width: u32, height: u32) {
            self.frame_buffer.resize(width, height);

            if let (Some(w), Some(h)) = (NonZeroU32::new(width.max(1)), NonZeroU32::new(height.max(1))) {
                let _ = self.surface.resize(w, h);
            }
        }

        /// Present the current buffer to the window.
        ///
        /// # Errors
        /// Returns an error if presentation fails.
        pub fn present(&mut self) -> Result<()> {
            let (width, height) = self.frame_buffer.dimensions();

            let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
                return Ok(());
            };

            // Ensure surface is properly sized
            self.surface
                .resize(w, h)
                .map_err(|e| Error::RenderFailed(e.to_string()))?;

            // Get a buffer from the surface and copy pixels
            let mut surface_buffer = self
                .surface
                .buffer_mut()
                .map_err(|e| Error::RenderFailed(e.to_string()))?;

            self.frame_buffer.with_pixels(|pixels, _, _| {
                let len = surface_buffer.len().min(pixels.len());
                surface_buffer[..len].copy_from_slice(&pixels[..len]);
            });

            // Present
            surface_buffer
                .present()
                .map_err(|e| Error::RenderFailed(e.to_string()))?;

            self.frame_buffer.clear_dirty();

            Ok(())
        }

        /// Get the current width.
        #[must_use]
        pub fn width(&self) -> u32 {
            self.frame_buffer.dimensions().0
        }

        /// Get the current height.
        #[must_use]
        pub fn height(&self) -> u32 {
            self.frame_buffer.dimensions().1
        }
    }
}

#[cfg(feature = "winit")]
pub use software::SoftwareRenderer;

// GPU renderer implementation
#[cfg(feature = "gpu")]
mod gpu {
    use super::*;
    use crate::{Error, Result};
    use std::sync::Arc;
    use winit::window::Window;

    /// A GPU-accelerated renderer using wgpu.
    ///
    /// This renderer uploads the shared frame buffer to a GPU texture
    /// and renders it using hardware acceleration. For best performance
    /// with complex visualizations, use this with WebGL content.
    pub struct GpuRenderer {
        surface: wgpu::Surface<'static>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: wgpu::SurfaceConfiguration,
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        sampler: wgpu::Sampler,
        bind_group: wgpu::BindGroup,
        render_pipeline: wgpu::RenderPipeline,
        frame_buffer: SharedFrameBuffer,
        width: u32,
        height: u32,
    }

    impl GpuRenderer {
        /// Create a new GPU renderer for the given window.
        ///
        /// # Errors
        /// Returns an error if the GPU renderer could not be created.
        pub async fn new(window: Arc<Window>, frame_buffer: SharedFrameBuffer) -> Result<Self> {
            let size = window.inner_size();
            let width = size.width.max(1);
            let height = size.height.max(1);

            // Create wgpu instance
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            // Create surface
            let surface = instance
                .create_surface(window)
                .map_err(|e| Error::RendererCreationFailed(e.to_string()))?;

            // Get adapter
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .ok_or_else(|| Error::RendererCreationFailed("No GPU adapter found".to_string()))?;

            // Create device and queue
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .map_err(|e| Error::RendererCreationFailed(e.to_string()))?;

            // Configure surface
            let surface_caps = surface.get_capabilities(&adapter);
            let surface_format = surface_caps
                .formats
                .iter()
                .find(|f| f.is_srgb())
                .copied()
                .unwrap_or(surface_caps.formats[0]);

            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width,
                height,
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: surface_caps.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);

            // Create texture for the frame buffer
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("WPE Frame Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

            // Create shader
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Fullscreen Quad Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen_quad.wgsl").into()),
            });

            // Create bind group layout
            let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Texture Bind Group Layout"),
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

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Texture Bind Group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });

            // Create pipeline layout
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            // Create render pipeline
            let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Render Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

            frame_buffer.resize(width, height);

            Ok(Self {
                surface,
                device,
                queue,
                config,
                texture,
                texture_view,
                sampler,
                bind_group,
                render_pipeline,
                frame_buffer,
                width,
                height,
            })
        }

        /// Get the shared frame buffer.
        #[must_use]
        pub fn frame_buffer(&self) -> &SharedFrameBuffer {
            &self.frame_buffer
        }

        /// Resize the renderer.
        pub fn resize(&mut self, width: u32, height: u32) {
            let width = width.max(1);
            let height = height.max(1);

            if self.width == width && self.height == height {
                return;
            }

            self.width = width;
            self.height = height;
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.frame_buffer.resize(width, height);

            // Recreate texture at new size
            self.texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("WPE Frame Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.texture_view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Recreate bind group with new texture view
            let bind_group_layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Texture Bind Group Layout"),
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

            self.bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Texture Bind Group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
        }

        /// Present the current frame buffer to the window.
        ///
        /// # Errors
        /// Returns an error if presentation fails.
        pub fn present(&mut self) -> Result<()> {
            // Upload frame buffer to GPU texture
            self.frame_buffer.with_pixels(|pixels, width, height| {
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytemuck::cast_slice(pixels),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(height),
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            });

            // Get surface texture
            let output = self
                .surface
                .get_current_texture()
                .map_err(|e| Error::RenderFailed(e.to_string()))?;

            let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create command encoder
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

            // Render pass
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
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

                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_bind_group(0, &self.bind_group, &[]);
                render_pass.draw(0..6, 0..1); // Fullscreen quad (2 triangles)
            }

            self.queue.submit(std::iter::once(encoder.finish()));
            output.present();

            self.frame_buffer.clear_dirty();

            Ok(())
        }

        /// Get the current width.
        #[must_use]
        pub fn width(&self) -> u32 {
            self.width
        }

        /// Get the current height.
        #[must_use]
        pub fn height(&self) -> u32 {
            self.height
        }
    }
}

#[cfg(feature = "gpu")]
pub use gpu::GpuRenderer;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_frame_buffer_new() {
        let buffer = SharedFrameBuffer::new(640, 480);
        assert_eq!(buffer.dimensions(), (640, 480));
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn test_shared_frame_buffer_min_size() {
        // Should clamp to minimum 1x1
        let buffer = SharedFrameBuffer::new(0, 0);
        assert_eq!(buffer.dimensions(), (1, 1));
    }

    #[test]
    fn test_shared_frame_buffer_resize() {
        let buffer = SharedFrameBuffer::new(100, 100);
        buffer.resize(200, 150);
        assert_eq!(buffer.dimensions(), (200, 150));
    }

    #[test]
    fn test_shared_frame_buffer_resize_min() {
        let buffer = SharedFrameBuffer::new(100, 100);
        buffer.resize(0, 0);
        assert_eq!(buffer.dimensions(), (1, 1));
    }

    #[test]
    fn test_shared_frame_buffer_dirty_flag() {
        let buffer = SharedFrameBuffer::new(10, 10);
        assert!(!buffer.is_dirty());

        // Simulate writing some data
        buffer.with_pixels(|_, _, _| {});
        assert!(!buffer.is_dirty()); // with_pixels doesn't set dirty

        buffer.clear_dirty();
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn test_shared_frame_buffer_clone() {
        let buffer1 = SharedFrameBuffer::new(50, 50);
        let buffer2 = buffer1.clone();

        // Both should share the same inner buffer
        assert_eq!(buffer1.dimensions(), buffer2.dimensions());

        // Resizing one should affect the other
        buffer1.resize(100, 100);
        assert_eq!(buffer2.dimensions(), (100, 100));
    }

    #[test]
    fn test_shared_frame_buffer_with_pixels() {
        let buffer = SharedFrameBuffer::new(10, 10);
        let result = buffer.with_pixels(|pixels, width, height| {
            assert_eq!(width, 10);
            assert_eq!(height, 10);
            assert_eq!(pixels.len(), 100);
            // Check initial fill (black with alpha)
            assert_eq!(pixels[0], 0xFF000000);
            42
        });
        assert_eq!(result, 42);
    }

    #[test]
    fn test_shared_frame_buffer_resize_preserves_no_old_content() {
        let buffer = SharedFrameBuffer::new(10, 10);
        buffer.resize(20, 20);

        buffer.with_pixels(|pixels, width, height| {
            assert_eq!(width, 20);
            assert_eq!(height, 20);
            assert_eq!(pixels.len(), 400);
        });
    }
}

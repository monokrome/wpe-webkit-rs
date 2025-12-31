//! Renderer for compositing WPE buffers to a window surface.
//!
//! This module handles the rendering of exported WPE buffers to a winit window
//! using softbuffer for software rendering.

#[cfg(feature = "winit")]
use std::cell::RefCell;
#[cfg(feature = "winit")]
use std::num::NonZeroU32;
#[cfg(feature = "winit")]
use std::rc::Rc;
#[cfg(feature = "winit")]
use std::sync::Arc;

#[cfg(feature = "winit")]
use winit::window::Window;

use crate::{Error, Result};

/// A software renderer that composites WPE buffers to a window.
#[cfg(feature = "winit")]
pub struct SoftwareRenderer {
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    width: u32,
    height: u32,
    /// Buffer to hold the current frame
    buffer: Vec<u32>,
}

#[cfg(feature = "winit")]
impl SoftwareRenderer {
    /// Create a new software renderer for the given window.
    ///
    /// # Errors
    /// Returns an error if the renderer could not be created.
    pub fn new(window: Arc<Window>) -> Result<Self> {
        let context = softbuffer::Context::new(window.clone())
            .map_err(|e| Error::RendererCreationFailed(e.to_string()))?;

        let surface = softbuffer::Surface::new(&context, window)
            .map_err(|e| Error::RendererCreationFailed(e.to_string()))?;

        let size = surface.window().inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        Ok(Self {
            surface,
            width,
            height,
            buffer: vec![0xFF000000; (width * height) as usize], // Black with alpha
        })
    }

    /// Resize the renderer to match the window size.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);

        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.buffer.resize((width * height) as usize, 0xFF000000);

            if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
                let _ = self.surface.resize(w, h);
            }
        }
    }

    /// Copy pixel data from an SHM buffer into the renderer.
    ///
    /// The data should be in ARGB32 format (alpha in high byte).
    #[allow(unsafe_code)]
    pub fn copy_shm_buffer(&mut self, data: *const u8, width: u32, height: u32, stride: u32) {
        if data.is_null() {
            return;
        }

        let dest_width = self.width.min(width);
        let dest_height = self.height.min(height);

        for y in 0..dest_height {
            for x in 0..dest_width {
                let src_offset = (y * stride + x * 4) as usize;
                let dest_offset = (y * self.width + x) as usize;

                // SAFETY: We've checked the pointer is not null and bounds are valid
                unsafe {
                    let pixel_ptr = data.add(src_offset) as *const u32;
                    if dest_offset < self.buffer.len() {
                        self.buffer[dest_offset] = *pixel_ptr;
                    }
                }
            }
        }
    }

    /// Fill the buffer with a solid color (for testing).
    pub fn fill(&mut self, color: u32) {
        self.buffer.fill(color);
    }

    /// Present the current buffer to the window.
    ///
    /// # Errors
    /// Returns an error if presentation fails.
    pub fn present(&mut self) -> Result<()> {
        let (Some(width), Some(height)) =
            (NonZeroU32::new(self.width), NonZeroU32::new(self.height))
        else {
            return Ok(());
        };

        // Ensure surface is properly sized
        self.surface
            .resize(width, height)
            .map_err(|e| Error::RenderFailed(e.to_string()))?;

        // Get a buffer from the surface
        let mut surface_buffer = self
            .surface
            .buffer_mut()
            .map_err(|e| Error::RenderFailed(e.to_string()))?;

        // Copy our buffer to the surface
        let len = surface_buffer.len().min(self.buffer.len());
        surface_buffer[..len].copy_from_slice(&self.buffer[..len]);

        // Present
        surface_buffer
            .present()
            .map_err(|e| Error::RenderFailed(e.to_string()))?;

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

/// Shared state for buffer export callbacks.
#[cfg(feature = "winit")]
pub struct RenderContext {
    pub renderer: Rc<RefCell<SoftwareRenderer>>,
    pub pending_frame: bool,
}

#[cfg(feature = "winit")]
impl RenderContext {
    /// Create a new render context.
    pub fn new(renderer: SoftwareRenderer) -> Self {
        Self {
            renderer: Rc::new(RefCell::new(renderer)),
            pending_frame: false,
        }
    }
}

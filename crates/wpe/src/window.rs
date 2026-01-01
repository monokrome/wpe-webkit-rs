//! Winit integration for WPE WebView.
//!
//! This module provides integration with the winit windowing library,
//! allowing you to embed WPE WebViews in winit windows.

#[cfg(feature = "winit")]
use std::sync::Arc;

#[cfg(feature = "winit")]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
#[cfg(feature = "winit")]
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::PhysicalKey,
    window::{Window, WindowAttributes, WindowId},
};

use crate::{Error, IpcBridge, Result, SoftwareRenderer, WebView, WebViewSettings};
use crate::renderer::SharedFrameBuffer;

/// A custom event type for the winit event loop.
#[derive(Debug, Clone)]
pub enum WpeEvent {
    /// WPE needs to process events
    Wake,
    /// Request a redraw
    Redraw,
}

/// A window containing a WPE WebView.
#[cfg(feature = "winit")]
pub struct WpeWindow {
    window: Option<Arc<Window>>,
    webview: Option<WebView>,
    renderer: Option<SoftwareRenderer>,
    frame_buffer: SharedFrameBuffer,
    ipc: IpcBridge,
    settings: WebViewSettings,
    ready: bool,
    /// Current cursor position
    cursor_pos: (f64, f64),
    /// Current modifier state
    modifiers: u32,
}

#[cfg(feature = "winit")]
impl WpeWindow {
    /// Create a new WPE window with the given settings.
    #[must_use]
    pub fn new(settings: WebViewSettings) -> Self {
        Self {
            window: None,
            webview: None,
            renderer: None,
            frame_buffer: SharedFrameBuffer::new(1280, 720),
            ipc: IpcBridge::new(),
            settings,
            ready: false,
            cursor_pos: (0.0, 0.0),
            modifiers: 0,
        }
    }

    /// Get a reference to the window.
    #[must_use]
    pub fn window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref()
    }

    /// Get a mutable reference to the renderer.
    #[must_use]
    pub fn renderer_mut(&mut self) -> Option<&mut SoftwareRenderer> {
        self.renderer.as_mut()
    }

    /// Get a reference to the webview.
    #[must_use]
    pub fn webview(&self) -> Option<&WebView> {
        self.webview.as_ref()
    }

    /// Get a mutable reference to the webview.
    #[must_use]
    pub fn webview_mut(&mut self) -> Option<&mut WebView> {
        self.webview.as_mut()
    }

    /// Get a reference to the IPC bridge.
    #[must_use]
    pub fn ipc(&self) -> &IpcBridge {
        &self.ipc
    }

    /// Get a mutable reference to the IPC bridge.
    #[must_use]
    pub fn ipc_mut(&mut self) -> &mut IpcBridge {
        &mut self.ipc
    }

    /// Check if the webview is ready.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Initialize the window and webview.
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        tracing::debug!("WpeWindow::initialize starting");

        let attrs = WindowAttributes::default()
            .with_title("WPE WebView")
            .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|_| Error::WebViewCreationFailed)?,
        );
        tracing::debug!("Window created");

        // Get the raw window handle for WPE
        let _display_handle = window.display_handle().map_err(|_| Error::WindowHandle)?;
        let _window_handle = window.window_handle().map_err(|_| Error::WindowHandle)?;
        tracing::debug!("Got window handles");

        // Create the software renderer with shared frame buffer
        let renderer = SoftwareRenderer::new(window.clone(), self.frame_buffer.clone())?;
        tracing::debug!("Renderer created");

        // Create the webview with the same shared frame buffer
        let mut webview = WebView::new(self.settings.clone(), self.frame_buffer.clone())?;
        tracing::debug!("WebView created");

        // Load initial content
        if let Some(ref url) = self.settings.url {
            tracing::debug!("Loading URL: {}", url);
            webview.load_url(url)?;
        } else if let Some(ref html) = self.settings.html {
            tracing::debug!("Loading HTML content");
            let html_with_bridge = IpcBridge::inject_bridge(html);
            webview.load_html(&html_with_bridge, None)?;
        }
        tracing::debug!("Content loaded");

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.webview = Some(webview);
        self.ready = true;

        tracing::debug!("WpeWindow::initialize complete");
        Ok(())
    }

    /// Handle a window event.
    fn handle_event(&mut self, event: WindowEvent) -> bool {
        match event {
            WindowEvent::CloseRequested => {
                return true; // Signal exit
            }
            WindowEvent::Resized(size) => {
                // Resize shared buffer first (both webview and renderer will see it)
                self.frame_buffer.resize(size.width, size.height);

                if let Some(ref mut webview) = self.webview {
                    webview.resize(size.width, size.height);
                }
                if let Some(ref mut renderer) = self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                tracing::trace!("RedrawRequested");

                // Process WPE events and request next frame
                if let Some(ref mut webview) = self.webview {
                    tracing::trace!("spin");
                    webview.spin();
                    tracing::trace!("render");
                    webview.render();
                    tracing::trace!("render done");
                }

                // Present directly - no copying needed, buffer is shared
                if let Some(ref mut renderer) = self.renderer {
                    tracing::trace!("present");
                    if let Err(e) = renderer.present() {
                        tracing::error!("Failed to present frame: {}", e);
                    }
                    tracing::trace!("present done");
                }

                // Request next frame
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Focused(focused) => {
                if let Some(ref mut webview) = self.webview {
                    if focused {
                        webview.focus();
                    } else {
                        webview.unfocus();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x, position.y);
                if let Some(ref mut webview) = self.webview {
                    webview.mouse_move(position.x, position.y, self.modifiers);
                }
            }
            WindowEvent::CursorEntered { .. } => {
                if let Some(ref mut webview) = self.webview {
                    webview.mouse_enter(self.cursor_pos.0, self.cursor_pos.1);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(ref mut webview) = self.webview {
                    webview.mouse_leave();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let wpe_button = match button {
                    MouseButton::Left => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Right => 3,
                    MouseButton::Back => 8,
                    MouseButton::Forward => 9,
                    MouseButton::Other(n) => n as u32,
                };
                let pressed = state == ElementState::Pressed;
                if let Some(ref mut webview) = self.webview {
                    webview.mouse_button(
                        wpe_button,
                        pressed,
                        self.cursor_pos.0,
                        self.cursor_pos.1,
                        self.modifiers,
                        1, // click count - TODO: track double-clicks
                    );
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy, precise) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        // Line-based scrolling (mouse wheel) - scale to pixels
                        (x as f64 * 40.0, y as f64 * 40.0, false)
                    }
                    MouseScrollDelta::PixelDelta(pos) => {
                        (pos.x, pos.y, true)
                    }
                };
                if let Some(ref mut webview) = self.webview {
                    webview.scroll(
                        self.cursor_pos.0,
                        self.cursor_pos.1,
                        dx,
                        -dy, // Invert Y for natural scrolling
                        self.modifiers,
                        precise,
                    );
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(keycode) = event.physical_key {
                    // Convert winit keycode to Linux keycode
                    // This is a simplified mapping - full mapping would be more complex
                    let linux_keycode = keycode as u32;
                    let pressed = event.state == ElementState::Pressed;

                    // Get keyval from text if available, otherwise use keycode
                    let keyval = event.text
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(|c| c as u32)
                        .unwrap_or(linux_keycode);

                    if let Some(ref mut webview) = self.webview {
                        webview.keyboard(linux_keycode, keyval, pressed, self.modifiers);
                    }
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                let state = mods.state();
                self.modifiers = 0;
                if state.control_key() {
                    self.modifiers |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_CONTROL;
                }
                if state.shift_key() {
                    self.modifiers |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_SHIFT;
                }
                if state.alt_key() {
                    self.modifiers |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_ALT;
                }
                if state.super_key() {
                    self.modifiers |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_META;
                }
            }
            _ => {}
        }
        false
    }
}

/// Builder for creating a WPE application with winit.
#[cfg(feature = "winit")]
pub struct WpeApp<F>
where
    F: FnMut(&mut WpeWindow, &crate::ipc::FrontendMessage) -> Option<serde_json::Value>,
{
    settings: WebViewSettings,
    message_handler: F,
}

#[cfg(feature = "winit")]
impl<F> WpeApp<F>
where
    F: FnMut(&mut WpeWindow, &crate::ipc::FrontendMessage) -> Option<serde_json::Value>,
{
    /// Create a new WPE application.
    pub fn new(settings: WebViewSettings, message_handler: F) -> Self {
        Self {
            settings,
            message_handler,
        }
    }

    /// Run the application.
    ///
    /// # Errors
    /// Returns an error if the event loop fails.
    pub fn run(self) -> Result<()> {
        let event_loop = EventLoop::<WpeEvent>::with_user_event()
            .build()
            .map_err(|_| Error::InitFailed)?;

        let mut app = WpeAppHandler {
            wpe_window: WpeWindow::new(self.settings),
            message_handler: self.message_handler,
        };

        event_loop.run_app(&mut app).map_err(|_| Error::InitFailed)?;

        Ok(())
    }
}

#[cfg(feature = "winit")]
struct WpeAppHandler<F>
where
    F: FnMut(&mut WpeWindow, &crate::ipc::FrontendMessage) -> Option<serde_json::Value>,
{
    wpe_window: WpeWindow,
    message_handler: F,
}

#[cfg(feature = "winit")]
impl<F> ApplicationHandler<WpeEvent> for WpeAppHandler<F>
where
    F: FnMut(&mut WpeWindow, &crate::ipc::FrontendMessage) -> Option<serde_json::Value>,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.wpe_window.window.is_none() {
            if let Err(e) = self.wpe_window.initialize(event_loop) {
                tracing::error!("Failed to initialize window: {}", e);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: WpeEvent) {
        match event {
            WpeEvent::Wake => {
                if let Some(ref mut webview) = self.wpe_window.webview {
                    webview.spin();
                }
            }
            WpeEvent::Redraw => {
                if let Some(ref window) = self.wpe_window.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // Process IPC messages before handling events
        // Borrow ipc and webview separately to satisfy borrow checker
        let messages: Vec<_> = if let Some(ref webview) = self.wpe_window.webview {
            self.wpe_window.ipc.poll(webview)
        } else {
            vec![]
        };

        for msg in messages {
            if let Some(result) = (self.message_handler)(&mut self.wpe_window, &msg) {
                if let Some(request_id) = &msg.request_id {
                    let response =
                        crate::ipc::BackendMessage::response(request_id.clone(), result);
                    if let Some(ref webview) = self.wpe_window.webview {
                        let _ = self.wpe_window.ipc.send(webview, &response);
                    }
                }
            }
        }

        if self.wpe_window.handle_event(event) {
            event_loop.exit();
        }
    }
}

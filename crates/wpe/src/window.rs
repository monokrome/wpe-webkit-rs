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
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{Error, IpcBridge, Result, SoftwareRenderer, WebView, WebViewSettings};

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
    ipc: IpcBridge,
    settings: WebViewSettings,
    ready: bool,
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
            ipc: IpcBridge::new(),
            settings,
            ready: false,
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
        let attrs = WindowAttributes::default()
            .with_title("WPE WebView")
            .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|_| Error::WebViewCreationFailed)?,
        );

        // Get the raw window handle for WPE
        let _display_handle = window.display_handle().map_err(|_| Error::WindowHandle)?;
        let _window_handle = window.window_handle().map_err(|_| Error::WindowHandle)?;

        // Create the software renderer
        let renderer = SoftwareRenderer::new(window.clone())?;

        // Create the webview
        let mut webview = WebView::new(self.settings.clone())?;

        // Load initial content
        if let Some(ref url) = self.settings.url {
            webview.load_url(url)?;
        } else if let Some(ref html) = self.settings.html {
            let html_with_bridge = IpcBridge::inject_bridge(html);
            webview.load_html(&html_with_bridge, None)?;
        }

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.webview = Some(webview);
        self.ready = true;

        Ok(())
    }

    /// Handle a window event.
    fn handle_event(&mut self, event: WindowEvent) -> bool {
        match event {
            WindowEvent::CloseRequested => {
                return true; // Signal exit
            }
            WindowEvent::Resized(size) => {
                if let Some(ref mut webview) = self.webview {
                    webview.resize(size.width, size.height);
                }
                if let Some(ref mut renderer) = self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                // Process WPE events
                if let Some(ref mut webview) = self.webview {
                    webview.spin();
                    webview.render();
                }

                // Present the frame
                if let Some(ref mut renderer) = self.renderer {
                    // For now, fill with a test color to verify rendering works
                    // Later this will be replaced with actual WPE buffer content
                    renderer.fill(0xFF2D2D2D); // Dark gray background

                    if let Err(e) = renderer.present() {
                        tracing::error!("Failed to present frame: {}", e);
                    }
                }

                // Request next frame
                if let Some(ref window) = self.window {
                    window.request_redraw();
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

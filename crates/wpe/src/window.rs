//! Winit integration for WPE WebView.
//!
//! This module provides integration with the winit windowing library,
//! allowing you to embed WPE WebViews in winit windows.

#[cfg(feature = "winit")]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
#[cfg(feature = "winit")]
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

use crate::{Error, IpcBridge, Result, WebView, WebViewSettings};

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
    window: Option<Window>,
    webview: Option<WebView>,
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
            ipc: IpcBridge::new(),
            settings,
            ready: false,
        }
    }

    /// Get a reference to the window.
    #[must_use]
    pub fn window(&self) -> Option<&Window> {
        self.window.as_ref()
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

        let window = event_loop
            .create_window(attrs)
            .map_err(|_| Error::WebViewCreationFailed)?;

        // Get the raw window handle for WPE
        let _display_handle = window.display_handle().map_err(|_| Error::WindowHandle)?;
        let _window_handle = window.window_handle().map_err(|_| Error::WindowHandle)?;

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
            }
            WindowEvent::RedrawRequested => {
                if let Some(ref mut webview) = self.webview {
                    webview.spin();
                    webview.render();
                }
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
            should_exit: false,
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
    should_exit: bool,
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
        if let Some(ref webview) = self.wpe_window.webview {
            let messages = self.wpe_window.ipc.poll(webview);
            for msg in messages {
                if let Some(result) = (self.message_handler)(&mut self.wpe_window, &msg) {
                    if let Some(request_id) = &msg.request_id {
                        let response =
                            crate::ipc::BackendMessage::response(request_id.clone(), result);
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

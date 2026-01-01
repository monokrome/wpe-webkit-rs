//! Native WPE window mode.
//!
//! This module provides a native WPE window that uses WPE's built-in
//! Wayland window management. This is simpler than embedding in winit
//! and avoids offscreen rendering complexity.
//!
//! When the `x11` feature is enabled, the window will automatically fall back
//! to X11 if Wayland is not available.

use std::collections::VecDeque;
use std::ffi::CString;
use std::ptr;
use std::sync::{Arc, Mutex};

use crate::ipc::{BackendMessage, FrontendMessage, IpcBridge};
use crate::{Error, Result, WebViewSettings};

/// Navigation load state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadState {
    /// A new load request has been made.
    Started,
    /// A provisional data source received a server redirect.
    Redirected,
    /// The content started arriving for a page load.
    Committed,
    /// Load completed (or failed).
    Finished,
}

impl LoadState {
    /// Convert from WebKit's load event enum.
    #[allow(unsafe_code)]
    fn from_webkit(event: wpe_sys::WebKitLoadEvent) -> Self {
        match event {
            wpe_sys::WebKitLoadEvent_WEBKIT_LOAD_STARTED => Self::Started,
            wpe_sys::WebKitLoadEvent_WEBKIT_LOAD_REDIRECTED => Self::Redirected,
            wpe_sys::WebKitLoadEvent_WEBKIT_LOAD_COMMITTED => Self::Committed,
            wpe_sys::WebKitLoadEvent_WEBKIT_LOAD_FINISHED => Self::Finished,
            _ => Self::Started,
        }
    }
}

/// Events emitted by the native window.
#[derive(Debug, Clone)]
pub enum NavigationEvent {
    /// Load state changed.
    LoadChanged(LoadState),
    /// Page title changed.
    TitleChanged(String),
    /// URL changed.
    UrlChanged(String),
    /// Load progress updated (0.0 to 1.0).
    ProgressChanged(f64),
}

/// Shared message queue for receiving messages from JavaScript.
type MessageQueue = Arc<Mutex<VecDeque<FrontendMessage>>>;

/// Shared event queue for navigation events.
type EventQueue = Arc<Mutex<VecDeque<NavigationEvent>>>;

/// The inner type that Arc::into_raw returns a pointer to.
type MessageQueueInner = Mutex<VecDeque<FrontendMessage>>;
type EventQueueInner = Mutex<VecDeque<NavigationEvent>>;

/// A native WPE window that uses WPE's built-in window management.
pub struct NativeWindow {
    /// The WPE display
    display: *mut wpe_sys::WPEDisplay,
    /// The WPE view
    view: *mut wpe_sys::WPEView,
    /// The WebKit web view
    web_view: *mut wpe_sys::WebKitWebView,
    /// User content manager for script message handling
    #[allow(dead_code)]
    user_content_manager: *mut wpe_sys::WebKitUserContentManager,
    /// Settings used to create this view
    #[allow(dead_code)]
    settings: WebViewSettings,
    /// Whether the window should close
    should_close: bool,
    /// IPC bridge for JavaScript communication
    ipc: IpcBridge,
    /// Message queue for incoming messages from JavaScript
    message_queue: MessageQueue,
    /// Event queue for navigation events
    event_queue: EventQueue,
    /// Raw pointer to message queue (for signal handler cleanup)
    message_queue_ptr: *const MessageQueueInner,
    /// Raw pointer to event queue (for signal handler cleanup)
    event_queue_ptr: *const EventQueueInner,
}

/// Signal handler for script-message-received.
/// This is called when JavaScript sends a message via webkit.messageHandlers.wpe.postMessage().
#[allow(unsafe_code)]
unsafe extern "C" fn on_script_message(
    _manager: *mut wpe_sys::WebKitUserContentManager,
    js_result: *mut wpe_sys::JSCValue,
    user_data: *mut std::ffi::c_void,
) {
    if user_data.is_null() || js_result.is_null() {
        tracing::warn!("on_script_message: null pointer");
        return;
    }

    // Get the message queue from user_data
    // Note: Arc::into_raw returns pointer to inner value (Mutex), not the Arc
    let queue = &*(user_data as *const MessageQueueInner);

    // Convert JSCValue to string
    let c_str = wpe_sys::jsc_value_to_string(js_result);
    if c_str.is_null() {
        tracing::warn!("Failed to convert JSCValue to string");
        return;
    }

    let rust_str = std::ffi::CStr::from_ptr(c_str).to_string_lossy();
    tracing::debug!("Received message from JS: {}", rust_str);

    // Parse the JSON message
    match serde_json::from_str::<FrontendMessage>(&rust_str) {
        Ok(msg) => {
            if let Ok(mut q) = queue.lock() {
                q.push_back(msg);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to parse message from JS: {}", e);
        }
    }

    // Free the C string
    wpe_sys::g_free(c_str as *mut _);
}

/// Signal handler for load-changed.
#[allow(unsafe_code)]
unsafe extern "C" fn on_load_changed(
    _web_view: *mut wpe_sys::WebKitWebView,
    load_event: wpe_sys::WebKitLoadEvent,
    user_data: *mut std::ffi::c_void,
) {
    if user_data.is_null() {
        return;
    }

    // Note: Arc::into_raw returns pointer to inner value (Mutex), not the Arc
    let queue = &*(user_data as *const EventQueueInner);
    let state = LoadState::from_webkit(load_event);
    tracing::debug!("Load changed: {:?}", state);

    if let Ok(mut q) = queue.lock() {
        q.push_back(NavigationEvent::LoadChanged(state));
    }
}

/// Signal handler for notify::title.
#[allow(unsafe_code)]
unsafe extern "C" fn on_notify_title(
    web_view: *mut wpe_sys::WebKitWebView,
    _pspec: *mut std::ffi::c_void,
    user_data: *mut std::ffi::c_void,
) {
    if user_data.is_null() || web_view.is_null() {
        return;
    }

    let queue = &*(user_data as *const EventQueueInner);
    let title_ptr = wpe_sys::webkit_web_view_get_title(web_view);

    if !title_ptr.is_null() {
        let title = std::ffi::CStr::from_ptr(title_ptr).to_string_lossy().into_owned();
        tracing::debug!("Title changed: {}", title);

        if let Ok(mut q) = queue.lock() {
            q.push_back(NavigationEvent::TitleChanged(title));
        }
    }
}

/// Signal handler for notify::uri.
#[allow(unsafe_code)]
unsafe extern "C" fn on_notify_uri(
    web_view: *mut wpe_sys::WebKitWebView,
    _pspec: *mut std::ffi::c_void,
    user_data: *mut std::ffi::c_void,
) {
    if user_data.is_null() || web_view.is_null() {
        return;
    }

    let queue = &*(user_data as *const EventQueueInner);
    let uri_ptr = wpe_sys::webkit_web_view_get_uri(web_view);

    if !uri_ptr.is_null() {
        let uri = std::ffi::CStr::from_ptr(uri_ptr).to_string_lossy().into_owned();
        tracing::debug!("URI changed: {}", uri);

        if let Ok(mut q) = queue.lock() {
            q.push_back(NavigationEvent::UrlChanged(uri));
        }
    }
}

/// Signal handler for notify::estimated-load-progress.
#[allow(unsafe_code)]
unsafe extern "C" fn on_notify_progress(
    web_view: *mut wpe_sys::WebKitWebView,
    _pspec: *mut std::ffi::c_void,
    user_data: *mut std::ffi::c_void,
) {
    if user_data.is_null() || web_view.is_null() {
        return;
    }

    let queue = &*(user_data as *const EventQueueInner);
    let progress = wpe_sys::webkit_web_view_get_estimated_load_progress(web_view);

    if let Ok(mut q) = queue.lock() {
        q.push_back(NavigationEvent::ProgressChanged(progress));
    }
}

impl NativeWindow {
    /// Create a new native WPE window with the given settings.
    ///
    /// This will attempt to use Wayland first. If Wayland is not available
    /// and the `x11` feature is enabled, it will fall back to X11.
    #[allow(unsafe_code)]
    pub fn new(settings: WebViewSettings) -> Result<Self> {
        // Try Wayland first
        match Self::new_wayland(settings.clone()) {
            Ok(window) => return Ok(window),
            Err(e) => {
                tracing::warn!("Wayland display not available: {}", e);
            }
        }

        // X11 fallback is handled by the X11Window type when the feature is enabled
        #[cfg(feature = "x11")]
        {
            tracing::info!("Attempting X11 fallback - use X11Window directly for X11 support");
        }

        Err(Error::NoDisplay)
    }

    /// Create a new native WPE window using Wayland.
    #[allow(unsafe_code)]
    fn new_wayland(settings: WebViewSettings) -> Result<Self> {
        // Create the message queue for IPC
        let message_queue: MessageQueue = Arc::new(Mutex::new(VecDeque::new()));
        // Create the event queue for navigation events
        let event_queue: EventQueue = Arc::new(Mutex::new(VecDeque::new()));

        // SAFETY: All WPE/GLib API calls require valid pointers which we check.
        // Arc::into_raw creates stable pointers for signal handlers.
        // Signal connections use stable function pointers and user_data.
        unsafe {
            // Get the default display (Wayland in this case)
            // Note: wpe_display_get_default() may auto-connect
            let display = wpe_sys::wpe_display_get_default();
            if display.is_null() {
                tracing::debug!("No default WPE display available (Wayland not running?)");
                return Err(Error::InitFailed);
            }

            tracing::debug!("Got WPE display");

            // Try to connect - ignore "already connected" error
            let mut error: *mut wpe_sys::GError = ptr::null_mut();
            let connected = wpe_sys::wpe_display_connect(display, &mut error);

            if connected == 0 {
                if !error.is_null() {
                    let msg = std::ffi::CStr::from_ptr((*error).message);
                    let msg_str = msg.to_string_lossy();
                    if msg_str.contains("already connected") {
                        tracing::debug!("Display already connected (this is fine)");
                        wpe_sys::g_error_free(error);
                    } else {
                        tracing::error!("Failed to connect display: {:?}", msg);
                        wpe_sys::g_error_free(error);
                        return Err(Error::InitFailed);
                    }
                } else {
                    tracing::error!("Failed to connect display");
                    return Err(Error::InitFailed);
                }
            } else {
                tracing::debug!("Connected to WPE display");
            }

            // Create a WebKitWebView (it creates its own WPEView)
            let web_view = wpe_sys::webkit_web_view_new(ptr::null_mut());
            if web_view.is_null() {
                tracing::error!("Failed to create WebKitWebView");
                return Err(Error::WebViewCreationFailed);
            }

            // Get the user content manager for IPC
            let user_content_manager = wpe_sys::webkit_web_view_get_user_content_manager(web_view);
            if user_content_manager.is_null() {
                wpe_sys::g_object_unref(web_view as *mut _);
                tracing::error!("Failed to get user content manager");
                return Err(Error::WebViewCreationFailed);
            }

            // Leak the Arc to keep it alive - we'll clean it up in Drop
            // Note: clone() creates a new Arc pointing to the same data
            let queue_ptr = Arc::into_raw(message_queue.clone());

            // Connect to the script-message-received signal BEFORE registering the handler
            let signal_name = CString::new("script-message-received::wpe")
                .expect("static string has no NUL bytes");
            let signal_id = wpe_sys::g_signal_connect_data(
                user_content_manager as *mut _,
                signal_name.as_ptr(),
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut wpe_sys::WebKitUserContentManager,
                        *mut wpe_sys::JSCValue,
                        *mut std::ffi::c_void,
                    ),
                    unsafe extern "C" fn(),
                >(on_script_message)),
                queue_ptr as *mut _,
                None,
                0, // G_CONNECT_DEFAULT
            );

            if signal_id == 0 {
                tracing::warn!("Failed to connect script-message-received signal");
            } else {
                tracing::debug!("Connected script-message-received signal: {}", signal_id);
            }

            // Register the script message handler
            let handler_name = CString::new("wpe")
                .expect("static string has no NUL bytes");
            let registered = wpe_sys::webkit_user_content_manager_register_script_message_handler(
                user_content_manager,
                handler_name.as_ptr(),
                ptr::null(), // default world
            );

            if registered == 0 {
                tracing::warn!("Failed to register script message handler (may already exist)");
            } else {
                tracing::debug!("Registered 'wpe' script message handler");
            }

            // Get the WPE view from the WebKitWebView
            let view = wpe_sys::webkit_web_view_get_wpe_view(web_view);
            if view.is_null() {
                wpe_sys::g_object_unref(web_view as *mut _);
                tracing::error!("Failed to get WPE view from WebKitWebView");
                return Err(Error::WebViewCreationFailed);
            }

            // Set initial size
            wpe_sys::wpe_view_resized(view, 1280, 720);

            // Make the view visible
            wpe_sys::wpe_view_set_visible(view, 1);

            // Focus the view
            wpe_sys::wpe_view_focus_in(view);

            // Set window title via toplevel
            let toplevel = wpe_sys::wpe_view_get_toplevel(view);
            if !toplevel.is_null() {
                let title = CString::new("WPE WebView")
                    .expect("static string has no NUL bytes");
                wpe_sys::wpe_toplevel_set_title(toplevel, title.as_ptr());
            }

            // Connect navigation event signals
            let event_queue_ptr = Arc::into_raw(event_queue.clone());

            // load-changed signal
            let signal_name = CString::new("load-changed")
                .expect("static string has no NUL bytes");
            let signal_id = wpe_sys::g_signal_connect_data(
                web_view as *mut _,
                signal_name.as_ptr(),
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut wpe_sys::WebKitWebView,
                        wpe_sys::WebKitLoadEvent,
                        *mut std::ffi::c_void,
                    ),
                    unsafe extern "C" fn(),
                >(on_load_changed)),
                event_queue_ptr as *mut _,
                None,
                0,
            );
            if signal_id > 0 {
                tracing::debug!("Connected load-changed signal: {}", signal_id);
            }

            // notify::title signal
            let signal_name = CString::new("notify::title")
                .expect("static string has no NUL bytes");
            let signal_id = wpe_sys::g_signal_connect_data(
                web_view as *mut _,
                signal_name.as_ptr(),
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut wpe_sys::WebKitWebView,
                        *mut std::ffi::c_void,
                        *mut std::ffi::c_void,
                    ),
                    unsafe extern "C" fn(),
                >(on_notify_title)),
                event_queue_ptr as *mut _,
                None,
                0,
            );
            if signal_id > 0 {
                tracing::debug!("Connected notify::title signal: {}", signal_id);
            }

            // notify::uri signal
            let signal_name = CString::new("notify::uri")
                .expect("static string has no NUL bytes");
            let signal_id = wpe_sys::g_signal_connect_data(
                web_view as *mut _,
                signal_name.as_ptr(),
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut wpe_sys::WebKitWebView,
                        *mut std::ffi::c_void,
                        *mut std::ffi::c_void,
                    ),
                    unsafe extern "C" fn(),
                >(on_notify_uri)),
                event_queue_ptr as *mut _,
                None,
                0,
            );
            if signal_id > 0 {
                tracing::debug!("Connected notify::uri signal: {}", signal_id);
            }

            // notify::estimated-load-progress signal
            let signal_name = CString::new("notify::estimated-load-progress")
                .expect("static string has no NUL bytes");
            let signal_id = wpe_sys::g_signal_connect_data(
                web_view as *mut _,
                signal_name.as_ptr(),
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut wpe_sys::WebKitWebView,
                        *mut std::ffi::c_void,
                        *mut std::ffi::c_void,
                    ),
                    unsafe extern "C" fn(),
                >(on_notify_progress)),
                event_queue_ptr as *mut _,
                None,
                0,
            );
            if signal_id > 0 {
                tracing::debug!("Connected notify::estimated-load-progress signal: {}", signal_id);
            }

            // Note: WPEToplevel doesn't have a close-request signal like GTK
            // Close handling would need to be done via the compositor protocol
            // or by checking if the toplevel is still valid

            tracing::info!("Created native WPE window with IPC and navigation events");

            Ok(Self {
                display,
                view,
                web_view,
                user_content_manager,
                settings,
                should_close: false,
                ipc: IpcBridge::new(),
                message_queue,
                event_queue,
                message_queue_ptr: queue_ptr,
                event_queue_ptr,
            })
        }
    }

    /// Load a URL in the web view.
    #[allow(unsafe_code)]
    pub fn load_url(&mut self, url: &str) -> Result<()> {
        if url.is_empty() {
            return Err(Error::InvalidUrl("URL cannot be empty".to_string()));
        }

        let c_url = CString::new(url).map_err(|_| Error::InvalidUrl(url.to_string()))?;

        // SAFETY: self.web_view is valid (checked in new()), c_url is a valid C string.
        unsafe {
            wpe_sys::webkit_web_view_load_uri(self.web_view, c_url.as_ptr());
        }

        tracing::debug!("Loading URL: {}", url);
        Ok(())
    }

    /// Load HTML content directly.
    #[allow(unsafe_code)]
    pub fn load_html(&mut self, html: &str, base_url: Option<&str>) -> Result<()> {
        let c_html = CString::new(html).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let c_base = base_url.and_then(|u| CString::new(u).ok());

        // SAFETY: self.web_view is valid, c_html and c_base are valid C strings (or null).
        unsafe {
            wpe_sys::webkit_web_view_load_html(
                self.web_view,
                c_html.as_ptr(),
                c_base.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            );
        }

        tracing::debug!("Loading HTML content ({} bytes)", html.len());
        Ok(())
    }

    /// Load HTML content with the IPC bridge automatically injected.
    pub fn load_html_with_ipc(&mut self, html: &str, base_url: Option<&str>) -> Result<()> {
        let html_with_bridge = IpcBridge::inject_bridge(html);
        self.load_html(&html_with_bridge, base_url)
    }

    /// Execute JavaScript in the web view.
    #[allow(unsafe_code)]
    pub fn evaluate_script(&self, script: &str) -> Result<()> {
        let c_script = CString::new(script).map_err(|e| Error::JavaScriptError(e.to_string()))?;

        // SAFETY: self.web_view is valid, c_script is a valid C string with known length.
        unsafe {
            wpe_sys::webkit_web_view_evaluate_javascript(
                self.web_view,
                c_script.as_ptr(),
                script.len() as i64,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            );
        }

        tracing::debug!("Evaluating script ({} bytes)", script.len());
        Ok(())
    }

    /// Send a message to the frontend JavaScript.
    pub fn send_message(&self, message: &BackendMessage) -> Result<()> {
        let json = serde_json::to_string(message)?;
        let script = format!("window.__wpe_receive({json})");
        self.evaluate_script(&script)
    }

    /// Send a typed message to the frontend.
    pub fn send_typed<T: serde::Serialize>(
        &self,
        message_type: &str,
        payload: &T,
    ) -> Result<()> {
        let payload_value = serde_json::to_value(payload)?;
        let message = BackendMessage::new(message_type, payload_value);
        self.send_message(&message)
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

    /// Set the window title.
    #[allow(unsafe_code)]
    pub fn set_title(&mut self, title: &str) {
        unsafe {
            let toplevel = wpe_sys::wpe_view_get_toplevel(self.view);
            if !toplevel.is_null() {
                if let Ok(c_title) = CString::new(title) {
                    wpe_sys::wpe_toplevel_set_title(toplevel, c_title.as_ptr());
                }
            }
        }
    }

    /// Resize the window.
    #[allow(unsafe_code)]
    pub fn resize(&mut self, width: u32, height: u32) {
        // SAFETY: self.view is valid.
        unsafe {
            wpe_sys::wpe_view_resized(self.view, width as i32, height as i32);
        }
        tracing::debug!("Resized to {}x{}", width, height);
    }

    /// Go back in history.
    #[allow(unsafe_code)]
    pub fn go_back(&mut self) {
        // SAFETY: self.web_view is valid.
        unsafe {
            wpe_sys::webkit_web_view_go_back(self.web_view);
        }
    }

    /// Go forward in history.
    #[allow(unsafe_code)]
    pub fn go_forward(&mut self) {
        // SAFETY: self.web_view is valid.
        unsafe {
            wpe_sys::webkit_web_view_go_forward(self.web_view);
        }
    }

    /// Reload the current page.
    #[allow(unsafe_code)]
    pub fn reload(&mut self) {
        // SAFETY: self.web_view is valid.
        unsafe {
            wpe_sys::webkit_web_view_reload(self.web_view);
        }
    }

    /// Stop loading the current page.
    #[allow(unsafe_code)]
    pub fn stop_loading(&mut self) {
        // SAFETY: self.web_view is valid.
        unsafe {
            wpe_sys::webkit_web_view_stop_loading(self.web_view);
        }
    }

    /// Check if the view can go back.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn can_go_back(&self) -> bool {
        // SAFETY: self.web_view is valid.
        unsafe { wpe_sys::webkit_web_view_can_go_back(self.web_view) != 0 }
    }

    /// Check if the view can go forward.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn can_go_forward(&self) -> bool {
        // SAFETY: self.web_view is valid.
        unsafe { wpe_sys::webkit_web_view_can_go_forward(self.web_view) != 0 }
    }

    /// Get the estimated load progress (0.0 to 1.0).
    #[must_use]
    #[allow(unsafe_code)]
    pub fn load_progress(&self) -> f64 {
        // SAFETY: self.web_view is valid.
        unsafe { wpe_sys::webkit_web_view_get_estimated_load_progress(self.web_view) }
    }

    /// Check if the window should close.
    #[must_use]
    pub fn should_close(&self) -> bool {
        self.should_close
    }

    /// Request to close the window.
    pub fn close(&mut self) {
        self.should_close = true;
    }

    /// Process pending events. Returns false if the window should close.
    #[allow(unsafe_code)]
    pub fn process_events(&mut self) -> bool {
        // SAFETY: GLib main context functions are safe to call; we use the default context.
        unsafe {
            let ctx = wpe_sys::g_main_context_default();
            // Process all pending events without blocking
            while wpe_sys::g_main_context_iteration(ctx, 0) != 0 {}
        }
        !self.should_close
    }

    /// Run the event loop until the window is closed.
    #[allow(unsafe_code)]
    pub fn run(&mut self) {
        // SAFETY: GLib main loop functions are safe to call.
        unsafe {
            let main_loop = wpe_sys::g_main_loop_new(ptr::null_mut(), 0);
            wpe_sys::g_main_loop_run(main_loop);
            wpe_sys::g_main_loop_unref(main_loop);
        }
    }

    /// Receive all pending messages from JavaScript.
    ///
    /// Returns a vector of messages that were queued since the last call.
    #[must_use]
    pub fn receive_messages(&mut self) -> Vec<FrontendMessage> {
        match self.message_queue.lock() {
            Ok(mut queue) => queue.drain(..).collect(),
            Err(e) => {
                tracing::warn!("Failed to lock message queue: {}", e);
                Vec::new()
            }
        }
    }

    /// Receive all pending navigation events.
    ///
    /// Returns a vector of events that were queued since the last call.
    #[must_use]
    pub fn receive_events(&mut self) -> Vec<NavigationEvent> {
        match self.event_queue.lock() {
            Ok(mut queue) => queue.drain(..).collect(),
            Err(e) => {
                tracing::warn!("Failed to lock event queue: {}", e);
                Vec::new()
            }
        }
    }

    /// Run a single iteration of the event loop with a message handler.
    ///
    /// This allows you to process messages from JavaScript while running
    /// your own event loop. The handler is called for each message and
    /// can return a response value for request/response patterns.
    pub fn step<F>(&mut self, handler: F) -> bool
    where
        F: Fn(&FrontendMessage) -> Option<serde_json::Value>,
    {
        // Process GLib events (this triggers the signal callbacks)
        self.process_events();

        // Process any messages from JavaScript
        let messages = self.receive_messages();
        for msg in messages {
            if let Some(result) = handler(&msg) {
                // If this was a request with an ID, send a response
                if let Some(request_id) = &msg.request_id {
                    let response = BackendMessage::response(request_id.clone(), result);
                    if let Err(e) = self.send_message(&response) {
                        tracing::warn!("Failed to send response: {}", e);
                    }
                }
            }
        }

        !self.should_close
    }

    /// Get the current URL.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn url(&self) -> Option<String> {
        // SAFETY: self.web_view is valid. The returned string is owned by WebKit
        // and valid until the URI changes, so we immediately copy it.
        unsafe {
            let uri = wpe_sys::webkit_web_view_get_uri(self.web_view);
            if uri.is_null() {
                None
            } else {
                let c_str = std::ffi::CStr::from_ptr(uri);
                Some(c_str.to_string_lossy().into_owned())
            }
        }
    }

    /// Get the current title.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn title(&self) -> Option<String> {
        // SAFETY: self.web_view is valid. The returned string is owned by WebKit
        // and valid until the title changes, so we immediately copy it.
        unsafe {
            let title = wpe_sys::webkit_web_view_get_title(self.web_view);
            if title.is_null() {
                None
            } else {
                let c_str = std::ffi::CStr::from_ptr(title);
                Some(c_str.to_string_lossy().into_owned())
            }
        }
    }

    /// Check if the view is currently loading.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn is_loading(&self) -> bool {
        // SAFETY: self.web_view is valid.
        unsafe { wpe_sys::webkit_web_view_is_loading(self.web_view) != 0 }
    }

    /// Enter fullscreen mode.
    #[allow(unsafe_code)]
    pub fn fullscreen(&mut self) -> bool {
        // SAFETY: self.view is valid. Toplevel may be null, which we check.
        unsafe {
            let toplevel = wpe_sys::wpe_view_get_toplevel(self.view);
            if !toplevel.is_null() {
                wpe_sys::wpe_toplevel_fullscreen(toplevel) != 0
            } else {
                false
            }
        }
    }

    /// Exit fullscreen mode.
    #[allow(unsafe_code)]
    pub fn unfullscreen(&mut self) -> bool {
        // SAFETY: self.view is valid. Toplevel may be null, which we check.
        unsafe {
            let toplevel = wpe_sys::wpe_view_get_toplevel(self.view);
            if !toplevel.is_null() {
                wpe_sys::wpe_toplevel_unfullscreen(toplevel) != 0
            } else {
                false
            }
        }
    }

    /// Maximize the window.
    #[allow(unsafe_code)]
    pub fn maximize(&mut self) -> bool {
        // SAFETY: self.view is valid. Toplevel may be null, which we check.
        unsafe {
            let toplevel = wpe_sys::wpe_view_get_toplevel(self.view);
            if !toplevel.is_null() {
                wpe_sys::wpe_toplevel_maximize(toplevel) != 0
            } else {
                false
            }
        }
    }

    /// Unmaximize the window.
    #[allow(unsafe_code)]
    pub fn unmaximize(&mut self) -> bool {
        // SAFETY: self.view is valid. Toplevel may be null, which we check.
        unsafe {
            let toplevel = wpe_sys::wpe_view_get_toplevel(self.view);
            if !toplevel.is_null() {
                wpe_sys::wpe_toplevel_unmaximize(toplevel) != 0
            } else {
                false
            }
        }
    }

    /// Minimize the window.
    #[allow(unsafe_code)]
    pub fn minimize(&mut self) -> bool {
        // SAFETY: self.view is valid. Toplevel may be null, which we check.
        unsafe {
            let toplevel = wpe_sys::wpe_view_get_toplevel(self.view);
            if !toplevel.is_null() {
                wpe_sys::wpe_toplevel_minimize(toplevel) != 0
            } else {
                false
            }
        }
    }

    /// Get the raw WebKitWebView pointer.
    ///
    /// # Safety
    /// The returned pointer is only valid for the lifetime of this NativeWindow.
    #[must_use]
    pub fn raw_web_view(&self) -> *mut wpe_sys::WebKitWebView {
        self.web_view
    }

    /// Get the raw WPEView pointer.
    ///
    /// # Safety
    /// The returned pointer is only valid for the lifetime of this NativeWindow.
    #[must_use]
    pub fn raw_view(&self) -> *mut wpe_sys::WPEView {
        self.view
    }

    /// Get the raw WPEDisplay pointer.
    ///
    /// # Safety
    /// The returned pointer is only valid for the lifetime of this NativeWindow.
    #[must_use]
    pub fn raw_display(&self) -> *mut wpe_sys::WPEDisplay {
        self.display
    }
}

impl Drop for NativeWindow {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        unsafe {
            // First, unref GLib objects - this disconnects any signals
            if !self.web_view.is_null() {
                wpe_sys::g_object_unref(self.web_view as *mut _);
            }
            if !self.view.is_null() {
                wpe_sys::g_object_unref(self.view as *mut _);
            }

            // Now reclaim the Arc pointers we leaked for signal handlers.
            // SAFETY: These pointers were created with Arc::into_raw() in new(),
            // and the signals that used them have been disconnected by the unref above.
            if !self.message_queue_ptr.is_null() {
                drop(Arc::from_raw(self.message_queue_ptr));
            }
            if !self.event_queue_ptr.is_null() {
                drop(Arc::from_raw(self.event_queue_ptr));
            }
        }
    }
}

// Note: NativeWindow is not Send or Sync because it holds raw pointers
// to GObject types that are not thread-safe. All operations must be
// performed on the thread where the window was created.

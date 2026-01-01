//! WPE WebKit WebView implementation using the Platform API.

use std::ffi::CString;
use std::ptr;
use std::sync::Once;

use crate::renderer::SharedFrameBuffer;
use crate::{Error, Result};

static INIT: Once = Once::new();
static mut INITIALIZED: bool = false;
static mut DISPLAY: *mut wpe_sys::WPEDisplay = ptr::null_mut();

/// Initialize the WPE display. Must be called before creating any WebViews.
#[allow(unsafe_code)]
pub fn initialize() -> Result<()> {
    INIT.call_once(|| {
        // SAFETY: WPE API calls are safe to call from a single thread during initialization.
        // The Once guard ensures this only runs once, and we check all returned pointers.
        unsafe {
            // Create a headless display for offscreen rendering
            // This avoids creating a second Wayland window when running with winit
            let display = wpe_sys::wpe_display_headless_new();

            if display.is_null() {
                tracing::error!("Failed to create headless WPE display");
                INITIALIZED = false;
                return;
            }

            tracing::debug!("Created headless WPE display");

            // Connect the headless display
            let mut error: *mut wpe_sys::GError = ptr::null_mut();
            let connected = wpe_sys::wpe_display_connect(display, &mut error);

            if connected == 0 {
                if !error.is_null() {
                    let msg = std::ffi::CStr::from_ptr((*error).message);
                    tracing::error!("Failed to connect headless display: {:?}", msg);
                    wpe_sys::g_error_free(error);
                } else {
                    tracing::error!("Failed to connect headless display");
                }
                wpe_sys::g_object_unref(display as *mut _);
                INITIALIZED = false;
                return;
            }

            // Set as primary display for WebKit to use
            wpe_sys::wpe_display_set_primary(display);

            DISPLAY = display;
            INITIALIZED = true;
            tracing::info!("WPE headless platform initialized successfully");
        }
    });

    // SAFETY: INITIALIZED is only written once inside the Once guard above,
    // so reading it here after call_once returns is safe.
    unsafe {
        if INITIALIZED {
            Ok(())
        } else {
            Err(Error::InitFailed)
        }
    }
}

/// Settings for creating a WebView.
#[derive(Debug, Clone)]
pub struct WebViewSettings {
    /// Initial URL to load
    pub url: Option<String>,
    /// Initial HTML content to load
    pub html: Option<String>,
    /// Enable developer tools
    pub developer_tools: bool,
    /// Enable JavaScript
    pub javascript_enabled: bool,
    /// User agent string
    pub user_agent: Option<String>,
}

impl Default for WebViewSettings {
    fn default() -> Self {
        Self {
            url: None,
            html: None,
            developer_tools: false,
            javascript_enabled: true,
            user_agent: None,
        }
    }
}

impl WebViewSettings {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    #[must_use]
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }

    #[must_use]
    pub fn with_developer_tools(mut self, enabled: bool) -> Self {
        self.developer_tools = enabled;
        self
    }
}

/// Callback data for render_buffer signal
struct RenderContext {
    frame_buffer: SharedFrameBuffer,
}

/// Signal handler for render-buffer
#[allow(unsafe_code)]
unsafe extern "C" fn on_render_buffer(
    view: *mut wpe_sys::WPEView,
    buffer: *mut wpe_sys::WPEBuffer,
    _damage_rects: *mut wpe_sys::WPERectangle,
    _n_damage_rects: u32,
    user_data: *mut std::ffi::c_void,
) -> i32 {
    if user_data.is_null() || buffer.is_null() {
        tracing::warn!("on_render_buffer: null pointer");
        return 0; // FALSE - render failed
    }

    let ctx = &mut *(user_data as *mut RenderContext);

    // Get buffer dimensions
    let width = wpe_sys::wpe_buffer_get_width(buffer) as u32;
    let height = wpe_sys::wpe_buffer_get_height(buffer) as u32;

    tracing::debug!("Render buffer: {}x{}", width, height);

    // Import buffer to pixels
    let mut error: *mut wpe_sys::GError = ptr::null_mut();
    let pixels = wpe_sys::wpe_buffer_import_to_pixels(buffer, &mut error);

    if pixels.is_null() {
        if !error.is_null() {
            let msg = std::ffi::CStr::from_ptr((*error).message);
            tracing::error!("Failed to import buffer to pixels: {:?}", msg);
            wpe_sys::g_error_free(error);
        } else {
            tracing::error!("Failed to import buffer to pixels");
        }
        return 0; // FALSE
    }

    // Get pixel data from GBytes
    let mut size: u64 = 0;
    let data = wpe_sys::g_bytes_get_data(pixels, &mut size as *mut u64);

    if !data.is_null() && size > 0 {
        let data_ptr = data as *const u8;
        // Assume BGRA format with 4 bytes per pixel
        let stride = width * 4;
        ctx.frame_buffer.copy_from_shm(data_ptr, width, height, stride);
        tracing::trace!("Copied {} bytes to frame buffer", size);
    }

    // Free the GBytes
    wpe_sys::g_bytes_unref(pixels);

    // Tell WPE we're done with the buffer
    wpe_sys::wpe_view_buffer_rendered(view, buffer);

    1 // TRUE - render succeeded
}

/// A WPE WebKit web view using the Platform API.
#[allow(dead_code)]
pub struct WebView {
    /// The WPE display
    display: *mut wpe_sys::WPEDisplay,
    /// The WPE view
    view: *mut wpe_sys::WPEView,
    /// The WebKit web view
    web_view: *mut wpe_sys::WebKitWebView,
    /// Render context for callbacks
    render_ctx: *mut RenderContext,
    /// Current width
    width: u32,
    /// Current height
    height: u32,
    /// Settings used to create this view
    settings: WebViewSettings,
    /// Signal handler ID for render-buffer
    render_signal_id: u64,
}

impl WebView {
    /// Create a new WebView with the given settings and shared frame buffer.
    #[allow(unsafe_code)]
    pub fn new(settings: WebViewSettings, frame_buffer: SharedFrameBuffer) -> Result<Self> {
        initialize()?;

        let width = 1280u32;
        let height = 720u32;

        frame_buffer.resize(width, height);

        // Create render context
        let render_ctx = Box::into_raw(Box::new(RenderContext { frame_buffer }));

        // SAFETY: All WPE/GLib API calls require valid pointers which we check.
        // The render_ctx pointer is valid because we just created it with Box::into_raw.
        // Signal connection uses a stable function pointer and user_data.
        unsafe {
            let display = DISPLAY;
            if display.is_null() {
                drop(Box::from_raw(render_ctx));
                return Err(Error::InitFailed);
            }

            // Create a WebKitWebView (it manages its own WPEView internally)
            // Pass NULL for backend to use the Platform API
            let web_view = wpe_sys::webkit_web_view_new(ptr::null_mut());
            if web_view.is_null() {
                drop(Box::from_raw(render_ctx));
                tracing::error!("Failed to create WebKitWebView");
                return Err(Error::WebViewCreationFailed);
            }

            // Get the WPE view from the WebKitWebView
            let view = wpe_sys::webkit_web_view_get_wpe_view(web_view);
            if view.is_null() {
                wpe_sys::g_object_unref(web_view as *mut _);
                drop(Box::from_raw(render_ctx));
                tracing::error!("Failed to get WPE view from WebKitWebView");
                return Err(Error::WebViewCreationFailed);
            }

            // Set view size
            wpe_sys::wpe_view_resized(view, width as i32, height as i32);

            // Make the view visible
            wpe_sys::wpe_view_set_visible(view, 1);

            // Connect to render-buffer signal
            let signal_name = CString::new("render-buffer")
                .expect("static string has no NUL bytes");
            let render_signal_id = wpe_sys::g_signal_connect_data(
                view as *mut _,
                signal_name.as_ptr(),
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut wpe_sys::WPEView,
                        *mut wpe_sys::WPEBuffer,
                        *mut wpe_sys::WPERectangle,
                        u32,
                        *mut std::ffi::c_void,
                    ) -> i32,
                    unsafe extern "C" fn(),
                >(on_render_buffer)),
                render_ctx as *mut _,
                None,
                0, // G_CONNECT_DEFAULT
            );

            tracing::debug!("Connected render-buffer signal: {}", render_signal_id);

            // Focus the view
            wpe_sys::wpe_view_focus_in(view);

            tracing::debug!("Created WebView with settings: {:?}", settings);

            Ok(Self {
                display,
                view,
                web_view,
                render_ctx,
                width,
                height,
                settings,
                render_signal_id,
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

    /// Check if the web view can go back.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn can_go_back(&self) -> bool {
        // SAFETY: self.web_view is valid.
        unsafe { wpe_sys::webkit_web_view_can_go_back(self.web_view) != 0 }
    }

    /// Check if the web view can go forward.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn can_go_forward(&self) -> bool {
        // SAFETY: self.web_view is valid.
        unsafe { wpe_sys::webkit_web_view_can_go_forward(self.web_view) != 0 }
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

    /// Stop loading.
    #[allow(unsafe_code)]
    pub fn stop(&mut self) {
        // SAFETY: self.web_view is valid.
        unsafe {
            wpe_sys::webkit_web_view_stop_loading(self.web_view);
        }
    }

    /// Resize the web view.
    #[allow(unsafe_code)]
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;

        // Resize the shared frame buffer
        if !self.render_ctx.is_null() {
            // SAFETY: render_ctx was created with Box::into_raw and is valid until drop.
            unsafe {
                (*self.render_ctx).frame_buffer.resize(width, height);
            }
        }

        // SAFETY: self.view is valid.
        unsafe {
            wpe_sys::wpe_view_resized(self.view, width as i32, height as i32);
        }

        tracing::debug!("Resized to {}x{}", width, height);
    }

    /// Process pending events. Call this in your event loop.
    #[allow(unsafe_code)]
    pub fn spin(&mut self) {
        // SAFETY: GLib main context functions are safe to call; we use the default context.
        unsafe {
            let ctx = wpe_sys::g_main_context_default();
            let mut count = 0;
            while wpe_sys::g_main_context_iteration(ctx, 0) != 0 {
                count += 1;
            }
            if count > 0 {
                tracing::trace!("Processed {} GLib events", count);
            }
        }
    }

    /// Render the web view. Call this when you need to redraw.
    #[allow(unsafe_code)]
    pub fn render(&mut self) {
        // In the Platform API, rendering is handled by the render-buffer signal
        // We just need to process events
        self.spin();
    }

    /// Check if the view is currently loading.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn is_loading(&self) -> bool {
        // SAFETY: self.web_view is valid.
        unsafe { wpe_sys::webkit_web_view_is_loading(self.web_view) != 0 }
    }

    /// Send a mouse button event to the view.
    ///
    /// # Arguments
    /// * `button` - The mouse button (1=left, 2=middle, 3=right)
    /// * `pressed` - True for press, false for release
    /// * `x` - X coordinate in view space
    /// * `y` - Y coordinate in view space
    /// * `modifiers` - Keyboard modifiers
    /// * `click_count` - Number of clicks (1 for single, 2 for double, etc.)
    #[allow(unsafe_code)]
    pub fn mouse_button(
        &mut self,
        button: u32,
        pressed: bool,
        x: f64,
        y: f64,
        modifiers: u32,
        click_count: u32,
    ) {
        let event_type = if pressed {
            wpe_sys::WPEEventType_WPE_EVENT_POINTER_DOWN
        } else {
            wpe_sys::WPEEventType_WPE_EVENT_POINTER_UP
        };

        // SAFETY: self.view is valid. Event is created, dispatched, and freed.
        unsafe {
            let event = wpe_sys::wpe_event_pointer_button_new(
                event_type,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
                crate::input::current_time_ms(),
                modifiers,
                button,
                x,
                y,
                click_count,
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::wpe_event_unref(event);
            }
        }
    }

    /// Send a mouse move event to the view.
    #[allow(unsafe_code)]
    pub fn mouse_move(&mut self, x: f64, y: f64, modifiers: u32) {
        // SAFETY: self.view is valid. Event is created, dispatched, and freed.
        unsafe {
            let event = wpe_sys::wpe_event_pointer_move_new(
                wpe_sys::WPEEventType_WPE_EVENT_POINTER_MOVE,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
                crate::input::current_time_ms(),
                modifiers,
                x,
                y,
                0.0, // delta_x
                0.0, // delta_y
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::wpe_event_unref(event);
            }
        }
    }

    /// Send a mouse enter event to the view.
    #[allow(unsafe_code)]
    pub fn mouse_enter(&mut self, x: f64, y: f64) {
        // SAFETY: self.view is valid. Event is created, dispatched, and freed.
        unsafe {
            let event = wpe_sys::wpe_event_pointer_move_new(
                wpe_sys::WPEEventType_WPE_EVENT_POINTER_ENTER,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
                crate::input::current_time_ms(),
                0,
                x,
                y,
                0.0,
                0.0,
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::wpe_event_unref(event);
            }
        }
    }

    /// Send a mouse leave event to the view.
    #[allow(unsafe_code)]
    pub fn mouse_leave(&mut self) {
        // SAFETY: self.view is valid. Event is created, dispatched, and freed.
        unsafe {
            let event = wpe_sys::wpe_event_pointer_move_new(
                wpe_sys::WPEEventType_WPE_EVENT_POINTER_LEAVE,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
                crate::input::current_time_ms(),
                0,
                0.0,
                0.0,
                0.0,
                0.0,
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::wpe_event_unref(event);
            }
        }
    }

    /// Send a scroll event to the view.
    ///
    /// # Arguments
    /// * `x` - X coordinate
    /// * `y` - Y coordinate
    /// * `delta_x` - Horizontal scroll delta
    /// * `delta_y` - Vertical scroll delta
    /// * `modifiers` - Keyboard modifiers
    /// * `precise` - Whether the deltas are precise (touchpad) or discrete (mouse wheel)
    #[allow(unsafe_code)]
    pub fn scroll(&mut self, x: f64, y: f64, delta_x: f64, delta_y: f64, modifiers: u32, precise: bool) {
        // SAFETY: self.view is valid. Event is created, dispatched, and freed.
        unsafe {
            let event = wpe_sys::wpe_event_scroll_new(
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
                crate::input::current_time_ms(),
                modifiers,
                delta_x,
                delta_y,
                if precise { 1 } else { 0 }, // precise_deltas
                0,                            // is_stop
                x,
                y,
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::wpe_event_unref(event);
            }
        }
    }

    /// Send a keyboard event to the view.
    ///
    /// # Arguments
    /// * `keycode` - The hardware keycode
    /// * `keyval` - The key symbol value (GDK keysym)
    /// * `pressed` - True for key press, false for key release
    /// * `modifiers` - Keyboard modifiers
    #[allow(unsafe_code)]
    pub fn keyboard(&mut self, keycode: u32, keyval: u32, pressed: bool, modifiers: u32) {
        let event_type = if pressed {
            wpe_sys::WPEEventType_WPE_EVENT_KEYBOARD_KEY_DOWN
        } else {
            wpe_sys::WPEEventType_WPE_EVENT_KEYBOARD_KEY_UP
        };

        // SAFETY: self.view is valid. Event is created, dispatched, and freed.
        unsafe {
            let event = wpe_sys::wpe_event_keyboard_new(
                event_type,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_KEYBOARD,
                crate::input::current_time_ms(),
                modifiers,
                keycode,
                keyval,
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::wpe_event_unref(event);
            }
        }
    }

    /// Give focus to the view.
    #[allow(unsafe_code)]
    pub fn focus(&mut self) {
        // SAFETY: self.view is valid.
        unsafe {
            wpe_sys::wpe_view_focus_in(self.view);
        }
    }

    /// Remove focus from the view.
    #[allow(unsafe_code)]
    pub fn unfocus(&mut self) {
        // SAFETY: self.view is valid.
        unsafe {
            wpe_sys::wpe_view_focus_out(self.view);
        }
    }
}

impl Drop for WebView {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: We own these GObjects (they were created in new()).
        // g_object_unref is safe to call on valid GObjects.
        // render_ctx was created with Box::into_raw and is reclaimed here.
        unsafe {
            // Unreference the WebKitWebView
            if !self.web_view.is_null() {
                wpe_sys::g_object_unref(self.web_view as *mut _);
            }

            // Unreference the WPE view
            if !self.view.is_null() {
                wpe_sys::g_object_unref(self.view as *mut _);
            }

            // Free the render context
            if !self.render_ctx.is_null() {
                drop(Box::from_raw(self.render_ctx));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webview_settings_default() {
        let settings = WebViewSettings::default();
        assert!(settings.url.is_none());
        assert!(settings.html.is_none());
        assert!(!settings.developer_tools);
        assert!(settings.javascript_enabled);
        assert!(settings.user_agent.is_none());
    }

    #[test]
    fn test_webview_settings_with_url() {
        let settings = WebViewSettings::new()
            .with_url("https://example.com");

        assert_eq!(settings.url, Some("https://example.com".to_string()));
        assert!(settings.html.is_none());
    }

    #[test]
    fn test_webview_settings_with_html() {
        let settings = WebViewSettings::new()
            .with_html("<h1>Hello</h1>");

        assert!(settings.url.is_none());
        assert_eq!(settings.html, Some("<h1>Hello</h1>".to_string()));
    }

    #[test]
    fn test_webview_settings_with_developer_tools() {
        let settings = WebViewSettings::new()
            .with_developer_tools(true);

        assert!(settings.developer_tools);

        let settings2 = WebViewSettings::new()
            .with_developer_tools(false);

        assert!(!settings2.developer_tools);
    }

    #[test]
    fn test_webview_settings_builder_chain() {
        let settings = WebViewSettings::new()
            .with_url("https://example.com")
            .with_developer_tools(true);

        assert_eq!(settings.url, Some("https://example.com".to_string()));
        assert!(settings.developer_tools);
        assert!(settings.javascript_enabled);
    }

    #[test]
    fn test_webview_settings_clone() {
        let settings1 = WebViewSettings::new()
            .with_url("https://test.com")
            .with_developer_tools(true);

        let settings2 = settings1.clone();

        assert_eq!(settings1.url, settings2.url);
        assert_eq!(settings1.developer_tools, settings2.developer_tools);
    }

    #[test]
    fn test_webview_settings_url_from_string() {
        let url = String::from("https://rust-lang.org");
        let settings = WebViewSettings::new().with_url(url);
        assert_eq!(settings.url, Some("https://rust-lang.org".to_string()));
    }

    #[test]
    fn test_webview_settings_html_from_string() {
        let html = String::from("<p>Content</p>");
        let settings = WebViewSettings::new().with_html(html);
        assert_eq!(settings.html, Some("<p>Content</p>".to_string()));
    }
}

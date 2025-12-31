use std::ffi::CString;
use std::ptr;
use std::sync::Once;

use crate::{Error, Result};

static INIT: Once = Once::new();
static mut INITIALIZED: bool = false;

/// Callback for when a buffer is ready for export
#[allow(unsafe_code)]
unsafe extern "C" fn export_buffer_cb(
    _data: *mut std::ffi::c_void,
    _buffer: *mut wpe_sys::wl_resource,
) {
    // Buffer export callback - we'll implement this when we add rendering
    tracing::trace!("Buffer exported (not yet rendered)");
}

/// Callback for DMABuf export
#[allow(unsafe_code)]
unsafe extern "C" fn export_dmabuf_cb(
    _data: *mut std::ffi::c_void,
    _dmabuf: *mut wpe_sys::wpe_view_backend_exportable_fdo_dmabuf_resource,
) {
    tracing::trace!("DMABuf exported (not yet rendered)");
}

/// Callback for SHM buffer export
#[allow(unsafe_code)]
unsafe extern "C" fn export_shm_cb(
    _data: *mut std::ffi::c_void,
    _buffer: *mut wpe_sys::wpe_fdo_shm_exported_buffer,
) {
    tracing::trace!("SHM buffer exported (not yet rendered)");
}

/// Initialize the WPE backend. Must be called before creating any WebViews.
///
/// # Safety
/// This function initializes global state and should only be called once.
#[allow(unsafe_code)]
pub fn initialize() -> Result<()> {
    INIT.call_once(|| {
        // SAFETY: wpe_loader_init is safe to call once at startup
        unsafe {
            // Initialize the FDO backend
            let result = wpe_sys::wpe_loader_init(
                c"libWPEBackend-fdo-1.0.so".as_ptr(),
            );

            INITIALIZED = result;

            if INITIALIZED {
                tracing::info!("WPE backend initialized successfully");
            } else {
                tracing::error!("Failed to initialize WPE backend");
            }
        }
    });

    // SAFETY: INITIALIZED is only written once inside call_once
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

/// A WPE WebKit web view.
///
/// This provides a GTK-free web view that can be embedded in any window.
#[allow(dead_code)]
pub struct WebView {
    /// The FDO exportable backend (handles buffer export)
    exportable: *mut wpe_sys::wpe_view_backend_exportable_fdo,
    /// The WebKit web view
    web_view: *mut wpe_sys::WebKitWebView,
    /// Current width
    width: u32,
    /// Current height
    height: u32,
    /// Settings used to create this view
    settings: WebViewSettings,
}

impl WebView {
    /// Create a new WebView with the given settings.
    ///
    /// # Errors
    /// Returns an error if the WebView could not be created.
    #[allow(unsafe_code)]
    pub fn new(settings: WebViewSettings) -> Result<Self> {
        initialize()?;

        let width = 1280u32;
        let height = 720u32;

        // SAFETY: Creating the FDO exportable backend
        let exportable = unsafe {
            // Set up the FDO client callbacks
            let client = wpe_sys::wpe_view_backend_exportable_fdo_client {
                export_buffer_resource: Some(export_buffer_cb),
                export_dmabuf_resource: Some(export_dmabuf_cb),
                export_shm_buffer: Some(export_shm_cb),
                _wpe_reserved0: None,
                _wpe_reserved1: None,
            };

            let exportable = wpe_sys::wpe_view_backend_exportable_fdo_create(
                &client,
                ptr::null_mut(), // user data
                width,
                height,
            );

            if exportable.is_null() {
                tracing::error!("Failed to create FDO exportable backend");
                return Err(Error::WebViewCreationFailed);
            }

            exportable
        };

        // SAFETY: Get the view backend from the exportable
        let view_backend = unsafe {
            wpe_sys::wpe_view_backend_exportable_fdo_get_view_backend(exportable)
        };

        if view_backend.is_null() {
            // Clean up the exportable before returning error
            unsafe {
                wpe_sys::wpe_view_backend_exportable_fdo_destroy(exportable);
            }
            tracing::error!("Failed to get view backend from exportable");
            return Err(Error::WebViewCreationFailed);
        }

        // SAFETY: Create the WebKit view backend wrapper
        let webkit_backend = unsafe {
            wpe_sys::webkit_web_view_backend_new(
                view_backend,
                None,           // destroy notify
                ptr::null_mut(), // user data
            )
        };

        if webkit_backend.is_null() {
            unsafe {
                wpe_sys::wpe_view_backend_exportable_fdo_destroy(exportable);
            }
            tracing::error!("Failed to create WebKit view backend");
            return Err(Error::WebViewCreationFailed);
        }

        // SAFETY: Create the WebKitWebView
        let web_view = unsafe {
            wpe_sys::webkit_web_view_new(webkit_backend)
        };

        if web_view.is_null() {
            unsafe {
                wpe_sys::wpe_view_backend_exportable_fdo_destroy(exportable);
            }
            tracing::error!("Failed to create WebKitWebView");
            return Err(Error::WebViewCreationFailed);
        }

        tracing::debug!("Created WebView with settings: {:?}", settings);

        Ok(Self {
            exportable,
            web_view,
            width,
            height,
            settings,
        })
    }

    /// Load a URL in the web view.
    ///
    /// # Errors
    /// Returns an error if the URL is invalid.
    #[allow(unsafe_code)]
    pub fn load_url(&mut self, url: &str) -> Result<()> {
        if url.is_empty() {
            return Err(Error::InvalidUrl("URL cannot be empty".to_string()));
        }

        let c_url = CString::new(url).map_err(|_| Error::InvalidUrl(url.to_string()))?;

        unsafe {
            wpe_sys::webkit_web_view_load_uri(self.web_view, c_url.as_ptr());
        }

        tracing::debug!("Loading URL: {}", url);
        Ok(())
    }

    /// Load HTML content directly.
    ///
    /// # Errors
    /// Returns an error if the HTML could not be loaded.
    #[allow(unsafe_code)]
    pub fn load_html(&mut self, html: &str, base_url: Option<&str>) -> Result<()> {
        let c_html = CString::new(html).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let c_base = base_url
            .map(|u| CString::new(u).ok())
            .flatten();

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
    ///
    /// # Errors
    /// Returns an error if the JavaScript execution failed.
    #[allow(unsafe_code)]
    pub fn evaluate_script(&self, script: &str) -> Result<()> {
        let c_script = CString::new(script).map_err(|e| Error::JavaScriptError(e.to_string()))?;

        unsafe {
            wpe_sys::webkit_web_view_evaluate_javascript(
                self.web_view,
                c_script.as_ptr(),
                script.len() as i64,
                ptr::null(),     // world_name
                ptr::null(),     // source_uri
                ptr::null_mut(), // cancellable
                None,            // callback
                ptr::null_mut(), // user_data
            );
        }

        tracing::debug!("Evaluating script ({} bytes)", script.len());
        Ok(())
    }

    /// Get the current URL.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn url(&self) -> Option<String> {
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
        unsafe { wpe_sys::webkit_web_view_can_go_back(self.web_view) != 0 }
    }

    /// Check if the web view can go forward.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn can_go_forward(&self) -> bool {
        unsafe { wpe_sys::webkit_web_view_can_go_forward(self.web_view) != 0 }
    }

    /// Go back in history.
    #[allow(unsafe_code)]
    pub fn go_back(&mut self) {
        unsafe {
            wpe_sys::webkit_web_view_go_back(self.web_view);
        }
    }

    /// Go forward in history.
    #[allow(unsafe_code)]
    pub fn go_forward(&mut self) {
        unsafe {
            wpe_sys::webkit_web_view_go_forward(self.web_view);
        }
    }

    /// Reload the current page.
    #[allow(unsafe_code)]
    pub fn reload(&mut self) {
        unsafe {
            wpe_sys::webkit_web_view_reload(self.web_view);
        }
    }

    /// Stop loading.
    #[allow(unsafe_code)]
    pub fn stop(&mut self) {
        unsafe {
            wpe_sys::webkit_web_view_stop_loading(self.web_view);
        }
    }

    /// Resize the web view.
    #[allow(unsafe_code)]
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;

        unsafe {
            let view_backend =
                wpe_sys::wpe_view_backend_exportable_fdo_get_view_backend(self.exportable);
            if !view_backend.is_null() {
                wpe_sys::wpe_view_backend_dispatch_set_size(view_backend, width, height);
            }
        }

        tracing::debug!("Resized to {}x{}", width, height);
    }

    /// Process pending events. Call this in your event loop.
    #[allow(unsafe_code)]
    pub fn spin(&mut self) {
        // Process pending GLib main context events
        // This is needed to process WebKit callbacks
        unsafe {
            // Iterate the main context without blocking
            while wpe_sys::g_main_context_iteration(ptr::null_mut(), 0) != 0 {}
        }
    }

    /// Render the web view. Call this when you need to redraw.
    #[allow(unsafe_code)]
    pub fn render(&mut self) {
        // Signal that we're ready for the next frame
        unsafe {
            wpe_sys::wpe_view_backend_exportable_fdo_dispatch_frame_complete(self.exportable);
        }
    }

    /// Check if the view is currently loading.
    #[must_use]
    #[allow(unsafe_code)]
    pub fn is_loading(&self) -> bool {
        unsafe { wpe_sys::webkit_web_view_is_loading(self.web_view) != 0 }
    }
}

impl Drop for WebView {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        unsafe {
            // Unreference the WebKitWebView (GObject reference counting)
            if !self.web_view.is_null() {
                wpe_sys::g_object_unref(self.web_view.cast());
            }

            // Destroy the FDO exportable backend
            if !self.exportable.is_null() {
                wpe_sys::wpe_view_backend_exportable_fdo_destroy(self.exportable);
            }
        }
    }
}

// WebView is not thread-safe (WPE uses GLib main loop)
// We intentionally don't implement Send/Sync

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wpe_version() {
        // SAFETY: These are simple version getter functions
        unsafe {
            let major = wpe_sys::wpe_get_major_version();
            let minor = wpe_sys::wpe_get_minor_version();
            let micro = wpe_sys::wpe_get_micro_version();

            // WPE should be version 1.x or 2.x
            assert!(major >= 1, "WPE major version should be >= 1, got {}", major);
            println!("WPE version: {}.{}.{}", major, minor, micro);
        }
    }

    #[test]
    fn test_wpe_fdo_version() {
        // SAFETY: These are simple version getter functions
        unsafe {
            let major = wpe_sys::wpe_fdo_get_major_version();
            let minor = wpe_sys::wpe_fdo_get_minor_version();
            let micro = wpe_sys::wpe_fdo_get_micro_version();

            // FDO backend should be version 1.x
            assert!(major >= 1, "WPE FDO major version should be >= 1, got {}", major);
            println!("WPE FDO version: {}.{}.{}", major, minor, micro);
        }
    }

    #[test]
    fn test_webview_settings() {
        let settings = WebViewSettings::new()
            .with_url("https://example.com")
            .with_developer_tools(true);

        assert_eq!(settings.url, Some("https://example.com".to_string()));
        assert!(settings.developer_tools);
        assert!(settings.javascript_enabled);
    }

    #[test]
    fn test_wpe_initialization() {
        // Test that we can initialize the WPE loader
        let result = initialize();
        assert!(result.is_ok(), "WPE initialization should succeed");

        // Calling it again should also succeed (idempotent)
        let result2 = initialize();
        assert!(result2.is_ok(), "WPE re-initialization should succeed");
    }
}

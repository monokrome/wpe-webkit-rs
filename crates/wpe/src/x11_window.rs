//! X11 window support for WPE WebKit.
//!
//! This module provides X11 window management for systems without Wayland.
//! It uses headless WPE rendering and blits to an X11 window via shared memory.

use std::ffi::CString;
use std::ptr;

use x11rb::connection::Connection;
use x11rb::protocol::shm::{self, ConnectionExt as ShmConnectionExt};
use x11rb::protocol::xproto::{
    self, ColormapAlloc, ConnectionExt, CreateWindowAux, EventMask, ImageFormat, WindowClass,
};
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::xcb_ffi::XCBConnection;

use crate::ipc::{BackendMessage, FrontendMessage, IpcBridge};
use crate::webview::WebViewSettings;
use crate::{Error, Result};

/// Shared memory segment for X11 image transfer.
struct ShmSegment {
    shm_id: i32,
    seg_id: u32,
    data: *mut u8,
    size: usize,
}

impl ShmSegment {
    /// Create a new shared memory segment.
    fn new(conn: &XCBConnection, size: usize) -> Result<Self> {
        // SAFETY: libc calls for shared memory are well-defined.
        unsafe {
            let shm_id = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if shm_id < 0 {
                return Err(Error::X11Error("Failed to create shared memory".to_string()));
            }

            let data = libc::shmat(shm_id, ptr::null(), 0);
            if data == (-1isize) as *mut libc::c_void {
                libc::shmctl(shm_id, libc::IPC_RMID, ptr::null_mut());
                return Err(Error::X11Error("Failed to attach shared memory".to_string()));
            }

            let seg_id = conn.generate_id().map_err(|e| Error::X11Error(e.to_string()))?;
            shm::attach(conn, seg_id, shm_id as u32, false)
                .map_err(|e| Error::X11Error(e.to_string()))?;

            // Mark for deletion when all processes detach
            libc::shmctl(shm_id, libc::IPC_RMID, ptr::null_mut());

            Ok(Self {
                shm_id,
                seg_id,
                data: data as *mut u8,
                size,
            })
        }
    }

    /// Get a mutable slice to the shared memory.
    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: data is valid for size bytes, allocated in new().
        unsafe { std::slice::from_raw_parts_mut(self.data, self.size) }
    }
}

impl Drop for ShmSegment {
    fn drop(&mut self) {
        // SAFETY: Detaching shared memory we attached in new().
        unsafe {
            libc::shmdt(self.data as *const libc::c_void);
        }
    }
}

/// An X11 window with WPE WebKit integration.
///
/// This uses headless WPE rendering and blits to an X11 window.
pub struct X11Window {
    conn: XCBConnection,
    screen_num: usize,
    window: u32,
    gc: u32,
    shm_seg: Option<ShmSegment>,
    width: u32,
    height: u32,
    /// WPE display (headless)
    display: *mut wpe_sys::WPEDisplay,
    /// WPE view
    view: *mut wpe_sys::WPEView,
    /// WebKit web view
    web_view: *mut wpe_sys::WebKitWebView,
    /// Frame buffer for rendered content
    pixels: Vec<u32>,
    /// IPC bridge
    ipc: IpcBridge,
    /// Whether the window should close
    should_close: bool,
    /// Pending messages from JavaScript
    message_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<FrontendMessage>>>,
}

impl X11Window {
    /// Create a new X11 window with the given settings.
    pub fn new(settings: WebViewSettings) -> Result<Self> {
        let (conn, screen_num) = XCBConnection::connect(None)
            .map_err(|e| Error::X11Error(format!("Failed to connect to X11: {}", e)))?;

        let screen = &conn.setup().roots[screen_num];
        let width = 1280u32;
        let height = 720u32;

        // Create window
        let window = conn.generate_id().map_err(|e| Error::X11Error(e.to_string()))?;
        let colormap = conn.generate_id().map_err(|e| Error::X11Error(e.to_string()))?;

        conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, screen.root_visual)
            .map_err(|e| Error::X11Error(e.to_string()))?;

        let win_aux = CreateWindowAux::new()
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::KEY_PRESS
                    | EventMask::KEY_RELEASE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::ENTER_WINDOW
                    | EventMask::LEAVE_WINDOW,
            )
            .background_pixel(screen.black_pixel)
            .colormap(colormap);

        conn.create_window(
            screen.root_depth,
            window,
            screen.root,
            0,
            0,
            width as u16,
            height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &win_aux,
        )
        .map_err(|e| Error::X11Error(e.to_string()))?;

        // Set window title
        let title = "WPE WebView";
        conn.change_property8(
            xproto::PropMode::REPLACE,
            window,
            xproto::AtomEnum::WM_NAME,
            xproto::AtomEnum::STRING,
            title.as_bytes(),
        )
        .map_err(|e| Error::X11Error(e.to_string()))?;

        // Create graphics context
        let gc = conn.generate_id().map_err(|e| Error::X11Error(e.to_string()))?;
        conn.create_gc(gc, window, &Default::default())
            .map_err(|e| Error::X11Error(e.to_string()))?;

        // Map window
        conn.map_window(window)
            .map_err(|e| Error::X11Error(e.to_string()))?;
        conn.flush().map_err(|e| Error::X11Error(e.to_string()))?;

        // Create shared memory segment for blitting
        let shm_size = (width * height * 4) as usize;
        let shm_seg = ShmSegment::new(&conn, shm_size).ok();

        // Create message queue for IPC
        let message_queue = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));

        // Initialize WPE in headless mode
        // SAFETY: WPE API calls with null checks.
        let (display, view, web_view) = unsafe {
            let display = wpe_sys::wpe_display_headless_new();
            if display.is_null() {
                return Err(Error::X11Error("Failed to create headless WPE display".to_string()));
            }

            let mut error: *mut wpe_sys::GError = ptr::null_mut();
            let connected = wpe_sys::wpe_display_connect(display, &mut error);
            if connected == 0 {
                wpe_sys::g_object_unref(display as *mut _);
                return Err(Error::X11Error("Failed to connect headless display".to_string()));
            }

            // Create view
            let view = wpe_sys::wpe_view_new(display);
            if view.is_null() {
                wpe_sys::g_object_unref(display as *mut _);
                return Err(Error::X11Error("Failed to create WPE view".to_string()));
            }

            // Set initial size
            wpe_sys::wpe_view_resized(view, width as i32, height as i32);
            wpe_sys::wpe_view_set_visible(view, 1);

            // Create WebKit web view (pass null for default backend)
            let web_view = wpe_sys::webkit_web_view_new(ptr::null_mut());
            if web_view.is_null() {
                wpe_sys::g_object_unref(view as *mut _);
                wpe_sys::g_object_unref(display as *mut _);
                return Err(Error::X11Error("Failed to create WebKit web view".to_string()));
            }

            // Get user content manager and set up IPC
            let user_content_manager = wpe_sys::webkit_web_view_get_user_content_manager(web_view);
            if !user_content_manager.is_null() {
                let queue_ptr = std::sync::Arc::into_raw(message_queue.clone());

                let signal_name = CString::new("script-message-received::wpe")
                    .expect("static string has no NUL bytes");
                wpe_sys::g_signal_connect_data(
                    user_content_manager as *mut _,
                    signal_name.as_ptr(),
                    Some(std::mem::transmute::<
                        *const (),
                        unsafe extern "C" fn(),
                    >(on_script_message as *const ())),
                    queue_ptr as *mut _,
                    None,
                    0,
                );

                let handler_name = CString::new("wpe").expect("static string has no NUL bytes");
                wpe_sys::webkit_user_content_manager_register_script_message_handler(
                    user_content_manager,
                    handler_name.as_ptr(),
                    ptr::null(),
                );
            }

            (display, view, web_view)
        };

        let mut window = Self {
            conn,
            screen_num,
            window,
            gc,
            shm_seg,
            width,
            height,
            display,
            view,
            web_view,
            pixels: vec![0xFF000000; (width * height) as usize],
            ipc: IpcBridge::new(),
            should_close: false,
            message_queue,
        };

        // Load initial content
        if let Some(ref url) = settings.url {
            window.load_url(url)?;
        } else if let Some(ref html) = settings.html {
            window.load_html(html, None)?;
        }

        Ok(window)
    }

    /// Load a URL.
    pub fn load_url(&mut self, url: &str) -> Result<()> {
        let c_url = CString::new(url).map_err(|_| Error::InvalidUrl(url.to_string()))?;
        // SAFETY: web_view is valid, c_url is a valid C string.
        unsafe {
            wpe_sys::webkit_web_view_load_uri(self.web_view, c_url.as_ptr());
        }
        Ok(())
    }

    /// Load HTML content.
    pub fn load_html(&mut self, html: &str, base_url: Option<&str>) -> Result<()> {
        let html_with_bridge = IpcBridge::inject_bridge(html);
        let c_html = CString::new(html_with_bridge).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let c_base = base_url.and_then(|u| CString::new(u).ok());

        // SAFETY: web_view is valid, c_html is a valid C string.
        unsafe {
            wpe_sys::webkit_web_view_load_html(
                self.web_view,
                c_html.as_ptr(),
                c_base.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            );
        }
        Ok(())
    }

    /// Process events. Returns false if the window should close.
    pub fn process_events(&mut self) -> Result<bool> {
        // Process X11 events
        while let Some(event) = self.conn.poll_for_event().map_err(|e| Error::X11Error(e.to_string()))? {
            match event {
                x11rb::protocol::Event::Expose(_) => {
                    self.present()?;
                }
                x11rb::protocol::Event::ConfigureNotify(e) => {
                    if e.width as u32 != self.width || e.height as u32 != self.height {
                        self.resize(e.width as u32, e.height as u32)?;
                    }
                }
                x11rb::protocol::Event::ClientMessage(e) => {
                    // Check for WM_DELETE_WINDOW
                    let wm_protocols = self.conn.intern_atom(false, b"WM_PROTOCOLS")
                        .map_err(|e| Error::X11Error(e.to_string()))?
                        .reply()
                        .map_err(|e| Error::X11Error(e.to_string()))?
                        .atom;
                    let wm_delete = self.conn.intern_atom(false, b"WM_DELETE_WINDOW")
                        .map_err(|e| Error::X11Error(e.to_string()))?
                        .reply()
                        .map_err(|e| Error::X11Error(e.to_string()))?
                        .atom;

                    if e.type_ == wm_protocols && e.data.as_data32()[0] == wm_delete {
                        self.should_close = true;
                    }
                }
                x11rb::protocol::Event::KeyPress(e) => {
                    self.handle_key(e.detail as u32, true);
                }
                x11rb::protocol::Event::KeyRelease(e) => {
                    self.handle_key(e.detail as u32, false);
                }
                x11rb::protocol::Event::ButtonPress(e) => {
                    self.handle_button(e.detail as u32, true, e.event_x as f64, e.event_y as f64);
                }
                x11rb::protocol::Event::ButtonRelease(e) => {
                    self.handle_button(e.detail as u32, false, e.event_x as f64, e.event_y as f64);
                }
                x11rb::protocol::Event::MotionNotify(e) => {
                    self.handle_motion(e.event_x as f64, e.event_y as f64);
                }
                _ => {}
            }
        }

        // Process WPE events
        // SAFETY: GLib context functions are safe to call.
        unsafe {
            let ctx = wpe_sys::g_main_context_default();
            while wpe_sys::g_main_context_iteration(ctx, 0) != 0 {}
        }

        // Render and present
        self.render()?;
        self.present()?;

        Ok(!self.should_close)
    }

    /// Handle keyboard input.
    fn handle_key(&mut self, keycode: u32, pressed: bool) {
        let event_type = if pressed {
            wpe_sys::WPEEventType_WPE_EVENT_KEYBOARD_KEY_DOWN
        } else {
            wpe_sys::WPEEventType_WPE_EVENT_KEYBOARD_KEY_UP
        };

        // SAFETY: view is valid.
        unsafe {
            let event = wpe_sys::wpe_event_keyboard_new(
                event_type,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_KEYBOARD,
                crate::input::current_time_ms(),
                0, // modifiers
                keycode,
                keycode,
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::g_object_unref(event as *mut _);
            }
        }
    }

    /// Handle mouse button input.
    fn handle_button(&mut self, button: u32, pressed: bool, x: f64, y: f64) {
        let event_type = if pressed {
            wpe_sys::WPEEventType_WPE_EVENT_POINTER_DOWN
        } else {
            wpe_sys::WPEEventType_WPE_EVENT_POINTER_UP
        };

        // Map X11 button to WPE button (X11: 1=left, 2=middle, 3=right)
        let wpe_button = button;

        // SAFETY: view is valid.
        unsafe {
            let event = wpe_sys::wpe_event_pointer_button_new(
                event_type,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
                crate::input::current_time_ms(),
                0, // modifiers
                wpe_button,
                x,
                y,
                1, // click count
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::g_object_unref(event as *mut _);
            }
        }
    }

    /// Handle mouse motion.
    fn handle_motion(&mut self, x: f64, y: f64) {
        // SAFETY: view is valid.
        unsafe {
            let event = wpe_sys::wpe_event_pointer_move_new(
                wpe_sys::WPEEventType_WPE_EVENT_POINTER_MOVE,
                self.view,
                wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
                crate::input::current_time_ms(),
                0, // modifiers
                x,
                y,
                0.0, // delta_x (unused for move)
                0.0, // delta_y (unused for move)
            );
            if !event.is_null() {
                wpe_sys::wpe_view_event(self.view, event);
                wpe_sys::g_object_unref(event as *mut _);
            }
        }
    }

    /// Render the current frame.
    fn render(&mut self) -> Result<()> {
        // WPE headless rendering happens during event processing
        // The buffer should be available after g_main_context_iteration
        Ok(())
    }

    /// Present the current frame to the X11 window.
    fn present(&mut self) -> Result<()> {
        let screen = &self.conn.setup().roots[self.screen_num];

        if let Some(ref mut shm) = self.shm_seg {
            // Use shared memory for faster blitting
            let data = shm.as_mut_slice();
            let pixel_bytes = bytemuck_cast_pixels(&self.pixels);
            let len = data.len().min(pixel_bytes.len());
            data[..len].copy_from_slice(&pixel_bytes[..len]);

            shm::put_image(
                &self.conn,
                self.window,
                self.gc,
                self.width as u16,
                self.height as u16,
                0,
                0,
                self.width as u16,
                self.height as u16,
                0,
                0,
                screen.root_depth,
                ImageFormat::Z_PIXMAP.into(),
                false,
                shm.seg_id,
                0,
            )
            .map_err(|e| Error::X11Error(e.to_string()))?;
        } else {
            // Fallback to PutImage without shared memory
            let pixel_bytes = bytemuck_cast_pixels(&self.pixels);
            self.conn
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    self.window,
                    self.gc,
                    self.width as u16,
                    self.height as u16,
                    0,
                    0,
                    0,
                    screen.root_depth,
                    pixel_bytes,
                )
                .map_err(|e| Error::X11Error(e.to_string()))?;
        }

        self.conn.flush().map_err(|e| Error::X11Error(e.to_string()))?;
        Ok(())
    }

    /// Resize the window.
    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width.max(1);
        self.height = height.max(1);
        self.pixels.resize((self.width * self.height) as usize, 0xFF000000);

        // Recreate shared memory segment
        let shm_size = (self.width * self.height * 4) as usize;
        self.shm_seg = ShmSegment::new(&self.conn, shm_size).ok();

        // Resize WPE view
        // SAFETY: view is valid.
        unsafe {
            wpe_sys::wpe_view_resized(self.view, self.width as i32, self.height as i32);
        }

        Ok(())
    }

    /// Send a message to JavaScript.
    pub fn send(&self, message: &BackendMessage) -> Result<()> {
        let script = format!(
            "if(window.__wpe_receive){{window.__wpe_receive({})}}",
            serde_json::to_string(message).map_err(|e| Error::JavaScriptError(e.to_string()))?
        );
        self.evaluate_script(&script)
    }

    /// Execute JavaScript.
    pub fn evaluate_script(&self, script: &str) -> Result<()> {
        let c_script = CString::new(script).map_err(|e| Error::JavaScriptError(e.to_string()))?;
        // SAFETY: web_view is valid.
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
        Ok(())
    }

    /// Receive pending messages from JavaScript.
    pub fn receive_messages(&mut self) -> Vec<FrontendMessage> {
        let mut queue = self.message_queue.lock().unwrap();
        queue.drain(..).collect()
    }
}

impl Drop for X11Window {
    fn drop(&mut self) {
        // SAFETY: Releasing GObjects we own.
        unsafe {
            if !self.web_view.is_null() {
                wpe_sys::g_object_unref(self.web_view as *mut _);
            }
            if !self.view.is_null() {
                wpe_sys::g_object_unref(self.view as *mut _);
            }
            if !self.display.is_null() {
                wpe_sys::g_object_unref(self.display as *mut _);
            }
        }
    }
}

/// Script message callback.
unsafe extern "C" fn on_script_message(
    _manager: *mut wpe_sys::WebKitUserContentManager,
    result: *mut wpe_sys::JSCValue,
    user_data: *mut std::ffi::c_void,
) {
    if result.is_null() || user_data.is_null() {
        return;
    }

    let queue = &*(user_data as *const std::sync::Mutex<std::collections::VecDeque<FrontendMessage>>);
    let json_str = wpe_sys::jsc_value_to_string(result);
    if json_str.is_null() {
        return;
    }

    let c_str = std::ffi::CStr::from_ptr(json_str);
    if let Ok(s) = c_str.to_str() {
        if let Ok(msg) = serde_json::from_str::<FrontendMessage>(s) {
            if let Ok(mut q) = queue.lock() {
                q.push_back(msg);
            }
        }
    }

    wpe_sys::g_free(json_str as *mut _);
}

/// Cast pixel slice to bytes without bytemuck dependency.
fn bytemuck_cast_pixels(pixels: &[u32]) -> &[u8] {
    // SAFETY: u32 slice can be viewed as u8 slice with 4x length.
    unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, pixels.len() * 4) }
}

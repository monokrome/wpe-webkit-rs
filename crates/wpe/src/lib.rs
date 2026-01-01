//! Safe Rust bindings for WPE WebKit.
//!
//! This crate provides a GTK-free way to embed web views in Rust applications.
//! It uses WPE WebKit with the FDO backend for rendering without GTK dependencies.
//!
//! ## Features
//!
//! - `winit` (default): Integration with the winit windowing library
//!
//! ## Example
//!
//! ```rust,ignore
//! use wpe::{WebViewSettings, WpeApp};
//!
//! fn main() -> wpe::Result<()> {
//!     let settings = WebViewSettings::new()
//!         .with_html("<h1>Hello, WPE!</h1>");
//!
//!     WpeApp::new(settings, |_window, msg| {
//!         println!("Received: {:?}", msg);
//!         None
//!     }).run()
//! }
//! ```
//!
//! ## System Requirements
//!
//! This crate requires the following system packages:
//!
//! ### Arch Linux / Artix
//! ```sh
//! pacman -S libwpe wpewebkit wpebackend-fdo
//! ```
//!
//! ### Debian / Ubuntu
//! ```sh
//! apt install libwpe-1.0-dev libwpewebkit-1.0-dev libwpebackend-fdo-1.0-dev
//! ```

pub mod error;
pub mod input;
pub mod ipc;
pub mod native;
pub mod webview;

#[cfg(feature = "x11")]
pub mod x11_window;

#[cfg(feature = "winit")]
pub mod renderer;
#[cfg(feature = "winit")]
pub mod window;

pub use error::{Error, Result};
pub use ipc::{BackendMessage, FrontendMessage, IpcBridge};
pub use native::{LoadState, NativeWindow, NavigationEvent};

#[cfg(feature = "x11")]
pub use x11_window::X11Window;
pub use webview::{initialize, WebView, WebViewSettings};

#[cfg(feature = "winit")]
pub use renderer::SharedFrameBuffer;

#[cfg(feature = "winit")]
pub use renderer::SoftwareRenderer;
#[cfg(feature = "gpu")]
pub use renderer::GpuRenderer;
#[cfg(feature = "winit")]
pub use window::{WpeApp, WpeEvent, WpeWindow};

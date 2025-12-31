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
pub mod ipc;
pub mod webview;

#[cfg(feature = "winit")]
pub mod renderer;
#[cfg(feature = "winit")]
pub mod window;

pub use error::{Error, Result};
pub use ipc::{BackendMessage, FrontendMessage, IpcBridge};
pub use webview::{initialize, WebView, WebViewSettings};

#[cfg(feature = "winit")]
pub use renderer::{RenderContext, SoftwareRenderer};
#[cfg(feature = "winit")]
pub use window::{WpeApp, WpeEvent, WpeWindow};

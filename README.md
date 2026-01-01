# wpe-webkit-rs

Safe Rust bindings for [WPE WebKit](https://wpewebkit.org/) - GTK-free embedded web views for Linux.

## Overview

`wpe-webkit-rs` provides idiomatic Rust bindings for WPE WebKit, allowing you to embed a full web browser in your application without GTK dependencies. It's designed for headless rendering, embedded systems, and custom windowing environments.

## Features

- **No GTK Required** - Works with Wayland compositors directly or in headless mode
- **Multiple Rendering Modes**:
  - `winit` - Cross-platform windowing via winit + softbuffer
  - `gpu` - GPU-accelerated rendering via wgpu
  - `x11` - X11 fallback using SHM blitting
  - Native Wayland compositors
- **IPC Bridge** - Bidirectional JavaScript ↔ Rust communication
- **Full Input Handling** - Keyboard, mouse, and scroll events
- **Safe Rust API** - Memory-safe wrappers around the C API

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
wpe = "0.1"
```

### System Dependencies

**Arch Linux:**
```bash
pacman -S wpewebkit wpebackend-fdo
```

**Ubuntu/Debian:**
```bash
apt install libwpe-1.0-dev libwpewebkit-2.0-dev libwpebackend-fdo-1.0-dev
```

## Quick Start

```rust
use wpe::{WebView, WebViewSettings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = WebViewSettings::new()
        .with_url("https://example.com")
        .with_size(1280, 720);

    let mut view = WebView::new(settings)?;
    view.run()?;

    Ok(())
}
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `winit` | Yes | Cross-platform windowing with softbuffer |
| `gpu` | No | GPU-accelerated rendering via wgpu |
| `x11` | No | X11 fallback for non-Wayland environments |

Enable features in `Cargo.toml`:

```toml
[dependencies]
wpe = { version = "0.1", features = ["gpu", "x11"] }
```

## Examples

### Basic WebView (winit)

```bash
cargo run --example basic
```

### Native Wayland

```bash
cargo run --example native
```

### JavaScript IPC

```rust
use wpe::{WebView, WebViewSettings, ipc::FrontendMessage};

// Send message to JavaScript
view.send_to_frontend(FrontendMessage::Custom {
    event: "greeting".to_string(),
    data: serde_json::json!({"message": "Hello from Rust!"}),
});

// Receive messages from JavaScript
while let Some(msg) = view.poll_message() {
    println!("Received: {:?}", msg);
}
```

In JavaScript:
```javascript
// Receive from Rust
window.addEventListener('message', (event) => {
    console.log('From Rust:', event.data);
});

// Send to Rust
window.webkit.messageHandlers.muckrake.postMessage({
    type: 'custom',
    event: 'clicked',
    data: { button: 'submit' }
});
```

## Architecture

```
┌─────────────────────────────────────────┐
│              Your Application           │
├─────────────────────────────────────────┤
│   wpe (safe Rust API)                   │
│   ├── WebView / NativeWindow / X11Window│
│   ├── IpcBridge                         │
│   └── Renderer (Software/GPU)           │
├─────────────────────────────────────────┤
│   wpe-sys (raw FFI bindings)            │
├─────────────────────────────────────────┤
│   WPE WebKit + WPEBackend-FDO           │
└─────────────────────────────────────────┘
```

## Minimum Supported Rust Version

Rust 1.75.0

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR on GitHub.

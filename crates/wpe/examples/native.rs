//! Native WPE window example with IPC.
//!
//! This example uses WPE's native Wayland window management
//! and demonstrates bidirectional JavaScript ↔ Rust communication.

use wpe::{LoadState, NativeWindow, NavigationEvent, Result, WebViewSettings};
#[allow(unused_imports)]
use wpe::FrontendMessage;

const HTML_CONTENT: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>WPE Native Window</title>
    <style>
        body {
            font-family: system-ui, sans-serif;
            margin: 0;
            padding: 20px;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            color: white;
        }
        .container {
            max-width: 600px;
            margin: 0 auto;
            background: rgba(255, 255, 255, 0.1);
            border-radius: 10px;
            padding: 20px;
            backdrop-filter: blur(10px);
        }
        h1 { margin-top: 0; }
        p { line-height: 1.6; }
        button {
            background: rgba(255,255,255,0.2);
            border: 1px solid rgba(255,255,255,0.3);
            color: white;
            padding: 10px 20px;
            border-radius: 5px;
            cursor: pointer;
            margin: 5px;
        }
        button:hover { background: rgba(255,255,255,0.3); }
        #messages {
            background: rgba(0,0,0,0.2);
            padding: 10px;
            border-radius: 5px;
            margin-top: 10px;
            min-height: 50px;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>WPE Native Window</h1>
        <p>This is a native WPE window with JavaScript ↔ Rust IPC.</p>

        <button onclick="sendPing()">Send Ping to Rust</button>
        <button onclick="sendGreeting()">Send Greeting</button>

        <div id="messages">Waiting for messages...</div>
    </div>

    <script>
    // IPC Bridge (injected by Rust when using load_html_with_ipc)
    (function() {
        'use strict';
        const hasWebKitHandler = typeof webkit !== 'undefined' &&
                                 webkit.messageHandlers &&
                                 webkit.messageHandlers.wpe;

        window.__wpe_receive = function(msg) {
            window.dispatchEvent(new CustomEvent('wpe:message', { detail: msg }));
        };

        window.__wpe_send = function(msg) {
            if (hasWebKitHandler) {
                webkit.messageHandlers.wpe.postMessage(JSON.stringify(msg));
            } else {
                console.log('No WebKit handler, would send:', msg);
            }
        };

        window.wpe = {
            send(type, payload) { window.__wpe_send({ type, payload }); },
            onMessage(callback) {
                window.addEventListener('wpe:message', (e) => callback(e.detail));
            }
        };

        console.log('IPC Bridge initialized. WebKit handler:', hasWebKitHandler);
        window.dispatchEvent(new CustomEvent('wpe:ready'));
    })();

    // UI functions
    function log(msg) {
        const el = document.getElementById('messages');
        el.innerHTML = new Date().toLocaleTimeString() + ': ' + msg + '<br>' + el.innerHTML;
    }

    function sendPing() {
        log('Sending ping...');
        wpe.send('ping', { timestamp: Date.now() });
    }

    function sendGreeting() {
        log('Sending greeting...');
        wpe.send('greeting', { name: 'JavaScript' });
    }

    // Handle messages from Rust
    wpe.onMessage((msg) => {
        log('Received: ' + JSON.stringify(msg));
    });

    log('Ready! Click buttons to test IPC.');
    </script>
</body>
</html>
"#;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let settings = WebViewSettings::new()
        .with_html(HTML_CONTENT)
        .with_developer_tools(true);

    let mut window = NativeWindow::new(settings)?;
    window.set_title("WPE Native IPC Example");
    window.load_html(HTML_CONTENT, None)?;

    println!("Running native WPE window with IPC...");
    println!("Click buttons in the web page to test JavaScript ↔ Rust communication.");
    println!("Press Ctrl+C to exit.");

    // Run event loop with message handling
    loop {
        // Process events and collect messages
        if !window.process_events() {
            break;
        }

        // Handle navigation events
        for event in window.receive_events() {
            match event {
                NavigationEvent::LoadChanged(state) => {
                    let state_str = match state {
                        LoadState::Started => "started",
                        LoadState::Redirected => "redirected",
                        LoadState::Committed => "committed",
                        LoadState::Finished => "finished",
                    };
                    println!("Load {}", state_str);
                }
                NavigationEvent::TitleChanged(title) => {
                    println!("Title: {}", title);
                }
                NavigationEvent::UrlChanged(url) => {
                    println!("URL: {}", url);
                }
                NavigationEvent::ProgressChanged(progress) => {
                    if progress > 0.0 && progress < 1.0 {
                        println!("Loading: {:.0}%", progress * 100.0);
                    }
                }
            }
        }

        // Handle received messages
        for msg in window.receive_messages() {
            println!("Received message: {:?}", msg);

            match msg.message_type.as_str() {
                "ping" => {
                    println!("  -> Received ping, sending pong");
                    if let Err(e) = window.send_typed("pong", &serde_json::json!({
                        "response": "pong",
                        "server_time": std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis()
                    })) {
                        eprintln!("Failed to send pong: {}", e);
                    }
                }
                "greeting" => {
                    let name = msg.payload.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");
                    println!("  -> Received greeting from: {}", name);
                    if let Err(e) = window.send_typed("greeting_response", &serde_json::json!({
                        "message": format!("Hello from Rust, {}!", name)
                    })) {
                        eprintln!("Failed to send greeting response: {}", e);
                    }
                }
                _ => {
                    println!("  -> Unknown message type: {}", msg.message_type);
                }
            }
        }

        // Small sleep to avoid busy-waiting
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    Ok(())
}

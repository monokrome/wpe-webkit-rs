//! Basic example of using WPE WebKit with winit.
//!
//! Run with: `cargo run --example basic`

use wpe::{WebViewSettings, WpeApp};

fn main() -> wpe::Result<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt::init();

    let html = r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>WPE WebKit Example</title>
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
        button {
            background: white;
            color: #764ba2;
            border: none;
            padding: 10px 20px;
            border-radius: 5px;
            cursor: pointer;
            font-size: 16px;
        }
        button:hover { background: #f0f0f0; }
        #response {
            margin-top: 20px;
            padding: 10px;
            background: rgba(0, 0, 0, 0.2);
            border-radius: 5px;
            font-family: monospace;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>WPE WebKit + Rust</h1>
        <p>This is a GTK-free web view powered by WPE WebKit.</p>
        <button onclick="sendMessage()">Send Message to Rust</button>
        <div id="response"></div>
    </div>

    <script>
        // Wait for the IPC bridge to be ready
        window.addEventListener('wpe:ready', () => {
            console.log('WPE bridge ready!');

            // Listen for messages from Rust
            wpe.onMessage((msg) => {
                console.log('Received from Rust:', msg);
                document.getElementById('response').textContent =
                    'Response: ' + JSON.stringify(msg, null, 2);
            });
        });

        async function sendMessage() {
            try {
                const result = await wpe.call('greet', { name: 'WPE User' });
                document.getElementById('response').textContent =
                    'Response: ' + JSON.stringify(result, null, 2);
            } catch (e) {
                document.getElementById('response').textContent =
                    'Error: ' + e.message;
            }
        }
    </script>
</body>
</html>
"#;

    let settings = WebViewSettings::new()
        .with_html(html)
        .with_developer_tools(true);

    WpeApp::new(settings, |_window, msg| {
        println!("Received message: {:?}", msg);

        // Handle the "greet" message
        if msg.message_type == "greet" {
            let name = msg.payload.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("World");

            return Some(serde_json::json!({
                "greeting": format!("Hello, {}!", name),
                "from": "Rust"
            }));
        }

        None
    }).run()
}

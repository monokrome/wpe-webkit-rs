use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::{Result, WebView};

/// JavaScript bridge code injected into web pages.
pub const JS_BRIDGE: &str = r#"
(function() {
    'use strict';

    // Message queue for outgoing messages
    const pendingMessages = [];

    // Receive message from Rust
    window.__wpe_receive = function(msg) {
        window.dispatchEvent(new CustomEvent('wpe:message', { detail: msg }));
    };

    // Send message to Rust (via custom URI scheme)
    window.__wpe_send = function(msg) {
        pendingMessages.push(msg);
        // Trigger fetch to custom endpoint that Rust intercepts
        fetch('wpe://message', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(msg)
        }).catch(() => {});
    };

    // Poll pending messages (called by Rust)
    window.__wpe_poll = function() {
        const msgs = pendingMessages.splice(0);
        return JSON.stringify(msgs);
    };

    // Public API
    window.wpe = {
        send(type, payload) {
            window.__wpe_send({ type, payload });
        },

        onMessage(callback) {
            window.addEventListener('wpe:message', (e) => callback(e.detail));
        },

        // Promise-based request/response
        async call(type, payload) {
            return new Promise((resolve, reject) => {
                const id = Math.random().toString(36).slice(2);
                const handler = (e) => {
                    if (e.detail && e.detail._responseId === id) {
                        window.removeEventListener('wpe:message', handler);
                        if (e.detail.error) {
                            reject(new Error(e.detail.error));
                        } else {
                            resolve(e.detail.result);
                        }
                    }
                };
                window.addEventListener('wpe:message', handler);
                window.__wpe_send({ type, payload, _requestId: id });

                // Timeout after 30 seconds
                setTimeout(() => {
                    window.removeEventListener('wpe:message', handler);
                    reject(new Error('Request timeout'));
                }, 30000);
            });
        }
    };

    // Signal that bridge is ready
    window.dispatchEvent(new CustomEvent('wpe:ready'));
})();
"#;

/// A message from the frontend (JavaScript) to the backend (Rust).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(rename = "_requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// A message from the backend (Rust) to the frontend (JavaScript).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(rename = "_responseId", skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BackendMessage {
    /// Create a new message.
    #[must_use]
    pub fn new(message_type: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            message_type: message_type.into(),
            payload,
            response_id: None,
            result: None,
            error: None,
        }
    }

    /// Create a success response to a request.
    #[must_use]
    pub fn response(request_id: String, result: serde_json::Value) -> Self {
        Self {
            message_type: "response".to_string(),
            payload: serde_json::Value::Null,
            response_id: Some(request_id),
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response to a request.
    #[must_use]
    pub fn error_response(request_id: String, error: impl Into<String>) -> Self {
        Self {
            message_type: "response".to_string(),
            payload: serde_json::Value::Null,
            response_id: Some(request_id),
            result: None,
            error: Some(error.into()),
        }
    }
}

/// IPC bridge for communication between Rust and JavaScript.
pub struct IpcBridge {
    pending_messages: VecDeque<FrontendMessage>,
}

impl IpcBridge {
    /// Create a new IPC bridge.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending_messages: VecDeque::new(),
        }
    }

    /// Get the JavaScript bridge code that should be injected into pages.
    #[must_use]
    pub fn js_bridge_code() -> &'static str {
        JS_BRIDGE
    }

    /// Inject the IPC bridge into HTML content.
    #[must_use]
    pub fn inject_bridge(html: &str) -> String {
        format!("{html}<script>{JS_BRIDGE}</script>")
    }

    /// Poll for pending messages from the frontend.
    ///
    /// This should be called in your event loop to receive messages.
    pub fn poll(&mut self, _webview: &WebView) -> Vec<FrontendMessage> {
        // TODO: Call window.__wpe_poll() and parse the result
        // For now, drain the internal queue
        self.pending_messages.drain(..).collect()
    }

    /// Send a message to the frontend.
    ///
    /// # Errors
    /// Returns an error if the message could not be serialized or sent.
    pub fn send(&self, webview: &WebView, message: &BackendMessage) -> Result<()> {
        let json = serde_json::to_string(message)?;
        let script = format!("window.__wpe_receive({json})");
        webview.evaluate_script(&script)
    }

    /// Send a typed message to the frontend.
    ///
    /// # Errors
    /// Returns an error if serialization or sending fails.
    pub fn send_typed<T: Serialize>(
        &self,
        webview: &WebView,
        message_type: &str,
        payload: &T,
    ) -> Result<()> {
        let payload_value = serde_json::to_value(payload)?;
        let message = BackendMessage::new(message_type, payload_value);
        self.send(webview, &message)
    }

    /// Handle an incoming message and optionally respond.
    ///
    /// The handler receives the message and returns an optional response.
    pub fn handle<F>(&mut self, webview: &WebView, handler: F) -> Result<()>
    where
        F: Fn(&FrontendMessage) -> Option<serde_json::Value>,
    {
        let messages = self.poll(webview);

        for msg in messages {
            if let Some(result) = handler(&msg) {
                if let Some(request_id) = &msg.request_id {
                    let response = BackendMessage::response(request_id.clone(), result);
                    self.send(webview, &response)?;
                }
            }
        }

        Ok(())
    }
}

impl Default for IpcBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// Typed message handler for convenient message routing.
pub trait MessageHandler {
    /// The message type this handler responds to.
    fn message_type(&self) -> &str;

    /// Handle the message and return a response.
    fn handle(&self, payload: serde_json::Value) -> Result<serde_json::Value>;
}

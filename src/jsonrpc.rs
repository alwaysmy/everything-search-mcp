//! Minimal JSON-RPC 2.0 / MCP request handler.
//!
//! The plugin feeds it one request per HTTP POST (v3 has no stdio loop).
//! Notifications (no `id`) produce no response, per spec.

use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "2024-11-05";
pub const SERVER_NAME: &str = "everything_search_mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Server<H: Handler> {
    handler: H,
    initialized: bool,
}

/// A tool result carrying both the text form and the structured form.
///
/// MCP allows `structuredContent` alongside `content`. Clients have disagreed
/// about which reaches the model (Codex had a regression where only
/// `structuredContent` was forwarded), so both channels carry the same
/// information instead of splitting results and metadata between them.
pub struct ToolOutput {
    pub text: String,
    pub structured: Value,
}

impl ToolOutput {
    pub fn new(text: impl Into<String>, structured: Value) -> Self {
        Self {
            text: text.into(),
            structured,
        }
    }
}

pub trait Handler {
    fn list_tools(&self) -> Value;
    fn call_tool(&mut self, name: &str, args: &Value) -> Result<ToolOutput, String>;
    /// Server-level guidance surfaced to clients on initialize.
    fn instructions(&self) -> Option<String> {
        None
    }
}

impl<H: Handler> Server<H> {
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            initialized: false,
        }
    }

    /// Handle one raw line. Returns `None` for notifications.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                ))
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Notifications carry no id: never reply, even on error.
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };

        let result = match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => {
                if !self.initialized {
                    // Be permissive: some clients skip the initialized notification.
                    self.initialized = true;
                }
                Ok(self.handler.list_tools())
            }
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let raw_args = params.get("arguments").cloned().unwrap_or(json!({}));
                let args = unwrap_params(raw_args);
                match self.handler.call_tool(name, &args) {
                    Ok(out) => Ok(json!({
                        "content": [{"type": "text", "text": out.text}],
                        "structuredContent": out.structured,
                        "isError": false,
                    })),
                    Err(e) => Ok(tool_result(&e, true)),
                }
            }
            other => Err((-32601, format!("method not found: {other}"))),
        };

        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, msg)) => error_response(id, code, &msg),
        })
    }

    fn initialize(&mut self, params: Value) -> Value {
        self.initialized = true;
        let requested = params
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or(PROTOCOL_VERSION)
            .to_string();
        let mut result = json!({
            "protocolVersion": requested,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
        });
        if let Some(text) = self.handler.instructions() {
            result["instructions"] = Value::String(text);
        }
        result
    }
}

/// Compatibility shim: older clients (and clients holding a stale cached schema)
/// send `{"params": {...}}` instead of a flat argument object. Unwrap it.
fn unwrap_params(args: Value) -> Value {
    if let Value::Object(ref map) = args {
        if map.len() == 1 {
            if let Some(Value::Object(inner)) = map.get("params") {
                return Value::Object(inner.clone());
            }
        }
    }
    args
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": is_error,
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

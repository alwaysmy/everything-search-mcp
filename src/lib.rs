//! Everything file search, one implementation behind an in-process MCP endpoint.
//!
//! * [`plugin`] — the in-process Everything 1.5 plugin (`cdylib`): Everything loads
//!   it into its own process, the MCP endpoint is a socket served from there, and
//!   queries still go through the same HTTP client code. The plugin only changes
//!   *where the MCP endpoint lives* — it rides on Everything's lifetime instead of
//!   being a separately supervised process.

// The `Handler` trait is public and its signatures name `serde_json::Value`, so
// integration tests implementing it need *this* crate's serde_json instance, not
// whichever one resolves in their own dependency graph.
pub use serde_json;

pub mod everything;
pub mod filetype;
pub mod jsonrpc;
pub mod plugin;
pub mod servers;
pub mod tools;

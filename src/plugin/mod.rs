//! In-process Everything 1.5 plugin: the MCP endpoint rides on Everything's
//! lifetime, the queries stay on the HTTP path.
//!
//! `src/main.rs` is the standalone binary — a client spawns it and speaks MCP over
//! stdio. This library is the same server with a different host: Everything.exe
//! loads this DLL into its own process, `PM_START` binds a loopback HTTP listener
//! inside that process, and every tool call runs through the crate's normal
//! [`crate::everything::Client`] against Everything's HTTP server
//! (`http_server64.dll`, `127.0.0.1:23333`).
//!
//! What that buys, and what it does not:
//!
//! * No separately supervised process — the endpoint exists exactly as long as
//!   Everything does. Nothing to restart by hand after a reboot, no orphan stdio
//!   children.
//! * One tool implementation. The 5 tools, their schemas, their multi-instance
//!   grouping and the remote `servers.json` registry are unchanged; only the front
//!   door moves from stdin/stdout to a socket.
//! * No shared server state. Each request builds its own [`Server`] over a copy of
//!   the config, so a slow tool call cannot delay the ones arriving beside it
//!   (the stdio front end is inherently one-at-a-time; the socket front end is
//!   not, and must not simulate it with a lock).
//! * No `db_*` main-thread problem. The host's database API may only be called on
//!   Everything's main thread (off-thread calls crash the host with `0xC0000005`),
//!   so it is deliberately unused here. A faster `db_*` path for the handful of
//!   calls that justify the marshaling can be added behind the same tool API later.
//!
//! Settings today come from the environment (`EVERYTHING_MCP_PORT`,
//! `EVERYTHING_MCP_BIND`); an Everything options page can replace that once the
//! settings API is wired up.
//!
//! # Safety posture
//!
//! This code runs inside `Everything.exe`, so an abort here takes the user's file
//! manager with it: plugin entry points never panic and host-owned memory only goes
//! through host guards.

#![allow(non_snake_case)]

pub mod server;

mod host;

use core::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

use crate::everything::{Client, Config};
use crate::jsonrpc::Server;
use crate::tools::Tools;

use server::{Endpoint, HandlerFactory};

// `EVERYTHING_PLUGIN_PM_*` (everything_plugin.h, lines 203-221).
const PM_INIT: u32 = 1;
const PM_GET_PLUGIN_VERSION: u32 = 2;
const PM_GET_NAME: u32 = 3;
const PM_GET_DESCRIPTION: u32 = 4;
const PM_GET_AUTHOR: u32 = 5;
const PM_GET_VERSION: u32 = 6;
const PM_GET_LINK: u32 = 7;
const PM_START: u32 = 8;
const PM_STOP: u32 = 9;
const PM_KILL: u32 = 10;
const PM_UNINSTALL: u32 = 11;

const PLUGIN_API_VERSION: usize = 1; // EVERYTHING_PLUGIN_VERSION

const PLUGIN_NAME: &[u8] = b"everything-search-mcp\0";
const PLUGIN_DESCRIPTION: &[u8] =
    b"Everything's file search as MCP tools; the endpoint is served in-process and rides on Everything's lifetime\0";
const PLUGIN_AUTHOR: &[u8] = b"Lyra-42\0";
const PLUGIN_VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
const PLUGIN_LINK: &[u8] = b"https://github.com/alwaysmy/everything-search-mcp\0";

/// Default MCP endpoint port. 8285 is deliberately skipped (another Everything MCP
/// plugin defaults there); 23333 belongs to Everything's own HTTP server.
const DEFAULT_PORT: u16 = 8286;

const HANDLED: *mut c_void = 1 as *mut c_void;

static ENDPOINT: OnceLock<Mutex<Option<Endpoint>>> = OnceLock::new();

fn endpoint_slot() -> &'static Mutex<Option<Endpoint>> {
    ENDPOINT.get_or_init(|| Mutex::new(None))
}

/// The MCP handler factory this plugin serves: the crate's tool layer over the
/// normal HTTP backend. One server is built per request, so concurrent calls do
/// not queue behind a slow one. Exposed so tests can drive the same wiring without
/// the host.
pub fn shared_handler() -> HandlerFactory {
    Arc::new(|| {
        let client = Client::new(Config::from_env());
        Server::new(Tools::new(client))
    })
}

/// Start (or restart) the MCP endpoint on `port`; `0` lets the OS choose.
pub fn start_endpoint(port: u16) -> std::io::Result<u16> {
    let mut slot = endpoint_slot().lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(running) = slot.as_ref() {
        return Ok(running.addr().port());
    }
    let endpoint = Endpoint::start(port, shared_handler())?;
    let bound = endpoint.addr().port();
    *slot = Some(endpoint);
    Ok(bound)
}

/// Stop the MCP endpoint, if one is running.
pub fn stop_endpoint() {
    let mut slot = endpoint_slot().lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(mut running) = slot.take() {
        running.stop();
    }
}

/// `everything_plugin_proc(DWORD msg, void *data)` — the plugin's only export.
///
/// Return values are per-message: `1` for "handled", `NULL` for "not handled", and
/// a pointer for the identity messages.
///
/// # Safety
///
/// Called by Everything on its main thread; `data` has the type named by the
/// `EVERYTHING_PLUGIN_PM_*` message.
#[no_mangle]
pub unsafe extern "system" fn everything_plugin_proc(msg: u32, data: *mut c_void) -> *mut c_void {
    match msg {
        PM_INIT => {
            log("PM_INIT");
            // `data` is everything_plugin_get_proc_address_t.
            let get: host::FnGetProcAddress = core::mem::transmute(data);
            match host::resolve(get) {
                Ok(()) => HANDLED,
                Err(missing) => {
                    log(&format!("PM_INIT failed: host lacks {missing}"));
                    core::ptr::null_mut()
                }
            }
        }
        PM_GET_PLUGIN_VERSION => PLUGIN_API_VERSION as *mut c_void,
        PM_GET_NAME => PLUGIN_NAME.as_ptr() as *mut c_void,
        PM_GET_DESCRIPTION => PLUGIN_DESCRIPTION.as_ptr() as *mut c_void,
        PM_GET_AUTHOR => PLUGIN_AUTHOR.as_ptr() as *mut c_void,
        PM_GET_VERSION => PLUGIN_VERSION.as_ptr() as *mut c_void,
        PM_GET_LINK => PLUGIN_LINK.as_ptr() as *mut c_void,

        PM_START => {
            let port = std::env::var("EVERYTHING_MCP_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(DEFAULT_PORT);
            match start_endpoint(port) {
                Ok(bound) => log(&format!(
                    "started: MCP on http://127.0.0.1:{bound}/ — plugin {} on Everything {}",
                    env!("CARGO_PKG_VERSION"),
                    host::version_text()
                )),
                Err(error) => log(&format!("start failed on port {port}: {error}")),
            }
            HANDLED
        }
        PM_STOP => {
            stop_endpoint();
            log("stopped (Everything is shutting down)");
            HANDLED
        }
        PM_KILL => HANDLED,
        PM_UNINSTALL => HANDLED,
        _ => core::ptr::null_mut(),
    }
}

/// Append one line to `%LOCALAPPDATA%\everything-search-mcp\plugin.log`.
///
/// No-ops when the directory or file cannot be used: diagnostics must never take
/// the host down.
fn log(line: &str) {
    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let dir = std::path::Path::new(&base).join("everything-search-mcp");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let text = format!("[{stamp}] {line}\n");
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("plugin.log"))
    {
        let _ = file.write_all(text.as_bytes());
    }
}

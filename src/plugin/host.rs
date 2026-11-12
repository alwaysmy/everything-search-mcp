//! Everything 1.5 host plugin API, resolved by name at `PM_INIT`.
//!
//! Everything.exe exports no symbols. At `PM_INIT` the host hands the plugin an
//! `everything_plugin_get_proc_address_t` callback, and every host function is
//! looked up through it by its short name. The signatures below are copied from
//! the official HTTP Server plugin source (`voidtools/http_server`,
//! `src/http_server.c`) so the ABI matches what the host actually calls.
//!
//! Only the functions this plugin uses are bound. Queries deliberately do **not**
//! go through the host's `db_*` API: they go over Everything's loopback HTTP
//! server through the crate's normal [`crate::everything::Client`]. That keeps one
//! tool implementation for both front ends and keeps `db_*`'s main-thread
//! affinity (calling it off the main thread crashes the host) out of the picture.
//! A `db_*` fast path can be added later without touching the tool layer.
//!
//! See `third_party/everything-sdk/README.md`.

use core::ffi::c_void;
use std::sync::OnceLock;

pub type Utf8 = u8;

/// `everything_plugin_get_proc_address_t`.
pub type FnGetProcAddress = unsafe extern "system" fn(name: *const Utf8) -> *mut c_void;

/// Growable UTF-8 buffer owned by the host.
///
/// `size == 0` means `buf` points at an external const buffer: never write it and
/// never free it. Always build one with [`Utf8Buf::new`] and drop it — the host
/// allocator, not Rust's, owns the heap part.
#[repr(C)]
pub struct Utf8Buf {
    pub buf: *mut Utf8,
    pub len: usize,
    pub size: usize,
    pub stack: [Utf8; 260],
}

impl Utf8Buf {
    /// Initialise with the host (`utf8_buf_init`). The buffer starts empty.
    pub fn new() -> Self {
        let mut cbuf = Self {
            buf: core::ptr::null_mut(),
            len: 0,
            size: 0,
            stack: [0; 260],
        };
        if let Some(p) = procs() {
            unsafe { (p.utf8_buf_init)(&mut cbuf) };
        }
        cbuf
    }

    /// The bytes as a `str` (empty when the buffer is unset or not UTF-8).
    pub fn as_str(&self) -> &str {
        if self.buf.is_null() || self.len == 0 {
            return "";
        }
        unsafe {
            let bytes = core::slice::from_raw_parts(self.buf, self.len);
            core::str::from_utf8(bytes).unwrap_or("")
        }
    }
}

impl Default for Utf8Buf {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Utf8Buf {
    fn drop(&mut self) {
        if let Some(p) = procs() {
            unsafe { (p.utf8_buf_kill)(self) };
        }
    }
}

/// The host functions this plugin needs. Resolved once, at `PM_INIT`.
pub struct Procs {
    pub utf8_buf_init: unsafe extern "system" fn(cbuf: *mut Utf8Buf),
    pub utf8_buf_kill: unsafe extern "system" fn(cbuf: *mut Utf8Buf),
    pub version_get_text: unsafe extern "system" fn(cbuf: *mut Utf8Buf),
}

static PROCS: OnceLock<Procs> = OnceLock::new();

/// The host's function table, or `None` before `PM_INIT` succeeded.
pub fn procs() -> Option<&'static Procs> {
    PROCS.get()
}

/// Look one host function up by its short name (`"utf8_buf_init"`, no prefix).
unsafe fn get_proc<T: Copy>(get: FnGetProcAddress, name: &[u8]) -> Option<T> {
    debug_assert_eq!(
        name.last(),
        Some(&0),
        "host function names must be NUL terminated"
    );
    let proc = get(name.as_ptr());
    if proc.is_null() {
        None
    } else {
        Some(core::mem::transmute_copy(&proc))
    }
}

macro_rules! required {
    ($get:expr, $name:literal) => {
        match get_proc($get, concat!($name, "\0").as_bytes()) {
            Some(f) => f,
            None => return Err($name),
        }
    };
}

/// Resolve every host function we need. On `Err` the name of the missing
/// function is returned and the plugin must fail `PM_INIT` (the host unloads it).
pub fn resolve(get: FnGetProcAddress) -> Result<(), &'static str> {
    let table = unsafe {
        Procs {
            utf8_buf_init: required!(get, "utf8_buf_init"),
            utf8_buf_kill: required!(get, "utf8_buf_kill"),
            version_get_text: required!(get, "version_get_text"),
        }
    };
    // Idempotent: the host may send PM_INIT again on a plugin stop/start cycle
    // without unloading the DLL, and refusing the second call would leave the
    // plugin permanently unusable.
    match PROCS.set(table) {
        Ok(()) => Ok(()),
        Err(_) if PROCS.get().is_some() => Ok(()),
        Err(_) => Err("plugin API already initialised"),
    }
}

/// Everything's version text, e.g. `1.5.0.1416b (x64)`.
pub fn version_text() -> String {
    let Some(p) = procs() else {
        return String::new();
    };
    let mut cbuf = Utf8Buf::new();
    unsafe { (p.version_get_text)(&mut cbuf) };
    cbuf.as_str().to_owned()
}

# Everything 1.5 Plugin SDK (vendored header)

`everything_plugin.h` — the whole of the official Plugin SDK download
(`https://www.voidtools.com/Everything-1.5-Plugin-SDK.zip`), which contains this
single header and nothing else.

| | |
|---|---|
| Source | https://www.voidtools.com/Everything-1.5-Plugin-SDK.zip |
| Archived copy | `Everything-1.5-Plugin-SDK.zip` in this directory (5,270 bytes) |
| Header size | 25,638 bytes, 522 lines |
| Header SHA-256 | `53A2D01EAEEE9227762D953A040096CCA00E2F2BF7C3B39BFD8D41889A5A68B7` |
| Last modified (upstream) | 2025-05-26 |
| License | MIT — Copyright 2025 voidtools / David Carpenter |

## What is in it, and what is not

In it: the `EVERYTHING_PLUGIN_*` constants, the `PM_*` plugin messages, and the
`#[repr(C)]` data structures the host exchanges with a plugin
(`everything_plugin_utf8_buf_t`, `everything_plugin_fileinfo_fd_t`, options-page
structs, Winsock shims).

**Not** in it: the host API function signatures (`db_query_*`, `os_*`,
`utf8_buf_*`, …). The host exports no symbols at all — a plugin resolves those by
name at `EVERYTHING_PLUGIN_PM_INIT` through the `everything_plugin_get_proc_address_t`
callback it is handed. Exact signatures live in the official sample plugin source:

- https://github.com/voidtools/http_server (`src/http_server.c`, the
  `static … (EVERYTHING_PLUGIN_API *everything_plugin_*)(…)` block)
- https://github.com/voidtools/etp_server

`src/plugin/host.rs` in this crate copies those signatures.

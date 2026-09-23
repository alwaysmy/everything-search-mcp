//! The server registry: which Everything instances a search may consult.
//!
//! Everything's HTTP API answers on any host, so "search" is no longer a synonym
//! for "search this machine". This module is the persistent half of that (the
//! `url` argument is the per-call half): a small JSON file listing named instances,
//! each with its own on/off switch.
//!
//! `local` is a reserved name and does not have to be registered - it resolves to
//! `EVERYTHING_HTTP_URL` (127.0.0.1:23333 by default). Registering it explicitly is
//! allowed, and is how the local instance gets its own switch and address.
//!
//! A switch is persisted rather than derived because a machine that is switched off
//! should stay off across restarts. Otherwise every search pays a connect timeout
//! for a host the caller already knows is down, and the only fix is to unregister
//! it - which loses its name and address.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const LOCAL: &str = "local";

/// The file lives under %APPDATA%, not beside the executable or in the working
/// directory: the MCP server is spawned by a client whose working directory is not
/// the caller's, and an installed binary's directory is not writable.
pub const ENV_FILE: &str = "EVERYTHING_SERVERS_FILE";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub url: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub servers: Vec<Entry>,
}

/// Where the registry is read from and written to.
///
/// `EVERYTHING_SERVERS_FILE` overrides it so a test (or a portable install) can
/// point somewhere else without touching the user's real registry.
pub fn path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var(ENV_FILE) {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let base = std::env::var("APPDATA").map_err(|_| {
        format!(
            "APPDATA is not set, so the server registry has no location. \
             Set {ENV_FILE} to choose one explicitly."
        )
    })?;
    Ok(PathBuf::from(base).join("everything-search-mcp").join("servers.json"))
}

/// The address of the local instance, which is what the registry cannot tell us
/// when `local` was never registered.
pub fn local_url() -> String {
    std::env::var("EVERYTHING_HTTP_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:23333".to_string())
}

impl Registry {
    /// Read the registry. A missing file is an empty registry, not an error: that
    /// is the zero-configuration state, where only `local` exists.
    ///
    /// An unparsable file IS an error. Silently falling back to empty would make a
    /// broken edit look like "my remote server disappeared", and the search would
    /// quietly answer from one machine while the caller believes it searched two.
    pub fn load() -> Result<Self, String> {
        let p = path()?;
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("cannot read the server registry at {}: {e}", p.display())),
        };
        // Strip a BOM: an editor that saves as UTF-8-with-BOM is common on Windows,
        // and serde rejects the leading character.
        let text = text.trim_start_matches('\u{feff}');
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let mut reg: Registry = serde_json::from_str(text)
            .map_err(|e| format!("{} is not valid JSON: {e}", p.display()))?;
        for s in &mut reg.servers {
            s.name = s.name.trim().to_string();
            s.url = s.url.trim().to_string();
        }
        Ok(reg)
    }

    /// Write the registry atomically, creating the directory on first use.
    pub fn save(&self) -> Result<PathBuf, String> {
        let p = path()?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let body = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        // Write beside the target and rename: a crash mid-write then leaves the old
        // file intact instead of a truncated one that fails to parse on next start.
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{body}\n"))
            .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &p).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot replace {}: {e}", p.display())
        })?;
        Ok(p)
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.servers.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Entry> {
        self.servers.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Add or replace one entry. Replacing keeps the previous switch state unless
    /// the caller asked for a specific one, so re-pointing an address does not
    /// silently re-enable a server that was deliberately switched off.
    pub fn upsert(&mut self, name: &str, url: &str, enabled: Option<bool>) -> bool {
        match self.get_mut(name) {
            Some(e) => {
                e.url = url.to_string();
                if let Some(on) = enabled {
                    e.enabled = on;
                }
                false
            }
            None => {
                self.servers.push(Entry {
                    name: name.to_string(),
                    url: url.to_string(),
                    enabled: enabled.unwrap_or(true),
                });
                true
            }
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.servers.len();
        self.servers.retain(|s| !s.name.eq_ignore_ascii_case(name));
        self.servers.len() != before
    }

    /// The set a search runs against when no `url` argument is given: everything
    /// switched on, with the local instance first.
    ///
    /// Order is the display order. Local goes first because it is the default
    /// answer to "where is this file", and the remote indexes are the addition.
    pub fn enabled_targets(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        match self.get(LOCAL) {
            // Registered local: its own switch decides, and its own address wins.
            Some(e) if e.enabled => out.push(e.clone()),
            Some(_) => {}
            // Unregistered local: always available, addressed by the environment.
            None => out.push(Entry {
                name: LOCAL.to_string(),
                url: local_url(),
                enabled: true,
            }),
        }
        for s in &self.servers {
            if s.enabled && !s.name.eq_ignore_ascii_case(LOCAL) {
                out.push(s.clone());
            }
        }
        out
    }

    /// Names a caller may pass in `url`, for the tool descriptions. Registered
    /// names only - `local` is always added by the caller of this function, since
    /// it exists whether or not it was registered.
    pub fn names(&self) -> Vec<String> {
        self.servers.iter().map(|s| s.name.clone()).collect()
    }
}

/// Is this a server name rather than an address? Names are what a caller types in
/// `url`, so the distinction has to be unambiguous: a value containing `://` or a
/// colon is treated as an address and never looked up in the registry.
pub fn looks_like_url(v: &str) -> bool {
    v.contains("://") || v.contains(':')
}

/// Reject names that would be ambiguous or unreadable in output.
pub fn validate_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("the server name cannot be empty".into());
    }
    if looks_like_url(n) {
        return Err(format!(
            "'{n}' looks like an address, not a name. Names may not contain ':' - \
             pass the address as the url instead."
        ));
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        return Err(format!(
            "'{n}' contains characters that are not allowed in a name. Use letters, \
             digits, '-', '_' or '.'."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(entries: &[(&str, &str, bool)]) -> Registry {
        Registry {
            servers: entries
                .iter()
                .map(|(n, u, e)| Entry { name: n.to_string(), url: u.to_string(), enabled: *e })
                .collect(),
        }
    }

    #[test]
    fn an_unregistered_local_is_always_available_and_comes_first() {
        let r = reg(&[("workshop", "http://10.0.0.2:23333", true)]);
        let t = r.enabled_targets();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].name, LOCAL);
        assert_eq!(t[1].name, "workshop");
    }

    #[test]
    fn a_switched_off_server_drops_out_but_keeps_its_entry() {
        let r = reg(&[
            ("workshop", "http://10.0.0.2:23333", false),
            ("nas", "http://10.0.0.3:23333", true),
        ]);
        let names: Vec<String> = r.enabled_targets().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec![LOCAL.to_string(), "nas".to_string()]);
        // Still registered, so enabling it again needs no address.
        assert!(r.get("workshop").is_some());
    }

    #[test]
    fn a_registered_local_can_be_switched_off_and_overrides_the_address() {
        let r = reg(&[(LOCAL, "http://127.0.0.1:9999", false)]);
        assert!(r.enabled_targets().is_empty(), "switched-off local must not be implicit");
        let r = reg(&[(LOCAL, "http://127.0.0.1:9999", true)]);
        let t = r.enabled_targets();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].url, "http://127.0.0.1:9999");
    }

    #[test]
    fn upsert_repointing_an_address_does_not_silently_switch_it_back_on() {
        let mut r = reg(&[("workshop", "http://10.0.0.2:23333", false)]);
        assert!(!r.upsert("workshop", "http://10.0.0.9:23333", None));
        let e = r.get("workshop").expect("kept");
        assert_eq!(e.url, "http://10.0.0.9:23333");
        assert!(!e.enabled, "re-pointing must not re-enable");
        // An explicit choice still wins.
        r.upsert("workshop", "http://10.0.0.9:23333", Some(true));
        assert!(r.get("workshop").expect("kept").enabled);
    }

    #[test]
    fn names_are_checked_before_they_can_collide_with_addresses() {
        assert!(validate_name("workshop").is_ok());
        assert!(validate_name("nas.2").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("http://10.0.0.2:23333").is_err());
        assert!(validate_name("10.0.0.2:23333").is_err());
        assert!(validate_name("my server").is_err());
    }

    #[test]
    fn addresses_are_recognised_so_a_name_lookup_never_swallows_one() {
        assert!(looks_like_url("http://10.0.0.2:23333"));
        assert!(looks_like_url("10.0.0.2:23333"));
        assert!(!looks_like_url("workshop"));
        assert!(!looks_like_url("local"));
    }
}

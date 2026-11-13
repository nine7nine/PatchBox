//! A session on disk: a folder under the sessions root holding `session.toml` (our format), each
//! client's files (`<name>.<id>/` for NSM clients), and `.patchbox.lock` while a daemon has it
//! open.
//!
//! ```toml
//! version = 1
//! name = "my song"
//! notes = ""
//!
//! [[client]]
//! id = "ardour"
//! exe = "ardour8"
//! nsm = true
//! name = "Ardour"        # the application name it announced (prefix of its folder)
//! autostart = true       # it was running when the session was saved
//!
//! [connections]          # JACK connections, restored as ports appear
//! list = [["system:capture_1", "ardour:in 1"]]
//! ports = ["system:capture_1", "ardour:in 1"]   # ports present at the last save
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE: &str = "session.toml";
pub const LOCK: &str = ".patchbox.lock";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ClientEntry {
    pub id: String,
    pub exe: String,
    pub args: Vec<String>,
    pub nsm: bool,
    pub name: String,
    pub label: String,
    pub autostart: bool,
    pub gui_visible: Option<bool>,
    /// Never announced last time: don't wait for it.
    pub no_announce: bool,
    pub last_save: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Connections {
    pub list: Vec<(String, String)>,
    pub ports: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SessionFile {
    pub version: u32,
    pub name: String,
    pub notes: String,
    /// Snapshot (git) at every save and close; default on.
    pub auto_snapshot: Option<bool>,
    #[serde(rename = "client")]
    pub clients: Vec<ClientEntry>,
    #[serde(rename = "trashed")]
    pub trash: Vec<ClientEntry>,
    pub connections: Connections,
    /// JACK settings restored when the session opens (jackdbus).
    pub jack: crate::jackcfg::JackSettings,
}

impl SessionFile {
    pub fn load(dir: &Path) -> Result<SessionFile, String> {
        let p = dir.join(FILE);
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// Write atomically (temp file + rename).
    pub fn save(&mut self, dir: &Path) -> Result<(), String> {
        self.version = 1;
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = dir.join(".session.toml.tmp");
        std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, dir.join(FILE))).map_err(|e| format!("{}: {e}", dir.display()))
    }
}

/// Session names are paths relative to the root ("live/gig 1" is allowed), never escaping it.
pub fn valid_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("a session needs a name".into());
    }
    if n.starts_with('/') || n.contains("//") || n.split('/').any(|c| c == ".." || c == "." || c.starts_with('.')) {
        return Err(format!("\"{n}\" is not a usable session name"));
    }
    Ok(())
}

/// Is `dir` inside (or equal to) a folder that holds a session already?
pub fn inside_session(root: &Path, dir: &Path) -> Option<PathBuf> {
    let mut p = dir.parent();
    while let Some(d) = p {
        if !d.starts_with(root) || d == root {
            break;
        }
        if d.join(FILE).is_file() {
            return Some(d.to_path_buf());
        }
        p = d.parent();
    }
    None
}

/// Every session under the root (folders holding session.toml; not below another session).
pub fn list(root: &Path) -> Vec<crate::proto::SessionEntry> {
    let mut out = Vec::new();
    walk(root, root, &mut out, 0);
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<crate::proto::SessionEntry>, depth: u32) {
    if depth > 6 {
        return;
    }
    let f = dir.join(FILE);
    if f.is_file() {
        let modified = std::fs::metadata(&f)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let s = SessionFile::load(dir).unwrap_or_default();
        out.push(crate::proto::SessionEntry {
            name: dir.strip_prefix(root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            modified,
            clients: s.clients.len(),
            notes: s.notes.lines().next().unwrap_or("").to_string(),
        });
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    dirs.sort();
    for d in dirs {
        walk(root, &d, out, depth + 1);
    }
}

/// A client id from an executable, readable like RaySession's: strip `ray-`/`non-`/`carla-`,
/// drop "jack", hyphenated words to initials, other characters to `_`, at most 10 characters,
/// `_2`, `_3`… when taken. `taken` holds the ids in use and the names of files in the folder.
pub fn client_id(exe: &str, taken: &[String]) -> String {
    let base = Path::new(exe).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let mut s = base.to_lowercase();
    for p in ["ray-", "non-", "carla-"] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.to_string();
        }
    }
    s = s.replace("jack", "");
    if s.contains('-') {
        let parts: Vec<&str> = s.split('-').filter(|p| !p.is_empty()).collect();
        if parts.len() > 1 {
            let (head, tail) = parts.split_at(1);
            s = format!("{}_{}", head[0], tail.iter().filter_map(|p| p.chars().next()).collect::<String>());
        }
    }
    let mut id: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    id = id.trim_matches('_').chars().take(10).collect();
    if id.is_empty() {
        id = "client".into();
    }
    let used = |c: &str| taken.iter().any(|t| t == c || t.ends_with(&format!(".{c}")));
    if !used(&id) {
        return id;
    }
    (2..).map(|n| format!("{id}_{n}")).find(|c| !used(c)).unwrap()
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(client_id("/usr/bin/ardour8", &[]), "ardour8");
        assert_eq!(client_id("carla-jack-single", &[]), "single");
        assert_eq!(client_id("ray-jackpatch", &[]), "patch");
        assert_eq!(client_id("non-mixer", &[]), "mixer");
        assert_eq!(client_id("sooperlooper-nsm-wrapper", &[]), "sooperloop_nw".chars().take(10).collect::<String>());
        assert_eq!(client_id("hydrogen", &["hydrogen".into()]), "hydrogen_2");
        assert_eq!(client_id("hydrogen", &["Hydrogen.hydrogen".into()]), "hydrogen_2");
    }

    #[test]
    fn names() {
        assert!(valid_name("song").is_ok());
        assert!(valid_name("live/gig 1").is_ok());
        assert!(valid_name("../x").is_err());
        assert!(valid_name("/abs").is_err());
        assert!(valid_name("a//b").is_err());
        assert!(valid_name(".hidden").is_err());
        assert!(valid_name("  ").is_err());
    }

    #[test]
    fn file_roundtrip() {
        let mut s = SessionFile { name: "x".into(), ..Default::default() };
        s.clients.push(ClientEntry { id: "a".into(), exe: "ardour8".into(), nsm: true, autostart: true, ..Default::default() });
        s.connections.list.push(("system:capture_1".into(), "a:in".into()));
        let text = toml::to_string_pretty(&s).unwrap();
        let back: SessionFile = toml::from_str(&text).unwrap();
        assert_eq!(back.clients, s.clients);
        assert_eq!(back.connections, s.connections);
    }
}

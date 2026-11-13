//! What PatchBox remembers about the canvas: node and matrix positions, pinned matrices, split and
//! hidden clients, the view. `~/.config/patchbox/layout.json`, written atomically after changes.
//! (Per-session layouts come with sessions; this is the global layer.)

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct View {
    /// World point at the canvas's top-left, and zoom.
    pub cam: [f32; 2],
    pub zoom: f32,
    /// Port type shown: AUDIO, MIDI, CV, OSC.
    pub kind: String,
    pub sidebar: bool,
    /// Port names drawn around every matrix (not only the hovered one).
    pub labels: bool,
    /// Interface scale for the window's contents (top bar, sidebar, canvas at zoom 1), on top of
    /// the desktop's scale. The window buttons follow the desktop's scale only.
    pub ui_scale: f32,
}

impl Default for View {
    fn default() -> View {
        View { cam: [0.0, 0.0], zoom: 1.0, kind: "AUDIO".into(), sidebar: true, labels: false, ui_scale: 1.25 }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Layout {
    pub version: u32,
    /// Node id ("M:client", "S:system", "K:system") -> centre, world coordinates.
    pub nodes: BTreeMap<String, [f32; 2]>,
    /// Matrix key ("<src id>><dst id>") -> centre.
    pub matrices: BTreeMap<String, [f32; 2]>,
    /// "<KIND>|<matrix key>": matrices kept on the canvas without connections.
    pub pinned: BTreeSet<String>,
    /// Client (group) -> split into source/sink nodes (default: physical ones are).
    pub split: BTreeMap<String, bool>,
    /// Node ids not drawn.
    pub hidden: BTreeSet<String>,
    /// Node ids showing their port list.
    pub expanded: BTreeSet<String>,
    pub view: View,
    /// Window size in points.
    pub window: Option<[u32; 2]>,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("PATCHBOX_CONFIG_DIR").map(PathBuf::from).or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("patchbox"))
    })?;
    Some(base.join("layout.json"))
}

impl Layout {
    pub fn load() -> Layout {
        let Some(p) = path() else { return Layout::default() };
        match std::fs::read(&p) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                eprintln!("[layout] {}: {e}; starting fresh", p.display());
                Layout::default()
            }),
            Err(_) => Layout::default(),
        }
    }

    pub fn save(&mut self) {
        self.version = 1;
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("json.tmp");
        let Ok(body) = serde_json::to_vec_pretty(self) else { return };
        if std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &p)).is_err() {
            eprintln!("[layout] could not write {}", p.display());
        }
    }
}

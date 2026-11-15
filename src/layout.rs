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
    /// The canvas grid is drawn.
    pub grid: bool,
    /// PatchMatrix's matrices between clients, one wire per client pair. Off (the default):
    /// ports on the blocks and a wire per connection.
    pub matrices: bool,
    /// Ports view only: signal flows top to bottom (Luluda's vertical layout), inputs along a
    /// block's top edge, outputs along its bottom.
    pub vertical: bool,
    /// Moving a block or a matrix lands its top-left corner on the grid.
    pub snap: bool,
    /// A moved block pushes the ones it lands on out of the way (RaySession's default).
    pub no_overlap: bool,
    /// Interface scale for the window's contents (top bar, sidebar, canvas at zoom 1), on top of
    /// the desktop's scale. The window buttons follow the desktop's scale only.
    pub ui_scale: f32,
    /// Our names for blocks and ports go to JACK's metadata (pretty-name) for other programs.
    pub export_names: bool,
    /// The camera (cam, zoom) each view style was last left at, so going back to one returns
    /// to where it was.
    pub style_cams: BTreeMap<String, ([f32; 2], f32)>,
}

impl View {
    /// The view style: "ports" (horizontal flow), "vertical" (ports, top to bottom), "matrix".
    pub fn style(&self) -> &'static str {
        if self.matrices {
            "matrix"
        } else if self.vertical {
            "vertical"
        } else {
            "ports"
        }
    }
}

impl Default for View {
    fn default() -> View {
        View { cam: [0.0, 0.0], zoom: 1.0, kind: "AUDIO".into(), sidebar: true, labels: false, grid: true, matrices: false, vertical: false, snap: false, no_overlap: true, ui_scale: 1.25, export_names: true, style_cams: BTreeMap::new() }
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
    /// Names given by the user: node id -> name, "port:<full port name>" -> name.
    pub names: BTreeMap<String, String>,
    /// Port groups made by the user: a name and its ports (full names, in order). They connect
    /// as one, 1:1; a group of two is a stereo pair.
    pub port_groups: Vec<PortGroup>,
    /// Clients shown as a block per track or plugin (Ardour's "Track/port", Carla's
    /// "Plugin:port").
    pub tracks: BTreeSet<String>,
    pub view: View,
    /// Window size in points.
    pub window: Option<[u32; 2]>,
    /// Saved views: name -> what the canvas looked like.
    pub views: BTreeMap<String, SavedView>,
    /// Block positions of the view styles not in `nodes` ("ports", "vertical", "matrix"): each
    /// style keeps its own arrangement (a matrix layout and a ports layout don't suit each
    /// other).
    pub style_nodes: BTreeMap<String, BTreeMap<String, [f32; 2]>>,
    /// The style `nodes` belongs to. The view is global and a canvas (a session's, the one
    /// before an undo) may have been left in another style: `settle` swaps the right positions
    /// in. Empty in files from before styles kept their own: `nodes` is then its view's.
    pub nodes_style: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct PortGroup {
    pub name: String,
    pub ports: Vec<String>,
}

/// A saved view: positions, hidden and expanded blocks, the view style and type, the camera.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct SavedView {
    pub nodes: BTreeMap<String, [f32; 2]>,
    pub matrices: BTreeMap<String, [f32; 2]>,
    pub hidden: BTreeSet<String>,
    pub expanded: BTreeSet<String>,
    pub kind: String,
    pub cam: [f32; 2],
    pub zoom: f32,
    pub matrix_view: bool,
    pub vertical: bool,
}

/// PatchBox's configuration directory: PATCHBOX_CONFIG_DIR, else ~/.config/patchbox.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("PATCHBOX_CONFIG_DIR").map(PathBuf::from).or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("patchbox"))
    })
}

fn path() -> Option<PathBuf> {
    Some(config_dir()?.join("layout.json"))
}

/// The file of a session's canvas, in its folder.
pub const SESSION_FILE: &str = "patchbox-layout.json";

impl Layout {
    /// A session's canvas (positions, pins, splits, hidden and expanded nodes), if it has one.
    pub fn load_session(dir: &std::path::Path) -> Option<Layout> {
        let b = std::fs::read(dir.join(SESSION_FILE)).ok()?;
        let mut l: Layout = serde_json::from_slice(&b).ok()?;
        l.settle();
        Some(l)
    }

    /// Make `nodes` the positions of the view's style: put the ones there aside under their
    /// style and take the view's back (none yet: empty, to be arranged).
    pub fn settle(&mut self) {
        let want = self.view.style();
        if self.nodes_style.is_empty() {
            self.nodes_style = want.to_string();
        }
        if self.nodes_style != want {
            let had = std::mem::replace(&mut self.nodes_style, want.to_string());
            let nodes = std::mem::take(&mut self.nodes);
            if !nodes.is_empty() {
                self.style_nodes.insert(had, nodes);
            }
            self.nodes = self.style_nodes.remove(want).unwrap_or_default();
        }
    }

    /// Write the canvas into a session folder.
    pub fn save_session(&mut self, dir: &std::path::Path) {
        self.version = 1;
        self.view.style_cams = self.cams();
        let p = dir.join(SESSION_FILE);
        let tmp = dir.join(".patchbox-layout.json.tmp");
        let Ok(body) = serde_json::to_vec_pretty(self) else { return };
        if std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &p)).is_err() {
            eprintln!("[layout] could not write {}", p.display());
        }
    }

    /// This canvas's cameras, one per view style, the one shown included (it lives in cam/zoom
    /// while shown). Every canvas has its own: JACK's, PipeWire's, each session's; the rest of the
    /// view (style, grid, scale…) is shared.
    pub fn cams(&self) -> BTreeMap<String, ([f32; 2], f32)> {
        let mut m = self.view.style_cams.clone();
        m.insert(self.view.style().to_string(), (self.view.cam, self.view.zoom));
        m
    }

    /// Take `cams` as this canvas's cameras and show the current style's. False: none for it yet
    /// (the caller fits the graph).
    pub fn set_cams(&mut self, cams: BTreeMap<String, ([f32; 2], f32)>) -> bool {
        self.view.style_cams = cams;
        match self.view.style_cams.get(self.view.style()).copied() {
            Some((c, z)) => {
                self.view.cam = c;
                self.view.zoom = z;
                true
            }
            None => false,
        }
    }

    /// `other`'s canvas and cameras with this view's shared settings (style, grid, scale…).
    pub fn with_view_of(mut other: Layout, view: &View) -> (Layout, bool) {
        let cams = other.cams();
        other.view = view.clone();
        let has = other.set_cams(cams);
        (other, has)
    }

    /// As with_view_of, but `other` keeps its own view style (ports, vertical or matrices) and
    /// the camera it had for it: JACK's canvas set aside while PipeWire's graph is shown.
    pub fn with_view_keeping_style(mut other: Layout, view: &View) -> Layout {
        let cams = other.cams();
        let style = (other.view.matrices, other.view.vertical);
        other.view = view.clone();
        (other.view.matrices, other.view.vertical) = style;
        other.set_cams(cams);
        other
    }

    /// Take another graph's canvas (JACK's or PipeWire's) with its own view style (ports,
    /// vertical or matrices); the rest of the view and the window stay ours. The style is set
    /// first: settle files the block positions under it.
    pub fn take_graph_canvas(&mut self, other: Layout) {
        (self.view.matrices, self.view.vertical) = (other.view.matrices, other.view.vertical);
        self.take_canvas(other);
    }

    /// Take the canvas of `other`, keeping our view and window (those stay global).
    pub fn take_canvas(&mut self, other: Layout) {
        let (view, window) = (self.view.clone(), self.window);
        let mut other = other;
        other.settle();
        *self = other;
        self.view = view;
        self.window = window;
        self.settle();
    }

    /// PipeWire's canvas (the drill-down), next to the JACK one.
    pub fn load_pw() -> Layout {
        let Some(p) = path().map(|p| p.with_file_name("layout-pw.json")) else { return Layout::default() };
        let mut l: Layout = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        l.settle();
        l
    }

    pub fn save_pw(&mut self) {
        self.version = 1;
        self.view.style_cams = self.cams();
        let Some(p) = path().map(|p| p.with_file_name("layout-pw.json")) else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("json.tmp");
        if let Ok(body) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &p));
        }
    }

    pub fn load() -> Layout {
        let Some(p) = path() else { return Layout::default() };
        let mut l = match std::fs::read(&p) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                eprintln!("[layout] {}: {e}; starting fresh", p.display());
                Layout::default()
            }),
            Err(_) => Layout::default(),
        };
        l.settle();
        l
    }

    pub fn save(&mut self) {
        self.version = 1;
        self.view.style_cams = self.cams();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> BTreeMap<String, [f32; 2]> {
        BTreeMap::from([("M:a".to_string(), [x, 0.0])])
    }

    /// A canvas saved in one style and opened while the view shows another gets each style's
    /// own positions; a round trip through the file keeps all of them.
    #[test]
    fn styles_keep_their_positions() {
        let mut saved = Layout::default();
        saved.nodes = at(1.0);
        saved.settle();
        assert_eq!(saved.nodes_style, "ports");
        saved.view.matrices = true;
        saved.settle();
        assert!(saved.nodes.is_empty());
        saved.nodes = at(2.0);
        let body = serde_json::to_vec(&saved).unwrap();

        // The global view is in the vertical style.
        let mut cur = Layout::default();
        cur.view.vertical = true;
        cur.settle();
        cur.nodes = at(3.0);
        let mut back: Layout = serde_json::from_slice(&body).unwrap();
        back.settle();
        cur.take_canvas(back);
        assert_eq!(cur.nodes_style, "vertical");
        assert!(cur.nodes.is_empty(), "the session had no vertical arrangement");
        cur.view.vertical = false;
        cur.settle();
        assert_eq!(cur.nodes, at(1.0));
        cur.view.matrices = true;
        cur.settle();
        assert_eq!(cur.nodes, at(2.0));
        cur.view.matrices = false;
        cur.settle();
        assert_eq!(cur.nodes, at(1.0));
    }

    /// Files from before the styles kept their own positions: `nodes` are their view's.
    #[test]
    fn old_files_keep_their_nodes() {
        let mut l: Layout = serde_json::from_str(r#"{"nodes":{"M:a":[5.0,0.0]},"view":{"matrices":true}}"#).unwrap();
        l.settle();
        assert_eq!(l.nodes_style, "matrix");
        assert_eq!(l.nodes, at(5.0));
    }
}

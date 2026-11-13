//! Application state: the JACK client, the graph and model, the layout, and the actions every
//! control maps to. Drawing and input live in canvas.rs and ui.rs.

use crate::csd::Button;
use crate::gfx::Rect;
use crate::layout::Layout;
use crate::model::{Matrix, Model, Role, pin_id};
use crate::theme;
use pbx_jack::{Client, Event, Graph, PortKind};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub xruns: u32,
    pub dsp: f32,
    pub realtime: bool,
    pub freewheel: bool,
}

#[derive(Clone, Debug)]
pub enum Action {
    SetKind(PortKind),
    ToggleSidebar,
    ToggleLabels,
    BufferMenu(Rect),
    SetBufferSize(u32),
    ResetXruns,
    ToggleFreewheel,
    /// Split (true) or join (false) a client's nodes.
    Split(String, bool),
    Hide(String),
    Show(String),
    ShowAll,
    Arrange,
    CenterNode(String),
    DisconnectMatrix(String),
    ConnectDiagonal(String),
    Unpin(String),
    DisconnectNode(String),
    ResetZoom,
    FitAll,
    Window(Button),
    AppMenu(Rect),
    Quit,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum WindowCmd {
    Minimize,
    ToggleMaximize,
}

/// A transient message for the sidebar's log.
pub struct LogLine {
    pub at: Instant,
    pub text: String,
    pub bad: bool,
}

pub struct App {
    pub jack: Option<Client>,
    pub waker: pbx_jack::Waker,
    pub graph: Graph,
    pub model: Model,
    pub mats: Vec<Matrix>,
    pub kind: PortKind,
    pub layout: Layout,
    layout_dirty: Option<Instant>,
    pub stats: Stats,
    /// Pixels per point.
    pub scale: f32,
    /// Window size, pixels.
    pub win: [f32; 2],
    pub maximized: bool,
    pub canvas: crate::canvas::CanvasState,
    pub ui: crate::ui::UiState,
    pub log: Vec<LogLine>,
    pub redraw: bool,
    graph_due: Option<Instant>,
    graph_first: Option<Instant>,
    retry_at: Instant,
    retry_logged: bool,
    dsp_at: Instant,
    pub quit: bool,
    pub window_cmd: Option<WindowCmd>,
    /// Fit the view to the graph at the next frame (the canvas size is known then).
    pub fit_pending: bool,
}

/// Snapshot the graph this long after the last change (a client registering 50 ports sends 50
/// events), but no later than GRAPH_MAX after the first.
const GRAPH_QUIET: Duration = Duration::from_millis(25);
const GRAPH_MAX: Duration = Duration::from_millis(150);
const RETRY: Duration = Duration::from_secs(2);
const DSP_POLL: Duration = Duration::from_millis(1000);
const LAYOUT_SAVE: Duration = Duration::from_millis(800);

impl App {
    pub fn new(waker: pbx_jack::Waker) -> App {
        let layout = Layout::load();
        let kind = kind_from(&layout.view.kind);
        App {
            jack: None,
            waker,
            graph: Graph::default(),
            model: Model::default(),
            mats: Vec::new(),
            kind,
            layout,
            layout_dirty: None,
            stats: Stats::default(),
            scale: 1.0,
            win: [1280.0, 760.0],
            maximized: false,
            canvas: Default::default(),
            ui: crate::ui::UiState::new(),
            log: Vec::new(),
            redraw: true,
            graph_due: None,
            graph_first: None,
            retry_at: Instant::now(),
            retry_logged: false,
            dsp_at: Instant::now(),
            quit: false,
            window_cmd: None,
            fit_pending: false,
        }
    }

    pub fn say(&mut self, text: impl Into<String>, bad: bool) {
        let text = text.into();
        eprintln!("[patchbox] {text}");
        self.log.push(LogLine { at: Instant::now(), text, bad });
        if self.log.len() > 50 {
            self.log.remove(0);
        }
        self.redraw = true;
    }

    // ---- JACK ----

    fn try_connect(&mut self) {
        match Client::open("PatchBox", self.waker.clone()) {
            Ok(c) => {
                self.stats = Stats {
                    sample_rate: c.sample_rate(),
                    buffer_size: c.buffer_size(),
                    xruns: 0,
                    dsp: c.cpu_load(),
                    realtime: c.is_realtime(),
                    freewheel: false,
                };
                let msg = format!(
                    "Connected to JACK as \"{}\" ({} Hz, {} frames{})",
                    c.name(),
                    self.stats.sample_rate,
                    self.stats.buffer_size,
                    if self.stats.realtime { ", realtime" } else { "" }
                );
                self.jack = Some(c);
                self.retry_logged = false;
                self.say(msg, false);
                self.rebuild();
            }
            Err(e) => {
                if !self.retry_logged {
                    self.say(format!("{e}; retrying every {} s", RETRY.as_secs()), true);
                    self.retry_logged = true;
                }
                self.retry_at = Instant::now() + RETRY;
            }
        }
    }

    /// The JACK notification thread woke us: drain its events.
    pub fn on_jack_events(&mut self) {
        let Some(c) = &self.jack else { return };
        let events = c.take_events();
        let now = Instant::now();
        let mut lost = None;
        for e in events {
            match &e {
                Event::Xrun => self.stats.xruns += 1,
                Event::BufferSize(n) => self.stats.buffer_size = *n,
                Event::SampleRate(n) => self.stats.sample_rate = *n,
                Event::Freewheel(on) => self.stats.freewheel = *on,
                Event::ClientRegistered { name, registered: false } => c.forget_client(name),
                Event::Shutdown { reason } => lost = Some(reason.clone()),
                _ => {}
            }
            if e.changes_graph() {
                self.graph_first.get_or_insert(now);
                self.graph_due = Some(now + GRAPH_QUIET);
            }
            self.redraw = true;
        }
        if let Some(reason) = lost {
            self.jack = None;
            self.graph = Graph::default();
            self.model = Model::default();
            self.mats.clear();
            self.graph_due = None;
            self.graph_first = None;
            self.canvas.drag = crate::canvas::Drag::None;
            let r = if reason.is_empty() { "server shut down".to_string() } else { reason };
            self.say(format!("JACK went away: {r}"), true);
            self.retry_at = Instant::now() + Duration::from_millis(500);
        }
    }

    /// Timers. Returns when to be called next (None: only on events).
    pub fn tick(&mut self) -> Option<Instant> {
        let now = Instant::now();
        let mut next: Option<Instant> = None;
        let mut at = |t: Instant| next = Some(next.map_or(t, |n: Instant| n.min(t)));
        if self.jack.is_none() {
            if now >= self.retry_at {
                self.try_connect();
            }
            if self.jack.is_none() {
                at(self.retry_at);
            }
        }
        if let Some(due) = self.graph_due {
            let cap = self.graph_first.map(|f| f + GRAPH_MAX).unwrap_or(due);
            if now >= due || now >= cap {
                self.graph_due = None;
                self.graph_first = None;
                self.rebuild();
            } else {
                at(due.min(cap));
            }
        }
        if let Some(c) = &self.jack {
            if now >= self.dsp_at {
                let d = c.cpu_load();
                if (d - self.stats.dsp).abs() >= 0.5 {
                    self.redraw = true;
                }
                self.stats.dsp = d;
                self.dsp_at = now + DSP_POLL;
            }
            at(self.dsp_at);
        }
        if let Some(t) = self.layout_dirty {
            if now >= t + LAYOUT_SAVE {
                self.save_layout();
            } else {
                at(t + LAYOUT_SAVE);
            }
        }
        next
    }

    pub fn save_layout(&mut self) {
        self.layout.view.kind = self.kind.label().to_string();
        self.layout.save();
        self.layout_dirty = None;
    }

    pub fn layout_changed(&mut self) {
        self.layout_dirty = Some(Instant::now());
        self.redraw = true;
    }

    /// Re-read the graph and rebuild what is shown.
    pub fn rebuild(&mut self) {
        let Some(c) = &self.jack else { return };
        let own = c.name().to_string();
        self.graph = c.snapshot();
        self.model = Model::build(&self.graph, &self.layout, &own);
        let first = self.layout.nodes.is_empty();
        self.place_new_nodes(false);
        self.refresh_matrices();
        if first && !self.model.nodes.is_empty() {
            self.fit_pending = true;
        }
        self.redraw = true;
    }

    pub fn refresh_matrices(&mut self) {
        self.mats = self.model.matrices(&self.graph, self.kind, &self.layout);
        self.place_new_matrices();
        self.redraw = true;
    }

    pub fn connect(&mut self, out: usize, inp: usize) {
        let (Some(c), Some(o), Some(i)) = (&self.jack, self.graph.ports.get(out), self.graph.ports.get(inp)) else { return };
        let (o, i) = (o.name.clone(), i.name.clone());
        if !c.connect(&o, &i) {
            self.say(format!("Could not connect {o} → {i}"), true);
        }
    }

    pub fn disconnect(&mut self, out: usize, inp: usize) {
        let (Some(c), Some(o), Some(i)) = (&self.jack, self.graph.ports.get(out), self.graph.ports.get(inp)) else { return };
        let (o, i) = (o.name.clone(), i.name.clone());
        if !c.disconnect(&o, &i) {
            self.say(format!("Could not disconnect {o} → {i}"), true);
        }
    }

    // ---- placement ----

    /// Columns by signal-chain depth: capture nodes left, playback right, the rest by the longest
    /// path from a capture node (a short form of RaySession's "follow signal chain"). Cycles
    /// (feedback, a joined hardware node) are cut at their back edges, found by a depth-first walk
    /// that starts from the sources.
    fn depths(&self) -> Vec<usize> {
        let n = self.model.nodes.len();
        let role = |i: usize| self.model.nodes[i].key.role;
        let mut edges: Vec<(usize, usize)> = self
            .graph
            .conns
            .iter()
            .filter_map(|(o, i)| Some((*self.model.port_node.get(o)?, *self.model.port_node.get(i)?)))
            .filter(|(a, b)| a != b)
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mut adj = vec![Vec::new(); n];
        let mut indeg = vec![0usize; n];
        for &(a, b) in &edges {
            adj[a].push(b);
            indeg[b] += 1;
        }
        // Roots first: capture nodes, hardware joined into one node, then by fewest inputs.
        let mut starts: Vec<usize> = (0..n).collect();
        starts.sort_by_key(|&i| (role(i) != Role::Source, !self.model.nodes[i].physical, indeg[i], i));
        let mut state = vec![0u8; n]; // 0 new, 1 on the walk's stack, 2 done
        let mut post = Vec::with_capacity(n);
        let mut back: std::collections::HashSet<(usize, usize)> = Default::default();
        for &s0 in &starts {
            if state[s0] != 0 {
                continue;
            }
            let mut stack = vec![(s0, 0usize)];
            state[s0] = 1;
            while let Some(&mut (v, ref mut k)) = stack.last_mut() {
                if *k < adj[v].len() {
                    let w = adj[v][*k];
                    *k += 1;
                    match state[w] {
                        0 => {
                            state[w] = 1;
                            stack.push((w, 0));
                        }
                        1 => {
                            back.insert((v, w));
                        }
                        _ => {}
                    }
                } else {
                    state[v] = 2;
                    post.push(v);
                    stack.pop();
                }
            }
        }
        let mut d = vec![0usize; n];
        for i in 0..n {
            d[i] = match role(i) {
                Role::Source => 0,
                _ if self.model.nodes[i].physical => 0,
                _ => 1,
            };
        }
        for &v in post.iter().rev() {
            for &w in &adj[v] {
                if !back.contains(&(v, w)) && role(w) == Role::Main && !self.model.nodes[w].physical {
                    d[w] = d[w].max(d[v] + 1);
                }
            }
        }
        let max_main = (0..n).filter(|&i| role(i) == Role::Main).map(|i| d[i]).max().unwrap_or(1).max(1);
        for i in 0..n {
            if role(i) == Role::Sink {
                d[i] = max_main + 1;
            }
        }
        d
    }

    /// Give nodes without a saved position one (`all`: re-place every node).
    pub fn place_new_nodes(&mut self, all: bool) {
        if all {
            self.layout.nodes.clear();
            self.layout.matrices.clear();
        }
        let missing: Vec<usize> = (0..self.model.nodes.len()).filter(|&i| !self.layout.nodes.contains_key(&self.model.nodes[i].key.id())).collect();
        if missing.is_empty() {
            return;
        }
        let depths = self.depths();
        const COL_W: f32 = 460.0;
        const X0: f32 = 160.0;
        let gap = theme::NODE_H + 48.0;
        let mut bottom: HashMap<usize, f32> = HashMap::new();
        for (i, node) in self.model.nodes.iter().enumerate() {
            if let Some(p) = self.layout.nodes.get(&node.key.id()) {
                let col = depths[i];
                let b = bottom.entry(col).or_insert(f32::MIN);
                *b = b.max(p[1]);
            }
        }
        let mut order = missing;
        order.sort_by_key(|&i| (depths[i], self.model.nodes[i].title.to_lowercase()));
        for i in order {
            let col = depths[i];
            let y = bottom.get(&col).map_or(80.0, |b| b + gap);
            bottom.insert(col, y);
            let x = X0 + col as f32 * COL_W;
            self.layout.nodes.insert(self.model.nodes[i].key.id(), [x, y]);
        }
        self.layout_changed();
    }

    pub fn node_pos(&self, i: usize) -> [f32; 2] {
        self.layout.nodes.get(&self.model.nodes[i].key.id()).copied().unwrap_or([0.0, 0.0])
    }

    /// World positions of a node's output and input handles.
    pub fn handles(&self, i: usize) -> ([f32; 2], [f32; 2]) {
        let p = self.node_pos(i);
        let off = theme::NODE_W / 2.0 + 2.0 * theme::HANDLE_R;
        ([p[0] + off, p[1]], [p[0] - off, p[1]])
    }

    pub fn matrix_size(m: &Matrix) -> [f32; 2] {
        [m.cols.len() as f32 * theme::CELL, m.rows.len() as f32 * theme::CELL]
    }

    fn default_matrix_pos(&self, m: &Matrix) -> [f32; 2] {
        let (out, _) = self.handles(m.src);
        let (_, inp) = self.handles(m.dst);
        let s = Self::matrix_size(m);
        if inp[0] > out[0] + s[0] + 60.0 {
            // Signal flows right: in the gap between the two columns.
            [(out[0] + inp[0]) / 2.0, (out[1] + inp[1]) / 2.0]
        } else {
            // Backwards or onto itself: below and right of the source.
            [out[0] + s[0] / 2.0 + 50.0, out[1].max(inp[1]) + s[1] / 2.0 + 40.0]
        }
    }

    /// World rectangles nodes take (with handles and the small label above).
    fn node_obstacles(&self) -> Vec<Rect> {
        (0..self.model.nodes.len())
            .filter(|&i| !self.layout.hidden.contains(&self.model.nodes[i].key.id()))
            .map(|i| {
                let p = self.node_pos(i);
                Rect::centered(p[0], p[1] - 5.0, theme::NODE_W + 6.0 * theme::HANDLE_R + 12.0, theme::NODE_H + 30.0)
            })
            .collect()
    }

    fn place_new_matrices(&mut self) {
        let mut placed: Vec<Rect> = self
            .mats
            .iter()
            .filter_map(|m| {
                let p = self.layout.matrices.get(&m.key)?;
                let s = Self::matrix_size(m);
                Some(Rect::centered(p[0], p[1], s[0], s[1]))
            })
            .collect();
        let obstacles = self.node_obstacles();
        let mut changed = false;
        for mi in 0..self.mats.len() {
            if self.layout.matrices.contains_key(&self.mats[mi].key) {
                continue;
            }
            let s = Self::matrix_size(&self.mats[mi]);
            let p0 = self.default_matrix_pos(&self.mats[mi]);
            // Nearest free spot along the column: 0, +d, -d, +2d, -2d, ...
            let mut p = p0;
            for k in 0..120 {
                let off = ((k + 1) / 2) as f32 * 16.0 * if k % 2 == 1 { 1.0 } else { -1.0 };
                let cand = [p0[0], p0[1] + off];
                let r = Rect::centered(cand[0], cand[1], s[0] + 20.0, s[1] + 20.0);
                if !placed.iter().chain(obstacles.iter()).any(|o| o.overlaps(&r)) {
                    p = cand;
                    break;
                }
            }
            placed.push(Rect::centered(p[0], p[1], s[0], s[1]));
            self.layout.matrices.insert(self.mats[mi].key.clone(), p);
            changed = true;
        }
        if changed {
            self.layout_changed();
        }
    }

    pub fn matrix_pos(&self, m: &Matrix) -> [f32; 2] {
        self.layout.matrices.get(&m.key).copied().unwrap_or_else(|| self.default_matrix_pos(m))
    }

    /// Pin a matrix between two nodes for the current kind (it stays without connections).
    pub fn pin(&mut self, src: usize, dst: usize) -> String {
        let key = format!("{}>{}", self.model.nodes[src].key.id(), self.model.nodes[dst].key.id());
        self.layout.pinned.insert(pin_id(self.kind, &key));
        self.layout_changed();
        key
    }

    // ---- actions ----

    pub fn apply(&mut self, a: Action) {
        self.redraw = true;
        match a {
            Action::SetKind(k) => {
                self.kind = k;
                self.canvas.drag = crate::canvas::Drag::None;
                self.refresh_matrices();
                self.layout_changed();
            }
            Action::ToggleSidebar => {
                self.layout.view.sidebar = !self.layout.view.sidebar;
                self.layout_changed();
            }
            Action::ToggleLabels => {
                self.layout.view.labels = !self.layout.view.labels;
                self.layout_changed();
            }
            Action::BufferMenu(r) => self.ui.open_buffer_menu(r, self.stats.buffer_size),
            Action::AppMenu(r) => {
                let labels = self.layout.view.labels;
                let sidebar = self.layout.view.sidebar;
                self.ui.open_app_menu(r, labels, sidebar)
            }
            Action::SetBufferSize(n) => {
                if let Some(c) = &self.jack
                    && !c.set_buffer_size(n) {
                        self.say(format!("JACK refused a buffer size of {n}"), true);
                    }
            }
            Action::ResetXruns => self.stats.xruns = 0,
            Action::ToggleFreewheel => {
                if let Some(c) = &self.jack {
                    c.set_freewheel(!self.stats.freewheel);
                }
            }
            Action::Split(group, on) => {
                self.layout.split.insert(group, on);
                self.layout_changed();
                self.rebuild();
            }
            Action::Hide(id) => {
                self.layout.hidden.insert(id);
                self.layout_changed();
                self.refresh_matrices();
            }
            Action::Show(id) => {
                self.layout.hidden.remove(&id);
                self.layout_changed();
                self.refresh_matrices();
            }
            Action::ShowAll => {
                self.layout.hidden.clear();
                self.layout_changed();
                self.refresh_matrices();
            }
            Action::Arrange => {
                self.place_new_nodes(true);
                self.refresh_matrices();
                self.fit_pending = true;
            }
            Action::CenterNode(id) => {
                if let Some(i) = self.model.node_by_id(&id) {
                    self.layout.hidden.remove(&id);
                    self.center_on(self.node_pos(i));
                    self.canvas.flash = Some((id, Instant::now()));
                    self.refresh_matrices();
                }
            }
            Action::DisconnectMatrix(key) => {
                if let Some(m) = self.mats.iter().find(|m| m.key == key) {
                    let conns: Vec<(usize, usize)> = m.conns.iter().copied().collect();
                    for (o, i) in conns {
                        self.disconnect(o, i);
                    }
                }
            }
            Action::ConnectDiagonal(key) => {
                if let Some(m) = self.mats.iter().find(|m| m.key == key) {
                    let pairs: Vec<(usize, usize)> = m.cols.iter().copied().zip(m.rows.iter().copied()).collect();
                    for (o, i) in pairs {
                        self.connect(o, i);
                    }
                }
            }
            Action::Unpin(key) => {
                self.layout.pinned.remove(&pin_id(self.kind, &key));
                self.layout_changed();
                self.refresh_matrices();
            }
            Action::DisconnectNode(id) => {
                if let Some(n) = self.model.node_by_id(&id) {
                    let conns: Vec<(usize, usize)> = self
                        .graph
                        .conns
                        .iter()
                        .copied()
                        .filter(|(o, i)| self.model.port_node.get(o) == Some(&n) || self.model.port_node.get(i) == Some(&n))
                        .filter(|(o, _)| self.graph.ports[*o].kind == self.kind)
                        .collect();
                    for (o, i) in conns {
                        self.disconnect(o, i);
                    }
                }
            }
            Action::ResetZoom => {
                let c = self.canvas_center_world();
                self.layout.view.zoom = 1.0;
                self.center_on(c);
            }
            Action::FitAll => self.fit_all(),
            Action::Window(b) => match b {
                Button::Close => self.quit = true,
                Button::Minimize => self.window_cmd = Some(WindowCmd::Minimize),
                Button::Maximize => self.window_cmd = Some(WindowCmd::ToggleMaximize),
            },
            Action::Quit => self.quit = true,
        }
    }

    // ---- view ----

    pub fn z(&self) -> f32 {
        self.layout.view.zoom * self.scale
    }

    pub fn canvas_center_world(&self) -> [f32; 2] {
        let a = self.canvas.area;
        let z = self.z();
        [self.layout.view.cam[0] + a.w / 2.0 / z, self.layout.view.cam[1] + a.h / 2.0 / z]
    }

    pub fn center_on(&mut self, p: [f32; 2]) {
        let a = self.canvas.area;
        let z = self.z();
        self.layout.view.cam = [p[0] - a.w / 2.0 / z, p[1] - a.h / 2.0 / z];
        self.layout_changed();
    }

    pub fn fit_all(&mut self) {
        let mut bb: Option<Rect> = None;
        let mut add = |r: Rect| {
            bb = Some(match bb {
                None => r,
                Some(b) => {
                    let x = b.x.min(r.x);
                    let y = b.y.min(r.y);
                    Rect::new(x, y, b.right().max(r.right()) - x, b.bottom().max(r.bottom()) - y)
                }
            })
        };
        for (i, n) in self.model.nodes.iter().enumerate() {
            if self.layout.hidden.contains(&n.key.id()) {
                continue;
            }
            let p = self.node_pos(i);
            add(Rect::centered(p[0], p[1], theme::NODE_W + 40.0, theme::NODE_H + 30.0));
        }
        for m in &self.mats {
            let p = self.matrix_pos(m);
            let s = Self::matrix_size(m);
            add(Rect::centered(p[0], p[1], s[0] + 20.0, s[1] + 20.0));
        }
        let Some(b) = bb else { return };
        let a = self.canvas.area;
        if a.w < 10.0 || a.h < 10.0 {
            return;
        }
        let zoom = ((a.w / self.scale) / (b.w + 80.0)).min((a.h / self.scale) / (b.h + 80.0)).clamp(0.3, 1.5);
        self.layout.view.zoom = zoom;
        self.center_on([b.cx(), b.cy()]);
    }
}

pub fn kind_from(s: &str) -> PortKind {
    match s {
        "MIDI" => PortKind::Midi,
        "CV" => PortKind::Cv,
        "OSC" => PortKind::Osc,
        _ => PortKind::Audio,
    }
}

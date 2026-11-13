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
    pub transport: pbx_jack::Transport,
}

#[derive(Clone, Debug)]
pub enum Action {
    SetKind(PortKind),
    ToggleSidebar,
    ToggleLabels,
    ToggleGrid,
    ToggleNoOverlap,
    /// Ports on the blocks with a wire per connection, or PatchMatrix's matrices.
    ToggleMatrices,
    ToggleVertical,
    /// Remove one connection (by port names).
    Disconnect(String, String),
    /// Copy (or cut) the connections of a port ("port:<name>") or a node (its id), current type.
    CopyConns(String, bool),
    /// Paste the copied connections onto a port or a node.
    PasteConns(String),
    ViewsPage,
    SaveView(String),
    RecallView(String),
    DeleteView(String),
    /// Recall the n-th saved view (Ctrl+1…9).
    RecallViewN(usize),
    ToggleSnap,
    BufferMenu(Rect),
    ScaleMenu(Rect),
    SetUiScale(f32),
    /// Multiply the zoom, about the canvas centre.
    ZoomBy(f32),
    ZoomTo(f32),
    /// The zoom slider's track (dragged in main.rs).
    ZoomSlider(Rect),
    SetBufferSize(u32),
    ResetXruns,
    ToggleFreewheel,
    /// Split (true) or join (false) a client's nodes.
    Split(String, bool),
    Hide(String),
    Show(String),
    /// Show (true) or fold (false) a node's port list.
    Expand(String, bool),
    /// Expand every node, or fold them all if any is expanded.
    ExpandAll,
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
    /// Play / pause.
    TransportToggle,
    /// Stop and go back to the start.
    TransportStop,
    TransportRewind,
    ServerStart,
    ServerStop,
    ServerRestart,
    ServerSettings,
    /// A choice picked from a dialog row's options: (row id, value).
    DialogSet(String, String),
    Undo,
    Redo,
    /// Show PipeWire's own graph (the drill-down into the PipeWire JACK client).
    OpenPipeWire,
    BackToJack,
    /// Rename a node (its id) or a port ("port:<full name>"): opens a dialog.
    Rename(String),
    /// Set (or, empty, clear) a user name: (layout key, name).
    SetName(String, String),
    /// Session manager actions (sessui.rs).
    Sess(crate::sessui::SessAction),
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
    /// Pixels per point for the window's contents: the desktop's scale × the interface scale.
    pub scale: f32,
    /// The desktop's scale alone: window decorations.
    pub chrome: f32,
    /// Window size, pixels.
    pub win: [f32; 2],
    pub maximized: bool,
    pub canvas: crate::canvas::CanvasState,
    pub ui: crate::ui::UiState,
    pub server: crate::server::Server,
    pub dialog: Option<crate::dialog::Dialog>,
    pub sess: crate::session::SessionLink,
    pub history: crate::undo::History,
    /// Copied connections: per input and output port (by position), the ports they reach.
    pub clip: Option<Clip>,
    /// Which graph the canvas shows.
    pub source: Source,
    pub pw: Option<crate::pw::PwLink>,
    pw_graph: Graph,
    /// The JACK canvas, put aside while PipeWire's is shown.
    jack_canvas: Option<Layout>,
    /// The open session's folder, whose canvas is in use (None: the global one).
    pub layout_dir: Option<std::path::PathBuf>,
    /// The global canvas, put aside while a session's is in use.
    global_canvas: Option<Layout>,
    /// Installed applications (.desktop), scanned when first needed.
    pub apps: Option<Vec<crate::session::App>>,
    pub log: Vec<LogLine>,
    pub redraw: bool,
    graph_due: Option<Instant>,
    graph_first: Option<Instant>,
    retry_at: Instant,
    retry_logged: bool,
    dsp_at: Instant,
    transport_at: Instant,
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
        let server = crate::server::Server::spawn(waker.clone());
        let sess = crate::session::SessionLink::new(waker.clone());
        App {
            server,
            dialog: None,
            sess,
            history: Default::default(),
            clip: None,
            source: Source::Jack,
            pw: None,
            pw_graph: Graph::default(),
            jack_canvas: None,
            layout_dir: None,
            global_canvas: None,
            apps: None,
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
            chrome: 1.0,
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
            transport_at: Instant::now(),
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
                    transport: c.transport(),
                };
                self.server.send(crate::server::Cmd::Refresh);
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

    /// A worker woke us: JACK notifications and jackdbus replies.
    pub fn on_wake(&mut self) {
        self.on_jack_events();
        self.on_daemon();
        self.on_pipewire();
        for r in self.server.take() {
            use crate::server::Reply;
            match r {
                Reply::Status(st) => {
                    if st.started != self.server.status.started || st.available != self.server.status.available {
                        self.redraw = true;
                        if st.started && self.jack.is_none() {
                            self.retry_at = Instant::now();
                        }
                    }
                    self.server.status = st;
                }
                Reply::Config(c) => self.server.config = Some(*c),
                Reply::Done(m) => self.say(m, false),
                Reply::Error(e) => self.say(e, true),
            }
            self.redraw = true;
        }
        if self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::Server) {
            let rows = self.server_rows();
            let b = self.server_buttons();
            if let Some(d) = &mut self.dialog {
                d.set_rows(rows);
                d.buttons = b;
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
            if self.source == Source::Jack {
                self.graph = Graph::default();
                self.model = Model::default();
                self.mats.clear();
            }
            self.graph_due = None;
            self.graph_first = None;
            self.canvas.drag = crate::canvas::Drag::None;
            self.server.send(crate::server::Cmd::Refresh);
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
        if let Some(m) = self.sess.tick() {
            self.say(m, false);
        }
        if let Some(t) = self.sess.retry_at() {
            at(t);
        }
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
            // The transport has no notification for a plain client (a sync callback would make
            // us a realtime client): poll it, often only while it rolls.
            if now >= self.transport_at {
                let t = c.transport();
                if t != self.stats.transport {
                    self.stats.transport = t;
                    self.redraw = true;
                }
                let every = if t.rolling || t.starting { 100 } else { 1000 };
                self.transport_at = now + Duration::from_millis(every);
            }
            at(self.transport_at);
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
        if self.source == Source::PipeWire {
            self.layout.save_pw();
            self.layout_dirty = None;
            return;
        }
        match &self.layout_dir {
            // A session's canvas goes with it; the view (and the canvas we left) stays global.
            Some(dir) => {
                if dir.is_dir() {
                    self.layout.save_session(dir);
                }
                let mut global = self.global_canvas.clone().unwrap_or_default();
                global.view = self.layout.view.clone();
                global.window = self.layout.window;
                global.save();
            }
            None => self.layout.save(),
        }
        self.layout_dirty = None;
    }

    /// The open session changed (or closed): switch the canvas to its own, keeping the view.
    pub fn session_dir_changed(&mut self, dir: Option<std::path::PathBuf>) {
        if dir == self.layout_dir {
            return;
        }
        // Keep what was arranged where it belongs.
        if self.layout_dirty.is_some() || self.layout_dir.is_some() {
            self.save_layout();
        }
        match (&self.layout_dir, &dir) {
            (None, Some(_)) => self.global_canvas = Some(self.layout.clone()),
            (Some(_), None) => {
                if let Some(g) = self.global_canvas.take() {
                    self.layout.take_canvas(g);
                }
            }
            _ => {}
        }
        if let Some(d) = &dir {
            // A session opened for the first time starts from the current arrangement.
            if let Some(l) = crate::layout::Layout::load_session(d) {
                self.layout.take_canvas(l);
            }
        }
        self.layout_dir = dir;
        if self.jack.is_some() {
            self.rebuild();
        }
        self.redraw = true;
    }

    pub fn layout_changed(&mut self) {
        self.layout_dirty = Some(Instant::now());
        self.redraw = true;
    }

    /// PipeWire's graph changed (or pw-dump ended).
    fn on_pipewire(&mut self) {
        let Some(pw) = &mut self.pw else { return };
        let (g, err, log) = pw.take();
        for (m, bad) in log {
            self.say(m, bad);
        }
        if let Some(g) = g {
            self.pw_graph = g;
            if self.source == Source::PipeWire {
                self.rebuild();
            }
        }
        if let Some(e) = err {
            self.say(format!("PipeWire: {e}"), true);
            self.pw = None;
            if self.source == Source::PipeWire {
                self.apply(Action::BackToJack);
            }
        }
    }

    fn view_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::{Control, Row};
        let mut rows = vec![Row { id: "name".into(), label: "Name for the current view".into(), help: String::new(), control: Control::Text(String::new()), modified: false }];
        rows.push(Row::header("Saved views"));
        if self.layout.views.is_empty() {
            rows.push(Row::info("None yet", "arrange the canvas, then save it here"));
        }
        for (k, (name, v)) in self.layout.views.iter().enumerate() {
            let what = format!(
                "{}{} · {} · {} blocks{}",
                if k < 9 { format!("Ctrl+{} · ", k + 1) } else { String::new() },
                if v.matrix_view { "matrices" } else if v.vertical { "vertical" } else { "ports" },
                v.kind,
                v.nodes.len(),
                if v.hidden.is_empty() { String::new() } else { format!(", {} hidden", v.hidden.len()) }
            );
            rows.push(Row { id: format!("view:{name}"), label: name.clone(), help: String::new(), control: Control::Link(what), modified: false });
        }
        if !self.layout.views.is_empty() {
            let mut opts = vec![(String::new(), "—".to_string())];
            opts.extend(self.layout.views.keys().map(|n| (n.clone(), n.clone())));
            rows.push(Row { id: "delete".into(), label: "Remove a view".into(), help: String::new(), control: Control::Choice { value: String::new(), options: opts, editable: false }, modified: false });
        }
        rows
    }

    fn refresh_views_page(&mut self) {
        if self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::Views) {
            let rows = self.view_rows();
            if let Some(d) = &mut self.dialog {
                d.set_rows(rows);
            }
        }
    }

    /// The ports a clipboard action is about: one port, or a node's ports of the current type.
    fn clip_ports(&self, target: &str) -> Vec<usize> {
        if let Some(p) = target.strip_prefix("port:") {
            return self.graph.by_name.get(p).copied().into_iter().collect();
        }
        let Some(n) = self.model.node_by_id(target) else { return Vec::new() };
        let node = &self.model.nodes[n];
        node.ins_of(&self.graph, self.kind).into_iter().chain(node.outs_of(&self.graph, self.kind)).collect()
    }

    /// The port types the current graph has tabs for.
    pub fn kinds(&self) -> Vec<PortKind> {
        let mut k = match self.source {
            Source::Jack => vec![PortKind::Audio, PortKind::Midi, PortKind::Cv, PortKind::Osc],
            Source::PipeWire => vec![PortKind::Audio, PortKind::Midi],
        };
        if self.graph.ports.iter().any(|p| p.kind == PortKind::Video) || self.source == Source::PipeWire {
            k.push(PortKind::Video);
        }
        k
    }

    /// Re-read the graph and rebuild what is shown.
    pub fn rebuild(&mut self) {
        if self.source == Source::PipeWire {
            self.graph = self.pw_graph.clone();
            self.model = Model::build(&self.graph, &self.layout, "");
            let first = self.layout.nodes.is_empty();
            self.place_new_nodes(false);
            self.refresh_matrices();
            if first && !self.model.nodes.is_empty() {
                self.fit_pending = true;
            }
            self.refresh_hover();
            self.redraw = true;
            return;
        }
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
        self.refresh_hover();
        self.redraw = true;
    }

    pub fn refresh_matrices(&mut self) {
        self.mats = self.model.matrices(&self.graph, self.kind, &self.layout);
        self.place_new_matrices();
        self.redraw = true;
    }

    pub fn connect(&mut self, out: usize, inp: usize) {
        let (Some(o), Some(i)) = (self.graph.ports.get(out), self.graph.ports.get(inp)) else { return };
        let (o, i) = (o.name.clone(), i.name.clone());
        self.connect_names(&o, &i);
    }

    pub fn connect_names(&mut self, o: &str, i: &str) {
        if self.source == Source::PipeWire {
            if let Some(pw) = &self.pw {
                pw.link(o, i, true);
                self.history.connected(o, i);
            }
            return;
        }
        let Some(c) = &self.jack else { return };
        if c.connect(o, i) {
            self.history.connected(o, i);
        } else {
            self.say(format!("Could not connect {o} → {i}"), true);
        }
    }

    pub fn disconnect_names(&mut self, o: &str, i: &str) {
        if self.source == Source::PipeWire {
            if let Some(pw) = &self.pw {
                pw.link(o, i, false);
                self.history.disconnected(o, i);
            }
            return;
        }
        let Some(c) = &self.jack else { return };
        if c.disconnect(o, i) {
            self.history.disconnected(o, i);
        } else {
            self.say(format!("Could not disconnect {o} → {i}"), true);
        }
    }

    pub fn undo(&mut self) {
        let Some(step) = self.history.pop_undo() else { return self.say("Nothing to undo", false) };
        self.history.replaying = true;
        for (o, i) in &step.made {
            self.disconnect_names(o, i);
        }
        for (o, i) in &step.removed {
            self.connect_names(o, i);
        }
        let after = step.canvas.as_ref().map(|before| {
            let now = self.layout.clone();
            self.layout.take_canvas(before.clone());
            now
        });
        self.history.replaying = false;
        self.history.push_redo(crate::undo::Step { made: step.made, removed: step.removed, canvas: after });
        self.after_history();
    }

    pub fn redo(&mut self) {
        let Some(step) = self.history.pop_redo() else { return self.say("Nothing to redo", false) };
        self.history.replaying = true;
        for (o, i) in &step.made {
            self.connect_names(o, i);
        }
        for (o, i) in &step.removed {
            self.disconnect_names(o, i);
        }
        let before = step.canvas.as_ref().map(|after| {
            let now = self.layout.clone();
            self.layout.take_canvas(after.clone());
            now
        });
        self.history.replaying = false;
        self.history.push_undo(crate::undo::Step { made: step.made, removed: step.removed, canvas: before });
        self.after_history();
    }

    fn after_history(&mut self) {
        let own = self.jack.as_ref().map(|c| c.name().to_string()).unwrap_or_default();
        self.model = Model::build(&self.graph, &self.layout, &own);
        self.refresh_matrices();
        self.layout_changed();
    }

    pub fn disconnect(&mut self, out: usize, inp: usize) {
        let (Some(c), Some(o), Some(i)) = (&self.jack, self.graph.ports.get(out), self.graph.ports.get(inp)) else { return };
        let _ = c;
        let (o, i) = (o.name.clone(), i.name.clone());
        self.disconnect_names(&o, &i);
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
        const ROW_H: f32 = 190.0;
        const X0: f32 = 160.0;
        const GAP: f32 = 48.0;
        // Lowest edge taken in each column (headers and expanded port lists).
        let mut bottom: HashMap<usize, f32> = HashMap::new();
        for (i, node) in self.model.nodes.iter().enumerate() {
            if self.layout.nodes.contains_key(&node.key.id()) {
                let b = bottom.entry(depths[i]).or_insert(f32::MIN);
                let e = self.node_extent(i);
                *b = b.max(if self.vertical() { e.right() } else { e.bottom() });
            }
        }
        // Column by column, left to right; within a column, blocks in the order of where what
        // feeds them sits (the mean height of their upstream neighbours, a barycentre pass), so
        // wires cross as little as they can; the rest by name.
        let mut cols: Vec<usize> = missing.iter().map(|&i| depths[i]).collect();
        cols.sort_unstable();
        cols.dedup();
        let mut feeds: HashMap<usize, Vec<usize>> = HashMap::new();
        for &(o, inp) in &self.graph.conns {
            if let (Some(&a), Some(&b)) = (self.model.port_node.get(&o), self.model.port_node.get(&inp))
                && a != b
            {
                feeds.entry(b).or_default().push(a);
            }
        }
        for col in cols {
            let mut here: Vec<usize> = missing.iter().copied().filter(|&i| depths[i] == col).collect();
            let vertical = self.vertical();
            let key = |i: usize, me: &App| -> f32 {
                let ys: Vec<f32> = feeds
                    .get(&i)
                    .map(|v| v.iter().filter(|&&a| depths[a] < col).filter_map(|&a| me.layout.nodes.get(&me.model.nodes[a].key.id()).map(|p| p[1])).collect())
                    .unwrap_or_default();
                let _ = &ys;
                let along: Vec<f32> = feeds
                    .get(&i)
                    .map(|v| v.iter().filter(|&&a| depths[a] < col).filter_map(|&a| me.layout.nodes.get(&me.model.nodes[a].key.id()).map(|p| if me.vertical() { p[0] } else { p[1] })).collect())
                    .unwrap_or_default();
                if along.is_empty() { f32::MAX } else { along.iter().sum::<f32>() / along.len() as f32 }
            };
            here.sort_by(|&a, &b| key(a, self).total_cmp(&key(b, self)).then(self.model.nodes[a].title.to_lowercase().cmp(&self.model.nodes[b].title.to_lowercase())));
            for i in here {
                if vertical {
                    // Rows by depth, left to right within a row.
                    let w = self.node_size(i)[0];
                    let x = bottom.get(&col).map_or(80.0, |r| r + GAP + w / 2.0);
                    let y = 80.0 + col as f32 * ROW_H;
                    self.layout.nodes.insert(self.model.nodes[i].key.id(), [x, y]);
                    bottom.insert(col, self.node_extent(i).right());
                } else {
                    let y = bottom.get(&col).map_or(80.0, |b| b + GAP + theme::NODE_H / 2.0);
                    let x = X0 + col as f32 * COL_W;
                    self.layout.nodes.insert(self.model.nodes[i].key.id(), [x, y]);
                    bottom.insert(col, self.node_extent(i).bottom());
                }
            }
        }
        self.layout_changed();
    }

    pub fn is_expanded(&self, i: usize) -> bool {
        self.layout.expanded.contains(&self.model.nodes[i].key.id())
    }

    /// An expanded node's port rows: inputs, then outputs (graph port indices).
    /// The ports on a node's body: (port, row, side). Ports view: the current type's inputs down
    /// the left half (side 0) and outputs down the right (side 1), every type when expanded.
    /// Matrix view: nothing unless expanded, then every port on a full-width row (side 2),
    /// inputs first.
    pub fn body_layout(&self, i: usize) -> Vec<(usize, usize, u8)> {
        let n = &self.model.nodes[i];
        let expanded = self.is_expanded(i);
        if self.layout.view.matrices {
            if !expanded {
                return Vec::new();
            }
            return n.ins.iter().chain(&n.outs).enumerate().map(|(r, &p)| (p, r, 2)).collect();
        }
        let (ins, outs) = if expanded { (n.ins.clone(), n.outs.clone()) } else { (n.ins_of(&self.graph, self.kind), n.outs_of(&self.graph, self.kind)) };
        ins.iter().enumerate().map(|(r, &p)| (p, r, 0)).chain(outs.iter().enumerate().map(|(r, &p)| (p, r, 1))).collect()
    }

    pub fn body_ports(&self, i: usize) -> Vec<usize> {
        self.body_layout(i).into_iter().map(|(p, _, _)| p).collect()
    }

    /// The vertical ports layout is on (it applies to the ports view).
    pub fn vertical(&self) -> bool {
        self.layout.view.vertical && !self.layout.view.matrices
    }

    /// A block's header size, world units: fixed in the horizontal views; in the vertical one as
    /// wide as its ports along an edge need.
    pub fn node_size(&self, i: usize) -> [f32; 2] {
        if !self.vertical() {
            return [theme::NODE_W, theme::NODE_H];
        }
        let l = self.body_layout(i);
        let n = l.iter().map(|&(_, r, _)| r + 1).max().unwrap_or(0);
        [(n as f32 * VPORT + 24.0).max(theme::NODE_W * 0.7), theme::NODE_H * 1.5]
    }

    /// Height of a node's port list (0 when there is none), world units.
    pub fn body_h(&self, i: usize) -> f32 {
        if self.vertical() {
            return 0.0;
        }
        let rows = self.body_layout(i).iter().map(|&(_, r, _)| r + 1).max().unwrap_or(0);
        if rows == 0 {
            return 0.0;
        }
        rows as f32 * theme::PORT_ROW + 2.0 * theme::BODY_PAD
    }

    /// The whole node (header and port list), world units.
    pub fn node_extent(&self, i: usize) -> Rect {
        let p = self.node_pos(i);
        let [w, h] = self.node_size(i);
        Rect::new(p[0] - w / 2.0, p[1] - h / 2.0, w, h + self.body_h(i))
    }

    /// Move a node; its matrices follow halfway (PatchMatrix), or fully when both ends are it.
    pub fn move_node(&mut self, id: &str, dx: f32, dy: f32) {
        if let Some(p) = self.layout.nodes.get_mut(id) {
            p[0] += dx;
            p[1] += dy;
        }
        let src_pre = format!("{id}>");
        let dst_suf = format!(">{id}");
        for (k, p) in self.layout.matrices.iter_mut() {
            let (a, b) = (k.starts_with(&src_pre), k.ends_with(&dst_suf));
            let f = if a && b { 1.0 } else if a || b { 0.5 } else { 0.0 };
            p[0] += dx * f;
            p[1] += dy * f;
        }
        self.layout_changed();
    }

    /// Prevent overlap (RaySession's repulse, in short): blocks that `from` now overlaps move
    /// out of its way: sideways when they sit level with it and further to one side, else up or
    /// down; a moved block pushes in turn (each a few times at most, so it always ends).
    pub fn repulse(&mut self, from: usize) {
        if !self.layout.view.no_overlap {
            return;
        }
        const GAP: f32 = 18.0;
        let grow = |r: Rect| Rect::new(r.x - 3.0 * theme::HANDLE_R, r.y - 14.0, r.w + 6.0 * theme::HANDLE_R, r.h + 14.0);
        let mut queue = vec![from];
        let mut moved: HashMap<usize, u32> = HashMap::new();
        let mut steps = 0;
        while let Some(p) = queue.pop() {
            steps += 1;
            if steps > 500 {
                break;
            }
            let pr = grow(self.node_extent(p));
            for j in 0..self.model.nodes.len() {
                if j == p || j == from || self.layout.hidden.contains(&self.model.nodes[j].key.id()) {
                    continue;
                }
                let jr = grow(self.node_extent(j));
                if !(pr.x < jr.right() + GAP && jr.x < pr.right() + GAP && pr.y < jr.bottom() + GAP && jr.y < pr.bottom() + GAP) {
                    continue;
                }
                let level = (jr.y < pr.cy() && pr.cy() < jr.bottom()) || (pr.y < jr.cy() && jr.cy() < pr.bottom());
                let (dx, dy) = if level && (jr.cx() - pr.cx()).abs() > pr.w * 0.5 {
                    if jr.cx() > pr.cx() { (pr.right() + GAP - jr.x, 0.0) } else { (pr.x - GAP - jr.right(), 0.0) }
                } else if jr.cy() >= pr.cy() {
                    (0.0, pr.bottom() + GAP - jr.y)
                } else {
                    (0.0, pr.y - GAP - jr.bottom())
                };
                let id = self.model.nodes[j].key.id();
                self.move_node(&id, dx, dy);
                let n = moved.entry(j).or_insert(0);
                *n += 1;
                if *n < 6 {
                    queue.push(j);
                }
            }
        }
    }

    /// After node `i` grew: push the nodes below it in its column down until nothing overlaps
    /// (a vertical cascade; RaySession's full repulse is phase 2).
    pub fn make_room(&mut self, i: usize) {
        const GAP: f32 = 24.0;
        let mut placed = vec![self.node_extent(i)];
        let base = self.node_pos(i)[1];
        let mut below: Vec<usize> = (0..self.model.nodes.len())
            .filter(|&j| j != i && !self.layout.hidden.contains(&self.model.nodes[j].key.id()) && self.node_pos(j)[1] > base)
            .collect();
        below.sort_by(|&a, &b| self.node_pos(a)[1].total_cmp(&self.node_pos(b)[1]));
        for j in below {
            let mut r = self.node_extent(j);
            let mut dy = 0.0;
            for _ in 0..placed.len() + 1 {
                let hit = placed.iter().find(|p| p.overlaps(&Rect::new(r.x, r.y - GAP, r.w, r.h + 2.0 * GAP)));
                match hit {
                    Some(p) => {
                        let d = p.bottom() + GAP - r.y;
                        r.y += d;
                        dy += d;
                    }
                    None => break,
                }
            }
            if dy > 0.0 {
                let id = self.model.nodes[j].key.id();
                self.move_node(&id, 0.0, dy);
            }
            placed.push(r);
        }
        // Matrices over the grown node move below it.
        let grown = self.node_extent(i);
        let keys: Vec<(String, [f32; 2])> = self.mats.iter().map(|m| (m.key.clone(), Self::matrix_size(m))).collect();
        for (key, size) in keys {
            if let Some(p) = self.layout.matrices.get_mut(&key) {
                let r = Rect::centered(p[0], p[1], size[0] + 16.0, size[1] + 16.0);
                if r.overlaps(&grown) {
                    p[1] = grown.bottom() + size[1] / 2.0 + GAP;
                }
            }
        }
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
                let r = self.node_extent(i);
                Rect::new(r.x - 3.0 * theme::HANDLE_R - 6.0, r.y - 15.0, r.w + 6.0 * theme::HANDLE_R + 12.0, r.h + 30.0)
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
        let record = !matches!(a, Action::Undo | Action::Redo);
        if record {
            self.history.begin(&self.layout);
        }
        self.apply_inner(a);
        if record {
            self.history.end(&self.layout);
        }
        // Geometry may have changed under the pointer.
        self.refresh_hover();
    }

    fn apply_inner(&mut self, a: Action) {
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
            Action::ToggleVertical => {
                self.layout.view.vertical = !self.layout.view.vertical;
                if self.layout.view.vertical {
                    self.layout.view.matrices = false;
                }
                self.canvas.drag = crate::canvas::Drag::None;
                // The flow turns: lay the graph out again for it.
                self.place_new_nodes(true);
                self.refresh_matrices();
                self.fit_pending = true;
                self.layout_changed();
            }
            Action::ToggleMatrices => {
                self.layout.view.matrices = !self.layout.view.matrices;
                self.canvas.drag = crate::canvas::Drag::None;
                self.refresh_matrices();
                self.layout_changed();
            }
            Action::Disconnect(o, i) => self.disconnect_names(&o, &i),
            Action::ViewsPage => {
                let mut d = crate::dialog::Dialog::new(crate::dialog::Kind::Views, "Views");
                d.rows = self.view_rows();
                d.buttons = vec!["Close".into(), "Save the current view".into()];
                d.note = "A view keeps positions, hidden and expanded blocks, the view style, the type and the camera. Ctrl+1…9 recalls them.".into();
                self.dialog = Some(d);
            }
            Action::SaveView(name) => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return self.say("A view needs a name", true);
                }
                let v = crate::layout::SavedView {
                    nodes: self.layout.nodes.clone(),
                    matrices: self.layout.matrices.clone(),
                    hidden: self.layout.hidden.clone(),
                    expanded: self.layout.expanded.clone(),
                    kind: self.kind.label().to_string(),
                    cam: self.layout.view.cam,
                    zoom: self.layout.view.zoom,
                    matrix_view: self.layout.view.matrices,
                    vertical: self.layout.view.vertical,
                };
                self.layout.views.insert(name.clone(), v);
                self.say(format!("Saved the view \"{name}\""), false);
                self.layout_changed();
                self.refresh_views_page();
            }
            Action::RecallView(name) => {
                let Some(v) = self.layout.views.get(&name).cloned() else { return };
                // Blocks the view doesn't know keep their place.
                self.layout.nodes.extend(v.nodes);
                self.layout.matrices.extend(v.matrices);
                self.layout.hidden = v.hidden;
                self.layout.expanded = v.expanded;
                self.layout.view.cam = v.cam;
                self.layout.view.zoom = v.zoom;
                self.layout.view.matrices = v.matrix_view;
                self.layout.view.vertical = v.vertical;
                self.kind = kind_from(&v.kind);
                self.dialog = None;
                self.refresh_matrices();
                self.layout_changed();
            }
            Action::DeleteView(name) => {
                self.layout.views.remove(&name);
                self.layout_changed();
                self.refresh_views_page();
            }
            Action::RecallViewN(n) => {
                if let Some(name) = self.layout.views.keys().nth(n).cloned() {
                    self.apply_inner(Action::RecallView(name));
                }
            }
            Action::CopyConns(target, cut) => {
                let ports = self.clip_ports(&target);
                let peers = |p: usize, g: &Graph| -> Vec<String> {
                    g.conns.iter().filter_map(|&(o, i)| if o == p { Some(i) } else if i == p { Some(o) } else { None }).map(|q| g.ports[q].name.clone()).collect()
                };
                let clip = Clip {
                    kind: self.kind,
                    ins: ports.iter().filter(|&&p| self.graph.ports[p].is_input()).map(|&p| peers(p, &self.graph)).collect(),
                    outs: ports.iter().filter(|&&p| self.graph.ports[p].is_output()).map(|&p| peers(p, &self.graph)).collect(),
                };
                let n: usize = clip.ins.iter().chain(&clip.outs).map(Vec::len).sum();
                if cut {
                    for &p in &ports {
                        let name = self.graph.ports[p].name.clone();
                        for q in peers(p, &self.graph) {
                            if self.graph.ports[p].is_output() { self.disconnect_names(&name, &q) } else { self.disconnect_names(&q, &name) }
                        }
                    }
                }
                self.say(format!("{} {n} connection{}", if cut { "Cut" } else { "Copied" }, if n == 1 { "" } else { "s" }), false);
                self.clip = Some(clip);
            }
            Action::PasteConns(target) => {
                let Some(clip) = self.clip.clone() else { return };
                let ports = self.clip_ports(&target);
                let ins: Vec<usize> = ports.iter().copied().filter(|&p| self.graph.ports[p].is_input()).collect();
                let outs: Vec<usize> = ports.iter().copied().filter(|&p| self.graph.ports[p].is_output()).collect();
                let mut made = 0;
                for (k, &p) in ins.iter().enumerate() {
                    let Some(peers) = (!clip.ins.is_empty()).then(|| &clip.ins[k % clip.ins.len()]) else { break };
                    let name = self.graph.ports[p].name.clone();
                    for q in peers {
                        if self.graph.port(q).is_some_and(|qp| qp.kind == self.graph.ports[p].kind) && !self.graph.port(q).is_some_and(|qp| self.graph.connected(self.graph.by_name[&qp.name], p)) {
                            self.connect_names(q, &name);
                            made += 1;
                        }
                    }
                }
                for (k, &p) in outs.iter().enumerate() {
                    let Some(peers) = (!clip.outs.is_empty()).then(|| &clip.outs[k % clip.outs.len()]) else { break };
                    let name = self.graph.ports[p].name.clone();
                    for q in peers {
                        if self.graph.port(q).is_some_and(|qp| qp.kind == self.graph.ports[p].kind) && !self.graph.port(q).is_some_and(|qp| self.graph.connected(p, self.graph.by_name[&qp.name])) {
                            self.connect_names(&name, q);
                            made += 1;
                        }
                    }
                }
                self.say(format!("Pasted {made} connection{}", if made == 1 { "" } else { "s" }), false);
            }
            Action::ToggleNoOverlap => {
                self.layout.view.no_overlap = !self.layout.view.no_overlap;
                self.layout_changed();
            }
            Action::ToggleGrid => {
                self.layout.view.grid = !self.layout.view.grid;
                self.layout_changed();
            }
            Action::ToggleSnap => {
                self.layout.view.snap = !self.layout.view.snap;
                self.layout_changed();
            }
            Action::ToggleLabels => {
                self.layout.view.labels = !self.layout.view.labels;
                self.layout_changed();
            }
            Action::BufferMenu(r) => self.ui.open_buffer_menu(r, self.stats.buffer_size),
            Action::ScaleMenu(r) => self.ui.open_scale_menu(r, self.layout.view.ui_scale),
            Action::SetUiScale(f) => {
                let c = self.canvas_center_world();
                self.layout.view.ui_scale = f.clamp(0.75, 3.0);
                self.apply_scales();
                self.layout_regions();
                self.center_on(c);
            }
            Action::ZoomBy(f) => {
                let a = self.canvas.area;
                self.zoom_at([a.cx(), a.cy()], self.layout.view.zoom * f);
            }
            Action::ZoomTo(z) => {
                let a = self.canvas.area;
                self.zoom_at([a.cx(), a.cy()], z);
            }
            Action::ZoomSlider(r) => self.ui.slider = Some(r),
            Action::AppMenu(r) => {
                let labels = self.layout.view.labels;
                let sidebar = self.layout.view.sidebar;
                let ui_scale = self.layout.view.ui_scale;
                let (grid, snap) = (self.layout.view.grid, self.layout.view.snap);
                self.ui.open_app_menu(r, labels, sidebar, ui_scale, grid, snap)
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
            Action::Expand(id, on) => {
                if on {
                    self.layout.expanded.insert(id.clone());
                } else {
                    self.layout.expanded.remove(&id);
                }
                if let Some(i) = self.model.node_by_id(&id) {
                    self.make_room(i);
                }
                self.layout_changed();
            }
            Action::ExpandAll => {
                let any = (0..self.model.nodes.len()).any(|i| self.is_expanded(i));
                if any {
                    self.layout.expanded.clear();
                } else {
                    for n in &self.model.nodes {
                        self.layout.expanded.insert(n.key.id());
                    }
                    let mut order: Vec<usize> = (0..self.model.nodes.len()).collect();
                    order.sort_by(|&a, &b| self.node_pos(a)[1].total_cmp(&self.node_pos(b)[1]));
                    for i in order {
                        self.make_room(i);
                    }
                }
                self.layout_changed();
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
            Action::TransportToggle => {
                if let Some(c) = &self.jack {
                    if self.stats.transport.rolling || self.stats.transport.starting {
                        c.transport_stop();
                    } else {
                        c.transport_start();
                    }
                    self.transport_at = Instant::now() + Duration::from_millis(30);
                }
            }
            Action::TransportStop => {
                if let Some(c) = &self.jack {
                    c.transport_stop();
                    c.transport_locate(0);
                    self.transport_at = Instant::now() + Duration::from_millis(30);
                }
            }
            Action::TransportRewind => {
                if let Some(c) = &self.jack {
                    c.transport_locate(0);
                    self.transport_at = Instant::now() + Duration::from_millis(30);
                }
            }
            Action::ServerStart => self.server.run(crate::server::Cmd::Start),
            Action::ServerStop => self.server.run(crate::server::Cmd::Stop),
            Action::ServerRestart => self.server.run(crate::server::Cmd::Restart),
            Action::ServerSettings => {
                self.server.send(crate::server::Cmd::LoadConfig);
                let mut d = crate::dialog::Dialog::new(crate::dialog::Kind::Server, "JACK server");
                d.note = "Driver and engine settings apply when the server (re)starts; jackdbus keeps them.".into();
                d.set_rows(self.server_rows());
                d.buttons = self.server_buttons();
                self.dialog = Some(d);
            }
            Action::DialogSet(id, v) => self.dialog_event(crate::dialog::Event::Set(id, v)),
            Action::OpenPipeWire => {
                if self.source == Source::PipeWire {
                    return;
                }
                if self.pw.is_none() {
                    match crate::pw::PwLink::start(self.waker.clone()) {
                        Ok(p) => self.pw = Some(p),
                        Err(e) => return self.say(format!("PipeWire: {e}"), true),
                    }
                }
                self.save_layout();
                let pw_canvas = Layout::load_pw();
                self.jack_canvas = Some(self.layout.clone());
                self.layout.take_canvas(pw_canvas);
                self.source = Source::PipeWire;
                self.history = Default::default();
                if !self.kinds().contains(&self.kind) {
                    self.kind = PortKind::Audio;
                }
                self.rebuild();
                self.say("PipeWire's graph (← JACK, top left, to go back)", false);
            }
            Action::BackToJack => {
                if self.source == Source::Jack {
                    return;
                }
                self.layout.save_pw();
                if let Some(j) = self.jack_canvas.take() {
                    self.layout.take_canvas(j);
                }
                self.source = Source::Jack;
                self.history = Default::default();
                // Keep the dump running for a quick way back; it is idle between changes.
                if !self.kinds().contains(&self.kind) {
                    self.kind = PortKind::Audio;
                }
                self.graph = Graph::default();
                self.rebuild();
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Rename(key) => {
                let current = match key.strip_prefix("port:") {
                    Some(p) => self.graph.by_name.get(p).map(|&i| self.model.labels[i].clone()).unwrap_or_default(),
                    None => self.model.node_by_id(&key).map(|i| self.model.nodes[i].title.clone()).unwrap_or_default(),
                };
                let what = if key.starts_with("port:") { "port" } else { "client" };
                let mut d = crate::dialog::Dialog::new(crate::dialog::Kind::RenameItem, format!("Rename the {what}"));
                d.tag = key;
                d.rows = vec![crate::dialog::Row {
                    id: "name".into(),
                    label: "Name".into(),
                    help: "Empty: the name from JACK again".into(),
                    control: crate::dialog::Control::Text(current),
                    modified: false,
                }];
                d.buttons = vec!["Cancel".into(), "Rename".into()];
                d.note = "Kept with the canvas (and with the session when one is open).".into();
                d.edit("name");
                self.dialog = Some(d);
            }
            Action::SetName(key, name) => {
                if name.trim().is_empty() {
                    self.layout.names.remove(&key);
                } else {
                    self.layout.names.insert(key, name.trim().to_string());
                }
                let own = self.jack.as_ref().map(|c| c.name().to_string()).unwrap_or_default();
                self.model = Model::build(&self.graph, &self.layout, &own);
                self.refresh_matrices();
                self.layout_changed();
            }
            Action::Sess(a) => self.session_action(a),
            Action::Quit => self.quit = true,
        }
    }

    // ---- JACK server settings (jackdbus) ----

    fn server_buttons(&self) -> Vec<String> {
        let st = &self.server.status;
        let mut b = Vec::new();
        if st.available {
            b.push(if st.started { "Stop".to_string() } else { "Start".to_string() });
            b.push("Restart".to_string());
        }
        b.push("Close".to_string());
        b
    }

    pub fn server_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::{Control, Row};
        use crate::server::Value;
        let st = &self.server.status;
        let mut rows = vec![Row::header("Server")];
        let state = if !st.available {
            "jackdbus not available on the session bus".to_string()
        } else if self.server.busy {
            "working…".to_string()
        } else if st.started {
            format!("running ({} Hz, {} frames)", self.stats.sample_rate, self.stats.buffer_size)
        } else {
            "stopped".to_string()
        };
        rows.push(Row::info("Status", state));
        let Some(cfg) = &self.server.config else {
            if st.available {
                rows.push(Row::info("Settings", "loading…"));
            }
            return rows;
        };
        rows.push(Row {
            id: "engine/driver".into(),
            label: "Driver".into(),
            help: "The backend jackd drives: alsa for sound cards, dummy for none, net/netone for network audio…".into(),
            control: Control::Choice { value: cfg.driver.clone(), options: cfg.drivers.iter().map(|d| (d.clone(), d.clone())).collect(), editable: false },
            modified: false,
        });
        let to_row = |p: &crate::server::Param| -> Row {
            let id = p.path.join("/");
            let control = match (&p.value, &p.constraint) {
                (Value::Bool(b), _) => Control::Toggle(*b),
                (v, Some(c)) if c.range && c.values.len() == 2 => {
                    let n = |x: &Value| match x {
                        Value::I32(i) => *i as i64,
                        Value::U32(u) => *u as i64,
                        _ => 0,
                    };
                    Control::Number { value: n(v), min: n(&c.values[0].0), max: n(&c.values[1].0) }
                }
                (v, Some(c)) => Control::Choice {
                    value: v.show(),
                    options: c
                        .values
                        .iter()
                        .map(|(val, label)| {
                            let shown = if c.fake || label == &val.show() || label.is_empty() {
                                if label.is_empty() { val.show() } else { label.clone() }
                            } else {
                                format!("{} — {label}", val.show())
                            };
                            (val.show(), shown)
                        })
                        .collect(),
                    editable: !c.strict,
                },
                (Value::U32(u), None) if p.name == "rate" => Control::Choice {
                    value: u.to_string(),
                    options: [22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000].iter().map(|r| (r.to_string(), format!("{r} Hz"))).collect(),
                    editable: true,
                },
                (Value::U32(u), None) if p.name == "period" => Control::Choice {
                    value: u.to_string(),
                    options: [16, 32, 64, 128, 256, 512, 1024, 2048, 4096].iter().map(|r| (r.to_string(), format!("{r} frames"))).collect(),
                    editable: true,
                },
                (Value::U32(u), None) => Control::Number { value: *u as i64, min: 0, max: u32::MAX as i64 },
                (Value::I32(i), None) => Control::Number { value: *i as i64, min: i32::MIN as i64, max: i32::MAX as i64 },
                (v, None) => Control::Text(v.show()),
            };
            let label = if p.short.is_empty() { p.name.clone() } else { p.short.clone() };
            Row { id, label, help: if p.long.is_empty() { p.short.clone() } else { p.long.clone() }, control, modified: p.is_set }
        };
        rows.push(Row::header(format!("Driver: {}", cfg.driver)));
        rows.extend(cfg.driver_params.iter().map(to_row));
        rows.push(Row::header("Engine"));
        rows.extend(cfg.engine.iter().filter(|p| p.name != "driver").map(to_row));
        rows
    }

    pub fn dialog_event(&mut self, ev: crate::dialog::Event) {
        use crate::dialog::{Event, Kind};
        self.redraw = true;
        let Some(kind) = self.dialog.as_ref().map(|d| d.kind) else { return };
        match (kind, ev) {
            (_, Event::Close) => self.dialog = None,
            (_, Event::Options(row, under)) => {
                let Some(d) = &self.dialog else { return };
                let Some(r) = d.rows.get(row) else { return };
                if let crate::dialog::Control::Choice { value, options, .. } = &r.control {
                    let items = options
                        .iter()
                        .map(|(v, l)| crate::ui::MenuItem::new(l.clone(), Action::DialogSet(r.id.clone(), v.clone())).checked(v == value))
                        .collect();
                    self.ui.open_menu([under.x, under.bottom() + 2.0], items);
                }
            }
            (Kind::Server, Event::Set(id, text)) => {
                let path: Vec<String> = id.split('/').map(str::to_string).collect();
                let value = if id == "engine/driver" {
                    Some(crate::server::Value::Str(text.clone()))
                } else {
                    self.server
                        .config
                        .as_ref()
                        .and_then(|c| c.engine.iter().chain(&c.driver_params).find(|p| p.path == path))
                        .and_then(|p| p.value.parse_like(&text))
                };
                match value {
                    Some(v) => self.server.run(crate::server::Cmd::Set(path, v)),
                    None => self.say(format!("\"{text}\" is not a valid value for {id}"), true),
                }
            }
            (Kind::Server, Event::Reset(id)) => self.server.run(crate::server::Cmd::Reset(id.split('/').map(str::to_string).collect())),
            (Kind::Server, Event::Button(b)) => {
                let label = self.dialog.as_ref().and_then(|d| d.buttons.get(b).cloned()).unwrap_or_default();
                match label.as_str() {
                    "Start" => self.apply(Action::ServerStart),
                    "Stop" => self.apply(Action::ServerStop),
                    "Restart" => self.apply(Action::ServerRestart),
                    _ => self.dialog = None,
                }
            }
            (Kind::Views, Event::Set(id, v)) => {
                if let Some(name) = id.strip_prefix("view:") {
                    self.apply(Action::RecallView(name.to_string()));
                } else if id == "delete" {
                    if !v.is_empty() {
                        self.apply(Action::DeleteView(v));
                    }
                } else if let Some(d) = &mut self.dialog {
                    d.set_local(&id, &v);
                }
            }
            (Kind::Views, Event::Button(b)) => {
                let Some(d) = &self.dialog else { return };
                if d.buttons.get(b).map(String::as_str) == Some("Save the current view") {
                    let name = d.value("name").unwrap_or_default();
                    self.apply(Action::SaveView(name));
                    if let Some(d) = &mut self.dialog {
                        d.set_local("name", "");
                    }
                } else {
                    self.dialog = None;
                }
            }
            (Kind::RenameItem, Event::Set(id, v)) => {
                if let Some(d) = &mut self.dialog {
                    d.set_local(&id, &v);
                }
            }
            (Kind::RenameItem, Event::Button(b)) => {
                let Some(d) = self.dialog.take() else { return };
                if d.buttons.get(b).map(String::as_str) == Some("Rename") {
                    let name = d.value("name").unwrap_or_default();
                    self.apply(Action::SetName(d.tag.clone(), name));
                }
            }
            (k, ev) => self.session_dialog_event(k, ev),
        }
    }

    // ---- view ----

    pub const ZOOM_MIN: f32 = 0.25;
    pub const ZOOM_MAX: f32 = 4.0;

    /// Recompute the content scale (after the desktop's or the interface scale changed).
    pub fn apply_scales(&mut self) {
        let ui = std::env::var("PATCHBOX_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(self.layout.view.ui_scale);
        self.scale = self.chrome * ui.clamp(0.75, 3.0);
        self.redraw = true;
    }

    /// Zoom to `zoom`, keeping the world point under the screen point `at` in place.
    pub fn zoom_at(&mut self, at: [f32; 2], zoom: f32) {
        let before = self.xf().w(at);
        self.layout.view.zoom = zoom.clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
        let z = self.z();
        self.layout.view.cam = [before[0] - (at[0] - self.canvas.area.x) / z, before[1] - (at[1] - self.canvas.area.y) / z];
        self.layout_changed();
    }

    /// The zoom slider: position (0..1 along the track) <-> zoom, logarithmic.
    pub fn zoom_to_t(z: f32) -> f32 {
        ((z / Self::ZOOM_MIN).ln() / (Self::ZOOM_MAX / Self::ZOOM_MIN).ln()).clamp(0.0, 1.0)
    }
    pub fn t_to_zoom(t: f32) -> f32 {
        Self::ZOOM_MIN * (Self::ZOOM_MAX / Self::ZOOM_MIN).powf(t.clamp(0.0, 1.0))
    }

    /// Dragging the zoom slider: the pointer is at x.
    pub fn zoom_slider_drag(&mut self, x: f32) {
        let Some(r) = self.ui.slider else { return };
        let z = Self::t_to_zoom((x - r.x) / r.w.max(1.0));
        // Snap to 100 % near it.
        let z = if (z - 1.0).abs() < 0.04 { 1.0 } else { z };
        let a = self.canvas.area;
        self.zoom_at([a.cx(), a.cy()], z);
    }

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
            let r = self.node_extent(i);
            add(Rect::new(r.x - 20.0, r.y - 15.0, r.w + 40.0, r.h + 30.0));
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

/// Copied connections (the connection clipboard).
#[derive(Clone, Debug)]
pub struct Clip {
    pub kind: PortKind,
    /// For each copied input port, by position: the output ports it was connected from.
    pub ins: Vec<Vec<String>>,
    /// For each copied output port: the input ports it reached.
    pub outs: Vec<Vec<String>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Jack,
    PipeWire,
}

/// Spacing of the port squares along a block's edge (vertical layout), world units.
pub const VPORT: f32 = 18.0;

pub fn kind_from(s: &str) -> PortKind {
    match s {
        "MIDI" => PortKind::Midi,
        "CV" => PortKind::Cv,
        "OSC" => PortKind::Osc,
        "VIDEO" => PortKind::Video,
        _ => PortKind::Audio,
    }
}

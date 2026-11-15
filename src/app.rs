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
    /// Show a view style: (matrices, vertical).
    SetStyle(bool, bool),
    PortsStyle,
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
    /// The event log page (toggles).
    EventLog,
    /// Put a JACK preset in place (restarts the server when something differs, except the buffer
    /// size and the CPU wake-up latency).
    ApplyPreset(String),
    /// The presets menu, under this rectangle.
    PresetMenu(Rect),
    /// Ask for a name and save the server's settings under it.
    SavePresetPrompt,
    /// The connection rules page (toggles).
    RulesPage,
    /// Line up the selected blocks.
    Align(crate::canvas::Align),
    /// Nothing (a click that a control keeps to itself).
    Noop,
    /// Ask for a group's name and size, from this port on; break up this port's group.
    GroupPrompt(String),
    Ungroup(String),
    /// Ask for a new name for this port's group.
    RenameGroup(String),
    /// Show a client as a block per track / plugin, or as one.
    Tracks(String, bool),
    /// Our names to JACK's metadata on/off.
    ToggleExportNames,
    /// The find bar (Ctrl+F): open, close, go to a result.
    Find,
    SearchGo(usize),
    /// Show a port: its type, its block centred, a ring round it.
    FindPort(String),
    HideSelected,
    SelectAll,
    ClearSelection,
    /// Add (true) or remove a rule for this connection: output, input.
    Rule(String, String, bool),
    /// Make every present connection of this node's ports a rule.
    RulesFromNode(String),
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
    /// PipeWire as the settings page shows it (from the pw-dump stream).
    pub pw_state: Option<crate::pw::PwState>,
    /// The settings page's tab last shown: the JACK tabs (JACK_TABS), then PipeWire.
    settings_tab: usize,
    /// JACK server presets, and the server's settings as a preset would keep them.
    pub presets: crate::presets::Presets,
    pub jack_now: Option<pbx_session::jackcfg::JackSettings>,
    /// A preset to be saved under this name when the server's settings come.
    preset_saving: Option<String>,
    /// Connection rules (patchboxd keeps and applies them).
    pub rules: Vec<pbx_session::rules::Rule>,
    /// The find bar, while open.
    pub search: Option<crate::canvas::Search>,
    /// Our names in JACK's metadata.
    pub export: crate::export::Export,
    /// Installed applications (.desktop), scanned when first needed.
    pub apps: Option<Vec<crate::session::App>>,
    pub log: Vec<LogLine>,
    /// The event log (xruns, the server, clients, connections, messages), with times.
    pub events: crate::events::Events,
    pub redraw: bool,
    graph_due: Option<Instant>,
    graph_first: Option<Instant>,
    retry_at: Instant,
    retry_logged: bool,
    dsp_at: Instant,
    transport_at: Instant,
    server_stats_at: Instant,
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
            pw_state: None,
            settings_tab: 0,
            presets: crate::presets::load(),
            jack_now: None,
            preset_saving: None,
            rules: Vec::new(),
            search: None,
            export: Default::default(),
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
            events: Default::default(),
            redraw: true,
            graph_due: None,
            graph_first: None,
            retry_at: Instant::now(),
            retry_logged: false,
            dsp_at: Instant::now(),
            transport_at: Instant::now(),
            server_stats_at: Instant::now(),
            quit: false,
            window_cmd: None,
            fit_pending: false,
        }
    }

    pub fn say(&mut self, text: impl Into<String>, bad: bool) {
        let text = text.into();
        eprintln!("[patchbox] {text}");
        self.events.push(crate::events::Kind::Message, text.clone(), bad);
        self.log.push(LogLine { text, bad });
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
                    let was = &self.server.status;
                    if st.available && (st.available != was.available || st.started != was.started) {
                        self.server.send(crate::server::Cmd::ReadSettings);
                    }
                    if st.available != was.available {
                        self.events.push(crate::events::Kind::Server, if st.available { "jackdbus is there" } else { "jackdbus went away" }, !st.available);
                    } else if st.started != was.started {
                        self.events.push(crate::events::Kind::Server, if st.started { "Server started (jackdbus)" } else { "Server stopped (jackdbus)" }, false);
                    }
                    if st.started != self.server.status.started || st.available != self.server.status.available {
                        self.redraw = true;
                        if st.started && self.jack.is_none() {
                            self.retry_at = Instant::now();
                        }
                    }
                    self.server.status = st;
                }
                Reply::ConstraintsChanged => {
                    self.events.push(crate::events::Kind::Server, "Sound cards changed: device lists read again", false);
                    // The page shows lists from the loaded tree: load it again (else it loads
                    // when the page opens).
                    if self.server.config.is_some() {
                        self.server.send(crate::server::Cmd::LoadConfig);
                    }
                }
                Reply::Config(c) => {
                    self.server.config = Some(*c);
                    // A parameter changed (or the page loaded): the settings a preset would keep.
                    self.server.send(crate::server::Cmd::ReadSettings);
                }
                Reply::Settings(s) => {
                    if let Some(name) = self.preset_saving.take() {
                        self.presets.insert(name.clone(), (*s).clone());
                        match crate::presets::save(&self.presets) {
                            Ok(()) => self.say(format!("Preset {name} saved: {}", crate::presets::summary(&s)), false),
                            Err(e) => self.say(format!("Could not save the presets: {e}"), true),
                        }
                    }
                    self.jack_now = Some(*s);
                }
                Reply::Stats(st) => self.server.stats = Some(st),
                Reply::Done(m) => self.say(m, false),
                Reply::Error(e) => self.say(e, true),
            }
            self.redraw = true;
        }
        if self.events.dirty && self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::Log && d.tab == 0) {
            self.refresh_log_page();
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
        let mut clock_changed = false;
        use crate::events::Kind as K;
        for e in events {
            match &e {
                Event::Xrun => {
                    self.stats.xruns += 1;
                    let (late, dsp) = (c.xrun_delay_ms(), c.cpu_load());
                    self.events.push(K::Xrun, format!("{late:.2} ms late · DSP {dsp:.1} % · {} frames at {} Hz", self.stats.buffer_size, self.stats.sample_rate), true);
                }
                Event::BufferSize(n) => {
                    if *n != self.stats.buffer_size {
                        let ms = *n as f32 * 1000.0 / self.stats.sample_rate.max(1) as f32;
                        self.events.push(K::Server, format!("Buffer size {n} frames ({ms:.2} ms)"), false);
                    }
                    self.stats.buffer_size = *n;
                    clock_changed = true;
                }
                Event::SampleRate(n) => {
                    if *n != self.stats.sample_rate {
                        self.events.push(K::Server, format!("Sample rate {n} Hz"), false);
                    }
                    self.stats.sample_rate = *n;
                    clock_changed = true;
                }
                Event::Freewheel(on) => {
                    self.events.push(K::Server, if *on { "Freewheeling" } else { "Freewheel off" }, false);
                    self.stats.freewheel = *on;
                }
                Event::ClientRegistered { name, registered } => {
                    self.events.push(K::Client, format!("{name} {}", if *registered { "joined" } else { "left" }), false);
                    if !registered {
                        c.forget_client(name);
                    }
                }
                Event::PortConnect { a, b, connected } => {
                    let n = |id: u32| c.port_name(id).unwrap_or_else(|| format!("port {id}"));
                    self.events.push(K::Connection, format!("{} {} {}", n(*a), if *connected { "→" } else { "✕" }, n(*b)), false);
                }
                Event::Shutdown { reason } => {
                    self.events.push(K::Server, format!("The server shut us down: {reason}"), true);
                    lost = Some(reason.clone());
                }
                _ => {}
            }
            if e.changes_graph() {
                self.graph_first.get_or_insert(now);
                self.graph_due = Some(now + GRAPH_QUIET);
            }
            self.redraw = true;
        }
        if clock_changed {
            self.refresh_pw_page(); // its clock is JACK's when bridged
        }
        if let Some(reason) = lost {
            self.export_reset();
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
        // A tooltip comes up once the pointer has rested on a control.
        if self.ui.hover_hit.is_some() && !self.ui.tip_shown && !self.ui.tip_off {
            let due = self.ui.hover_since + crate::ui::TIP_DELAY;
            if now >= due {
                self.ui.tip_shown = true;
                self.redraw = true;
            } else {
                at(due);
            }
        }
        if self.jack.is_none() {
            if now >= self.retry_at {
                self.try_connect();
            }
            if self.jack.is_none() {
                at(self.retry_at);
            }
        }
        if let Some(due) = self.export.due {
            if now >= due {
                self.export.due = self.export_names();
            }
            if let Some(t) = self.export.due {
                at(t);
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
        // The server's figures, while its page is open.
        if self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::Server) && self.server.status.started {
            if now >= self.server_stats_at {
                self.server.send(crate::server::Cmd::Stats);
                // the bridges' states change by the second (locking, a card unplugged)
                self.server_stats_at = now + Duration::from_secs(if self.settings_tab == TAB_BRIDGES { 1 } else { 2 });
            }
            at(self.server_stats_at);
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
            // JACK's canvas, set aside: on disk too, with the shared settings as they are now and
            // its own cameras.
            if let Some(j) = self.jack_canvas.clone() {
                let mut j = Layout::with_view_keeping_style(j, &self.layout.view);
                j.window = self.layout.window;
                self.write_jack_canvas(&mut j);
            }
            self.layout_dirty = None;
            return;
        }
        let mut l = std::mem::take(&mut self.layout);
        self.write_jack_canvas(&mut l);
        self.layout = l;
        self.layout_dirty = None;
    }

    /// Write a JACK canvas: the open session's into its folder (the shared view, and the canvas
    /// left for it, stay global), else the global one.
    fn write_jack_canvas(&self, l: &mut Layout) {
        match &self.layout_dir {
            Some(dir) => {
                if dir.is_dir() {
                    l.save_session(dir);
                }
                let (mut global, _) = Layout::with_view_of(self.global_canvas.clone().unwrap_or_default(), &l.view);
                global.window = l.window;
                global.save();
            }
            None => l.save(),
        }
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
        if self.source == Source::PipeWire {
            // A session's canvas is JACK's: switch the one set aside, not PipeWire's on screen.
            if let Some(j) = self.jack_canvas.take() {
                let mut j = Layout::with_view_keeping_style(j, &self.layout.view);
                self.switch_session_canvas(&mut j, &dir);
                self.jack_canvas = Some(j);
            }
            self.layout_dir = dir;
            self.redraw = true;
            return;
        }
        let mut l = std::mem::take(&mut self.layout);
        if !self.switch_session_canvas(&mut l, &dir) {
            self.fit_pending = true;
        }
        self.layout = l;
        self.layout_dir = dir;
        if self.jack.is_some() {
            self.rebuild();
        }
        self.redraw = true;
    }

    /// Give JACK's canvas `l` the one of the session in `dir` (None: the global one back), each
    /// with its own cameras. False: the canvas has no camera for the current style yet.
    fn switch_session_canvas(&mut self, l: &mut Layout, dir: &Option<std::path::PathBuf>) -> bool {
        let mut has = true;
        match (&self.layout_dir, dir) {
            (None, Some(_)) => self.global_canvas = Some(l.clone()),
            (Some(_), None) => {
                if let Some(g) = self.global_canvas.take() {
                    let cams = g.cams();
                    l.take_canvas(g);
                    has = l.set_cams(cams);
                }
            }
            _ => {}
        }
        if let Some(d) = dir {
            // A session opened for the first time starts from the current arrangement and view.
            if let Some(s) = crate::layout::Layout::load_session(d) {
                let cams = s.cams();
                l.take_canvas(s);
                has = l.set_cams(cams);
            }
        }
        has
    }

    pub fn layout_changed(&mut self) {
        self.layout_dirty = Some(Instant::now());
        self.redraw = true;
    }

    /// PipeWire's graph changed (or pw-dump ended).
    fn on_pipewire(&mut self) {
        let Some(pw) = &mut self.pw else { return };
        let t = pw.take();
        for (m, bad) in t.log {
            self.say(m, bad);
        }
        if let Some(g) = t.graph {
            self.pw_graph = g;
            if self.source == Source::PipeWire {
                self.rebuild();
            }
        }
        if let Some(st) = t.state
            && self.pw_state.as_ref() != Some(&st)
        {
            self.pw_state = Some(st);
            self.refresh_pw_page();
        }
        if let Some(e) = t.error {
            self.say(format!("PipeWire: {e}"), true);
            self.pw = None;
            self.pw_state = None;
            self.refresh_pw_page();
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

    /// The view style's name, for its own block positions.
    /// Change the view style (matrices; vertical, which applies to the ports view only and is
    /// remembered while the matrices show). Each style keeps its own block positions and camera:
    /// the shown ones go aside, the new style's come back, or (first time) the graph is
    /// arranged for it and fitted.
    pub fn set_style(&mut self, matrices: bool, vertical: bool) {
        let v = &mut self.layout.view;
        let old = v.style();
        v.style_cams.insert(old.to_string(), (v.cam, v.zoom));
        v.matrices = matrices;
        v.vertical = vertical;
        let new = v.style();
        if new == old {
            return;
        }
        let cam = v.style_cams.get(new).copied();
        self.layout.settle();
        self.canvas.drag = crate::canvas::Drag::None;
        let first = self.layout.nodes.is_empty();
        self.place_new_nodes(false);
        self.refresh_matrices();
        match cam {
            Some((c, z)) if !first => {
                self.layout.view.cam = c;
                self.layout.view.zoom = z;
            }
            _ => self.fit_pending = true,
        }
        self.layout_changed();
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
        // The rules page says which rules are connected now.
        self.refresh_rules_page();
        self.export_soon();
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
    /// (a vertical cascade, not RaySession's full repulse).
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
                // Vertical is a ports-view style: from the matrix view it goes to ports, vertical.
                let (m, v) = if self.layout.view.matrices { (false, true) } else { (false, !self.layout.view.vertical) };
                self.set_style(m, v);
            }
            Action::SetStyle(m, v) => self.set_style(m, v),
            Action::PortsStyle => {
                // The Ports button: back to the ports view, or (already there) switch its flow.
                let v = &self.layout.view;
                let vertical = if v.matrices { v.vertical } else { !v.vertical };
                self.set_style(false, vertical);
            }
            Action::ToggleMatrices => {
                let m = !self.layout.view.matrices;
                let v = self.layout.view.vertical;
                self.set_style(m, v);
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
                let (m, vv) = (v.matrix_view, v.vertical && !v.matrix_view);
                if (m, vv) != (self.layout.view.matrices, self.layout.view.vertical) {
                    self.set_style(m, vv);
                }
                // Blocks the view doesn't know keep their place.
                self.layout.nodes.extend(v.nodes);
                self.layout.matrices.extend(v.matrices);
                self.layout.hidden = v.hidden;
                self.layout.expanded = v.expanded;
                self.layout.view.cam = v.cam;
                self.layout.view.zoom = v.zoom;
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
                let undo = (self.history.can_undo(), self.history.can_redo());
                let export = self.layout.view.export_names;
                self.ui.open_app_menu(r, labels, sidebar, ui_scale, grid, snap, undo, export)
            }
            Action::SetBufferSize(n) => {
                if let Some(c) = &self.jack
                    && !c.set_buffer_size(n) {
                        self.say(format!("JACK refused a buffer size of {n}"), true);
                    }
            }
            Action::ResetXruns => {
                self.stats.xruns = 0;
                self.events.reset_xruns();
                self.refresh_log_page();
            }
            Action::EventLog if self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::Log) => self.dialog = None,
            Action::EventLog => self.open_log(0),
            Action::ApplyPreset(name) => match self.presets.get(&name).cloned() {
                Some(s) => {
                    self.say(format!("Preset {name}: putting it in place…"), false);
                    self.server.run(crate::server::Cmd::ApplySettings(Box::new(s), name));
                }
                None => self.say(format!("No preset {name}"), true),
            },
            Action::PresetMenu(r) => {
                use crate::ui::MenuItem;
                let now = self.jack_now.as_ref().and_then(|n| crate::presets::matching(&self.presets, n)).map(str::to_string);
                let mut items: Vec<MenuItem> = self.presets.keys().map(|n| MenuItem::new(n.clone(), Action::ApplyPreset(n.clone())).checked(now.as_deref() == Some(n))).collect();
                if items.is_empty() {
                    items.push(MenuItem::header("No presets yet"));
                }
                items.push(MenuItem::new("Save the current settings as a preset…", Action::SavePresetPrompt).enabled(self.jack_now.is_some()));
                items.push(MenuItem::new("Presets on the settings page", Action::ServerSettings));
                self.ui.open_menu([r.x, r.bottom() + 2.0], items);
            }
            Action::Align(how) => self.align(how),
            Action::Noop => {}
            Action::GroupPrompt(port) => {
                use crate::dialog::{Control, Dialog, Kind, Row};
                let Some(&p) = self.graph.by_name.get(&port) else { return };
                let avail = self.port_run(p).len();
                if avail < 2 {
                    return self.say("No ports after this one to group it with", true);
                }
                let mut d = Dialog::new(Kind::PortGroup, "Group ports");
                d.tag = port.clone();
                let sizes: Vec<(String, String)> = [2usize, 3, 4, 5, 6, 8, 16].iter().filter(|&&n| n <= avail).map(|n| (n.to_string(), format!("{n} ports"))).collect();
                d.rows = vec![
                    Row { id: "name".into(), label: "Name".into(), help: String::new(), control: Control::Text(if avail >= 2 { "Stereo".into() } else { String::new() }), modified: false },
                    Row { id: "count".into(), label: "Ports, from this one".into(), help: "This port and the ones after it in its block, of its direction and type".into(), control: Control::Choice { value: "2".into(), options: sizes, editable: false }, modified: false },
                ];
                d.buttons = vec!["Cancel".into(), "Group".into()];
                d.note = format!("From {port}: they connect as one, 1:1 (Ctrl+drag one port alone)");
                d.edit("name");
                self.dialog = Some(d);
            }
            Action::Tracks(client, on) => {
                if on {
                    self.layout.tracks.insert(client.clone());
                } else {
                    self.layout.tracks.remove(&client);
                }
                self.remodel();
                self.place_new_nodes(false);
                // The new blocks (and the client's own, back in one) push the others aside.
                let prefix = format!("M:{client}");
                let mine: Vec<usize> = (0..self.model.nodes.len()).filter(|&i| self.model.nodes[i].key.id().starts_with(&prefix)).collect();
                let was = self.layout.view.no_overlap;
                self.layout.view.no_overlap = true;
                for i in mine {
                    self.repulse(i);
                }
                self.layout.view.no_overlap = was;
                self.refresh_matrices();
            }
            Action::RenameGroup(port) => {
                use crate::dialog::{Control, Dialog, Kind, Row};
                let Some(g) = self.layout.port_groups.iter().find(|g| g.ports.contains(&port)) else { return };
                let mut d = Dialog::new(Kind::GroupName, "Rename the group");
                d.tag = port.clone();
                d.rows = vec![Row { id: "name".into(), label: "Name".into(), help: String::new(), control: Control::Text(g.name.clone()), modified: false }];
                d.buttons = vec!["Cancel".into(), "Rename".into()];
                d.edit("name");
                self.dialog = Some(d);
            }
            Action::Ungroup(port) => {
                let before = self.layout.port_groups.len();
                self.layout.port_groups.retain(|g| !g.ports.contains(&port));
                if self.layout.port_groups.len() != before {
                    self.remodel();
                }
            }
            Action::ToggleExportNames => {
                self.layout.view.export_names = !self.layout.view.export_names;
                self.layout_changed();
                self.export_soon();
            }
            Action::Find => {
                self.search = if self.search.is_some() { None } else { Some(Default::default()) };
                self.redraw = true;
            }
            Action::SearchGo(k) => {
                if let Some((_, _, a)) = self.search_results().into_iter().nth(k) {
                    self.search = None;
                    self.apply(a);
                }
            }
            Action::FindPort(name) => {
                let Some(&p) = self.graph.by_name.get(&name) else { return };
                let kind = self.graph.ports[p].kind;
                if kind != self.kind && self.kinds().contains(&kind) {
                    self.apply(Action::SetKind(kind));
                }
                if let Some(&ni) = self.model.port_node.get(&p) {
                    let id = self.model.nodes[ni].key.id();
                    self.apply(Action::CenterNode(id));
                }
                self.canvas.flash_port = Some((name, Instant::now()));
                self.redraw = true;
            }
            Action::HideSelected => {
                let ids: Vec<String> = std::mem::take(&mut self.canvas.selected).into_iter().collect();
                for id in ids {
                    self.apply(Action::Hide(id));
                }
            }
            Action::SelectAll => {
                self.canvas.selected = (0..self.model.nodes.len()).filter(|&i| !self.layout.hidden.contains(&self.model.nodes[i].key.id())).map(|i| self.model.nodes[i].key.id()).collect();
                self.redraw = true;
            }
            Action::ClearSelection => {
                self.canvas.selected.clear();
                self.redraw = true;
            }
            Action::RulesPage if self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::Rules) => self.dialog = None,
            Action::RulesPage => {
                let mut d = crate::dialog::Dialog::new(crate::dialog::Kind::Rules, "Connection rules");
                d.buttons = vec!["Close".into()];
                d.note = "patchboxd keeps these connections whenever both ports are there, session or not.".into();
                self.dialog = Some(d);
                self.sess.send(&pbx_session::proto::Request::Rules);
                self.refresh_rules_page();
            }
            Action::Rule(o, i, on) => {
                let mut rules = self.rules.clone();
                let r = pbx_session::rules::Rule { out: o, inp: i };
                if on {
                    rules.push(r);
                } else {
                    rules.retain(|x| *x != r);
                }
                self.send_rules(rules);
            }
            Action::RulesFromNode(id) => {
                let Some(n) = self.model.node_by_id(&id) else { return };
                let mine: std::collections::HashSet<usize> = self.model.nodes[n].ins.iter().chain(&self.model.nodes[n].outs).copied().collect();
                let mut rules = self.rules.clone();
                let before = rules.len();
                for &(o, i) in &self.graph.conns {
                    if mine.contains(&o) || mine.contains(&i) {
                        let r = pbx_session::rules::Rule { out: self.graph.ports[o].name.clone(), inp: self.graph.ports[i].name.clone() };
                        if !rules.contains(&r) {
                            rules.push(r);
                        }
                    }
                }
                let added = rules.len() - before;
                self.say(format!("{added} connection rule{} added", if added == 1 { "" } else { "s" }), false);
                self.send_rules(rules);
            }
            Action::SavePresetPrompt => {
                use crate::dialog::{Control, Dialog, Kind, Row};
                let mut d = Dialog::new(Kind::SavePreset, "Save the JACK settings as a preset");
                d.rows = vec![Row { id: "name".into(), label: "Name".into(), help: String::new(), control: Control::Text(String::new()), modified: false }];
                d.buttons = vec!["Cancel".into(), "Save".into()];
                d.note = self.jack_now.as_ref().map(crate::presets::summary).unwrap_or_default();
                d.edit("name");
                self.dialog = Some(d);
            }
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
            Action::ServerSettings if self.dialog.as_ref().is_some_and(|d| matches!(d.kind, crate::dialog::Kind::Server | crate::dialog::Kind::PipeWire)) => self.dialog = None,
            Action::ServerSettings => {
                // Without jackdbus the PipeWire tab is the one that matters (pure PipeWire).
                let tab = if self.server.status.available { self.settings_tab } else { TAB_PIPEWIRE };
                self.open_settings(tab);
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
                // PipeWire's graph has its own cameras, one per view style (its file's; the graph
                // is fitted for a style it has none for yet).
                let pw_cams = pw_canvas.cams();
                // and its own view style (ports, vertical, matrices): JACK's stays JACK's
                self.jack_canvas = Some(self.layout.clone());
                self.layout.take_graph_canvas(pw_canvas);
                if !self.layout.set_cams(pw_cams) {
                    self.fit_pending = true;
                }
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
                    // JACK's cameras back as they were left, the current style's shown (none for
                    // it yet, the style having changed meanwhile: fitted).
                    let jack_cams = j.cams();
                    self.layout.take_graph_canvas(j);
                    if !self.layout.set_cams(jack_cams) {
                        self.fit_pending = true;
                    }
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
                self.export_soon();
            }
            Action::Sess(a) => self.session_action(a),
            Action::Quit => self.quit = true,
        }
    }

    // ---- JACK server settings (jackdbus) ----

    /// `p` and the ports after it in its block, of its direction and type.
    pub fn port_run(&self, p: usize) -> Vec<String> {
        let Some(&ni) = self.model.port_node.get(&p) else { return Vec::new() };
        let node = &self.model.nodes[ni];
        let pp = &self.graph.ports[p];
        let list: Vec<usize> = (if pp.is_input() { &node.ins } else { &node.outs }).iter().copied().filter(|&q| self.graph.ports[q].kind == pp.kind).collect();
        let at = list.iter().position(|&q| q == p).unwrap_or(0);
        list.into_iter().skip(at).map(|q| self.graph.ports[q].name.clone()).collect()
    }

    /// The model again after the layout's names or groups changed.
    pub fn remodel(&mut self) {
        let own = self.jack.as_ref().map(|c| c.name().to_string()).unwrap_or_default();
        self.model = Model::build(&self.graph, &self.layout, if self.source == Source::Jack { &own } else { "" });
        self.refresh_matrices();
        self.layout_changed();
    }

    /// Send the rules to patchboxd (it saves, applies and sends them back).
    pub fn send_rules(&mut self, rules: Vec<pbx_session::rules::Rule>) {
        if !self.sess.connected {
            return self.say("The session daemon isn't running: rules can't be kept", true);
        }
        self.sess.send(&pbx_session::proto::Request::SetRules { rules });
    }

    /// Does a rule ask for this connection?
    pub fn rule_wants(&self, o: &str, i: &str) -> bool {
        self.rules.iter().any(|r| pbx_session::rules::glob(&r.out, o) && pbx_session::rules::glob(&r.inp, i))
    }

    pub fn refresh_rules_page(&mut self) {
        use crate::dialog::{Control, Row};
        if !self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::Rules) {
            return;
        }
        let mut rows = vec![Row::header("Rules")];
        if self.rules.is_empty() {
            rows.push(Row::info("None yet", "right-click a wire or a block, or add one below"));
        }
        for (k, r) in self.rules.iter().enumerate() {
            let present = self.graph.conns.iter().any(|&(o, i)| pbx_session::rules::glob(&r.out, &self.graph.ports[o].name) && pbx_session::rules::glob(&r.inp, &self.graph.ports[i].name));
            rows.push(Row {
                id: format!("rule:del:{k}"),
                label: r.out.clone(),
                help: format!("{} → {}{}; click to remove the rule (the connection stays)", r.out, r.inp, if present { " (connected now)" } else { "" }),
                control: Control::Link(format!("→ {}{} · remove", r.inp, if present { "" } else { "  (not now)" })),
                modified: false,
            });
        }
        rows.push(Row::header("Add a rule"));
        let (o, i) = self.dialog.as_ref().map(|d| (d.value("rule:out").unwrap_or_default(), d.value("rule:in").unwrap_or_default())).unwrap_or_default();
        rows.push(Row { id: "rule:out".into(), label: "Output".into(), help: "A port's full name, e.g. PipeWire:capture_FL; * matches any run of characters (PipeWire*:capture_FL)".into(), control: Control::Text(o), modified: false });
        rows.push(Row { id: "rule:in".into(), label: "Input".into(), help: "A port's full name, e.g. system:playback_1; * matches any run of characters".into(), control: Control::Text(i), modified: false });
        rows.push(Row { id: "rule:add".into(), label: "Keep this connection".into(), help: String::new(), control: Control::Link("add".into()), modified: false });
        if let Some(d) = &mut self.dialog {
            d.set_rows(rows);
        }
    }

    /// The event log page: tab 0 our events, 1 jackdbus's log.
    pub fn open_log(&mut self, tab: usize) {
        let mut d = crate::dialog::Dialog::new(crate::dialog::Kind::Log, "Log");
        d.tabs = vec!["Events".into(), "jackdbus log".into()];
        d.tab = tab;
        d.buttons = vec!["Close".into()];
        self.dialog = Some(d);
        self.refresh_log_page();
    }

    /// Rebuild the log page's rows, when it shows and something changed.
    pub fn refresh_log_page(&mut self) {
        let Some(tab) = self.dialog.as_ref().filter(|d| d.kind == crate::dialog::Kind::Log).map(|d| d.tab) else { return };
        let (rows, note) = if tab == 0 { (self.event_rows(), "Newest first; the last 5000 are kept while PatchBox runs.".to_string()) } else { self.jackdbus_rows() };
        self.events.dirty = false;
        if let Some(d) = &mut self.dialog {
            d.set_rows(rows);
            d.note = note;
        }
    }

    fn event_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::{Control, Row};
        use crate::events::{FILTERS, clock, shows};
        let ev = &self.events;
        let mut rows = vec![Row::header("Xruns")];
        let server = self.server.stats.as_ref().map(|s| format!(" · the server counts {}", s.xruns)).unwrap_or_default();
        rows.push(Row::info("Since", format!("{}: {} xrun{}{server}", clock(ev.since).split('.').next().unwrap_or(""), ev.xruns, if ev.xruns == 1 { "" } else { "s" })));
        if let Some(last) = ev.list.iter().rev().find(|e| e.kind == crate::events::Kind::Xrun) {
            rows.push(Row::info("Last", format!("{}  {}", clock(last.at), last.text)));
        }
        rows.push(Row { id: "act:reset".into(), label: "Reset the count".into(), help: "Here, in the toolbar, and the server's own count (jackdbus)".into(), control: Control::Link("reset".into()), modified: false });
        rows.push(Row::header("Events"));
        rows.push(Row {
            id: "filter".into(),
            label: "Show".into(),
            help: "Which events the list shows".into(),
            control: Control::Choice { value: ev.filter.clone(), options: FILTERS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect(), editable: false },
            modified: false,
        });
        let mut n = 0;
        for e in ev.list.iter().rev().filter(|e| shows(&ev.filter, e.kind)) {
            if n == 400 {
                rows.push(Row::info("…", "older ones are kept but not listed"));
                break;
            }
            n += 1;
            let mark = if e.bad { "! " } else { "" };
            rows.push(Row { id: String::new(), label: format!("{}  {}", clock(e.at), e.kind.label()), help: e.text.clone(), control: Control::Info(format!("{mark}{}", e.text)), modified: false });
        }
        if n == 0 {
            rows.push(Row::info("Nothing yet", ""));
        }
        rows
    }

    fn jackdbus_rows(&self) -> (Vec<crate::dialog::Row>, String) {
        use crate::dialog::Row;
        let mut rows = vec![Row::header("jackdbus's log")];
        let Some(path) = crate::events::jackdbus_log() else { return (rows, String::new()) };
        match crate::events::tail(&path, 96 * 1024, 300) {
            Ok((lines, size)) => {
                rows.push(Row::info("File", format!("{} · {}", path.display(), crate::events::size(size))));
                rows.push(Row::header("Newest first"));
                for l in lines.iter().rev() {
                    let (when, what) = crate::events::split_jackdbus(l);
                    rows.push(Row { id: String::new(), label: when, help: what.clone(), control: crate::dialog::Control::Info(what), modified: false });
                }
                (rows, "The end of jackdbus's own log; it grows without end (nothing rotates it).".into())
            }
            Err(e) => {
                rows.push(Row::info("File", format!("{}: {e}", path.display())));
                (rows, String::new())
            }
        }
    }

    /// The ALSA capture and playback devices to reset when the device changes to `new`: those
    /// that name the old device's card would keep it (they override the device), so they
    /// follow the new one. Set to another card, they are a deliberate split and stay.
    fn device_followers(&self, new: &str) -> Vec<Vec<String>> {
        let Some(cfg) = self.server.config.as_ref() else { return Vec::new() };
        let find = |n: &str| cfg.all_params().find(|p| p.path.len() == 2 && p.path[0] == "driver" && p.path[1] == n);
        let Some(old) = find("device").map(|p| p.value.show()) else { return Vec::new() };
        let Some(card) = alsa_card(&old) else { return Vec::new() };
        if alsa_card(new).as_deref() == Some(card.as_str()) {
            return Vec::new();
        }
        ["capture", "playback"]
            .iter()
            .filter_map(|n| find(n))
            .filter(|p| p.is_set && alsa_card(&p.value.show()).as_deref() == Some(card.as_str()))
            .map(|p| p.path.clone())
            .collect()
    }

    /// The settings page, on one of the JACK tabs (jackdbus) or PipeWire's.
    pub fn open_settings(&mut self, tab: usize) {
        use crate::dialog::{Dialog, Kind};
        self.settings_tab = tab.min(TAB_PIPEWIRE);
        let tab = self.settings_tab;
        let mut d = if tab < TAB_PIPEWIRE {
            self.server.send(crate::server::Cmd::LoadConfig);
            if self.server.status.started {
                self.server.send(crate::server::Cmd::Stats);
            }
            let mut d = Dialog::new(Kind::Server, "Settings");
            d.note = tab_note(tab).into();
            d.message = tab_message(tab).into();
            d.set_rows(self.server_rows());
            d.buttons = self.server_buttons();
            d
        } else {
            if self.pw.is_none() {
                match crate::pw::PwLink::start(self.waker.clone()) {
                    Ok(p) => self.pw = Some(p),
                    Err(e) => self.say(format!("PipeWire: {e}"), true),
                }
            }
            let mut d = Dialog::new(Kind::PipeWire, "Settings");
            d.set_rows(self.pw_rows());
            d.note = self.pw_note();
            d.buttons = vec!["Close".into()];
            d
        };
        d.tabs = JACK_TABS.iter().map(|t| t.to_string()).chain(["PipeWire".to_string()]).collect();
        d.tab = tab;
        self.dialog = Some(d);
    }

    fn refresh_pw_page(&mut self) {
        if self.dialog.as_ref().is_some_and(|d| d.kind == crate::dialog::Kind::PipeWire) {
            let (rows, note) = (self.pw_rows(), self.pw_note());
            if let Some(d) = &mut self.dialog {
                d.set_rows(rows);
                d.note = note;
            }
        }
    }

    fn pw_note(&self) -> String {
        match &self.pw_state {
            Some(st) if st.bridge.is_some() => "PipeWire runs on top of JACK here: its clock follows JACK's (the JACK tab).".into(),
            Some(_) => "Clock settings apply now and last until PipeWire restarts, unless kept.".into(),
            None => String::new(),
        }
    }

    /// The PipeWire tab: what runs, the clock, the JACK bridge, default devices, device
    /// profiles, WirePlumber's settings. Built from the pw-dump stream; adapts to PipeWire on
    /// top of JACK (the clock is JACK's) or PipeWire alone.
    pub fn pw_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::{Control, Row};
        let mut rows = vec![Row::header("PipeWire")];
        let Some(st) = &self.pw_state else {
            let why = if self.pw.is_some() { "reading…" } else { "not running (pw-dump could not reach it)" };
            rows.push(Row::info("Status", why));
            return rows;
        };
        let bridged = st.bridge.is_some();
        let mode = if bridged {
            "on top of JACK (the bridge drives its graph)"
        } else if st.serves_jack {
            "the JACK server too (pipewire-jack)"
        } else {
            "beside JACK"
        };
        rows.push(Row::info("Status", format!("{} · {mode}", if st.version.is_empty() { "running".to_string() } else { st.version.clone() })));
        let info = |id: &str, label: &str, text: String, help: &str| Row { id: id.into(), label: label.into(), help: help.into(), control: Control::Info(text), modified: false };
        let set = |k: &str| st.settings.get(k).cloned().unwrap_or_default();

        // The clock: PipeWire's own when it is alone; JACK's when it runs on top of it.
        rows.push(Row::header("Clock"));
        let clock_help = if bridged { "Follows JACK: the bridge drives PipeWire's graph at JACK's rate and buffer size" } else { "" };
        if bridged {
            // The bridge runs PipeWire's graph one JACK cycle at a time: its rate and quantum
            // are JACK's (the `settings` metadata only holds what PipeWire uses on its own)
            let (rate, frames) = (self.stats.sample_rate, self.stats.buffer_size);
            let rate = if rate > 0 { format!("{rate} Hz") } else { format!("{} Hz", set("clock.rate")) };
            let quantum = if frames > 0 { format!("{frames} frames (JACK's buffer size)") } else { "JACK's buffer size".to_string() };
            rows.push(info("", "Rate", rate, clock_help));
            rows.push(info("", "Quantum", quantum, clock_help));
            rows.push(info(
                "",
                "Default quantum",
                format!("{} frames (when PipeWire runs alone)", set("clock.quantum")),
                "default.clock.quantum: the buffer size PipeWire uses when it drives its own graph, not while JACK drives it",
            ));
        } else {
            let rates = [44100u32, 48000, 88200, 96000, 176400, 192000];
            let quanta = [16u32, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192];
            let choice = |id: &str, label: &str, help: &str, opts: Vec<(String, String)>, editable: bool| {
                let value = set(id.trim_start_matches("settings:"));
                Row { id: id.into(), label: label.into(), help: help.into(), control: Control::Choice { value, options: opts, editable }, modified: false }
            };
            rows.push(choice("settings:clock.rate", "Default rate", "The graph's rate when nothing asks for another (default.clock.rate)", rates.iter().map(|r| (r.to_string(), format!("{r} Hz"))).collect(), true));
            rows.push(Row {
                id: "settings:clock.allowed-rates".into(),
                label: "Allowed rates".into(),
                help: "Rates the graph may switch to for a stream, e.g. [ 44100 48000 ] (default.clock.allowed-rates)".into(),
                control: Control::Text(set("clock.allowed-rates")),
                modified: false,
            });
            rows.push(choice("settings:clock.quantum", "Default quantum", "The buffer size, frames, when no stream asks for another (default.clock.quantum)", quanta.iter().map(|q| (q.to_string(), format!("{q} frames"))).collect(), true));
            rows.push(choice("settings:clock.min-quantum", "Smallest quantum", "The smallest buffer a stream may ask for (default.clock.min-quantum)", quanta.iter().map(|q| (q.to_string(), format!("{q} frames"))).collect(), true));
            rows.push(choice("settings:clock.max-quantum", "Largest quantum", "The largest buffer a stream may ask for (default.clock.max-quantum)", quanta.iter().map(|q| (q.to_string(), format!("{q} frames"))).collect(), true));
            let mut fr = vec![("0".to_string(), "not forced".to_string())];
            fr.extend(rates.iter().map(|r| (r.to_string(), format!("{r} Hz"))));
            let mut fq = vec![("0".to_string(), "not forced".to_string())];
            fq.extend(quanta.iter().map(|q| (q.to_string(), format!("{q} frames"))));
            let mut r = choice("settings:clock.force-rate", "Force rate", "Run the graph at this rate whatever the streams ask (until PipeWire restarts)", fr, false);
            r.modified = set("clock.force-rate") != "0" && !set("clock.force-rate").is_empty();
            rows.push(r);
            let mut r = choice("settings:clock.force-quantum", "Force quantum", "Run the graph with this buffer size whatever the streams ask (until PipeWire restarts)", fq, false);
            r.modified = set("clock.force-quantum") != "0" && !set("clock.force-quantum").is_empty();
            rows.push(r);
            rows.push(Row {
                id: "act:keep-clock".into(),
                label: "Keep the defaults".into(),
                help: format!("Write the default rate, allowed rates and quanta to {} so PipeWire starts with them", crate::pwconf::clock_dropin().map(|p| p.display().to_string()).unwrap_or_default()),
                control: Control::Link("write the drop-in".into()),
                modified: false,
            });
        }
        let levels = ["none", "errors", "warnings", "info", "debug", "trace"];
        rows.push(Row {
            id: "settings:log.level".into(),
            label: "Log level".into(),
            help: "How much PipeWire's daemon logs (until it restarts)".into(),
            control: Control::Choice { value: set("log.level"), options: levels.iter().enumerate().map(|(i, l)| (i.to_string(), format!("{i} · {l}"))).collect(), editable: false },
            modified: false,
        });

        // The JACK bridge (module-jackdbus-detect / module-jack-tunnel).
        if let Some(b) = &st.bridge {
            rows.push(Row::header("JACK bridge"));
            let what = if b.module == "jackdbus-detect" { "module-jackdbus-detect: follows jackdbus, appears as the PipeWire JACK client" } else { "module-jack-tunnel" };
            rows.push(info("", "Module", what.into(), ""));
            for (name, desc, audio, midi, drives) in &b.nodes {
                let mut text = format!("{audio} audio");
                if *midi > 0 {
                    text.push_str(&format!(" + {midi} MIDI"));
                }
                if *drives {
                    text.push_str(" · drives PipeWire's graph");
                }
                rows.push(info("", desc, text, name));
            }
            match crate::pwconf::bridge_conf(&b.module) {
                Some(p) => rows.push(Row {
                    id: format!("act:open:{}", p.display()),
                    label: "Configured in".into(),
                    help: "Channels, MIDI ports, auto-connect and the tunnel mode are the module's arguments there; they apply when PipeWire restarts".into(),
                    control: Control::Link(p.display().to_string()),
                    modified: false,
                }),
                None => rows.push(info("", "Configured in", "PipeWire's defaults".into(), "")),
            }
        }

        // Default devices (the session manager's).
        if let Some(defs) = &st.defaults {
            rows.push(Row::header("Default devices"));
            for (kind, label, list) in [("sink", "Output", &st.sinks), ("source", "Input", &st.sources)] {
                let configured = defs.get(&format!("default.configured.audio.{kind}")).cloned();
                let now = defs.get(&format!("default.audio.{kind}")).cloned();
                let value = configured.clone().or(now.clone()).unwrap_or_default();
                let mut options: Vec<(String, String)> = list.iter().map(|(n, d)| (n.clone(), d.clone())).collect();
                if !value.is_empty() && !options.iter().any(|(n, _)| *n == value) {
                    options.push((value.clone(), format!("{value} (not a{} {kind})", if kind == "input" { "n" } else { "" })));
                }
                let desc_of = |n: &str| list.iter().find(|(x, _)| x == n).map(|(_, d)| d.clone()).unwrap_or_else(|| n.to_string());
                let help = match (&configured, &now) {
                    (Some(c), Some(n)) if c != n => format!("Chosen: {c}; in use: {} (the chosen one is missing or not a {kind})", desc_of(n)),
                    _ => format!("Where applications' {} goes unless they ask for another", if kind == "sink" { "output" } else { "input comes from" }),
                };
                rows.push(Row { id: format!("default:{kind}"), label: label.into(), help, control: Control::Choice { value, options, editable: false }, modified: false });
                if let (Some(c), Some(n)) = (&configured, &now)
                    && c != n
                {
                    rows.push(info("", "  in use", desc_of(n), "The session manager uses this one: the chosen device is missing or not of this kind"));
                }
            }
        }

        // Device profiles.
        if !st.devices.is_empty() {
            rows.push(Row::header("Devices"));
            for d in &st.devices {
                let options: Vec<(String, String)> = d.profiles.iter().map(|(i, n, desc, avail)| (i.to_string(), format!("{}{}", if desc.is_empty() { n } else { desc }, if *avail { "" } else { " (unavailable)" }))).collect();
                rows.push(Row {
                    id: format!("device:{}", d.id),
                    label: d.description.clone(),
                    help: format!("{}: its profile (which inputs and outputs it offers); off frees it, e.g. for JACK", d.name),
                    control: Control::Choice { value: d.current.map(|c| c.to_string()).unwrap_or_default(), options, editable: false },
                    modified: false,
                });
            }
        }

        // WirePlumber's settings, from its schema.
        if !st.sm.is_empty() {
            rows.push(Row::header("Session manager (WirePlumber)"));
            for x in &st.sm {
                let v = x.value.clone().unwrap_or_else(|| x.default.clone());
                let shown = serde_json::from_str::<serde_json::Value>(&v).ok().map(|j| match j {
                    serde_json::Value::String(t) => t,
                    j => j.to_string(),
                });
                let control = match x.ty.as_str() {
                    "bool" => Control::Toggle(v == "true"),
                    "int" => Control::Number { value: v.parse().unwrap_or(0), min: x.min.unwrap_or(i64::MIN as f64) as i64, max: x.max.unwrap_or(i64::MAX as f64) as i64 },
                    _ => Control::Text(shown.unwrap_or(v.clone())),
                };
                let mut help = x.description.clone();
                if let (Some(a), Some(b)) = (x.min, x.max) {
                    help.push_str(&format!(" ({a} to {b})"));
                }
                rows.push(Row { id: format!("sm:{}", x.key), label: x.name.clone(), help: format!("{help} (default: {})", x.default.trim_matches('"')), control, modified: x.value.as_ref().is_some_and(|v| *v != x.default) });
            }
        }

        rows.push(Row::header("Service"));
        rows.push(Row {
            id: "act:restart-pw".into(),
            label: "Restart PipeWire".into(),
            help: "systemctl --user restart pipewire: WirePlumber and pipewire-pulse restart with it; on top of JACK the bridge reconnects".into(),
            control: Control::Link("restart".into()),
            modified: false,
        });
        rows
    }

    /// A change on the PipeWire tab.
    fn pw_set(&mut self, id: &str, v: &str) {
        let Some(st) = self.pw_state.clone() else { return };
        if let Some(path) = id.strip_prefix("act:open:") {
            let _ = std::process::Command::new("xdg-open").arg(path).stdin(std::process::Stdio::null()).spawn();
            return;
        }
        if id == "act:restart-pw" {
            self.say("Restarting PipeWire…", false);
            let _ = std::process::Command::new("systemctl").args(["--user", "restart", "pipewire.service"]).stdin(std::process::Stdio::null()).spawn();
            return;
        }
        if id == "act:keep-clock" {
            match crate::pwconf::write_clock_dropin(&st.settings) {
                Ok(p) => self.say(format!("Clock defaults kept in {}", p.display()), false),
                Err(e) => self.say(format!("Could not write the drop-in: {e}"), true),
            }
            return;
        }
        let Some(pw) = &self.pw else { return };
        if let Some(key) = id.strip_prefix("settings:") {
            let v = v.trim();
            let ok = match key {
                "clock.allowed-rates" => v.starts_with('['),
                _ => v.parse::<u32>().is_ok(),
            };
            if ok {
                pw.set_meta("settings", key, Some(v), None);
            } else {
                self.say(format!("{key}: not a valid value: {v}"), true);
            }
        } else if let Some(kind) = id.strip_prefix("default:") {
            let json = serde_json::json!({ "name": v }).to_string();
            pw.set_meta("default", &format!("default.configured.audio.{kind}"), Some(&json), Some("Spa:String:JSON"));
        } else if let Some(dev) = id.strip_prefix("device:") {
            if let (Ok(d), Ok(i)) = (dev.parse(), v.parse()) {
                pw.set_profile(d, i);
            }
        } else if let Some(key) = id.strip_prefix("sm:")
            && let Some(x) = st.sm.iter().find(|x| x.key == key)
        {
            let json = match (x.ty.as_str(), v) {
                ("bool", "on") => Some("true".to_string()),
                ("bool", "off") => Some("false".to_string()),
                ("int", n) => n.trim().parse::<i64>().ok().map(|n| n.to_string()),
                ("float", n) => n.trim().parse::<f64>().ok().filter(|f| x.min.is_none_or(|m| *f >= m) && x.max.is_none_or(|m| *f <= m)).map(|f| f.to_string()),
                (_, t) => Some(serde_json::Value::String(t.trim().to_string()).to_string()),
            };
            match json {
                Some(j) => {
                    pw.set_meta("sm-settings", key, Some(&j), Some("Spa:String:JSON"));
                    if st.sm_persistent {
                        pw.set_meta("persistent-sm-settings", key, Some(&j), Some("Spa:String:JSON"));
                    }
                }
                None => self.say(format!("{}: not a valid value: {v}", x.name), true),
            }
        }
    }

    fn pw_reset(&mut self, id: &str) {
        let Some(st) = self.pw_state.clone() else { return };
        let Some(pw) = &self.pw else { return };
        if let Some(key) = id.strip_prefix("settings:") {
            pw.set_meta("settings", key, Some("0"), None);
        } else if let Some(key) = id.strip_prefix("sm:")
            && let Some(x) = st.sm.iter().find(|x| x.key == key)
        {
            pw.set_meta("sm-settings", key, Some(&x.default), Some("Spa:String:JSON"));
            if st.sm_persistent {
                pw.set_meta("persistent-sm-settings", key, None, None);
            }
        }
    }

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

    /// The JACK tabs' rows (the settings page's tab `settings_tab`).
    pub fn server_rows(&self) -> Vec<crate::dialog::Row> {
        match self.settings_tab {
            TAB_MASTER => self.master_rows(),
            TAB_BRIDGES => self.bridge_rows(),
            TAB_SLAVES => self.slave_rows(),
            TAB_NETJACK => self.netjack_rows(),
            TAB_INTERNALS => self.internal_rows(),
            _ => self.server_tab_rows(),
        }
    }

    /// The Server tab: the server's state and figures, presets, the engine, jackdbus itself.
    fn server_tab_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::{Control, Row};
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
        if st.started
            && let Some(sx) = &self.server.stats
        {
            rows.push(Row::info("Load · latency", format!("{:.1} % · {:.2} ms{}", sx.load, sx.latency_ms, if sx.realtime { " · realtime" } else { "" })));
            // Round trip through the hardware: a physical capture port's capture latency plus a
            // physical playback port's playback latency.
            if let Some(c) = &self.jack {
                let phys = |out: bool| self.graph.ports.iter().find(|p| p.is_physical() && p.is_output() == out && p.kind == pbx_jack::PortKind::Audio).map(|p| p.name.clone());
                if let (Some(cap), Some(play)) = (phys(true), phys(false))
                    && let (Some((c_in, _)), Some((_, p_out))) = (c.port_latency(&cap), c.port_latency(&play))
                {
                    let frames = c_in.max + p_out.max;
                    let ms = frames as f32 * 1000.0 / self.stats.sample_rate.max(1) as f32;
                    rows.push(Row {
                        id: String::new(),
                        label: "Round trip".into(),
                        help: format!("{cap} captures {} frames late, {play} plays {} frames later: what a signal through the computer and back out takes, the hardware's own converters aside", c_in.max, p_out.max),
                        control: Control::Info(format!("{frames} frames · {ms:.2} ms (capture {} + playback {})", c_in.max, p_out.max)),
                        modified: false,
                    });
                }
            }
            rows.push(link_row("act:reset-xruns", &format!("Server xruns: {}", sx.xruns), "reset them"));
        }
        // Presets: named configurations, applied in one click (also the sidebar's Preset menu).
        if st.available {
            rows.push(Row::header("Presets"));
            let now = self.jack_now.as_ref().and_then(|n| crate::presets::matching(&self.presets, n));
            if self.presets.is_empty() {
                rows.push(Row::info("None yet", "set the server up, then save it here"));
            }
            for (name, s) in &self.presets {
                rows.push(Row {
                    id: format!("preset:apply:{name}"),
                    label: if now == Some(name.as_str()) { format!("{name} (in place)") } else { name.clone() },
                    help: format!("Put {name} in place: the server restarts with it when something differs, except the buffer size, the CPU wake-up latency and the bridges, which change without a restart; parameters it doesn't set go back to their defaults"),
                    control: Control::Link(crate::presets::summary(s)),
                    modified: false,
                });
            }
            rows.push(Row {
                id: "preset:save".into(),
                label: "Save the current settings as".into(),
                help: "A name; Enter saves the driver and every parameter set away from its default (an existing name is replaced)".into(),
                control: Control::Text(String::new()),
                modified: false,
            });
            if !self.presets.is_empty() {
                rows.push(Row {
                    id: "preset:delete".into(),
                    label: "Delete a preset".into(),
                    help: "Choose the one to delete".into(),
                    control: Control::Choice { value: String::new(), options: self.presets.keys().map(|n| (n.clone(), n.clone())).collect(), editable: false },
                    modified: false,
                });
            }
        }
        let Some(cfg) = &self.server.config else {
            if st.available {
                rows.push(Row::info("Settings", "loading…"));
            }
            return rows;
        };
        rows.push(Row::header("Engine"));
        // The driver is the Master tab's, the slaves and bridges have their own tabs
        rows.extend(cfg.engine.iter().filter(|p| !matches!(p.name.as_str(), "driver" | "slave-drivers" | "bridges")).map(param_row));
        rows.push(Row::header("Service"));
        rows.push(link_row("act:exit", "Quit jackdbus", "stops the server; the session bus starts jackdbus again when asked"));
        rows
    }

    /// The Master tab: the driver the server runs on, and its parameters.
    fn master_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::{Control, Row};
        let Some(cfg) = &self.server.config else { return vec![Row::info("Settings", "loading…")] };
        let mut rows = vec![Row::header(format!("Master driver: {}", cfg.driver))];
        rows.push(Row {
            id: "engine/driver".into(),
            label: "Driver".into(),
            help: "The backend the server runs on: alsa for a sound card, dummy for none, net/netone for a netjack master's audio…".into(),
            control: Control::Choice { value: cfg.driver.clone(), options: master_drivers(cfg).map(|d| (d.clone(), d.clone())).collect(), editable: false },
            modified: false,
        });
        if self.server.status.started {
            rows.push(link_row("act:switch-master", "Switch master", "make the selected driver the running server's master, no restart"));
        }
        rows.extend(cfg.driver_params.iter().map(param_row));
        rows
    }

    /// The Bridges tab: every sound card but the master's, each with a switch (jack2dbus-nspa).
    fn bridge_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::bridges::{Spec, DEFAULT_NPERIODS, DEFAULT_PERIOD};
        use crate::dialog::{Control, Row};
        let Some(cfg) = &self.server.config else { return vec![Row::info("Settings", "loading…")] };
        let mut rows = Vec::new();
        let Some(param) = cfg.engine.iter().find(|p| p.name == "bridges") else {
            rows.push(Row::header("Sound card bridges"));
            rows.push(Row::info("Not available", "this jackdbus has no sound card bridges (jack2dbus-nspa has them)"));
            return rows;
        };
        let specs = crate::bridges::specs(&param.value.show());
        let started = self.server.status.started;
        let live: &[crate::server::Bridge] = self.server.stats.as_ref().and_then(|s| s.bridges.as_deref()).unwrap_or(&[]);
        let bridge_of = |spec: &str| live.iter().find(|b| b.spec == spec);
        let master_card = (cfg.driver == "alsa")
            .then(|| cfg.driver_params.iter().find(|p| p.name == "device").and_then(|p| alsa_card(&p.value.show())))
            .flatten();
        let mut shown = Vec::new();
        rows.push(Row::header("Sound cards"));
        for card in crate::bridges::cards() {
            let label = if card.name.is_empty() { card.id.clone() } else { format!("{} — {}", card.id, card.name) };
            if master_card.as_deref().is_some_and(|m| m == card.id.to_lowercase() || m == card.index.to_string()) {
                rows.push(Row { id: String::new(), label, help: "The master driver's card: the server's clock".into(), control: Control::Info("the master (Master tab)".into()), modified: false });
                continue;
            }
            let spec = specs.iter().find(|s| Spec::parse(s).is_card(&card));
            rows.push(Row {
                id: format!("bridge:card:{}", card.id),
                label,
                help: format!("Bridge {}: its ports join the graph as {}:capture_N / {}:playback_N, resampled to the master's clock (the desktop's sound server lets go of the card)", card.id, card.id, card.id),
                control: Control::Toggle(spec.is_some()),
                modified: false,
            });
            let Some(spec) = spec else { continue };
            shown.push(spec.clone());
            let sp = Spec::parse(spec);
            rows.push(Row {
                id: String::new(),
                label: "      state".into(),
                help: "Locking, then locked: channels capture/playback, the delay each way (as the ports report it) and the card's clock against the master's".into(),
                control: Control::Info(crate::bridges::state(bridge_of(spec), started, self.stats.sample_rate)),
                modified: false,
            });
            let period = sp.period.unwrap_or(DEFAULT_PERIOD);
            rows.push(Row {
                id: format!("bridge:period:{}", card.id),
                label: "      card period".into(),
                help: format!("The card's own period (default {DEFAULT_PERIOD} frames): shorter is less delay, longer is safer on a busy USB bus"),
                control: Control::Choice { value: period.to_string(), options: [32u32, 64, 128, 256, 512, 1024].iter().map(|p| (p.to_string(), format!("{p} frames"))).collect(), editable: false },
                modified: period != DEFAULT_PERIOD,
            });
            let nperiods = sp.nperiods.unwrap_or(DEFAULT_NPERIODS);
            rows.push(Row {
                id: format!("bridge:nperiods:{}", card.id),
                label: "      card periods".into(),
                help: format!("Periods in the card's buffer (default {DEFAULT_NPERIODS}); a USB card that only plays may want 3"),
                control: Control::Number { value: nperiods as i64, min: 2, max: 8 },
                modified: nperiods != DEFAULT_NPERIODS,
            });
            let wake = sp.wake.clone().unwrap_or_else(|| "auto".into());
            rows.push(Row {
                id: format!("bridge:wake:{}", card.id),
                label: "      wake-ups".into(),
                help: "How the card's thread wakes: auto (a timer on cards whose interrupts come late, as Intel HDA; the interrupt on USB), timer, or irq".into(),
                control: Control::Choice { value: wake.clone(), options: ["auto", "timer", "irq"].iter().map(|w| (w.to_string(), w.to_string())).collect(), editable: false },
                modified: wake != "auto",
            });
        }
        // Listed but not among the cards: unplugged ones, other devices of a card
        let others: Vec<&String> = specs.iter().filter(|s| !shown.contains(s)).collect();
        if !others.is_empty() {
            rows.push(Row::header("Also listed"));
            for spec in others {
                rows.push(Row {
                    id: format!("bridge:spec:{spec}"),
                    label: spec.clone(),
                    help: "Bridged whenever its card is there; × removes it from the list".into(),
                    control: Control::LinkRemove(crate::bridges::state(bridge_of(spec), started, self.stats.sample_rate)),
                    modified: false,
                });
            }
        }
        rows
    }

    /// The Slaves & MIDI tab: drivers that run beside the master (alsarawmidi for MIDI, a card
    /// in hardware sync with the master's).
    fn slave_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::{Control, Row};
        let Some(cfg) = &self.server.config else { return vec![Row::info("Settings", "loading…")] };
        let mut rows = vec![Row::header("Slave drivers")];
        for d in master_drivers(cfg).filter(|d| **d != cfg.driver) {
            rows.push(Row {
                id: format!("slave:{d}"),
                label: d.clone(),
                help: "Run this driver as a slave of the master driver".into(),
                control: Control::Toggle(cfg.slaves.contains(d)),
                modified: false,
            });
        }
        for (name, ps) in &cfg.slave_params {
            rows.push(Row::header(format!("Slave driver: {name}")));
            rows.extend(ps.iter().map(param_row));
        }
        rows
    }

    /// The Netjack tab: this server as a netjack master (netmanager), remote audio through a
    /// netadapter, and this machine as a netjack slave (the net driver as its master).
    fn netjack_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::Row;
        let Some(cfg) = &self.server.config else { return vec![Row::info("Settings", "loading…")] };
        let mut rows = Vec::new();
        for (name, header) in [("netmanager", "This server as a netjack master"), ("netadapter", "A remote server's audio, resampled (netadapter)")] {
            if cfg.internals.iter().any(|i| i == name) {
                rows.push(Row::header(header));
                rows.push(self.internal_row(name));
                if let Some(ps) = cfg.internal_params.get(name) {
                    rows.extend(ps.iter().map(param_row));
                }
            }
        }
        let nets: Vec<&String> = cfg.drivers.iter().filter(|d| *d == "net" || *d == "netone").collect();
        if !nets.is_empty() {
            rows.push(Row::header("This machine as a netjack slave"));
            for d in nets {
                if cfg.driver == *d {
                    rows.push(Row::info(d.clone(), "the master driver now (its parameters on the Master tab)"));
                } else {
                    rows.push(crate::dialog::Row {
                        id: format!("act:use-driver:{d}"),
                        label: format!("Use {d} as the master driver"),
                        help: "This machine's audio then comes from a netjack master on the network: at the next start, or at once with Switch master (Master tab)".into(),
                        control: crate::dialog::Control::Link("audio from a netjack master".into()),
                        modified: false,
                    });
                }
            }
        }
        if rows.is_empty() {
            rows.push(Row::info("Netjack", "this jackdbus offers no netjack drivers or internal clients"));
        }
        rows
    }

    /// The Internals tab: the other internal clients jackdbus can load into the server.
    fn internal_rows(&self) -> Vec<crate::dialog::Row> {
        use crate::dialog::Row;
        let Some(cfg) = &self.server.config else { return vec![Row::info("Settings", "loading…")] };
        let mut rows = vec![Row::header("Internal clients")];
        let list: Vec<&String> = cfg.internals.iter().filter(|i| *i != "netmanager" && *i != "netadapter").collect();
        if list.is_empty() {
            rows.push(Row::info("None", "jackdbus offers no other internal clients"));
        }
        for i in &list {
            rows.push(self.internal_row(i));
        }
        for (name, ps) in &cfg.internal_params {
            if ps.is_empty() || !list.contains(&name) {
                continue;
            }
            rows.push(Row::header(format!("Internal client: {name}")));
            rows.extend(ps.iter().map(param_row));
        }
        rows
    }

    fn internal_row(&self, name: &str) -> crate::dialog::Row {
        let help = match name {
            "audioadapter" => "A card on its own clock, resampled into the graph (the Bridges tab does this better: live, per card, hotplug)".to_string(),
            "netmanager" => "Let netjack slaves on the network join this server".to_string(),
            "netadapter" => "A netjack master's audio here, resampled (no network clock sync)".to_string(),
            _ => "Load this internal client into the running server".to_string(),
        };
        crate::dialog::Row {
            id: format!("internal:{name}"),
            label: name.to_string(),
            help: format!("{help} (jackdbus can't tell which are loaded; this shows what was loaded from here)"),
            control: crate::dialog::Control::Toggle(self.server.internals_loaded.contains(name)),
            modified: false,
        }
    }

    /// A Bridges tab row: a card's switch (`bridge:card:ID`), one of its options
    /// (`bridge:period|nperiods|wake:ID`, None: back to the default), a listed bridge removed
    /// (`bridge:spec:SPEC`).
    fn bridge_event(&mut self, id: &str, value: Option<&str>) {
        use crate::bridges::Spec;
        use crate::server::{Cmd, Value};
        let list = self.server.config.as_ref().and_then(|c| c.engine.iter().find(|p| p.name == "bridges")).map(|p| p.value.show()).unwrap_or_default();
        let specs = crate::bridges::specs(&list);
        let Some((what, key)) = id.strip_prefix("bridge:").and_then(|r| r.split_once(':')) else { return };
        match (what, value) {
            ("card", Some(v)) => self.server.run(if v == "on" { Cmd::AddBridge(key.to_string()) } else { Cmd::RemoveBridge(key.to_string()) }),
            ("spec", Some("remove")) => self.server.run(Cmd::RemoveBridge(key.to_string())),
            ("period" | "nperiods" | "wake", v) => {
                let Some(card) = crate::bridges::cards().into_iter().find(|c| c.id == key) else { return };
                let Some(i) = specs.iter().position(|s| Spec::parse(s).is_card(&card)) else { return };
                let mut sp = Spec::parse(&specs[i]);
                match what {
                    "period" => sp.period = v.and_then(|v| v.parse().ok()),
                    "nperiods" => sp.nperiods = v.and_then(|v| v.parse().ok()),
                    _ => sp.wake = v.map(str::to_string),
                }
                let mut next = specs.clone();
                next[i] = sp.to_spec();
                if next != specs {
                    // jackdbus replaces the bridge (the card opened again with its new options)
                    self.server.run(Cmd::Set(vec!["engine".into(), "bridges".into()], Value::Str(next.join(" "))));
                }
            }
            _ => {}
        }
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
            (Kind::Server, Event::Set(id, v)) if id.starts_with("preset:") => {
                if let Some(name) = id.strip_prefix("preset:apply:") {
                    self.apply(Action::ApplyPreset(name.to_string()));
                } else if id == "preset:save" && !v.trim().is_empty() {
                    self.preset_saving = Some(v.trim().to_string());
                    self.server.send(crate::server::Cmd::ReadSettings);
                } else if id == "preset:delete" && self.presets.remove(&v).is_some() {
                    match crate::presets::save(&self.presets) {
                        Ok(()) => self.say(format!("Preset {v} deleted"), false),
                        Err(e) => self.say(format!("Could not save the presets: {e}"), true),
                    }
                }
            }
            (Kind::Server, Event::Set(id, v)) if id.starts_with("bridge:") => self.bridge_event(&id, Some(&v)),
            (Kind::Server, Event::Reset(id)) if id.starts_with("bridge:") => self.bridge_event(&id, None),
            (Kind::Server, Event::Set(id, _)) if id.starts_with("act:use-driver:") => {
                let d = id.trim_start_matches("act:use-driver:").to_string();
                self.server.run(crate::server::Cmd::Set(vec!["engine".into(), "driver".into()], crate::server::Value::Str(d)));
            }
            (Kind::Server, Event::Set(id, text)) if id.starts_with("slave:") || id.starts_with("internal:") || id.starts_with("act:") => {
                use crate::server::Cmd;
                let on = text == "on";
                if let Some(n) = id.strip_prefix("slave:") {
                    self.server.run(Cmd::Slave(n.to_string(), on));
                } else if let Some(n) = id.strip_prefix("internal:") {
                    if on {
                        self.server.internals_loaded.insert(n.to_string());
                    } else {
                        self.server.internals_loaded.remove(n);
                    }
                    self.server.run(Cmd::Internal(n.to_string(), on));
                } else {
                    match id.as_str() {
                        "act:reset-xruns" => self.server.run(Cmd::ResetXruns),
                        "act:switch-master" => self.server.run(Cmd::SwitchMaster),
                        "act:exit" => self.server.run(Cmd::Exit),
                        _ => {}
                    }
                }
                self.server.send(Cmd::Stats);
            }
            (Kind::Server, Event::Set(id, text)) => {
                let path: Vec<String> = id.split('/').map(str::to_string).collect();
                let value = if id == "engine/driver" {
                    Some(crate::server::Value::Str(text.clone()))
                } else {
                    self.server
                        .config
                        .as_ref()
                        .and_then(|c| c.all_params().find(|p| p.path == path))
                        .and_then(|p| p.value.parse_like(&text))
                };
                match value {
                    Some(v) => {
                        let follow = if id == "driver/device" { self.device_followers(&v.show()) } else { Vec::new() };
                        self.server.run(crate::server::Cmd::Set(path, v));
                        for p in follow {
                            self.server.run(crate::server::Cmd::Reset(p));
                        }
                    }
                    None => self.say(format!("\"{text}\" is not a valid value for {id}"), true),
                }
            }
            (Kind::Server | Kind::PipeWire, Event::Tab(t)) => self.open_settings(t),
            (Kind::Log, Event::Tab(t)) => self.open_log(t),
            (Kind::Rules, Event::Set(id, v)) => {
                if let Some(k) = id.strip_prefix("rule:del:").and_then(|k| k.parse::<usize>().ok()) {
                    let mut rules = self.rules.clone();
                    if k < rules.len() {
                        rules.remove(k);
                        self.send_rules(rules);
                    }
                } else if id == "rule:add" {
                    let Some(d) = &self.dialog else { return };
                    let (o, i) = (d.value("rule:out").unwrap_or_default().trim().to_string(), d.value("rule:in").unwrap_or_default().trim().to_string());
                    if o.is_empty() || i.is_empty() {
                        return self.say("A rule needs an output and an input", true);
                    }
                    let mut rules = self.rules.clone();
                    rules.push(pbx_session::rules::Rule { out: o, inp: i });
                    if let Some(d) = &mut self.dialog {
                        d.set_local("rule:out", "");
                        d.set_local("rule:in", "");
                    }
                    self.send_rules(rules);
                } else if let Some(d) = &mut self.dialog {
                    d.set_local(&id, &v);
                }
            }
            (Kind::Rules, Event::Button(_)) => self.dialog = None,
            (Kind::GroupName, Event::Set(id, v)) => {
                if let Some(d) = &mut self.dialog {
                    d.set_local(&id, &v);
                }
            }
            (Kind::GroupName, Event::Button(b)) => {
                let Some(d) = self.dialog.take() else { return };
                let name = d.value("name").unwrap_or_default().trim().to_string();
                if d.buttons.get(b).map(String::as_str) == Some("Rename") && !name.is_empty()
                    && let Some(g) = self.layout.port_groups.iter_mut().find(|g| g.ports.contains(&d.tag))
                {
                    g.name = name;
                    self.remodel();
                }
            }
            (Kind::PortGroup, Event::Set(id, v)) => {
                if let Some(d) = &mut self.dialog {
                    d.set_local(&id, &v);
                }
            }
            (Kind::PortGroup, Event::Button(b)) => {
                let Some(d) = self.dialog.take() else { return };
                if d.buttons.get(b).map(String::as_str) != Some("Group") {
                    return;
                }
                let Some(&p) = self.graph.by_name.get(&d.tag) else { return };
                let n: usize = d.value("count").and_then(|v| v.parse().ok()).unwrap_or(2);
                let ports: Vec<String> = self.port_run(p).into_iter().take(n).collect();
                let name = d.value("name").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{n} ports"));
                // A port is in one group at most.
                self.layout.port_groups.retain(|g| !g.ports.iter().any(|q| ports.contains(q)));
                self.layout.port_groups.push(crate::layout::PortGroup { name, ports });
                self.remodel();
            }
            (Kind::SavePreset, Event::Set(id, v)) => {
                if let Some(d) = &mut self.dialog {
                    d.set_local(&id, &v);
                }
            }
            (Kind::SavePreset, Event::Button(b)) => {
                let Some(d) = self.dialog.take() else { return };
                let name = d.value("name").unwrap_or_default().trim().to_string();
                if d.buttons.get(b).map(String::as_str) == Some("Save") && !name.is_empty() {
                    self.preset_saving = Some(name);
                    self.server.send(crate::server::Cmd::ReadSettings);
                }
            }
            (Kind::Log, Event::Set(id, v)) => match id.as_str() {
                "filter" => {
                    self.events.filter = v;
                    self.refresh_log_page();
                }
                "act:reset" => {
                    self.apply(Action::ResetXruns);
                    self.server.run(crate::server::Cmd::ResetXruns);
                }
                _ => {}
            },
            (Kind::Log, Event::Button(_)) => self.dialog = None,
            (Kind::PipeWire, Event::Set(id, v)) => self.pw_set(&id, &v),
            (Kind::PipeWire, Event::Reset(id)) => self.pw_reset(&id),
            (Kind::PipeWire, Event::Button(_)) => self.dialog = None,
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
        // What is drawn: the blocks (with the ports on their edges and the label above), and
        // the matrices only in the matrix view (the others keep their positions but don't show).
        for (i, n) in self.model.nodes.iter().enumerate() {
            if self.layout.hidden.contains(&n.key.id()) {
                continue;
            }
            let r = self.node_extent(i);
            add(Rect::new(r.x - 8.0, r.y - 20.0, r.w + 16.0, r.h + 28.0));
        }
        if self.layout.view.matrices {
            for m in &self.mats {
                let p = self.matrix_pos(m);
                let s = Self::matrix_size(m);
                add(Rect::centered(p[0], p[1], s[0] + 16.0, s[1] + 16.0));
            }
        }
        let Some(b) = bb else { return };
        let a = self.canvas.area;
        if a.w < 10.0 || a.h < 10.0 {
            return;
        }
        // Fill the canvas, a small margin round it and the zoom widget's strip kept clear.
        let margin = 20.0 * self.scale;
        let strip = 44.0 * self.scale;
        let (uw, uh) = ((a.w - 2.0 * margin).max(10.0), (a.h - 2.0 * margin - strip).max(10.0));
        let zoom = (uw / (self.scale * b.w)).min(uh / (self.scale * b.h)).clamp(0.3, 1.5);
        self.layout.view.zoom = zoom;
        let z = zoom * self.scale;
        self.layout.view.cam = [b.cx() - (a.w / 2.0) / z, b.cy() - (margin + uh / 2.0) / z];
        self.layout_changed();
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

/// The settings page's tabs: the JACK server's (through jackdbus), then PipeWire's.
const JACK_TABS: [&str; 6] = ["Server", "Master", "Bridges", "Slaves & MIDI", "Netjack", "Internals"];
const TAB_MASTER: usize = 1;
const TAB_BRIDGES: usize = 2;
const TAB_SLAVES: usize = 3;
const TAB_NETJACK: usize = 4;
const TAB_INTERNALS: usize = 5;
const TAB_PIPEWIRE: usize = 6;

/// What each JACK tab's footer says.
fn tab_note(tab: usize) -> &'static str {
    match tab {
        TAB_BRIDGES => "Bridges change at once while the server runs; jackdbus keeps the list.",
        TAB_SLAVES => "Slave drivers are added and removed at once while the server runs.",
        TAB_NETJACK | TAB_INTERNALS => "Internal clients load into the running server; their parameters apply when loaded.",
        _ => "Driver and engine settings apply when the server (re)starts; jackdbus keeps them.",
    }
}

/// What a JACK tab is about, above its rows.
fn tab_message(tab: usize) -> &'static str {
    match tab {
        TAB_BRIDGES => "Sound cards on their own clocks, joined to the graph as the master's card is: each resampled to the master's clock, no word clock needed. A card switched on is bridged now and whenever the server starts; unplugged, its ports stay, silent, until it is back.",
        TAB_SLAVES => "Drivers run in the master's own cycle: alsarawmidi for MIDI ports, or a second card clocked by the master's (word clock). A card on its own clock is a bridge.",
        TAB_NETJACK => "Audio over the network: other machines joining this server (netmanager), or this machine joining another's (the net driver as master).",
        _ => "",
    }
}

/// The drivers that can be a master or a slave (the bridge driver is neither: the Bridges tab).
fn master_drivers(cfg: &crate::server::Config) -> impl Iterator<Item = &String> {
    cfg.drivers.iter().filter(|d| *d != "bridge")
}

fn link_row(id: &str, label: &str, text: &str) -> crate::dialog::Row {
    crate::dialog::Row { id: id.into(), label: label.into(), help: String::new(), control: crate::dialog::Control::Link(text.into()), modified: false }
}

/// A jackdbus parameter as a row: a switch, a number, a list or a text, with its help.
fn param_row(p: &crate::server::Param) -> crate::dialog::Row {
    use crate::dialog::{Control, Row};
    use crate::server::Value;
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
    let about = if p.long.is_empty() { p.short.clone() } else { p.long.clone() };
    let default = p.default.show();
    let help = if default.is_empty() { about } else { format!("{about} (default: {default})") };
    Row { id, label, help, control, modified: p.is_set }
}

/// The card an ALSA device name points at, lower case: "hw:PCH,0" and "plughw:CARD=PCH,DEV=0"
/// give "pch". None for names without one ("none", "default", a port count).
fn alsa_card(dev: &str) -> Option<String> {
    let first = dev.split_once(':')?.1.split(',').next()?;
    let card = first.strip_prefix("CARD=").unwrap_or(first);
    (!card.is_empty()).then(|| card.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::alsa_card;

    #[test]
    fn alsa_card_names() {
        assert_eq!(alsa_card("hw:PCH,0").as_deref(), Some("pch"));
        assert_eq!(alsa_card("hw:U44").as_deref(), Some("u44"));
        assert_eq!(alsa_card("hw:u44,0").as_deref(), Some("u44"));
        assert_eq!(alsa_card("plughw:CARD=PCH,DEV=0").as_deref(), Some("pch"));
        assert_eq!(alsa_card("hw:1,0").as_deref(), Some("1"));
        assert_eq!(alsa_card("none"), None);
        assert_eq!(alsa_card("default"), None);
        assert_eq!(alsa_card("2"), None);
        assert_eq!(alsa_card("hw:"), None);
    }
}

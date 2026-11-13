//! Sessions in the GUI: the sidebar's session panel, its dialogs (new, open, add application,
//! rename, duplicate, notes) and what they send to patchboxd.

use crate::app::{Action, App};
use crate::dialog::{Control, Dialog, Event, Kind, Row};
use crate::gfx::{Gfx, Rect, alpha};
use crate::text::Face;
use crate::theme::*;
use crate::ui::MenuItem;
use pbx_session::proto::{FromDaemon, Push, Reply, Request, Status};

/// Session actions (part of `Action`).
#[derive(Clone, Debug)]
pub enum SessAction {
    New,
    Open,
    Save,
    Close,
    Abort,
    Menu(Rect),
    Rename,
    Duplicate,
    Notes,
    OpenFolder,
    AddApp,
    Start(String),
    Stop(String),
    Kill(String),
    SaveClient(String),
    Gui(String, bool),
    Trash(String),
    Restore(String),
    Forget(String),
    ClientMenu(String, Rect),
    TrashMenu(Rect),
}

fn status_color(s: Status) -> crate::gfx::Color {
    match s {
        Status::Ready | Status::Running => OK,
        Status::Launching | Status::Opening | Status::Saving | Status::Stopping => WARN,
        Status::Error => BAD,
        Status::NoAnnounce => BORDER,
        Status::Stopped => TEXT_DIM,
    }
}

fn status_text(s: Status) -> &'static str {
    match s {
        Status::Stopped => "stopped",
        Status::Launching => "starting",
        Status::Opening => "opening",
        Status::Ready => "ready",
        Status::Saving => "saving",
        Status::Stopping => "stopping",
        Status::Running => "running",
        Status::NoAnnounce => "running (no NSM)",
        Status::Error => "error",
    }
}

impl App {
    /// What patchboxd said.
    pub fn on_daemon(&mut self) {
        for m in self.sess.take() {
            match m {
                FromDaemon::Push(Push::State(st)) => self.sess.state = st,
                FromDaemon::Push(Push::Log { text, bad }) => self.say(text, bad),
                FromDaemon::Reply(Reply::Sessions { sessions, .. }) => {
                    self.sess.sessions = sessions;
                    if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::OpenSession) {
                        let rows = self.open_rows();
                        if let Some(d) = &mut self.dialog {
                            d.set_rows(rows);
                        }
                    }
                }
                FromDaemon::Reply(Reply::State(st)) => self.sess.state = st,
                FromDaemon::Reply(Reply::Error { msg }) => self.say(msg, true),
                FromDaemon::Reply(Reply::Ok { .. }) => {}
            }
            self.redraw = true;
        }
        if self.sess.lost {
            self.sess.lost = false;
            self.say("Lost the session daemon; reconnecting", true);
        }
    }

    fn send(&mut self, r: Request) {
        if !self.sess.connected {
            self.say("The session daemon is not running (yet)", true);
            return;
        }
        self.sess.send(&r);
    }

    fn name_dialog(&mut self, kind: Kind, title: &str, label: &str, value: &str, ok: &str) {
        let mut d = Dialog::new(kind, title);
        d.rows = vec![Row { id: "name".into(), label: label.into(), help: String::new(), control: Control::Text(value.into()), modified: false }];
        d.buttons = vec!["Cancel".into(), ok.into()];
        d.note = format!("Sessions live in {}", self.sess.state.root);
        d.edit("name");
        self.dialog = Some(d);
    }

    fn open_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.sess.sessions.is_empty() {
            rows.push(Row::info("No sessions yet", "create one with New"));
        }
        for s in &self.sess.sessions {
            let when = ago(s.modified);
            let mut text = format!("{} client{} · {when}", s.clients, if s.clients == 1 { "" } else { "s" });
            if !s.notes.is_empty() {
                text.push_str(&format!(" · {}", s.notes));
            }
            rows.push(Row { id: format!("session:{}", s.name), label: s.name.clone(), help: String::new(), control: Control::Link(text), modified: false });
        }
        rows
    }

    fn add_app_dialog(&mut self) {
        let apps = self.apps.get_or_insert_with(crate::session::scan_apps).clone();
        let mut rows = Vec::new();
        let (nsm, other): (Vec<_>, Vec<_>) = apps.iter().enumerate().partition(|(_, a)| a.nsm);
        rows.push(Row::header("NSM applications (saved with the session)"));
        if nsm.is_empty() {
            rows.push(Row::info("None installed", "no .desktop file says X-NSM-Capable=true"));
        }
        for (i, a) in nsm {
            rows.push(Row { id: format!("app:{i}"), label: a.name.clone(), help: a.comment.clone(), control: Control::Link(a.exe.clone()), modified: false });
        }
        rows.push(Row::header("Other audio applications (started and stopped only)"));
        for (i, a) in other {
            rows.push(Row { id: format!("app:{i}"), label: a.name.clone(), help: a.comment.clone(), control: Control::Link(a.exe.clone()), modified: false });
        }
        rows.push(Row::header("Any command"));
        rows.push(Row { id: "cmd".into(), label: "Command".into(), help: "An executable and its arguments".into(), control: Control::Text(String::new()), modified: false });
        rows.push(Row {
            id: "nsm".into(),
            label: "Speaks NSM".into(),
            help: "On: it gets NSM_URL and is asked to open and save. Off: a plain program.".into(),
            control: Control::Toggle(true),
            modified: false,
        });
        let mut d = Dialog::new(Kind::AddApp, "Add an application");
        d.rows = rows;
        d.buttons = vec!["Cancel".into(), "Add command".into()];
        d.note = "Click an application to add it to the session and start it.".into();
        self.dialog = Some(d);
    }

    pub fn session_action(&mut self, a: SessAction) {
        self.redraw = true;
        let st = self.sess.state.clone();
        match a {
            SessAction::New => self.name_dialog(Kind::NewSession, "New session", "Name", "", "Create"),
            SessAction::Open => {
                self.sess.send(&Request::ListSessions);
                let mut d = Dialog::new(Kind::OpenSession, "Open a session");
                d.rows = self.open_rows();
                d.buttons = vec!["Cancel".into()];
                d.note = format!("{} · the open session is saved first", st.root);
                self.dialog = Some(d);
            }
            SessAction::Save => self.send(Request::Save),
            SessAction::Close => self.send(Request::Close),
            SessAction::Abort => self.send(Request::Abort),
            SessAction::Menu(r) => {
                let open = st.session.is_some();
                let items = vec![
                    MenuItem::new("Duplicate…", Action::Sess(SessAction::Duplicate)).enabled(open),
                    MenuItem::new("Rename…", Action::Sess(SessAction::Rename)).enabled(open),
                    MenuItem::new("Notes…", Action::Sess(SessAction::Notes)).enabled(open),
                    MenuItem::new("Open the session folder", Action::Sess(SessAction::OpenFolder)).enabled(open),
                    MenuItem::new("Close without saving", Action::Sess(SessAction::Abort)).enabled(open),
                ];
                self.ui.open_menu([r.x, r.bottom() + 2.0], items);
            }
            SessAction::Rename => {
                let cur = st.session.clone().unwrap_or_default();
                self.name_dialog(Kind::Rename, "Rename the session", "New name", &cur, "Rename");
            }
            SessAction::Duplicate => {
                let cur = format!("{} copy", st.session.clone().unwrap_or_default());
                self.name_dialog(Kind::Duplicate, "Duplicate the session", "Name of the copy", &cur, "Duplicate");
            }
            SessAction::Notes => {
                let mut d = Dialog::new(Kind::Notes, "Session notes");
                d.rows = vec![Row { id: "notes".into(), label: "Notes".into(), help: String::new(), control: Control::Text(st.notes.clone()), modified: false }];
                d.buttons = vec!["Cancel".into(), "Save".into()];
                d.edit("notes");
                self.dialog = Some(d);
            }
            SessAction::OpenFolder => {
                if let Some(p) = &st.path {
                    let _ = std::process::Command::new("xdg-open").arg(p).stdin(std::process::Stdio::null()).spawn();
                }
            }
            SessAction::AddApp => {
                if st.session.is_none() {
                    self.say("Create or open a session first", true);
                } else {
                    self.add_app_dialog();
                }
            }
            SessAction::Start(id) => self.send(Request::Start { id }),
            SessAction::Stop(id) => self.send(Request::Stop { id }),
            SessAction::Kill(id) => self.send(Request::Kill { id }),
            SessAction::SaveClient(id) => self.send(Request::SaveClient { id }),
            SessAction::Gui(id, show) => self.send(if show { Request::ShowGui { id } } else { Request::HideGui { id } }),
            SessAction::Trash(id) => self.send(Request::Trash { id }),
            SessAction::Restore(id) => self.send(Request::Restore { id }),
            SessAction::Forget(id) => self.send(Request::Forget { id }),
            SessAction::ClientMenu(id, r) => {
                let Some(c) = st.clients.iter().find(|c| c.id == id) else { return };
                let running = c.pid.is_some();
                let mut items = vec![MenuItem::header(format!("{} ({})", if c.label.is_empty() { &c.name } else { &c.label }, c.id))];
                if running {
                    items.push(MenuItem::new("Stop", Action::Sess(SessAction::Stop(id.clone()))));
                    items.push(MenuItem::new("Kill", Action::Sess(SessAction::Kill(id.clone()))));
                } else {
                    items.push(MenuItem::new("Start", Action::Sess(SessAction::Start(id.clone()))));
                }
                items.push(MenuItem::new("Save", Action::Sess(SessAction::SaveClient(id.clone()))).enabled(c.nsm && c.status == Status::Ready));
                if let Some(shown) = c.gui {
                    items.push(MenuItem::new(if shown { "Hide its window" } else { "Show its window" }, Action::Sess(SessAction::Gui(id.clone(), !shown))));
                }
                items.push(MenuItem::new("Move to the trash", Action::Sess(SessAction::Trash(id.clone()))).enabled(!running));
                if !c.jack_name.is_empty() {
                    let node = format!("M:{}", c.jack_name);
                    items.push(MenuItem::new("Find in the graph", Action::CenterNode(node)));
                }
                self.ui.open_menu([r.x, r.bottom() + 2.0], items);
            }
            SessAction::TrashMenu(r) => {
                let mut items = Vec::new();
                for c in &st.trash {
                    let name = if c.label.is_empty() { c.name.clone() } else { c.label.clone() };
                    items.push(MenuItem::new(format!("Restore {name} ({})", c.id), Action::Sess(SessAction::Restore(c.id.clone()))));
                    items.push(MenuItem::new(format!("Forget {name} (files stay)"), Action::Sess(SessAction::Forget(c.id.clone()))));
                }
                self.ui.open_menu([r.x, r.bottom() + 2.0], items);
            }
        }
    }

    /// Events from the session dialogs.
    pub fn session_dialog_event(&mut self, kind: Kind, ev: Event) {
        let Some(d) = &mut self.dialog else { return };
        match ev {
            Event::Set(id, v) if id.starts_with("session:") && v == "activate" => {
                let name = id.trim_start_matches("session:").to_string();
                self.dialog = None;
                self.send(Request::Open { name });
            }
            Event::Set(id, v) if id.starts_with("app:") && v == "activate" => {
                let i: usize = id.trim_start_matches("app:").parse().unwrap_or(usize::MAX);
                let Some(a) = self.apps.as_ref().and_then(|a| a.get(i)).cloned() else { return };
                self.dialog = None;
                self.send(Request::Add { exe: a.exe, args: a.args, nsm: a.nsm, label: if a.nsm { String::new() } else { a.name } });
            }
            Event::Set(id, v) => d.set_local(&id, &v),
            Event::Button(b) => {
                let label = d.buttons.get(b).cloned().unwrap_or_default();
                let name = d.value("name").unwrap_or_default();
                let req = match (kind, label.as_str()) {
                    (_, "Cancel") => None,
                    (Kind::NewSession, _) => Some(Request::New { name }),
                    (Kind::Rename, _) => Some(Request::Rename { name }),
                    (Kind::Duplicate, _) => Some(Request::Duplicate { name }),
                    (Kind::Notes, _) => Some(Request::SetNotes { notes: d.value("notes").unwrap_or_default() }),
                    (Kind::AddApp, _) => {
                        let cmd = d.value("cmd").unwrap_or_default();
                        let nsm = d.value("nsm").as_deref() == Some("on");
                        let mut w = cmd.split_whitespace().map(str::to_string);
                        w.next().map(|exe| Request::Add { exe, args: w.collect(), nsm, label: String::new() })
                    }
                    _ => None,
                };
                self.dialog = None;
                if let Some(r) = req {
                    self.send(r);
                }
            }
            Event::Close => self.dialog = None,
            _ => {}
        }
    }

    /// The sidebar's session panel; returns where it ends.
    pub fn draw_session_panel(&mut self, g: &mut Gfx, r: Rect, mut y: f32) -> f32 {
        let s = self.scale;
        let pad = 10.0 * s;
        let px = FONT * s;
        let lh = g.line_height(Face::Regular, px);
        let st = self.sess.state.clone();
        g.text(Face::Bold, px, r.x + pad, y, TEXT_BRIGHT, "Session");
        let x0 = r.x + pad + g.measure(Face::Bold, px, "Session") + 10.0 * s;
        let (name, col) = match (&st.session, self.sess.connected) {
            (_, false) => ("starting the daemon…".to_string(), TEXT_DIM),
            (Some(n), _) => (n.clone(), HILIGHT),
            (None, _) => ("none open".to_string(), TEXT_DIM),
        };
        g.text_fit(Face::Bold, px, x0, y, r.right() - x0 - pad, col, &name);
        y += lh + 4.0 * s;
        if let Some(b) = &st.busy {
            g.text_fit(Face::Regular, FONT_SMALL * s, r.x + pad, y, r.w - 2.0 * pad, WARN, b);
            y += lh;
        }
        // New / Open / Save / Close / ⋯
        let bh = (lh + 10.0 * s).round();
        let open = st.session.is_some();
        let more_w = bh;
        let bw = ((r.w - 2.0 * pad - more_w - 4.0 * 4.0 * s) / 4.0).floor();
        let mut bx = r.x + pad;
        for (label, act, on) in [
            ("New", SessAction::New, true),
            ("Open", SessAction::Open, true),
            ("Save", SessAction::Save, open),
            ("Close", SessAction::Close, open),
        ] {
            let br = Rect::new(bx, y, bw, bh);
            if on {
                self.sbutton(g, br, label, Action::Sess(act));
            } else {
                g.rrect(br, 3.0 * s, alpha(BUTTON, 0.5));
                g.text_center(Face::Regular, px, br, TEXT_DIM, label);
            }
            bx += bw + 4.0 * s;
        }
        let mr = Rect::new(bx, y, more_w, bh);
        self.sbutton(g, mr, "⋯", Action::Sess(SessAction::Menu(mr)));
        y += bh + 8.0 * s;
        if !open {
            return y;
        }
        // The session's clients.
        let row_h = (lh + 8.0 * s).round();
        for c in &st.clients {
            let rr = Rect::new(r.x + 4.0 * s, y, r.w - 8.0 * s, row_h);
            y += row_h;
            let hov = rr.contains(self.ui.mouse[0], self.ui.mouse[1]) && self.ui.menu.is_none() && self.dialog.is_none();
            if hov {
                g.rrect(rr, 3.0 * s, BUTTON_HOVER);
            }
            g.circle([rr.x + 9.0 * s, rr.cy()], 4.0 * s, status_color(c.status));
            let mut name = if c.label.is_empty() { c.name.clone() } else { c.label.clone() };
            if c.dirty == Some(true) {
                name.push_str(" *");
            }
            // Icons on the right: GUI eye, save, start/stop.
            let ib = row_h - 4.0 * s;
            let mut ix = rr.right() - ib - 2.0 * s;
            let running = c.pid.is_some();
            let ssr = Rect::new(ix, rr.y + 2.0 * s, ib, ib);
            let act = if running { SessAction::Stop(c.id.clone()) } else { SessAction::Start(c.id.clone()) };
            self.icon(g, ssr, if running { Icon::Stop } else { Icon::Play }, Action::Sess(act));
            ix -= ib + 2.0 * s;
            if c.nsm && c.status == Status::Ready {
                let sr = Rect::new(ix, rr.y + 2.0 * s, ib, ib);
                self.icon(g, sr, Icon::Save, Action::Sess(SessAction::SaveClient(c.id.clone())));
                ix -= ib + 2.0 * s;
            }
            if let Some(shown) = c.gui {
                let gr = Rect::new(ix, rr.y + 2.0 * s, ib, ib);
                self.icon(g, gr, if shown { Icon::Eye } else { Icon::EyeOff }, Action::Sess(SessAction::Gui(c.id.clone(), !shown)));
                ix -= ib + 2.0 * s;
            }
            let tx = rr.x + 20.0 * s;
            let label_r = Rect::new(tx, rr.y, ix - tx - 4.0 * s, rr.h);
            let status = if c.message.is_empty() || c.status != Status::Error { status_text(c.status).to_string() } else { c.message.clone() };
            let sw = g.measure(Face::Regular, FONT_SMALL * s, &status).min(label_r.w * 0.45);
            g.text_fit(Face::Regular, px, tx, rr.cy() - lh / 2.0, label_r.w - sw - 6.0 * s, if running { TEXT_BRIGHT } else { TEXT }, &name);
            let slh = g.line_height(Face::Regular, FONT_SMALL * s);
            g.text_fit(Face::Regular, FONT_SMALL * s, label_r.right() - sw, rr.cy() - slh / 2.0, sw, status_color(c.status), &status);
            if c.progress > 0.0 && c.progress < 1.0 {
                g.rect(Rect::new(rr.x, rr.bottom() - 2.0 * s, rr.w * c.progress, 2.0 * s), WARN);
            }
            self.add_hit_pub(label_r, Action::Sess(SessAction::ClientMenu(c.id.clone(), Rect::new(label_r.x, label_r.y, 0.0, label_r.h))));
        }
        // + Add application, trash.
        let ar = Rect::new(r.x + pad, y + 2.0 * s, (r.w - 2.0 * pad) * 0.62, bh);
        self.sbutton(g, ar, "+ Application", Action::Sess(SessAction::AddApp));
        if !st.trash.is_empty() {
            let tr = Rect::new(ar.right() + 6.0 * s, ar.y, r.right() - pad - ar.right() - 6.0 * s, bh);
            self.sbutton(g, tr, &format!("Trash {}", st.trash.len()), Action::Sess(SessAction::TrashMenu(tr)));
        }
        y += bh + 10.0 * s;
        if st.connections_saved > 0 {
            let t = format!("connections: {} saved, {} present", st.connections_saved, st.connections_present);
            g.text_fit(Face::Regular, FONT_SMALL * s, r.x + pad, y, r.w - 2.0 * pad, TEXT_DIM, &t);
            y += lh;
        }
        y
    }

    fn sbutton(&mut self, g: &mut Gfx, r: Rect, label: &str, a: Action) {
        let s = self.scale;
        let hov = r.contains(self.ui.mouse[0], self.ui.mouse[1]) && self.ui.menu.is_none() && self.dialog.is_none();
        g.rrect(r, 3.0 * s, if hov { BUTTON_HOVER } else { BUTTON });
        g.rrect_stroke(r, 3.0 * s, s.max(1.0), alpha(BORDER, 0.8));
        if label == "⋯" {
            // The font has no ⋯: three dots.
            for k in -1..=1 {
                g.circle([r.cx() + k as f32 * 5.0 * s, r.cy()], 1.6 * s, TEXT_BRIGHT);
            }
        } else {
            g.text_center(Face::Regular, FONT * s, r, TEXT_BRIGHT, label);
        }
        self.add_hit_pub(r, a);
    }

    fn icon(&mut self, g: &mut Gfx, r: Rect, icon: Icon, a: Action) {
        let s = self.scale;
        let hov = r.contains(self.ui.mouse[0], self.ui.mouse[1]) && self.ui.menu.is_none() && self.dialog.is_none();
        if hov {
            g.rrect(r, 3.0 * s, BUTTON);
        }
        let (cx, cy, k) = (r.cx(), r.cy(), r.w * 0.28);
        let col = if hov { HILIGHT } else { TEXT };
        match icon {
            Icon::Play => g.triangle([cx - k * 0.7, cy - k], [cx - k * 0.7, cy + k], [cx + k, cy], OK),
            Icon::Stop => g.rect(Rect::new(cx - k * 0.8, cy - k * 0.8, k * 1.6, k * 1.6), col),
            Icon::Save => {
                g.rect_stroke(Rect::new(cx - k, cy - k, 2.0 * k, 2.0 * k), s.max(1.0), col);
                g.rect(Rect::new(cx - k * 0.55, cy - k, k * 1.1, k * 0.7), col);
            }
            Icon::Eye | Icon::EyeOff => {
                g.arc([cx, cy], k, 0.0, std::f32::consts::TAU, s.max(1.0), col);
                if icon == Icon::Eye {
                    g.circle([cx, cy], k * 0.45, col);
                } else {
                    g.line([cx - k, cy + k], [cx + k, cy - k], s.max(1.0), col);
                }
            }
        }
        self.add_hit_pub(r, a);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Play,
    Stop,
    Save,
    Eye,
    EyeOff,
}

fn ago(t: u64) -> String {
    let d = pbx_session::store::now().saturating_sub(t);
    match d {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", d / 60),
        3600..86400 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}

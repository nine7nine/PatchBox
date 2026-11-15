//! Sessions in the GUI: the sidebar's session panel, its dialogs (new, open, add application,
//! rename, duplicate, notes) and what they send to patchboxd.

use crate::app::{Action, App};
use crate::dialog::{Control, Dialog, Event, Kind, Row};
use crate::session::AddNext;
use crate::gfx::{Gfx, Rect};
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
    Snapshot,
    Snapshots,
    SaveTemplate,
    JackMemory(bool),
    AddApp,
    AddExe,
    Start(String),
    Stop(String),
    Kill(String),
    SaveClient(String),
    Gui(String, bool),
    Restore(String),
    Forget(String),
    ClientMenu(String, Rect),
    TrashMenu(Rect),
    /// Remove a client from the session (asks first if it runs).
    Remove(String),
    /// Delete a removed client and its files (asks first).
    DeleteClient(String),
    /// The templates page.
    Templates,
    /// Delete the open session (asks first; closes it without saving, then deletes).
    DeleteOpen,
    /// The daemon is older than this window: save, close, restart it, reopen (asks first).
    UpdateDaemon,
    /// A client's properties dialog.
    Props(String),
    /// Keep a client as a client template (asks the name).
    ClientTemplate(String),
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

pub fn status_text(s: Status) -> &'static str {
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

/// Stopping it would lose work: an NSM client with changes it reports unsaved, or (one that
/// doesn't report them) not saved in the last minute.
pub fn unsaved(c: &pbx_session::proto::ClientState) -> bool {
    let running = c.pid.is_some() && matches!(c.status, Status::Ready | Status::Saving);
    running && c.nsm && match c.dirty {
        Some(d) => d,
        None => c.last_save.is_none_or(|t| pbx_session::store::now().saturating_sub(t) >= 60),
    }
}

impl App {
    /// A daemon from before an update: with no session open, replaced at once (asked to quit;
    /// the link starts the new one); with one open, the panel offers Update. Once the new one
    /// answers, a session closed for the update is opened again.
    fn check_daemon_version(&mut self) {
        let st = &self.sess.state;
        if self.sess.stale() {
            if st.session.is_none() && st.busy.is_none() && !self.sess.quit_sent {
                self.sess.quit_sent = true;
                self.sess.send(&Request::Quit);
                self.say("PatchBox was updated: restarting its session daemon", false);
            }
        } else if self.sess.connected && st.session.is_none() && st.busy.is_none()
            && let Some(name) = self.sess.reopen_after_update.take()
        {
            self.send(Request::Open { name });
        }
    }

    /// What patchboxd said.
    pub fn on_daemon(&mut self) {
        for m in self.sess.take() {
            match m {
                FromDaemon::Push(Push::State(st)) => {
                    let dir = st.path.clone().map(std::path::PathBuf::from);
                    let opened = st.session.is_some() && st.busy.is_none();
                    self.sess.state = st;
                    self.session_dir_changed(dir);
                    if self.sess.state.session.is_none() && self.sess.state.busy.is_none()
                        && let Some(name) = self.sess.delete_after_close.take()
                    {
                        self.send(Request::Delete { name });
                    }
                    self.check_daemon_version();
                    if opened && self.dialog.is_none() {
                        match self.sess.add_after_open.take() {
                            Some(AddNext::App) => self.add_app_dialog(),
                            Some(AddNext::Exe) => self.add_exe_dialog(),
                            None => {}
                        }
                    }
                }
                FromDaemon::Push(Push::Log { text, bad }) => self.say(text, bad),
                FromDaemon::Reply(Reply::Sessions { sessions, recent, .. }) => {
                    self.sess.sessions = sessions;
                    self.sess.recent = recent;
                    if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::OpenSession) {
                        let rows = self.open_rows();
                        if let Some(d) = &mut self.dialog {
                            d.set_rows(rows);
                        }
                    }
                }
                FromDaemon::Reply(Reply::Snapshots { snapshots }) => {
                    self.sess.snapshots = snapshots;
                    if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::Snapshots) {
                        let rows = self.snapshot_rows();
                        if let Some(d) = &mut self.dialog {
                            d.set_rows(rows);
                        }
                    }
                }
                FromDaemon::Reply(Reply::ClientTemplates { templates }) => {
                    self.sess.client_templates = templates;
                    self.refresh_templates_page();
                    if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::AddApp) {
                        let rows = self.add_app_rows();
                        if let Some(d) = &mut self.dialog {
                            d.set_rows(rows);
                        }
                    }
                }
                FromDaemon::Reply(Reply::Rules { rules }) => {
                    self.rules = rules;
                    self.refresh_rules_page();
                }
                FromDaemon::Reply(Reply::Templates { templates }) => {
                    self.sess.templates = templates;
                    self.refresh_templates_page();
                    let opts = self.template_options();
                    if let Some(d) = self.dialog.as_mut().filter(|d| d.kind == Kind::NewSession)
                        && let Some(r) = d.rows.iter_mut().find(|r| r.id == "template")
                        && let Control::Choice { options, .. } = &mut r.control
                    {
                        *options = opts;
                    }
                }
                FromDaemon::Reply(Reply::State(st)) => self.sess.state = st,
                FromDaemon::Reply(Reply::Error { msg }) => {
                    self.sess.add_after_open = None;
                    self.sess.delete_after_close = None;
                    if msg.starts_with("bad request") && self.sess.stale() {
                        self.say("The session daemon is from before PatchBox was updated and can't do that: Update, in the session panel, restarts it", true);
                    } else {
                        self.say(msg, true);
                    }
                }
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
        // Recent ones first (left open for a while), then all of them.
        let recent: Vec<&pbx_session::proto::SessionEntry> = self.sess.recent.iter().filter_map(|n| self.sess.sessions.iter().find(|s| &s.name == n)).collect();
        if !recent.is_empty() {
            rows.push(Row::header("Recent"));
            for s in recent {
                rows.push(Row { id: format!("session:{}", s.name), label: s.name.clone(), help: String::new(), control: self.session_link(&s.name, ago(s.modified)), modified: false });
            }
            rows.push(Row::header("All sessions"));
        }
        for s in &self.sess.sessions {
            let when = ago(s.modified);
            let mut text = format!("{} client{} · {when}", s.clients, if s.clients == 1 { "" } else { "s" });
            if !s.notes.is_empty() {
                text.push_str(&format!(" · {}", s.notes));
            }
            rows.push(Row { id: format!("session:{}", s.name), label: s.name.clone(), help: String::new(), control: self.session_link(&s.name, text), modified: false });
        }
        rows
    }

    /// A confirmation card: `message` says what happens; the last button does it.
    fn confirm(&mut self, kind: Kind, tag: &str, title: &str, message: &str, buttons: &[&str]) {
        let mut d = Dialog::new(kind, title);
        d.tag = tag.to_string();
        d.message = message.to_string();
        d.buttons = buttons.iter().map(|b| b.to_string()).collect();
        self.dialog = Some(d);
    }

    fn client_name(&self, id: &str) -> String {
        let st = &self.sess.state;
        st.clients.iter().map(|c| (&c.id, &c.label, &c.name)).chain(st.trash.iter().map(|c| (&c.id, &c.label, &c.name)))
            .find(|(i, _, _)| *i == id)
            .map(|(_, l, n)| if l.is_empty() { n.clone() } else { l.clone() })
            .unwrap_or_else(|| id.to_string())
    }

    fn templates_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::header("Session templates: a new session starts as a copy")];
        if self.sess.templates.is_empty() {
            rows.push(Row::info("None yet", "the session's ... menu → Save as template…"));
        }
        for t in &self.sess.templates {
            rows.push(Row { id: format!("stemplate:{t}"), label: t.clone(), help: String::new(), control: Control::LinkRemove("new session from it".into()), modified: false });
        }
        rows.push(Row::header("Client templates: a client with its settings and files"));
        if self.sess.client_templates.is_empty() {
            rows.push(Row::info("None yet", "a client's menu → Save as a client template…"));
        }
        let open = self.sess.state.session.is_some();
        for t in &self.sess.client_templates {
            let text = if open { "add it to the session" } else { "open a session to add it" };
            rows.push(Row { id: format!("ctemplate:{t}"), label: t.clone(), help: String::new(), control: Control::LinkRemove(text.into()), modified: false });
        }
        rows
    }

    fn refresh_templates_page(&mut self) {
        if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::Templates) {
            let rows = self.templates_rows();
            if let Some(d) = &mut self.dialog {
                d.set_rows(rows);
            }
        }
    }

    fn templates_page(&mut self) {
        self.sess.send(&Request::ListTemplates);
        self.sess.send(&Request::ListClientTemplates);
        let mut d = Dialog::new(Kind::Templates, "Templates");
        d.rows = self.templates_rows();
        d.buttons = vec!["Close".into()];
        d.note = "Click one to use it; × deletes it (to your file manager's Trash).".into();
        self.dialog = Some(d);
    }

    /// A session's row in the open list: removable unless it is the open one.
    fn session_link(&self, name: &str, text: String) -> Control {
        if self.sess.state.session.as_deref() == Some(name) {
            Control::Link(format!("open now · {text}"))
        } else {
            Control::LinkRemove(text)
        }
    }

        fn template_options(&self) -> Vec<(String, String)> {
        std::iter::once((String::new(), "(an empty session)".to_string())).chain(self.sess.templates.iter().map(|t| (t.clone(), t.clone()))).collect()
    }

    fn snapshot_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row {
            id: "auto".into(),
            label: "Automatic snapshots".into(),
            help: "A snapshot at every save and close, when something changed".into(),
            control: Control::Toggle(self.sess.state.auto_snapshot),
            modified: false,
        }];
        rows.push(Row::header("Go back to"));
        if self.sess.snapshots.is_empty() {
            rows.push(Row::info("No snapshots yet", "save the session, or take one"));
        }
        for s in &self.sess.snapshots {
            rows.push(Row { id: format!("snap:{}", s.id), label: format!("{} — {}", ago(s.time), s.label), help: String::new(), control: Control::Link(s.id.clone()), modified: false });
        }
        rows
    }

    fn add_app_dialog(&mut self) {
        self.apps.get_or_insert_with(crate::session::scan_apps);
        self.sess.send(&Request::ListClientTemplates);
        let rows = self.add_app_rows();
        let mut d = Dialog::new(Kind::AddApp, "Add an application");
        d.rows = rows;
        d.buttons = vec!["Cancel".into(), "Add command".into()];
        d.note = "Click an application to add it to the session and start it.".into();
        self.dialog = Some(d);
    }

    /// RaySession's "Executable": one command line, NSM or not.
    fn add_exe_dialog(&mut self) {
        let mut d = Dialog::new(Kind::AddExe, "Add an executable");
        d.rows = vec![
            Row { id: "cmd".into(), label: "Command".into(), help: "The program and its arguments; quote an argument with spaces".into(), control: Control::Text(String::new()), modified: false },
            Row { id: "label".into(), label: "Label".into(), help: "Its name in the session (empty: the program's)".into(), control: Control::Text(String::new()), modified: false },
            Row {
                id: "nsm".into(),
                label: "Speaks NSM".into(),
                help: "On: it gets NSM_URL and is asked to open and save. Off: a plain program, started and stopped.".into(),
                control: Control::Toggle(true),
                modified: false,
            },
        ];
        d.buttons = vec!["Cancel".into(), "Add".into()];
        d.note = "It is started now, and with the session from then on.".into();
        d.edit("cmd");
        self.dialog = Some(d);
    }

    fn add_app_rows(&self) -> Vec<Row> {
        let apps = self.apps.clone().unwrap_or_default();
        let mut rows = Vec::new();
        // The user's client templates first: a client with its settings and files.
        if !self.sess.client_templates.is_empty() {
            rows.push(Row::header("Your client templates (with their files)"));
            for t in &self.sess.client_templates {
                rows.push(Row { id: format!("ctemplate:{t}"), label: t.clone(), help: "Add it with its own copy of the template's files".into(), control: Control::Link("add".into()), modified: false });
            }
        }
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
        // Any other installed application, found by a search (a browser, OBS…): not listed, there
        // are hundreds
        rows.push(Row::header("Another application (started and stopped only)"));
        rows.push(Row {
            id: "find".into(),
            label: "Search all applications".into(),
            help: "Part of a name, a command or a description; Enter shows what matches".into(),
            control: Control::Text(self.app_query.clone()),
            modified: false,
        });
        if !self.app_query.trim().is_empty() {
            let all = self.all_apps.as_deref().unwrap_or(&[]);
            let found = crate::session::find_apps(all, &self.app_query, 20);
            if found.is_empty() {
                rows.push(Row::info("Nothing found", format!("no installed application matches \"{}\"", self.app_query.trim())));
            }
            for (i, a) in found {
                rows.push(Row { id: format!("any:{i}"), label: a.name.clone(), help: a.comment.clone(), control: Control::Link(a.exe.clone()), modified: false });
            }
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
        rows
    }

    pub fn session_action(&mut self, a: SessAction) {
        self.redraw = true;
        let st = self.sess.state.clone();
        match a {
            SessAction::New => {
                self.sess.send(&Request::ListTemplates);
                self.name_dialog(Kind::NewSession, "New session", "Name", "", "Create");
                let opts = self.template_options();
                if let Some(d) = &mut self.dialog {
                    d.rows.push(Row {
                        id: "template".into(),
                        label: "From template".into(),
                        help: "Start from a session saved as a template (clients, their files, connections)".into(),
                        control: Control::Choice { value: String::new(), options: opts, editable: false },
                        modified: false,
                    });
                }
            }
            SessAction::SaveTemplate => {
                let cur = st.session.clone().unwrap_or_default().replace('/', " ");
                self.name_dialog(Kind::SaveTemplate, "Save the session as a template", "Template name", &cur, "Save");
            }
            SessAction::Open if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::OpenSession) => self.dialog = None,
            SessAction::AddApp if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::AddApp) => self.dialog = None,
            SessAction::AddExe if self.dialog.as_ref().is_some_and(|d| d.kind == Kind::AddExe) => self.dialog = None,
            SessAction::Open => {
                self.sess.send(&Request::ListSessions);
                let mut d = Dialog::new(Kind::OpenSession, "Open a session");
                d.rows = self.open_rows();
                d.buttons = vec!["Cancel".into()];
                d.note = format!("{} · the open session is saved first", st.root);
                self.dialog = Some(d);
            }
            SessAction::Save => self.send(Request::Save),
            SessAction::JackMemory(on) => self.send(Request::SetJackMemory { on }),
            SessAction::Close => self.send(Request::Close),
            SessAction::Abort => {
                let name = st.session.clone().unwrap_or_default();
                self.confirm(Kind::AbortSession, "", "Close without saving?", &format!("{name} closes and its clients stop without being asked to save: changes since the last save are lost."), &["Cancel", "Close without saving"]);
            }
            SessAction::DeleteOpen => {
                let name = st.session.clone().unwrap_or_default();
                self.confirm(Kind::DeleteSession, &name, "Delete this session?", &format!("{name} is closed without saving (its clients stop), then its folder, with every client's files in it, goes to your file manager's Trash (Dolphin: Places → Trash), where you can restore it."), &["Cancel", "Move to Trash"]);
            }
            SessAction::Templates => self.templates_page(),
            SessAction::UpdateDaemon => {
                let name = st.session.clone().unwrap_or_default();
                self.confirm(Kind::UpdateDaemon, &name, "Restart the session daemon?", &format!("PatchBox was updated, but {name} still runs in the session daemon from before, which can't do everything the new window asks. This saves {name} and closes it (its programs stop), starts the new daemon, and opens {name} again."), &["Cancel", "Save and restart"]);
            }
            SessAction::Remove(id) => {
                let Some(c) = st.clients.iter().find(|c| c.id == id) else { return };
                let name = self.client_name(&id);
                if c.pid.is_none() {
                    // The daemon says when it is done (or why not).
                    self.send(Request::Remove { id, save: false });
                } else {
                    let mut b = vec!["Cancel", "Stop and remove"];
                    let can_save = c.nsm && c.status == Status::Ready;
                    if can_save {
                        b.push("Save, stop and remove");
                    }
                    let detail = if unsaved(c) { "is running, with unsaved changes" } else { "is running" };
                    self.confirm(Kind::RemoveClient, &id, &format!("Remove {name}?"), &format!("{name} {detail}: it is stopped first. Removed, below the session's clients, brings it back."), &b);
                }
            }
            SessAction::DeleteClient(id) => {
                let name = self.client_name(&id);
                self.confirm(Kind::DeleteClient, &id, &format!("Delete {name}?"), &format!("{name} leaves the session for good; the files it saved in the session's folder go to your file manager's Trash."), &["Cancel", "Delete"]);
            }
            SessAction::Menu(r) => {
                let open = st.session.is_some();
                let items = vec![
                    MenuItem::new("Duplicate…", Action::Sess(SessAction::Duplicate)).enabled(open),
                    MenuItem::new("Rename…", Action::Sess(SessAction::Rename)).enabled(open),
                    MenuItem::new("Notes…", Action::Sess(SessAction::Notes)).enabled(open),
                    MenuItem::new("Save as template…", Action::Sess(SessAction::SaveTemplate)).enabled(open),
                    MenuItem::new("Remember JACK settings with it", Action::Sess(SessAction::JackMemory(!st.jack_memory))).enabled(open).checked(st.jack_memory),
                    MenuItem::new("Take a snapshot…", Action::Sess(SessAction::Snapshot)).enabled(open),
                    MenuItem::new("Go back to a snapshot…", Action::Sess(SessAction::Snapshots)).enabled(open),
                    MenuItem::new("Open the session folder", Action::Sess(SessAction::OpenFolder)).enabled(open),
                    MenuItem::new("Close without saving…", Action::Sess(SessAction::Abort)).enabled(open),
                    MenuItem::new("Delete this session…", Action::Sess(SessAction::DeleteOpen)).enabled(open),
                    MenuItem::new("Templates…", Action::Sess(SessAction::Templates)),
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
            SessAction::Snapshot => self.name_dialog(Kind::Snapshot, "Take a snapshot", "Label", "", "Take"),
            SessAction::Snapshots => {
                self.sess.send(&Request::ListSnapshots);
                let mut d = Dialog::new(Kind::Snapshots, "Snapshots");
                d.rows = self.snapshot_rows();
                d.buttons = vec!["Close".into()];
                d.note = "Going back saves and snapshots the present first, so it can be undone too.".into();
                self.dialog = Some(d);
            }
            SessAction::OpenFolder => {
                if let Some(p) = &st.path {
                    let _ = std::process::Command::new("xdg-open").arg(p).stdin(std::process::Stdio::null()).spawn();
                }
            }
            SessAction::AddApp | SessAction::AddExe => {
                let next = if matches!(a, SessAction::AddApp) { AddNext::App } else { AddNext::Exe };
                if st.session.is_none() {
                    // Programs live in a session: name one first, then pick the program.
                    self.session_action(SessAction::New);
                    self.sess.add_after_open = Some(next);
                    if let Some(d) = &mut self.dialog {
                        d.note = "Name the session first, then add to it.".into();
                    }
                } else if next == AddNext::App {
                    self.add_app_dialog();
                } else {
                    self.add_exe_dialog();
                }
            }
            SessAction::Start(id) => self.send(Request::Start { id }),
            SessAction::Stop(id) => match st.clients.iter().find(|c| c.id == id) {
                Some(c) if unsaved(c) => {
                    let name = self.client_name(&id);
                    let detail = if c.dirty == Some(true) { "has unsaved changes" } else { "was not saved in the last minute" };
                    self.confirm(Kind::StopClient, &id, &format!("Stop {name}?"), &format!("{name} {detail}. Save and stop waits for its save; if the save fails, it keeps running."), &["Cancel", "Stop without saving", "Save and stop"]);
                }
                _ => self.send(Request::Stop { id }),
            },
            SessAction::Kill(id) => self.send(Request::Kill { id }),
            SessAction::SaveClient(id) => self.send(Request::SaveClient { id }),
            SessAction::Gui(id, show) => self.send(if show { Request::ShowGui { id } } else { Request::HideGui { id } }),
            SessAction::Restore(id) => self.send(Request::Restore { id }),
            SessAction::Forget(id) => self.send(Request::Forget { id, files: false }),
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
                items.push(MenuItem::new("Properties…", Action::Sess(SessAction::Props(id.clone()))));
                items.push(MenuItem::new("Save as a client template…", Action::Sess(SessAction::ClientTemplate(id.clone()))));
                items.push(MenuItem::new(if running { "Remove from the session…" } else { "Remove from the session" }, Action::Sess(SessAction::Remove(id.clone()))));
                if !c.jack_name.is_empty() {
                    let node = format!("M:{}", c.jack_name);
                    items.push(MenuItem::new("Find in the graph", Action::CenterNode(node)));
                }
                self.ui.open_menu([r.x, r.bottom() + 2.0], items);
            }
            SessAction::ClientTemplate(id) => {
                self.name_dialog(Kind::ClientTemplate, "Save as a client template", "Template name", &id, "Save");
                if let Some(d) = &mut self.dialog {
                    d.tag = id.clone();
                    d.note = "Its command, properties and files: Add an application lists it in any session.".into();
                }
            }
            SessAction::Props(id) => {
                let Some(c) = st.clients.iter().find(|c| c.id == id) else { return };
                let sig = |none: &str| -> Vec<(String, String)> {
                    let mut v = vec![("0".to_string(), none.to_string())];
                    v.extend([(libc::SIGUSR1, "SIGUSR1"), (libc::SIGUSR2, "SIGUSR2"), (libc::SIGHUP, "SIGHUP"), (libc::SIGINT, "SIGINT"), (libc::SIGTERM, "SIGTERM"), (libc::SIGQUIT, "SIGQUIT")].iter().map(|(s, l)| (s.to_string(), l.to_string())));
                    v
                };
                let mut cmd = vec![c.exe.clone()];
                cmd.extend(c.args.iter().cloned());
                let env = pbx_session::store::join_args(&c.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>());
                let mut d = Dialog::new(Kind::ClientProps, format!("{} ({})", if c.label.is_empty() { &c.name } else { &c.label }, c.id));
                d.tag = id.clone();
                d.rows = vec![
                    Row { id: "label".into(), label: "Label".into(), help: "Its name in the session (plain programs: also $JACK_CLIENT_NAME)".into(), control: Control::Text(c.label.clone()), modified: false },
                    Row { id: "cmd".into(), label: "Command".into(), help: "The program and its arguments; '…' and \"…\" keep spaces. $SESSION_NAME, $SESSION_PATH, $CLIENT_ID and $JACK_CLIENT_NAME become theirs when it starts".into(), control: Control::Text(pbx_session::store::join_args(&cmd)), modified: false },
                    Row { id: "env".into(), label: "Environment".into(), help: "NAME=value pairs, space separated (quote values with spaces); the same $… names work".into(), control: Control::Text(env), modified: false },
                    Row::info("Protocol", if c.nsm { "NSM: saves and quits when the session tells it" } else { "plain program: the signals below" }),
                ];
                if !c.nsm {
                    d.rows.push(Row { id: "save".into(), label: "Save signal".into(), help: "Sent at the session's save (it counts as saved after 300 ms); none: it saves on its own".into(), control: Control::Choice { value: c.save_signal.to_string(), options: sig("none"), editable: false }, modified: false });
                    d.rows.push(Row { id: "stop".into(), label: "Stop signal".into(), help: "Sent to stop it (killed if it is still there after a while)".into(), control: Control::Choice { value: c.stop_signal.to_string(), options: sig("SIGTERM (default)"), editable: false }, modified: false });
                }
                d.rows.push(Row { id: "terminal".into(), label: "Run in a terminal".into(), help: "In a terminal emulator ($TERMINAL, else foot, alacritty, kitty, konsole, gnome-terminal or xterm), for programs that want one".into(), control: Control::Toggle(c.terminal), modified: false });
                d.buttons = vec!["Cancel".into(), "Apply".into()];
                d.note = "These apply when it starts next.".into();
                self.dialog = Some(d);
            }
            SessAction::TrashMenu(r) => {
                let mut items = Vec::new();
                for c in &st.trash {
                    let name = if c.label.is_empty() { c.name.clone() } else { c.label.clone() };
                    items.push(MenuItem::header(format!("{name} ({})", c.id)));
                    items.push(MenuItem::new("Bring it back", Action::Sess(SessAction::Restore(c.id.clone()))));
                    items.push(MenuItem::new("Forget it (its files stay)", Action::Sess(SessAction::Forget(c.id.clone()))));
                    items.push(MenuItem::new("Delete it and its files…", Action::Sess(SessAction::DeleteClient(c.id.clone()))));
                }
                self.ui.open_menu([r.x, r.bottom() + 2.0], items);
            }
        }
    }

    /// Events from the session dialogs.
    pub fn session_dialog_event(&mut self, kind: Kind, ev: Event) {
        let Some(d) = &mut self.dialog else { return };
        match ev {
            Event::Set(id, v) if (id.starts_with("stemplate:") || id.starts_with("ctemplate:")) && v == "remove" => {
                let (client, name) = (id.starts_with("ctemplate:"), id.split_once(':').map(|x| x.1).unwrap_or_default().to_string());
                let tag = format!("{}:{name}", if client { "c" } else { "s" });
                let what = if client { "client template" } else { "session template" };
                self.confirm(Kind::DeleteTemplate, &tag, &format!("Delete the {what}?"), &format!("{name}, with the files kept with it, goes to your file manager's Trash (Dolphin: Places → Trash), where you can restore it."), &["Cancel", "Move to Trash"]);
            }
            Event::Set(id, v) if id.starts_with("stemplate:") && v == "activate" => {
                let t = id.trim_start_matches("stemplate:").to_string();
                self.dialog = None;
                self.session_action(SessAction::New);
                if let Some(d) = &mut self.dialog
                    && let Some(r) = d.rows.iter_mut().find(|r| r.id == "template")
                    && let Control::Choice { value, .. } = &mut r.control
                {
                    *value = t;
                }
            }
            Event::Set(id, v) if id.starts_with("session:") && v == "remove" => {
                let name = id.trim_start_matches("session:").to_string();
                let msg = format!("{name} goes to your file manager's Trash (Dolphin: Places → Trash), with every client's files in it. You can restore it from there.");
                self.confirm(Kind::DeleteSession, &name, "Delete this session?", &msg, &["Cancel", "Move to Trash"]);
            }
                        Event::Set(id, v) if id.starts_with("session:") && v == "activate" => {
                let name = id.trim_start_matches("session:").to_string();
                self.dialog = None;
                self.send(Request::Open { name });
            }
            Event::Set(id, v) if id.starts_with("ctemplate:") && v == "activate" => {
                let name = id.trim_start_matches("ctemplate:").to_string();
                self.dialog = None;
                self.send(Request::AddClientTemplate { name });
            }
            Event::Set(id, v) if id.starts_with("snap:") && v == "activate" => {
                let id = id.trim_start_matches("snap:").to_string();
                self.dialog = None;
                self.send(Request::Rewind { id });
            }
            Event::Set(id, v) if kind == Kind::Snapshots && id == "auto" => {
                d.set_local(&id, &v);
                self.send(Request::SetAutoSnapshot { on: v == "on" });
            }
            Event::Set(id, v) if kind == Kind::AddApp && id == "find" => {
                self.all_apps.get_or_insert_with(crate::session::scan_all_apps);
                self.app_query = v;
                let rows = self.add_app_rows();
                if let Some(d) = &mut self.dialog {
                    d.set_rows(rows);
                }
            }
            Event::Set(id, v) if id.starts_with("any:") && v == "activate" => {
                let i: usize = id.trim_start_matches("any:").parse().unwrap_or(usize::MAX);
                let Some(a) = self.all_apps.as_ref().and_then(|a| a.get(i)).cloned() else { return };
                self.dialog = None;
                // NSM-capable or not, as its .desktop file says
                self.send(Request::Add { exe: a.exe, args: a.args, nsm: a.nsm, label: if a.nsm { String::new() } else { a.name } });
            }
            Event::Set(id, v) if id.starts_with("app:") && v == "activate" => {
                let i: usize = id.trim_start_matches("app:").parse().unwrap_or(usize::MAX);
                let Some(a) = self.apps.as_ref().and_then(|a| a.get(i)).cloned() else { return };
                self.dialog = None;
                self.send(Request::Add { exe: a.exe, args: a.args, nsm: a.nsm, label: if a.nsm { String::new() } else { a.name } });
            }
            Event::Set(id, v) => d.set_local(&id, &v),
            Event::Button(b) if kind == Kind::DeleteSession => {
                let delete = d.buttons.get(b).is_some_and(|l| l == "Move to Trash");
                let name = d.tag.clone();
                self.dialog = None;
                if delete && self.sess.state.session.as_deref() == Some(name.as_str()) {
                    // The open one: closed (unsaved) first, deleted once closed.
                    self.sess.delete_after_close = Some(name);
                    self.send(Request::Abort);
                    return;
                }
                if delete {
                    self.send(Request::Delete { name });
                }
                // Back to the list.
                self.session_action(SessAction::Open);
            }
            Event::Button(b) if kind == Kind::UpdateDaemon => {
                let go = d.buttons.get(b).is_some_and(|l| l == "Save and restart");
                let name = d.tag.clone();
                self.dialog = None;
                if go {
                    self.sess.reopen_after_update = Some(name);
                    self.send(Request::Close);
                }
            }
            Event::Button(b) if matches!(kind, Kind::RemoveClient | Kind::DeleteClient | Kind::StopClient | Kind::AbortSession | Kind::DeleteTemplate) => {
                let label = d.buttons.get(b).cloned().unwrap_or_default();
                let tag = d.tag.clone();
                self.dialog = None;
                let req = match (kind, label.as_str()) {
                    (_, "Cancel") => None,
                    (Kind::RemoveClient, l) => Some(Request::Remove { id: tag, save: l.starts_with("Save") }),
                    (Kind::DeleteClient, _) => Some(Request::Forget { id: tag, files: true }),
                    (Kind::StopClient, "Save and stop") => Some(Request::SaveStop { id: tag }),
                    (Kind::StopClient, _) => Some(Request::Stop { id: tag }),
                    (Kind::AbortSession, _) => Some(Request::Abort),
                    (Kind::DeleteTemplate, _) => {
                        let (k, name) = tag.split_once(':').unwrap_or(("s", ""));
                        Some(Request::DeleteTemplate { name: name.to_string(), client: k == "c" })
                    }
                    _ => None,
                };
                if let Some(r) = req {
                    self.send(r);
                }
                if kind == Kind::DeleteTemplate {
                    self.templates_page();
                }
            }
                        Event::Button(b) if kind == Kind::ClientProps => {
                let ok = d.buttons.get(b).map(String::as_str) == Some("Apply");
                let d = self.dialog.take().unwrap();
                if !ok {
                    return;
                }
                let mut words = pbx_session::store::split_args(&d.value("cmd").unwrap_or_default());
                if words.is_empty() {
                    return self.say("A client needs a program", true);
                }
                let exe = words.remove(0);
                let env = pbx_session::store::split_args(&d.value("env").unwrap_or_default())
                    .into_iter()
                    .filter_map(|w| w.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                    .collect();
                let num = |id: &str| d.value(id).and_then(|v| v.parse().ok()).unwrap_or(0);
                let props = pbx_session::proto::ClientProps {
                    label: d.value("label").unwrap_or_default().trim().to_string(),
                    exe,
                    args: words,
                    env,
                    save_signal: num("save"),
                    stop_signal: num("stop"),
                    terminal: d.value("terminal").as_deref() == Some("on"),
                };
                self.send(Request::SetClientProps { id: d.tag.clone(), props });
            }
            Event::Button(b) => {
                let label = d.buttons.get(b).cloned().unwrap_or_default();
                let name = d.value("name").unwrap_or_default();
                let req = match (kind, label.as_str()) {
                    (_, "Cancel") => {
                        self.sess.add_after_open = None;
                        None
                    }
                    (Kind::NewSession, _) => Some(Request::New { name, template: d.value("template").filter(|t| !t.is_empty()) }),
                    (Kind::SaveTemplate, _) => Some(Request::SaveTemplate { name }),
                    (Kind::ClientTemplate, _) => Some(Request::SaveClientTemplate { id: d.tag.clone(), name }),
                    (Kind::Rename, _) => Some(Request::Rename { name }),
                    (Kind::Duplicate, _) => Some(Request::Duplicate { name }),
                    (Kind::Notes, _) => Some(Request::SetNotes { notes: d.value("notes").unwrap_or_default() }),
                    (Kind::Snapshot, _) => Some(Request::Snapshot { label: name }),
                    (Kind::AddApp | Kind::AddExe, _) => {
                        let mut words = pbx_session::store::split_args(&d.value("cmd").unwrap_or_default());
                        let nsm = d.value("nsm").as_deref() == Some("on");
                        let label = d.value("label").unwrap_or_default().trim().to_string();
                        if words.is_empty() {
                            if kind == Kind::AddExe {
                                // Keep the dialog: nothing to add yet.
                                return self.say("Type the command to run", true);
                            }
                            None
                        } else {
                            let exe = words.remove(0);
                            Some(Request::Add { exe, args: words, nsm, label })
                        }
                    }
                    _ => None,
                };
                self.dialog = None;
                if let Some(r) = req {
                    self.send(r);
                }
            }
            Event::Close => {
                self.dialog = None;
                self.sess.add_after_open = None;
            }
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
            (_, false) => ("starting the daemon…".to_string(), TEXT_SOFT),
            (Some(n), _) => (n.clone(), HILIGHT),
            (None, _) => ("none open".to_string(), TEXT_SOFT),
        };
        g.text_fit(Face::Bold, px, x0, y, r.right() - x0 - pad, col, &name);
        y += lh + 4.0 * s;
        if let Some(b) = &st.busy {
            g.text_fit(Face::Regular, FONT_SMALL * s, r.x + pad, y, r.w - 2.0 * pad, WARN, b);
            y += lh;
        }
        // An older daemon still running the open session (PatchBox was updated meanwhile).
        if self.sess.stale() && st.session.is_some() && st.busy.is_none() {
            let spx = FONT_SMALL * s;
            let bh = (lh + 10.0 * s).round();
            let bw = g.measure(Face::Regular, px, "Update") + 28.0 * s;
            let tw = r.w - 2.0 * pad - bw - 8.0 * s;
            let lines = crate::ui::wrap(g, spx, "Its daemon is from before the update", tw);
            let slh = g.line_height(Face::Regular, spx);
            let th = lines.len() as f32 * slh;
            for (k, l) in lines.iter().enumerate() {
                g.text(Face::Regular, spx, r.x + pad, y + (bh - th) / 2.0 + k as f32 * slh, WARN, l);
            }
            let ur = Rect::new(r.right() - pad - bw, y, bw, bh);
            self.sbutton(g, ur, "Update", Action::Sess(SessAction::UpdateDaemon));
            y += bh + 6.0 * s;
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
                crate::ui::lbutton(g, br, s, crate::ui::tint_for(label), false, false, false);
                g.text_center(Face::Regular, px, br, TEXT_DIM, label);
            }
            bx += bw + 4.0 * s;
        }
        let mr = Rect::new(bx, y, more_w, bh);
        self.sbutton(g, mr, "⋯", Action::Sess(SessAction::Menu(mr)));
        y += bh + 8.0 * s;
        if !open {
            // Nothing open: say what a session is for, and offer "+ Application" anyway (it makes
            // the session first).
            if self.sess.connected && st.busy.is_none() {
                let spx = FONT_SMALL * s;
                let slh = g.line_height(Face::Regular, spx);
                let hint = "A session keeps programs, their files and their connections. New starts one; adding a program below starts one too.";
                for line in crate::ui::wrap(g, spx, hint, r.w - 2.0 * pad) {
                    g.text(Face::Regular, spx, r.x + pad, y, TEXT_SOFT, &line);
                    y += slh;
                }
                y += 6.0 * s;
                y = self.add_buttons(g, r, y, bh);
            }
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
            // Icons on the right: GUI eye, save, start/stop, remove.
            let ib = row_h - 4.0 * s;
            let mut ix = rr.right() - ib - 2.0 * s;
            let running = c.pid.is_some();
            self.icon(g, Rect::new(ix, rr.y + 2.0 * s, ib, ib), Icon::Remove, Action::Sess(SessAction::Remove(c.id.clone())));
            ix -= ib + 2.0 * s;
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
        // + Application, Executable; the trash.
        y = self.add_buttons(g, r, y + 2.0 * s, bh);
        if !st.trash.is_empty() {
            let label = format!("Removed {}", st.trash.len());
            let tw = g.measure(Face::Regular, px, &label) + 24.0 * s;
            let tr = Rect::new(r.right() - pad - tw, y - 4.0 * s, tw, bh);
            self.sbutton(g, tr, &label, Action::Sess(SessAction::TrashMenu(tr)));
            y += bh + 6.0 * s;
        }
        if st.connections_saved > 0 {
            let t = format!("connections: {} saved, {} present", st.connections_saved, st.connections_present);
            g.text_fit(Face::Regular, FONT_SMALL * s, r.x + pad, y, r.w - 2.0 * pad, TEXT_SOFT, &t);
            y += lh;
        }
        y
    }

    /// "+ Application" (from the installed ones) and "Executable" (any command), as RaySession.
    fn add_buttons(&mut self, g: &mut Gfx, r: Rect, y: f32, bh: f32) -> f32 {
        let s = self.scale;
        let pad = 10.0 * s;
        let gap = 6.0 * s;
        let w = ((r.w - 2.0 * pad - gap) / 2.0).floor();
        self.sbutton(g, Rect::new(r.x + pad, y, w, bh), "+ Application", Action::Sess(SessAction::AddApp));
        self.sbutton(g, Rect::new(r.x + pad + w + gap, y, w, bh), "Executable", Action::Sess(SessAction::AddExe));
        y + bh + 10.0 * s
    }

    fn sbutton(&mut self, g: &mut Gfx, r: Rect, label: &str, a: Action) {
        let s = self.scale;
        let hov = r.contains(self.ui.mouse[0], self.ui.mouse[1]) && self.ui.menu.is_none() && self.dialog.is_none();
        let tint = crate::ui::tint_for(label);
        crate::ui::lbutton(g, r, s, tint, hov, false, true);
        if label == "⋯" {
            // The font has no ⋯: three dots.
            for k in -1..=1 {
                g.circle([r.cx() + k as f32 * 5.0 * s, r.cy()], 1.6 * s, TEXT_BRIGHT);
            }
        } else {
            g.text_center(Face::Regular, FONT * s, r, crate::ui::lbutton_text(tint, hov, false, true), label);
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
            Icon::Remove => {
                let c = if hov { BAD } else { TEXT };
                let k = k * 0.8;
                g.line([cx - k, cy - k], [cx + k, cy + k], 1.5 * s, c);
                g.line([cx - k, cy + k], [cx + k, cy - k], 1.5 * s, c);
            }
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
    Remove,
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

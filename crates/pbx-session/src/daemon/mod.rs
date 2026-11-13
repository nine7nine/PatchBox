//! patchboxd: the session manager. One thread, one poll() loop over the control socket, the NSM
//! UDP socket, a signalfd for SIGCHLD and an eventfd the JACK notification thread writes to.
//!
//! Session operations (open, save, close…) are queues of steps; a step either finishes at once or
//! waits for clients (replies, exits) until its deadline. Timeouts follow RaySession's
//! (docs/ref-raysession-sessions.md).

mod clients;
mod ctemplates;
mod nsm;
mod patch;
mod snapshots;

use crate::proto::{self, ClientState, FromDaemon, Push, Reply, Request, State, Status};
use crate::store::{self, ClientEntry, SessionFile};
use clients::Client;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::UdpSocket;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) struct Ctl {
    stream: UnixStream,
    buf: Vec<u8>,
    subscribed: bool,
    dead: bool,
}

pub(crate) struct Session {
    pub name: String,
    pub dir: PathBuf,
    pub file: SessionFile,
    pub clients: Vec<Client>,
    pub trash: Vec<ClientEntry>,
    pub loaded: bool,
    /// When it opened (recent sessions: those open for more than 30 s).
    pub opened: Instant,
}

#[derive(Debug)]
enum Step {
    SaveClients,
    WriteFile,
    StopClients,
    Unload,
    /// Open (or create) a session by name.
    Load(String, bool),
    CopyTo(String),
    /// Copy the session into the templates (name).
    ToTemplate(String),
    /// Make the new session (name) from a template (template, name).
    FromTemplate(String, String),
    MoveTo(String),
    /// Snapshot the session folder: (label, auto: skip when nothing changed).
    Snapshot(String, bool),
    /// Put the folder back as it was in a snapshot (clients stopped).
    Checkout(String),
    /// Run the session's hook script of that name (load, save, close), if it has one.
    Script(&'static str),
    /// Before a switch to the session of that name: keep the running clients it can use.
    Reserve(String),
    /// Put the session's remembered JACK settings back (jackdbus), before its clients start.
    Jack,
    Launch,
    WaitReady,
    Loaded,
}

struct Op {
    label: String,
    steps: VecDeque<Step>,
    /// The current step started (it waits) and must end by then.
    deadline: Option<Instant>,
    started: bool,
}

pub struct Daemon {
    root: PathBuf,
    listener: UnixListener,
    ctls: Vec<Ctl>,
    pub(crate) udp: UdpSocket,
    pub(crate) nsm_url: String,
    sigfd: RawFd,
    wake_fd: RawFd,
    pub(crate) jack: Option<pbx_jack::Client>,
    jack_retry: Instant,
    pub(crate) session: Option<Session>,
    op: Option<Op>,
    pub(crate) patch: patch::PatchMem,
    last_state: Option<State>,
    /// A git child running for a snapshot or a checkout, and how it ended.
    git: Option<u32>,
    git_status: Option<i32>,
    /// Clients kept running across a session switch: (the new session's client id, client).
    reserved: Vec<(String, Client)>,
    /// A jackdbus job running on a thread, and its result.
    jack_job: Option<std::sync::mpsc::Receiver<Result<bool, String>>>,
    quit: bool,
    /// No session and no front end since then: exit after a while.
    idle_since: Option<Instant>,
    /// Its executable was replaced on disk (checked every few seconds); with no session it
    /// re-executes as the new one.
    outdated: bool,
    exe_check_at: Instant,
    reexec: bool,
    /// When the clients' logs were last checked for size.
    logs_checked: Instant,
    /// Connection rules: made whenever their ports are there, session or not.
    pub(crate) rules: crate::rules::Rules,
}

const IDLE_EXIT: Duration = Duration::from_secs(30);

fn eventfd() -> RawFd {
    unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) }
}

fn sigchld_fd() -> RawFd {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGCHLD);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        libc::signalfd(-1, &set, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK)
    }
}

fn drain_fd(fd: RawFd) {
    let mut buf = [0u8; 512];
    while unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
}

pub fn run() -> Result<(), String> {
    let sock = proto::socket_path();
    if let Some(dir) = sock.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if UnixStream::connect(&sock).is_ok() {
        return Err(format!("another patchboxd is running ({})", sock.display()));
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let udp = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    udp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = udp.local_addr().map_err(|e| e.to_string())?.port();
    let nsm_url = format!("osc.udp://127.0.0.1:{port}/");
    let root = proto::sessions_root();
    let _ = std::fs::create_dir_all(&root);
    let mut d = Daemon {
        root,
        listener,
        ctls: Vec::new(),
        udp,
        nsm_url,
        sigfd: sigchld_fd(),
        wake_fd: eventfd(),
        jack: None,
        jack_retry: Instant::now(),
        session: None,
        op: None,
        patch: patch::PatchMem::default(),
        last_state: None,
        git: None,
        git_status: None,
        reserved: Vec::new(),
        jack_job: None,
        quit: false,
        idle_since: Some(Instant::now()),
        outdated: false,
        exe_check_at: Instant::now(),
        reexec: false,
        logs_checked: Instant::now(),
        rules: crate::rules::Rules::load(),
    };
    eprintln!("[patchboxd] {} · NSM at {} · sessions in {}", sock.display(), d.nsm_url, d.root.display());
    d.main_loop();
    let reexec = d.reexec;
    drop(d);
    let _ = std::fs::remove_file(&sock);
    if reexec && let Some(exe) = new_exe() {
        // Updated on disk: the same process, the new program (the window reconnects to it).
        use std::os::unix::process::CommandExt;
        eprintln!("[patchboxd] updated on disk: restarting as {}", exe.display());
        let err = std::process::Command::new(&exe).args(std::env::args_os().skip(1)).exec();
        return Err(format!("{}: {err}", exe.display()));
    }
    Ok(())
}

/// Our executable's path, if the file we run from was replaced (an update): /proc/self/exe then
/// reads "<path> (deleted)", and the new file is at <path>.
fn new_exe() -> Option<PathBuf> {
    let p = std::fs::read_link("/proc/self/exe").ok()?;
    let s = p.to_str()?.strip_suffix(" (deleted)")?;
    let p = PathBuf::from(s);
    p.is_file().then_some(p)
}

impl Daemon {
    fn main_loop(&mut self) {
        while !self.quit {
            let timeout = self.next_deadline().map(|t| t.saturating_duration_since(Instant::now()).as_millis().min(60_000) as i32).unwrap_or(60_000);
            let mut fds = vec![
                libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.udp.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.sigfd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.wake_fd, events: libc::POLLIN, revents: 0 },
            ];
            for c in &self.ctls {
                fds.push(libc::pollfd { fd: c.stream.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            }
            let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout.max(0)) };
            if n < 0 {
                continue; // EINTR
            }
            if fds[0].revents != 0 {
                self.accept();
            }
            if fds[1].revents != 0 {
                self.read_udp();
            }
            if fds[2].revents != 0 {
                drain_fd(self.sigfd);
                self.reap();
            }
            if fds[3].revents != 0 {
                drain_fd(self.wake_fd);
                self.jack_events();
            }
            let ready: Vec<usize> = fds[4..].iter().enumerate().filter(|(_, f)| f.revents != 0).map(|(i, _)| i).collect();
            for i in ready {
                self.read_ctl(i);
            }
            self.ctls.retain(|c| !c.dead);
            self.timers();
            self.sweep_removed();
            self.push_state();
        }
        // Leaving: stop what runs, like close without saving would.
        if let Some(s) = &mut self.session {
            for c in &mut s.clients {
                c.stop();
            }
            let _ = std::fs::remove_file(s.dir.join(store::LOCK));
        }
    }

    /// Clients removed while running go to the trash once they have stopped.
    fn sweep_removed(&mut self) {
        let Some(s) = &mut self.session else { return };
        let mut gone = Vec::new();
        let mut i = 0;
        while i < s.clients.len() {
            if s.clients[i].remove_after_stop && !s.clients[i].running() {
                let c = s.clients.remove(i);
                gone.push(c.entry.id.clone());
                s.trash.push(c.entry);
            } else {
                i += 1;
            }
        }
        for id in gone {
            self.log(format!("{id} stopped and removed"), false);
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        let mut next: Option<Instant> = None;
        let mut at = |t: Instant| next = Some(next.map_or(t, |n: Instant| n.min(t)));
        if let Some(op) = &self.op {
            match op.deadline {
                Some(d) => at(d),
                None => at(Instant::now()),
            }
            // Waiting steps re-check often: replies and exits come as events, deadlines here.
            at(Instant::now() + Duration::from_millis(250));
        }
        if self.jack.is_none() {
            at(self.jack_retry);
        }
        if let Some(s) = &self.session {
            for c in &s.clients {
                if let Some(t) = c.next_deadline() {
                    at(t);
                }
            }
        }
        if let Some(t) = self.patch.due {
            at(t);
        }
        if let Some(t) = self.idle_since {
            at(t + IDLE_EXIT);
        }
        next
    }

    // ---- control socket ----

    fn accept(&mut self) {
        while let Ok((stream, _)) = self.listener.accept() {
            let _ = stream.set_nonblocking(true);
            self.ctls.push(Ctl { stream, buf: Vec::new(), subscribed: false, dead: false });
            self.idle_since = None;
        }
    }

    fn read_ctl(&mut self, i: usize) {
        let mut tmp = [0u8; 4096];
        loop {
            match self.ctls[i].stream.read(&mut tmp) {
                Ok(0) => {
                    self.ctls[i].dead = true;
                    break;
                }
                Ok(n) => self.ctls[i].buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.ctls[i].dead = true;
                    break;
                }
            }
            if self.ctls[i].buf.len() > 1 << 20 {
                self.ctls[i].dead = true;
                break;
            }
        }
        while let Some(nl) = self.ctls[i].buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.ctls[i].buf.drain(..=nl).collect();
            let reply = match serde_json::from_slice::<Request>(&line) {
                Ok(r) => self.request(Some(i), r),
                Err(e) => Reply::Error { msg: format!("bad request: {e}") },
            };
            self.send_to(i, &FromDaemon::Reply(reply));
        }
    }

    fn send_to(&mut self, i: usize, m: &FromDaemon) {
        let Some(c) = self.ctls.get_mut(i) else { return };
        let mut line = serde_json::to_vec(m).unwrap_or_default();
        line.push(b'\n');
        // Front ends read promptly; a blocked one is dropped rather than stalling the daemon.
        let _ = c.stream.set_nonblocking(false);
        let _ = c.stream.set_write_timeout(Some(Duration::from_millis(500)));
        if c.stream.write_all(&line).is_err() {
            c.dead = true;
        }
        let _ = c.stream.set_nonblocking(true);
    }

    pub(crate) fn log(&mut self, text: impl Into<String>, bad: bool) {
        let text = text.into();
        eprintln!("[patchboxd] {text}");
        let m = FromDaemon::Push(Push::Log { text, bad });
        for i in 0..self.ctls.len() {
            if self.ctls[i].subscribed {
                self.send_to(i, &m);
            }
        }
    }

    pub(crate) fn state(&self) -> State {
        let mut st = State { root: self.root.display().to_string(), nsm_url: self.nsm_url.clone(), ..Default::default() };
        st.busy = self.op.as_ref().map(|o| o.label.clone());
        st.proto = proto::PROTO;
        st.outdated = self.outdated;
        st.connections_saved = self.patch.saved.len();
        st.connections_present = self.patch.present;
        if let Some(s) = &self.session {
            st.session = Some(s.name.clone());
            st.path = Some(s.dir.display().to_string());
            st.notes = s.file.notes.clone();
            st.auto_snapshot = s.file.auto_snapshot.unwrap_or(true);
            st.jack_memory = s.file.jack.remember;
            st.clients = s.clients.iter().map(Client::state).collect();
            st.trash = s
                .trash
                .iter()
                .map(|e| ClientState { id: e.id.clone(), name: e.name.clone(), label: e.label.clone(), exe: e.exe.clone(), nsm: e.nsm, ..Default::default() })
                .collect();
        }
        st
    }

    fn push_state(&mut self) {
        let st = self.state();
        if self.last_state.as_ref() == Some(&st) {
            return;
        }
        let m = FromDaemon::Push(Push::State(st.clone()));
        for i in 0..self.ctls.len() {
            if self.ctls[i].subscribed {
                self.send_to(i, &m);
            }
        }
        self.last_state = Some(st);
    }

    pub(crate) fn root_dir(&self) -> PathBuf {
        self.root.clone()
    }

    /// A request from an NSM client's server control.
    pub(crate) fn request_from_nsm(&mut self, r: Request) -> Result<String, String> {
        match self.request(None, r) {
            Reply::Ok { msg } => Ok(msg),
            Reply::Error { msg } => Err(msg),
            _ => Ok("ok".into()),
        }
    }

    fn request(&mut self, from: Option<usize>, r: Request) -> Reply {
        let ok = |m: &str| Reply::Ok { msg: m.to_string() };
        let busy = || Reply::Error { msg: "an operation is in progress".into() };
        match r {
            Request::Subscribe => {
                if let Some(f) = from {
                    self.ctls[f].subscribed = true;
                }
                self.last_state = None;
                ok("subscribed")
            }
            Request::State => Reply::State(self.state()),
            Request::ListSessions => {
                let sessions = store::list(&self.root);
                // Recent ones that still exist (and not the one open now).
                let open = self.session.as_ref().map(|s| s.name.clone());
                let recent = store::recent::load().into_iter().filter(|n| sessions.iter().any(|s| &s.name == n) && Some(n) != open.as_ref()).collect();
                Reply::Sessions { root: self.root.display().to_string(), sessions, recent }
            }
            Request::New { .. } | Request::Open { .. } if self.op.is_some() => busy(),
            Request::New { name, template } => match self.check_new(&name) {
                Err(e) => Reply::Error { msg: e },
                Ok(()) => {
                    let n = name.trim().to_string();
                    match template.filter(|t| !t.is_empty()) {
                        Some(t) => {
                            if !proto::templates_root().join(&t).join(store::FILE).is_file() {
                                return Reply::Error { msg: format!("no template \"{t}\"") };
                            }
                            let mut steps = self.switch_steps(Step::Load(n.clone(), false));
                            let at = steps.iter().position(|s| matches!(s, Step::Load(..))).unwrap_or(0);
                            steps.insert(at, Step::FromTemplate(t, n));
                            self.start_op(format!("Creating {name}…"), steps);
                        }
                        None => self.start_op(format!("Creating {name}…"), self.switch_steps(Step::Load(n, true))),
                    }
                    ok("creating")
                }
            },
            Request::ListTemplates => {
                let mut t: Vec<String> = std::fs::read_dir(proto::templates_root())
                    .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().join(store::FILE).is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                t.sort();
                Reply::Templates { templates: t }
            }
            Request::SaveTemplate { name } => {
                if self.session.is_none() {
                    return Reply::Error { msg: "no session is open".into() };
                }
                if self.op.is_some() {
                    return busy();
                }
                if let Err(e) = store::valid_name(&name).and_then(|_| if name.contains('/') { Err("a template name has no '/'".to_string()) } else { Ok(()) }) {
                    return Reply::Error { msg: e };
                }
                self.start_op(format!("Saving template {name}…"), vec![Step::SaveClients, Step::WriteFile, Step::ToTemplate(name.trim().to_string())]);
                ok("saving template")
            }
            Request::Open { name } => {
                if !self.root.join(name.trim()).join(store::FILE).is_file() {
                    return Reply::Error { msg: format!("no session \"{name}\"") };
                }
                self.start_op(format!("Opening {name}…"), self.switch_steps(Step::Load(name.trim().to_string(), false)));
                ok("opening")
            }
            Request::Save | Request::Close | Request::Duplicate { .. } | Request::Rename { .. } if self.session.is_none() => {
                Reply::Error { msg: "no session is open".into() }
            }
            Request::Save if self.op.is_some() => busy(),
            Request::Save => {
                let snap = self.auto_snapshot();
                self.start_op("Saving…".into(), vec![Step::SaveClients, Step::WriteFile, snap, Step::Script("save")]);
                ok("saving")
            }
            Request::Close if self.op.is_some() => busy(),
            Request::Close => {
                let snap = self.auto_snapshot();
                self.start_op("Closing…".into(), vec![Step::SaveClients, Step::WriteFile, snap, Step::Script("close"), Step::StopClients, Step::Unload]);
                ok("closing")
            }
            Request::Abort => {
                if self.session.is_none() && self.op.is_none() {
                    return Reply::Error { msg: "no session is open".into() };
                }
                self.op = None;
                self.start_op("Aborting…".into(), vec![Step::StopClients, Step::Unload]);
                ok("aborting")
            }
            Request::Duplicate { name } => match self.check_new(&name) {
                Err(e) => Reply::Error { msg: e },
                Ok(()) if self.op.is_some() => busy(),
                Ok(()) => {
                    let n = name.trim().to_string();
                    let mut steps = vec![Step::SaveClients, Step::WriteFile, Step::CopyTo(n.clone()), Step::Script("close"), Step::StopClients, Step::Unload];
                    steps.extend([Step::Load(n, false), Step::Jack, Step::Launch, Step::WaitReady, Step::Loaded, Step::Script("load")]);
                    self.start_op(format!("Duplicating to {name}…"), steps);
                    ok("duplicating")
                }
            },
            Request::Rename { name } => match self.check_new(&name) {
                Err(e) => Reply::Error { msg: e },
                Ok(()) if self.op.is_some() => busy(),
                Ok(()) => {
                    let n = name.trim().to_string();
                    let mut steps = vec![Step::SaveClients, Step::WriteFile, Step::Script("close"), Step::StopClients, Step::MoveTo(n.clone()), Step::Unload];
                    steps.extend([Step::Load(n, false), Step::Jack, Step::Launch, Step::WaitReady, Step::Loaded, Step::Script("load")]);
                    self.start_op(format!("Renaming to {name}…"), steps);
                    ok("renaming")
                }
            },
            Request::Delete { name } => {
                let n = name.trim();
                if let Err(e) = store::valid_name(n) {
                    return Reply::Error { msg: e };
                }
                let dir = self.root.join(n);
                if !dir.join(store::FILE).is_file() {
                    return Reply::Error { msg: format!("no session \"{n}\"") };
                }
                if self.session.as_ref().is_some_and(|s| s.dir == dir || s.name == n) {
                    return Reply::Error { msg: format!("{n} is open: close it first") };
                }
                if self.op.is_some() {
                    return busy();
                }
                if let Some(pid) = locked_by_other(&dir) {
                    return Reply::Error { msg: format!("{n} is open in another patchboxd (pid {pid})") };
                }
                match crate::trash::to_trash(&dir) {
                    Ok(_) => {
                        self.log(format!("{n} moved to the Trash"), false);
                        ok("deleted")
                    }
                    Err(e) => Reply::Error { msg: format!("{n} not deleted: {e}") },
                }
            }
            Request::SetNotes { notes } => match &mut self.session {
                Some(s) => {
                    s.file.notes = notes;
                    ok("notes set")
                }
                None => Reply::Error { msg: "no session is open".into() },
            },
            Request::Add { exe, args, nsm, label } => self.add_client(exe, args, nsm, label),
            Request::Start { id } => self.with_client(&id, |d, i| d.launch(i)),
            Request::Stop { id } => self.with_client(&id, |d, i| {
                d.session.as_mut().unwrap().clients[i].stop();
                Ok("stopping".into())
            }),
            Request::SaveStop { id } => self.with_client(&id, |d, i| {
                d.save_client(i)?;
                d.session.as_mut().unwrap().clients[i].stop_after_save = true;
                Ok("saving, then stopping".into())
            }),
            Request::Kill { id } => self.with_client(&id, |d, i| {
                d.session.as_mut().unwrap().clients[i].kill();
                Ok("killed".into())
            }),
            Request::SaveClient { id } => self.with_client(&id, |d, i| d.save_client(i)),
            Request::SaveClientTemplate { id, name } => self.save_client_template(&id, &name),
            Request::ListClientTemplates => self.list_client_templates(),
            Request::AddClientTemplate { name } => self.add_client_template(&name),
            Request::SetClientProps { id, props } => self.with_client(&id.clone(), move |d, i| {
                let s = d.session.as_mut().ok_or("no session")?;
                let e = &mut s.clients[i].entry;
                if props.exe.trim().is_empty() {
                    return Err("a client needs a program".into());
                }
                e.label = props.label;
                e.exe = props.exe;
                e.args = props.args;
                e.env = props.env;
                e.save_signal = props.save_signal;
                e.stop_signal = props.stop_signal;
                e.terminal = props.terminal;
                d.write_file();
                Ok(format!("{id}: properties set (from its next start)"))
            }),
            Request::ShowGui { id } => self.with_client(&id, |d, i| d.gui(i, true)),
            Request::HideGui { id } => self.with_client(&id, |d, i| d.gui(i, false)),
            Request::SetLabel { id, label } => self.with_client(&id, |d, i| {
                d.session.as_mut().unwrap().clients[i].entry.label = label;
                Ok("label set".into())
            }),
            Request::Trash { id } => self.with_client(&id, |d, i| {
                let s = d.session.as_mut().unwrap();
                if s.clients[i].running() {
                    return Err("stop it first".into());
                }
                let c = s.clients.remove(i);
                s.trash.push(c.entry);
                Ok("moved to the trash".into())
            }),
            Request::Restore { id } => match &mut self.session {
                Some(s) => match s.trash.iter().position(|e| e.id == id) {
                    Some(i) => {
                        let e = s.trash.remove(i);
                        s.clients.push(Client::new(e));
                        ok("restored")
                    }
                    None => Reply::Error { msg: format!("no trashed client {id}") },
                },
                None => Reply::Error { msg: "no session is open".into() },
            },
            Request::Forget { id, files } => match &mut self.session {
                Some(s) => {
                    let Some(e) = s.trash.iter().find(|e| e.id == id).cloned() else {
                        return Reply::Error { msg: format!("no removed client {id}") };
                    };
                    if files {
                        // Its files: those named after it in the session folder (the path NSM gave it).
                        let p = ctemplates::prefix(&e);
                        let mut n = 0;
                        for f in std::fs::read_dir(&s.dir).into_iter().flatten().flatten() {
                            let name = f.file_name().to_string_lossy().into_owned();
                            if name == p || name.starts_with(&format!("{p}.")) {
                                if let Err(err) = crate::trash::to_trash(&f.path()) {
                                    return Reply::Error { msg: format!("{id}: {err}") };
                                }
                                n += 1;
                            }
                        }
                        s.trash.retain(|e| e.id != id);
                        self.log(format!("{id} deleted; {n} file{} moved to the Trash", if n == 1 { "" } else { "s" }), false);
                        return ok("deleted");
                    }
                    s.trash.retain(|e| e.id != id);
                    ok("forgotten")
                }
                None => Reply::Error { msg: "no session is open".into() },
            },
            Request::Remove { id, save } => self.with_client(&id, |d, i| {
                let s = d.session.as_mut().unwrap();
                if !s.clients[i].running() {
                    let c = s.clients.remove(i);
                    let name = if c.entry.label.is_empty() { c.entry.name.clone() } else { c.entry.label.clone() };
                    s.trash.push(c.entry);
                    d.log(format!("{name} removed (Removed, in the session panel, brings it back)"), false);
                    return Ok("removed".into());
                }
                if save {
                    d.save_client(i)?;
                    let c = &mut d.session.as_mut().unwrap().clients[i];
                    c.stop_after_save = true;
                    c.remove_after_stop = true;
                    return Ok("saving, stopping, then removing".into());
                }
                let c = &mut s.clients[i];
                c.remove_after_stop = true;
                c.stop();
                Ok("stopping, then removing".into())
            }),
            Request::DeleteTemplate { name, client } => {
                let n = name.trim();
                if n.is_empty() || n.contains('/') || n.starts_with('.') {
                    return Reply::Error { msg: format!("no template \"{n}\"") };
                }
                let dir = if client { proto::client_templates_root().map(|r| r.join(n)) } else { Some(proto::templates_root().join(n)) };
                match dir.filter(|d| d.is_dir()) {
                    None => Reply::Error { msg: format!("no template \"{n}\"") },
                    Some(d) => match crate::trash::to_trash(&d) {
                        Ok(_) => {
                            self.log(format!("template {n} moved to the Trash"), false);
                            ok("deleted")
                        }
                        Err(e) => Reply::Error { msg: format!("template {n} not deleted: {e}") },
                    },
                }
            }
            Request::Connect { src, dst } | Request::Disconnect { src, dst } if self.jack.is_none() => {
                let _ = (src, dst);
                Reply::Error { msg: "not connected to JACK".into() }
            }
            Request::Connect { src, dst } => {
                if self.jack.as_ref().unwrap().connect(&src, &dst) { ok("connected") } else { Reply::Error { msg: format!("cannot connect {src} -> {dst}") } }
            }
            Request::Disconnect { src, dst } => {
                if self.jack.as_ref().unwrap().disconnect(&src, &dst) { ok("disconnected") } else { Reply::Error { msg: format!("cannot disconnect {src} -> {dst}") } }
            }
            Request::Graph => match &self.jack {
                Some(j) => {
                    let g = j.snapshot();
                    let mut lines: Vec<String> = g.conns.iter().map(|&(o, i)| format!("{} -> {}", g.ports[o].name, g.ports[i].name)).collect();
                    lines.sort();
                    Reply::Ok { msg: lines.join("\n") }
                }
                None => Reply::Error { msg: "not connected to JACK".into() },
            },
            Request::Snapshot { .. } | Request::Rewind { .. } | Request::SetAutoSnapshot { .. } | Request::SetJackMemory { .. } | Request::ListSnapshots
                if self.session.is_none() =>
            {
                Reply::Error { msg: "no session is open".into() }
            }
            Request::Snapshot { .. } | Request::Rewind { .. } if self.op.is_some() => busy(),
            Request::Snapshot { label } => {
                let label = if label.trim().is_empty() { "snapshot".to_string() } else { label.trim().to_string() };
                self.start_op("Taking a snapshot…".into(), vec![Step::SaveClients, Step::WriteFile, Step::Snapshot(label, false)]);
                ok("snapshot")
            }
            Request::ListSnapshots => Reply::Snapshots { snapshots: snapshots::list(&self.session.as_ref().unwrap().dir) },
            Request::Rewind { id } => {
                let s = self.session.as_ref().unwrap();
                if !snapshots::list(&s.dir).iter().any(|e| e.id == id) {
                    return Reply::Error { msg: format!("no snapshot {id}") };
                }
                let name = s.name.clone();
                let steps = vec![
                    Step::SaveClients,
                    Step::WriteFile,
                    Step::Snapshot(format!("before going back to {id}"), false),
                    Step::Script("close"),
                    Step::StopClients,
                    Step::Checkout(id.clone()),
                    Step::Unload,
                    Step::Load(name, false),
                    Step::Jack,
                    Step::Launch,
                    Step::WaitReady,
                    Step::Loaded,
                    Step::Script("load"),
                ];
                self.start_op(format!("Going back to {id}…"), steps);
                ok("rewinding")
            }
            Request::SetJackMemory { on } => {
                let s = self.session.as_mut().unwrap();
                s.file.jack.remember = on;
                if !on {
                    s.file.jack = Default::default();
                }
                ok(if on { "JACK settings kept with the session from its next save" } else { "JACK settings no longer kept" })
            }
            Request::SetAutoSnapshot { on } => {
                self.session.as_mut().unwrap().file.auto_snapshot = Some(on);
                ok(if on { "automatic snapshots on" } else { "automatic snapshots off" })
            }
            Request::Rules => Reply::Rules { rules: self.rules.list.clone() },
            Request::SetRules { rules } => match self.rules.set(rules) {
                Ok(()) => {
                    // Applied to the ports there now, then as ports appear.
                    self.patch.due = Some(Instant::now());
                    Reply::Rules { rules: self.rules.list.clone() }
                }
                Err(e) => Reply::Error { msg: format!("rules: {e}") },
            },
            Request::Quit => {
                self.quit = true;
                ok("bye")
            }
        }
    }

    fn with_client(&mut self, id: &str, f: impl FnOnce(&mut Daemon, usize) -> Result<String, String>) -> Reply {
        let Some(i) = self.session.as_ref().and_then(|s| s.clients.iter().position(|c| c.entry.id == id)) else {
            return Reply::Error { msg: format!("no client {id}") };
        };
        match f(self, i) {
            Ok(m) => Reply::Ok { msg: m },
            Err(e) => Reply::Error { msg: e },
        }
    }

    fn check_new(&self, name: &str) -> Result<(), String> {
        store::valid_name(name)?;
        let dir = self.root.join(name.trim());
        if dir.exists() {
            return Err(format!("\"{}\" exists already", name.trim()));
        }
        if let Some(s) = store::inside_session(&self.root, &dir) {
            return Err(format!("\"{}\" would be inside the session {}", name.trim(), s.display()));
        }
        Ok(())
    }

    /// Close the current session (saving it) before `load`.
    fn switch_steps(&self, load: Step) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.session.is_some() {
            let snap = self.auto_snapshot();
            steps.extend([Step::SaveClients, Step::WriteFile, snap, Step::Script("close")]);
            // Opening an existing session: clients it can use carry on (:switch:).
            if let Step::Load(name, false) = &load {
                steps.push(Step::Reserve(name.clone()));
            }
            steps.extend([Step::StopClients, Step::Unload]);
        }
        steps.extend([load, Step::Jack, Step::Launch, Step::WaitReady, Step::Loaded, Step::Script("load")]);
        steps
    }

    fn start_op(&mut self, label: String, steps: Vec<Step>) {
        self.log(label.clone(), false);
        self.op = Some(Op { label, steps: steps.into(), deadline: None, started: false });
        self.idle_since = None;
    }

    // ---- operations ----

    fn timers(&mut self) {
        let now = Instant::now();
        if now >= self.exe_check_at {
            self.exe_check_at = now + Duration::from_secs(5);
            self.outdated = new_exe().is_some();
        }
        // Updated, nothing open and nobody connected: become the new version now. (A window keeps
        // a connection: it replaces us itself, after offering to save, close and reopen a session.
        // A request from patchbox-ctl is answered first: its connection ends, then we restart.)
        if self.outdated && self.session.is_none() && self.op.is_none() && self.ctls.is_empty() && !self.quit {
            self.reexec = true;
            self.quit = true;
        }
        if self.jack.is_none() && now >= self.jack_retry {
            self.connect_jack();
        }
        if let Some(s) = &mut self.session {
            let mut msgs = Vec::new();
            for c in &mut s.clients {
                if let Some(m) = c.timers(now) {
                    msgs.push(m);
                }
            }
            // The loop wakes at least once a minute: the clients' logs stay under their cap.
            if now.duration_since(self.logs_checked) >= std::time::Duration::from_secs(30) {
                self.logs_checked = now;
                msgs.extend(s.clients.iter().filter_map(|c| c.cap_log()).map(|m| (m, false)));
            }
            for (m, bad) in msgs {
                self.log(m, bad);
            }
        }
        if self.patch.due.is_some_and(|t| now >= t) {
            self.patch.due = None;
            self.restore_connections();
        }
        // Run operation steps until one waits.
        for _ in 0..32 {
            if !self.step() {
                break;
            }
        }
        // Nothing to manage and nobody watching: go away. Connection rules are something to
        // manage: they hold whenever ports appear, the window open or not.
        let idle = self.session.is_none() && self.op.is_none() && self.ctls.is_empty() && self.rules.list.is_empty();
        match (idle, self.idle_since) {
            (true, None) => self.idle_since = Some(now),
            (true, Some(t)) if now >= t + IDLE_EXIT && std::env::var_os("PATCHBOXD_STAY").is_none() => {
                eprintln!("[patchboxd] idle: exiting");
                self.quit = true;
            }
            (false, Some(_)) => self.idle_since = None,
            _ => {}
        }
    }

    /// Advance the operation by one step. True: call again (the step finished).
    fn step(&mut self) -> bool {
        let Some(op) = &mut self.op else { return false };
        let Some(step) = op.steps.front() else {
            let label = op.label.trim_end_matches('…').to_string();
            self.op = None;
            self.log(format!("{label}: done"), false);
            return false;
        };
        let now = Instant::now();
        let first = !op.started;
        op.started = true;
        let deadline = op.deadline;
        let late = deadline.is_some_and(|d| now >= d);
        let done = match step {
            Step::SaveClients => {
                if first {
                    let saved: Vec<usize> = self.session.as_mut().map_or(Vec::new(), |s| s.clients.iter_mut().enumerate().filter_map(|(k, c)| c.save().then_some(k)).collect());
                    let n = saved.len();
                    // Plain programs told by signal: their save hook now (NSM ones' at their reply).
                    for k in saved {
                        if let Some(c) = self.session.as_ref().map(|s| &s.clients[k]).filter(|c| !c.entry.nsm) {
                            self.client_hook("save", c);
                        }
                    }
                    self.set_deadline(if n > 0 { Some(now + Duration::from_secs(10)) } else { None });
                }
                let waiting = self.session.as_ref().is_some_and(|s| s.clients.iter().any(|c| c.status == Status::Saving));
                if late && waiting {
                    self.log("some clients did not confirm their save in time", true);
                }
                !waiting || late
            }
            Step::WriteFile => {
                self.write_file();
                true
            }
            Step::StopClients => {
                if first {
                    let n = self
                        .session
                        .as_mut()
                        .map_or(0, |s| s.clients.iter_mut().rev().filter(|c| c.running() && c.reserved_for.is_none()).map(|c| c.stop()).count());
                    self.set_deadline(if n > 0 { Some(now + Duration::from_secs(30)) } else { None });
                }
                let running = self.session.as_ref().is_some_and(|s| s.clients.iter().any(|c| c.running() && c.reserved_for.is_none()));
                if late && running {
                    if let Some(s) = &mut self.session {
                        for c in &mut s.clients {
                            c.kill();
                        }
                    }
                    self.log("clients did not quit in 30 s: killed", true);
                }
                !running || late
            }
            Step::Unload => {
                if let Some(mut s) = self.session.take() {
                    // Clients carrying on into the next session leave with us.
                    let (keep, _): (Vec<Client>, Vec<Client>) = std::mem::take(&mut s.clients).into_iter().partition(|c| c.reserved_for.is_some());
                    for c in keep {
                        let id = c.reserved_for.clone().unwrap_or_default();
                        self.reserved.push((id, c));
                    }
                    let _ = std::fs::remove_file(s.dir.join(store::LOCK));
                    if s.opened.elapsed() >= Duration::from_secs(30) {
                        store::recent::push(&s.name);
                    }
                    self.log(format!("Closed {}", s.name), false);
                }
                self.patch = patch::PatchMem::default();
                true
            }
            Step::Load(name, create) => {
                let (name, create) = (name.clone(), *create);
                if let Err(e) = self.load(&name, create) {
                    self.log(e, true);
                    self.op = None;
                    return false;
                }
                true
            }
            Step::CopyTo(name) => {
                let name = name.clone();
                if let Some(s) = &self.session {
                    let to = self.root.join(&name);
                    // Not its snapshot history or logs: the copy starts its own.
                    if let Err(e) = copy_dir(&s.dir, &to, &[snapshots::DIR, ".patchbox", store::LOCK]) {
                        self.log(format!("copy to {}: {e}", to.display()), true);
                        self.op = None;
                        return false;
                    }
                    let _ = std::fs::remove_file(to.join(store::LOCK));
                }
                true
            }
            Step::ToTemplate(name) => {
                let name = name.clone();
                if let Some(s) = &self.session {
                    let to = proto::templates_root().join(&name);
                    let _ = std::fs::remove_dir_all(&to);
                    match copy_dir(&s.dir, &to, &[snapshots::DIR, ".patchbox", store::LOCK]) {
                        Ok(()) => self.log(format!("Saved as template {name}"), false),
                        Err(e) => self.log(format!("template {}: {e}", to.display()), true),
                    }
                }
                true
            }
            Step::FromTemplate(t, name) => {
                let (t, name) = (t.clone(), name.clone());
                let to = self.root.join(&name);
                let res = copy_dir(&proto::templates_root().join(&t), &to, &[store::LOCK]).map_err(|e| e.to_string()).and_then(|_| {
                    let mut f = SessionFile::load(&to)?;
                    f.name = name.clone();
                    f.save(&to)
                });
                if let Err(e) = res {
                    self.log(format!("from template {t}: {e}"), true);
                    self.op = None;
                    return false;
                }
                true
            }
            Step::MoveTo(name) => {
                let name = name.clone();
                if let Some(s) = &mut self.session {
                    let to = self.root.join(&name);
                    if let Some(p) = to.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    if let Err(e) = std::fs::rename(&s.dir, &to) {
                        self.log(format!("rename to {}: {e}", to.display()), true);
                        self.op = None;
                        return false;
                    }
                    s.dir = to;
                    // Its place among the recent sessions follows it.
                    store::recent::rename(&s.name, &name);
                    s.name = name;
                }
                true
            }
            Step::Snapshot(label, _) if label.is_empty() => true,
            Step::Snapshot(label, auto) => {
                if first {
                    let (label, auto) = (label.clone(), *auto);
                    let Some(dir) = self.session.as_ref().map(|s| s.dir.clone()) else { return self.finish_step() };
                    match snapshots::spawn_snapshot(&dir, &label, auto) {
                        Ok(pid) => {
                            self.git = Some(pid);
                            self.git_status = None;
                            self.set_deadline(Some(now + Duration::from_secs(120)));
                        }
                        Err(e) => {
                            self.log(e, true);
                            return self.finish_step();
                        }
                    }
                }
                self.git_done("snapshot", late)
            }
            Step::Checkout(id) => {
                if first {
                    let id = id.clone();
                    let Some(dir) = self.session.as_ref().map(|s| s.dir.clone()) else { return self.finish_step() };
                    match snapshots::spawn_checkout(&dir, &id) {
                        Ok(pid) => {
                            self.git = Some(pid);
                            self.git_status = None;
                            self.set_deadline(Some(now + Duration::from_secs(120)));
                        }
                        Err(e) => {
                            self.log(e, true);
                            self.op = None;
                            return false;
                        }
                    }
                }
                self.git_done("checkout", late)
            }
            Step::Reserve(name) => {
                let name = name.clone();
                let future = SessionFile::load(&self.root.join(&name)).map(|f| f.clients).unwrap_or_default();
                let mut taken: Vec<String> = Vec::new();
                let mut kept = Vec::new();
                if let Some(s) = &mut self.session {
                    for c in &mut s.clients {
                        if !(c.running() && c.status == Status::Ready && c.has("switch") && c.out.is_some()) {
                            continue;
                        }
                        let fits = |e: &&ClientEntry| e.nsm && e.autostart && e.exe == c.entry.exe && e.args == c.entry.args && !taken.contains(&e.id);
                        // The same id first, else any client running the same program.
                        let pick = future.iter().filter(fits).find(|e| e.id == c.entry.id).or_else(|| future.iter().find(fits));
                        if let Some(e) = pick {
                            taken.push(e.id.clone());
                            c.reserved_for = Some(e.id.clone());
                            kept.push(c.entry.id.clone());
                        }
                    }
                }
                if !kept.is_empty() {
                    self.log(format!("Keeping {} running into {name} (:switch:)", kept.join(", ")), false);
                }
                true
            }
            Step::Jack => {
                if first {
                    let want = self.session.as_ref().map(|s| s.file.jack.clone()).filter(|j| j.remember && !j.driver.is_empty());
                    let Some(want) = want else { return self.finish_step() };
                    let (tx, rx) = std::sync::mpsc::channel();
                    let fd = self.wake_fd;
                    std::thread::spawn(move || {
                        let r = match crate::jackcfg::connect() {
                            Some(c) => crate::jackcfg::apply(&c, &want),
                            None => Err("jackdbus is not running: the session's JACK settings were not applied".into()),
                        };
                        let _ = tx.send(r);
                        let one: u64 = 1;
                        unsafe { libc::write(fd, (&one as *const u64).cast(), 8) };
                    });
                    self.jack_job = Some(rx);
                    self.set_deadline(Some(now + Duration::from_secs(30)));
                }
                // No job: nothing to put back (it never started).
                if self.jack_job.is_none() {
                    return self.finish_step();
                }
                let res = self.jack_job.as_ref().and_then(|r| r.try_recv().ok());
                match res {
                    Some(Ok(true)) => self.log("JACK restarted with the session's settings", false),
                    Some(Ok(false)) => {}
                    Some(Err(e)) => self.log(e, true),
                    None if late => self.log("JACK settings: jackdbus did not answer in time", true),
                    None => return false,
                }
                self.jack_job = None;
                true
            }
            Step::Script(which) => {
                if first {
                    let which = *which;
                    let Some((dir, name)) = self.session.as_ref().map(|s| (s.dir.clone(), s.name.clone())) else { return self.finish_step() };
                    match self.spawn_script(&dir, &name, which) {
                        Some(Ok(pid)) => {
                            self.git = Some(pid);
                            self.git_status = None;
                            self.set_deadline(Some(now + Duration::from_secs(60)));
                            self.log(format!("Running the {which} script"), false);
                        }
                        Some(Err(e)) => {
                            self.log(e, true);
                            return self.finish_step();
                        }
                        None => return self.finish_step(),
                    }
                }
                self.git_done("script", late)
            }
            Step::Launch => {
                // Clients kept from the last session take their new places and are told where
                // their files are now.
                let kept = std::mem::take(&mut self.reserved);
                if let Some(s) = &mut self.session {
                    for (id, mut c) in kept {
                        match s.clients.iter().position(|n| n.entry.id == id) {
                            Some(k) => {
                                let entry = s.clients[k].entry.clone();
                                c.entry = ClientEntry { name: c.entry.name.clone(), ..entry };
                                c.reserved_for = None;
                                c.send_open(&s.dir, &s.name);
                                s.clients[k] = c;
                            }
                            None => c.stop(),
                        }
                    }
                }
                let n = self.session.as_ref().map_or(0, |s| s.clients.len());
                for i in 0..n {
                    let running = self.session.as_ref().is_some_and(|s| s.clients[i].running());
                    if running {
                        continue;
                    }
                    let auto = self.session.as_ref().is_some_and(|s| s.clients[i].entry.autostart);
                    if auto && let Err(e) = self.launch(i) {
                        self.log(e, true);
                    }
                }
                true
            }
            Step::WaitReady => {
                if first {
                    let n = self.session.as_ref().map_or(0, |s| s.clients.iter().filter(|c| c.running()).count()) as u64;
                    self.set_deadline(if n > 0 { Some(now + Duration::from_secs(8 + 2 * n)) } else { None });
                }
                let waiting = self.session.as_ref().is_some_and(|s| s.clients.iter().any(|c| matches!(c.status, Status::Launching | Status::Opening)));
                !waiting || late
            }
            Step::Loaded => {
                if let Some(s) = &mut self.session {
                    for c in &mut s.clients {
                        c.session_loaded();
                    }
                    s.loaded = true;
                }
                self.patch.loaded = true;
                self.patch.all_new = true;
                self.restore_connections();
                true
            }
        };
        if done {
            if let Some(op) = &mut self.op {
                op.steps.pop_front();
                op.started = false;
                op.deadline = None;
            }
        }
        done
    }

    fn auto_snapshot(&self) -> Step {
        let on = self.session.as_ref().is_some_and(|s| s.file.auto_snapshot.unwrap_or(true));
        // An empty label: automatic snapshots are off for this session (the step does nothing).
        Step::Snapshot(if on { "saved".into() } else { String::new() }, true)
    }

    /// The session's hook script `which` (load, save, close): `patchbox-scripts/<which>.sh` in the
    /// session folder or the nearest folder above it, up to the sessions root (RaySession's
    /// scheme). It gets the session in its environment and our socket, so it can drive
    /// patchbox-ctl (connections, clients); the operation waits for it (60 s at most). None:
    /// there is no such script.
    fn spawn_script(&self, dir: &Path, name: &str, which: &str) -> Option<Result<u32, String>> {
        let mut d = Some(dir);
        let script = loop {
            let here = d?;
            let s = here.join("patchbox-scripts").join(format!("{which}.sh"));
            if s.is_file() {
                break s;
            }
            if here == self.root {
                return None;
            }
            d = here.parent().filter(|p| p.starts_with(&self.root));
        };
        let mut path = std::env::var_os("PATH").unwrap_or_default();
        if let Some(bin) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
            let mut p = bin.into_os_string();
            p.push(":");
            p.push(&path);
            path = p;
        }
        clients::rotate(&dir.join(".patchbox").join("scripts.log"), 256 << 10);
        let log = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(".patchbox").join("scripts.log")).ok();
        let mut cmd = std::process::Command::new("sh");
        cmd.arg(&script)
            .current_dir(dir)
            .env("PATH", path)
            .env("PATCHBOX_SCRIPT", which)
            .env("PATCHBOX_SESSION_NAME", name)
            .env("PATCHBOX_SESSION_PATH", dir)
            .env("PATCHBOX_SOCKET", proto::socket_path())
            .stdin(std::process::Stdio::null());
        if let Some(f) = log.as_ref().and_then(|f| Some((f.try_clone().ok()?, f.try_clone().ok()?))) {
            cmd.stdout(f.0).stderr(f.1);
        }
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGCHLD);
                libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
                Ok(())
            });
        }
        Some(cmd.spawn().map(|c| c.id()).map_err(|e| format!("{}: {e}", script.display())))
    }

    /// The git child of the current step finished (or ran out of time).
    fn git_done(&mut self, what: &str, late: bool) -> bool {
        if self.git.is_some() && !late {
            return false;
        }
        if let Some(pid) = self.git.take() {
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            self.log(format!("git {what} took too long: stopped"), true);
        }
        match self.git_status.take() {
            Some(0) | None => {}
            Some(c) => self.log(format!("git {what} failed (status {c}); see .patchbox/git.log"), true),
        }
        true
    }

    /// End the current step from inside it (early returns must still move the queue on).
    fn finish_step(&mut self) -> bool {
        if let Some(op) = &mut self.op {
            op.steps.pop_front();
            op.started = false;
            op.deadline = None;
        }
        true
    }

    fn set_deadline(&mut self, d: Option<Instant>) {
        if let Some(op) = &mut self.op {
            op.deadline = d;
        }
    }

    fn load(&mut self, name: &str, create: bool) -> Result<(), String> {
        let dir = self.root.join(name);
        let file = if create {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let mut f = SessionFile { name: name.to_string(), ..Default::default() };
            f.save(&dir)?;
            f
        } else {
            SessionFile::load(&dir)?
        };
        if let Some(pid) = locked_by_other(&dir) {
            return Err(format!("{name} is open in another patchboxd (pid {pid})"));
        }
        let _ = std::fs::write(dir.join(store::LOCK), std::process::id().to_string());
        self.patch = patch::PatchMem::from_file(&file.connections);
        let clients = file.clients.iter().cloned().map(Client::new).collect();
        let trash = file.trash.clone();
        self.session = Some(Session { name: name.to_string(), dir, file, clients, trash, loaded: false, opened: Instant::now() });
        self.log(format!("Opened {name}"), false);
        Ok(())
    }

    fn write_file(&mut self) {
        let snapshot = self.jack.as_ref().map(|j| j.snapshot());
        if let Some(g) = &snapshot {
            self.patch.update_on_save(g);
        }
        let conns = self.patch.to_file();
        let remember = self.session.as_ref().is_some_and(|s| s.file.jack.remember);
        let jack = if remember { crate::jackcfg::connect().map(|c| crate::jackcfg::read(&c)) } else { None };
        if let Some(Err(e)) = &jack {
            self.log(format!("JACK settings not saved: {e}"), true);
        }
        let Some(s) = &mut self.session else { return };
        if let Some(Ok(j)) = jack {
            s.file.jack = j;
        }
        s.file.clients = s.clients.iter().map(Client::to_entry).collect();
        s.file.trash = s.trash.clone();
        s.file.connections = conns;
        s.file.name = s.name.clone();
        match s.file.save(&s.dir) {
            Ok(()) => {
                let n = s.name.clone();
                self.log(format!("Saved {n}"), false);
            }
            Err(e) => self.log(e, true),
        }
    }

    // ---- clients ----

    fn add_client(&mut self, exe: String, args: Vec<String>, nsm: bool, label: String) -> Reply {
        let Some(s) = &mut self.session else { return Reply::Error { msg: "open or create a session first".into() } };
        if exe.trim().is_empty() {
            return Reply::Error { msg: "no executable".into() };
        }
        let mut taken: Vec<String> = s.clients.iter().map(|c| c.entry.id.clone()).chain(s.trash.iter().map(|e| e.id.clone())).collect();
        if let Ok(rd) = std::fs::read_dir(&s.dir) {
            taken.extend(rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()));
        }
        let id = store::client_id(&exe, &taken);
        let name = Path::new(&exe).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let entry = ClientEntry { id: id.clone(), exe, args, nsm, name, label, autostart: true, ..Default::default() };
        s.clients.push(Client::new(entry));
        let i = s.clients.len() - 1;
        match self.launch(i) {
            Ok(_) => Reply::Ok { msg: format!("added {id}") },
            Err(e) => Reply::Error { msg: e },
        }
    }

    pub(crate) fn launch(&mut self, i: usize) -> Result<String, String> {
        let url = self.nsm_url.clone();
        let loaded = self.session.as_ref().is_some_and(|s| s.loaded);
        let s = self.session.as_mut().ok_or("no session is open")?;
        let (dir, name) = (s.dir.clone(), s.name.clone());
        let c = &mut s.clients[i];
        if c.running() {
            return Err(format!("{} runs already", c.entry.id));
        }
        c.launch(&url, &dir, &name)?;
        // Started by hand in a loaded session: it gets session_is_loaded when it is ready.
        c.late = loaded;
        let (id, pid) = (c.entry.id.clone(), c.pid.unwrap_or(0));
        self.log(format!("Started {id} (pid {pid})"), false);
        if let Some(c) = self.session.as_ref().map(|s| &s.clients[i]) {
            self.client_hook("start", c);
        }
        Ok(format!("started {id}"))
    }

    fn save_client(&mut self, i: usize) -> Result<String, String> {
        let c = &mut self.session.as_mut().ok_or("no session")?.clients[i];
        if c.save() {
            if !c.entry.nsm {
                let c = &self.session.as_ref().unwrap().clients[i];
                self.client_hook("save", c);
            }
            Ok("saving".into())
        } else {
            Err(format!("{} can't save now", c.entry.id))
        }
    }

    fn gui(&mut self, i: usize, show: bool) -> Result<String, String> {
        let s = self.session.as_mut().ok_or("no session")?;
        let c = &mut s.clients[i];
        c.show_gui(show).then(|| "ok".to_string()).ok_or_else(|| format!("{} has no optional GUI", c.entry.id))
    }

    fn reap(&mut self) {
        loop {
            let mut status = 0;
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 {
                break;
            }
            let code = if libc::WIFEXITED(status) { format!("exit {}", libc::WEXITSTATUS(status)) } else { format!("signal {}", libc::WTERMSIG(status)) };
            if self.git == Some(pid as u32) {
                self.git = None;
                self.git_status = Some(if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 });
                continue;
            }
            let mut msg = None;
            let mut gone = None;
            if let Some(s) = &mut self.session {
                for (k, c) in s.clients.iter_mut().enumerate() {
                    if c.pid == Some(pid as u32) {
                        msg = Some((c.exited(&code), c.status == Status::Error));
                        gone = Some(k);
                    }
                }
            }
            if let Some((m, bad)) = msg {
                self.log(m, bad);
            }
            if let Some(c) = gone.and_then(|k| self.session.as_ref().map(|s| &s.clients[k])) {
                self.client_hook("stop", c);
            }
        }
    }

    // ---- JACK (connection memory) ----

    fn connect_jack(&mut self) {
        let fd = self.wake_fd;
        let waker: pbx_jack::Waker = std::sync::Arc::new(move || {
            let one: u64 = 1;
            unsafe { libc::write(fd, (&one as *const u64).cast(), 8) };
        });
        match pbx_jack::Client::open("patchboxd", waker) {
            Ok(c) => {
                self.jack = Some(c);
                self.log("connected to JACK", false);
                self.patch.all_new = true;
                self.patch.due = Some(Instant::now() + Duration::from_millis(200));
            }
            Err(_) => self.jack_retry = Instant::now() + Duration::from_secs(2),
        }
    }

    fn jack_events(&mut self) {
        let Some(j) = &self.jack else { return };
        let mut ports_changed = false;
        let mut lost = false;
        for e in j.take_events() {
            match e {
                pbx_jack::Event::PortRegistered { registered: true, .. } | pbx_jack::Event::PortRename { .. } => ports_changed = true,
                pbx_jack::Event::Shutdown { .. } => lost = true,
                _ => {}
            }
        }
        if lost {
            self.jack = None;
            self.jack_retry = Instant::now() + Duration::from_secs(1);
            self.log("JACK went away", true);
            return;
        }
        if ports_changed {
            // A client registering 40 ports: restore once, 200 ms after the last.
            self.patch.due = Some(Instant::now() + Duration::from_millis(200));
        }
    }

    fn restore_connections(&mut self) {
        let Some(j) = &self.jack else { return };
        let g = j.snapshot();
        let (connect, mut disconnect) = self.patch.plan(&g);
        // A session never undoes what a rule asks for.
        disconnect.retain(|(o, i)| !self.rules.wants(o, i));
        for (o, i) in &disconnect {
            j.disconnect(o, i);
        }
        let mut made = 0;
        for (o, i) in &connect {
            if j.connect(o, i) {
                made += 1;
            }
        }
        let ruled = self.rules.plan(&g);
        let ruled = ruled.into_iter().filter(|(o, i)| j.connect(o, i)).count();
        if made > 0 || !disconnect.is_empty() {
            self.log(format!("connections: {made} restored, {} removed", disconnect.len()), false);
        }
        if ruled > 0 {
            self.log(format!("connection rules: {ruled} made"), false);
        }
    }
}

pub(crate) fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        if skip.iter().any(|s| e.file_name() == *s) {
            continue;
        }
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let t = e.file_type()?;
        if t.is_dir() {
            copy_dir(&src, &dst, &[])?;
        } else if t.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(&src)?, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// The pid of another live patchboxd holding `dir`'s session lock.
fn locked_by_other(dir: &std::path::Path) -> Option<i32> {
    let pid = std::fs::read_to_string(dir.join(store::LOCK)).ok()?.trim().parse::<i32>().ok()?;
    // SAFETY: kill(pid, 0) only checks the process exists.
    (pid > 0 && pid != std::process::id() as i32 && unsafe { libc::kill(pid, 0) } == 0).then_some(pid)
}

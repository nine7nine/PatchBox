//! The GUI's link to patchboxd (crates/pbx-session): it starts the daemon when none runs,
//! subscribes to its state, and sends it requests. A reader thread queues what the daemon says
//! and wakes the main loop. The daemon outlives the GUI while a session is open.

use pbx_session::proto::{Conn, FromDaemon, Request, SessionEntry, State};
use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct SessionLink {
    tx: Option<UnixStream>,
    inbox: Arc<Mutex<Vec<FromDaemon>>>,
    waker: pbx_jack::Waker,
    pub state: State,
    pub sessions: Vec<SessionEntry>,
    pub snapshots: Vec<pbx_session::proto::SnapshotEntry>,
    /// Recent sessions, newest first.
    pub recent: Vec<String>,
    pub templates: Vec<String>,
    pub connected: bool,
    retry_at: Instant,
    spawned_at: Option<Instant>,
    /// Lost the daemon (it was there before): say so once.
    pub lost: bool,
}

/// patchboxd next to our executable (a build tree, an install), else on PATH.
fn daemon_exe() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("patchboxd")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "patchboxd".into())
}

impl SessionLink {
    pub fn new(waker: pbx_jack::Waker) -> SessionLink {
        SessionLink {
            tx: None,
            inbox: Arc::new(Mutex::new(Vec::new())),
            waker,
            state: State::default(),
            sessions: Vec::new(),
            snapshots: Vec::new(),
            recent: Vec::new(),
            templates: Vec::new(),
            connected: false,
            retry_at: Instant::now(),
            spawned_at: None,
            lost: false,
        }
    }

    pub fn retry_at(&self) -> Option<Instant> {
        (!self.connected).then_some(self.retry_at)
    }

    /// Connect (starting the daemon if needed). Called from the main loop's timers.
    pub fn tick(&mut self) -> Option<String> {
        if self.connected || Instant::now() < self.retry_at {
            return None;
        }
        match Conn::connect() {
            Ok(conn) => {
                let (tx, reader) = conn.into_parts();
                let inbox = self.inbox.clone();
                let waker = self.waker.clone();
                std::thread::Builder::new()
                    .name("patchboxd-link".into())
                    .spawn(move || {
                        for line in reader.lines() {
                            let Ok(line) = line else { break };
                            if let Ok(m) = serde_json::from_str::<FromDaemon>(&line) {
                                if let Ok(mut q) = inbox.lock() {
                                    q.push(m);
                                }
                                waker();
                            }
                        }
                        // The daemon went away: an empty state marks it.
                        if let Ok(mut q) = inbox.lock() {
                            q.push(FromDaemon::Push(pbx_session::proto::Push::Log { text: "\u{0}lost".into(), bad: true }));
                        }
                        waker();
                    })
                    .ok()?;
                self.tx = Some(tx);
                self.connected = true;
                self.lost = false;
                self.send(&Request::Subscribe);
                self.send(&Request::ListSessions);
                self.send(&Request::Rules);
                Some("Connected to the session daemon".into())
            }
            Err(_) => {
                let spawn = self.spawned_at.is_none_or(|t| t.elapsed() > Duration::from_secs(10));
                self.retry_at = Instant::now() + Duration::from_millis(if spawn { 300 } else { 1000 });
                if spawn {
                    self.spawned_at = Some(Instant::now());
                    let log = pbx_session::proto::socket_path().with_file_name("patchboxd.log");
                    let out = std::fs::File::create(&log).ok();
                    let mut cmd = std::process::Command::new(daemon_exe());
                    cmd.stdin(std::process::Stdio::null());
                    if let Some(f) = out.as_ref().and_then(|f| Some((f.try_clone().ok()?, f.try_clone().ok()?))) {
                        cmd.stdout(f.0).stderr(f.1);
                    }
                    use std::os::unix::process::CommandExt;
                    unsafe {
                        // Its own process session: it outlives us (and our terminal).
                        cmd.pre_exec(|| {
                            libc::setsid();
                            Ok(())
                        });
                    }
                    return match cmd.spawn() {
                        Ok(_) => Some("Starting the session daemon".into()),
                        Err(e) => Some(format!("Cannot start patchboxd: {e}")),
                    };
                }
                None
            }
        }
    }

    pub fn send(&mut self, r: &Request) {
        let Some(tx) = &mut self.tx else { return };
        let mut line = serde_json::to_vec(r).unwrap_or_default();
        line.push(b'\n');
        if tx.write_all(&line).is_err() {
            self.drop_link();
        }
    }

    fn drop_link(&mut self) {
        self.tx = None;
        self.connected = false;
        self.lost = true;
        self.state = State::default();
        self.retry_at = Instant::now() + Duration::from_millis(500);
    }

    /// What the daemon said since the last call.
    pub fn take(&mut self) -> Vec<FromDaemon> {
        let v = self.inbox.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        if v.iter().any(|m| matches!(m, FromDaemon::Push(pbx_session::proto::Push::Log { text, .. }) if text == "\u{0}lost")) {
            self.drop_link();
        }
        v.into_iter().filter(|m| !matches!(m, FromDaemon::Push(pbx_session::proto::Push::Log { text, .. }) if text == "\u{0}lost")).collect()
    }
}

/// An application from a .desktop file.
#[derive(Clone, Debug)]
pub struct App {
    pub name: String,
    pub exe: String,
    pub args: Vec<String>,
    pub nsm: bool,
    pub comment: String,
}

fn in_path(exe: &str) -> bool {
    if exe.contains('/') {
        return std::path::Path::new(exe).is_file();
    }
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
}

/// NSM-capable applications (X-NSM-Capable=true) and other audio/MIDI applications, from the
/// XDG data directories, installed ones only.
pub fn scan_apps() -> Vec<App> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(h) = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))) {
        dirs.push(h);
    }
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(std::path::PathBuf::from));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d.join("applications")) else { continue };
        let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
        files.sort();
        for f in files {
            let id = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !seen.insert(id) {
                continue; // the user's copy shadows the system's
            }
            if let Some(a) = parse_desktop(&f) {
                out.push(a);
            }
        }
    }
    out.sort_by(|a, b| b.nsm.cmp(&a.nsm).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

fn parse_desktop(path: &std::path::Path) -> Option<App> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut kv = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry && let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
        }
    }
    let get = |k: &str| kv.get(k).cloned().unwrap_or_default();
    if get("Type") != "Application" || get("NoDisplay") == "true" || get("Hidden") == "true" {
        return None;
    }
    let nsm = get("X-NSM-Capable") == "true";
    let cats = get("Categories");
    let audio = cats.split(';').any(|c| matches!(c, "Audio" | "Midi" | "Music" | "Sequencer" | "Mixer"));
    if !nsm && !audio {
        return None;
    }
    let exec = if nsm && !get("X-NSM-Exec").is_empty() { get("X-NSM-Exec") } else { get("Exec") };
    let mut words = exec.split_whitespace().filter(|w| !w.starts_with('%')).map(|w| w.trim_matches('"').to_string());
    let exe = words.next()?;
    if exe == "env" || exe.ends_with("flatpak") {
        return None;
    }
    if !in_path(&exe) {
        return None;
    }
    let args = if nsm { Vec::new() } else { words.collect() };
    Some(App { name: get("Name"), exe, args, nsm, comment: get("Comment") })
}

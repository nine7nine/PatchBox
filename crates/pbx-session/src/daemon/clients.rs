//! One client of the session: an NSM client (it announces to us over OSC and is told to open and
//! save) or a plain program (started, stopped with SIGTERM, nothing else).

use crate::osc::{self, Arg, Msg};
use crate::proto::{ClientState, Status};
use crate::store::ClientEntry;
use std::net::{SocketAddr, UdpSocket};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pending {
    Open,
    Save,
}

pub struct Client {
    pub entry: ClientEntry,
    pub status: Status,
    pub pid: Option<u32>,
    /// Where its OSC messages come from, and a socket to answer on.
    pub out: Option<(UdpSocket, SocketAddr)>,
    pub caps: String,
    pub dirty: Option<bool>,
    pub gui: Option<bool>,
    pub progress: f32,
    pub message: String,
    pub jack_name: String,
    /// Started after the session loaded: tell it session_is_loaded once it is ready.
    pub late: bool,
    /// Announced on its own (NSM_URL set by the user): not our child, liveness by kill(pid, 0).
    pub external: bool,
    /// Kept running across the session switch in progress, for the new session's client of
    /// this id.
    pub reserved_for: Option<String>,
    announce_by: Option<Instant>,
    pub reply_by: Option<(Pending, Instant)>,
    kill_at: Option<Instant>,
    /// Stop it once its save is done (the GUI's "Save and stop"); a failed save leaves it running.
    pub stop_after_save: bool,
    /// Removed while running: to the trash once it has stopped.
    pub remove_after_stop: bool,
    alive_check: Option<Instant>,
    open_sent: Option<Instant>,
    /// Its stdout/stderr file (.patchbox/<id>.log), cut back when it grows too big.
    log: Option<std::path::PathBuf>,
}

/// `$NAME` and `${NAME}` in `s` replaced by their values.
pub fn expand(s: &str, vars: &[(&str, &str)]) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v).replace(&format!("${k}"), v);
    }
    out
}

/// `exe args` in a terminal emulator: $TERMINAL, else the first of foot, alacritty, kitty,
/// konsole, gnome-terminal, xterm on PATH. The terminal is the process we watch: closing it ends
/// the program.
pub fn in_terminal(exe: &str, args: &[String]) -> Command {
    let on_path = |n: &str| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(n).is_file()));
    let term = std::env::var("TERMINAL")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| ["foot", "alacritty", "kitty", "konsole", "gnome-terminal", "xterm"].into_iter().find(|t| on_path(t)).map(str::to_string))
        .unwrap_or_else(|| "xterm".into());
    let base = Path::new(&term).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut c = Command::new(&term);
    match base.as_str() {
        // A separate process each (an existing konsole would take the program and its env).
        "konsole" => c.args(["--separate", "--nofork", "-e", exe]),
        "gnome-terminal" => c.args(["--wait", "--", exe]),
        "foot" | "kitty" => c.arg(exe),
        _ => c.args(["-e", exe]),
    };
    c.args(args);
    c
}

/// A client's log is cut back past this (the previous run's is kept as <id>.log.1).
pub const LOG_MAX: u64 = 4 << 20;

/// Put `path` aside as `<path>.1` when it passed `max` (or always, `max` 0): what comes next
/// starts a fresh file. Logs never grow without end.
pub fn rotate(path: &Path, max: u64) {
    let big = std::fs::metadata(path).is_ok_and(|m| m.len() >= max);
    if big {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::rename(path, old);
    }
}

/// NSM clients that never announce become plain programs after this.
const ANNOUNCE_WAIT: Duration = Duration::from_secs(10);
const STOP_WAIT: Duration = Duration::from_secs(10);

impl Client {
    pub fn new(entry: ClientEntry) -> Client {
        Client {
            entry,
            status: Status::Stopped,
            pid: None,
            out: None,
            caps: String::new(),
            dirty: None,
            gui: None,
            progress: 0.0,
            message: String::new(),
            jack_name: String::new(),
            late: false,
            external: false,
            reserved_for: None,
            announce_by: None,
            reply_by: None,
            kill_at: None,
            stop_after_save: false,
            remove_after_stop: false,
            alive_check: None,
            open_sent: None,
            log: None,
        }
    }

    /// Cut the running client's log back when it passed LOG_MAX. The client writes in append
    /// mode, so it carries on at the new end.
    pub fn cap_log(&self) -> Option<String> {
        let p = self.log.as_ref()?;
        if std::fs::metadata(p).ok()?.len() < LOG_MAX {
            return None;
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).open(p).ok()?;
        f.set_len(0).ok()?;
        let _ = writeln!(f, "[patchboxd: the log passed {} MB and was cut]", LOG_MAX >> 20);
        Some(format!("{}: its log passed {} MB and was cut", self.entry.id, LOG_MAX >> 20))
    }

    pub fn running(&self) -> bool {
        self.pid.is_some()
    }

    pub fn has(&self, cap: &str) -> bool {
        self.caps.contains(&format!(":{cap}:"))
    }

    pub fn state(&self) -> ClientState {
        ClientState {
            id: self.entry.id.clone(),
            name: self.entry.name.clone(),
            label: self.entry.label.clone(),
            exe: self.entry.exe.clone(),
            args: self.entry.args.clone(),
            nsm: self.entry.nsm,
            status: self.status,
            pid: self.pid,
            dirty: self.dirty,
            gui: self.gui,
            progress: self.progress,
            message: self.message.clone(),
            capabilities: self.caps.clone(),
            jack_name: self.jack_name.clone(),
            autostart: self.entry.autostart,
            last_save: self.entry.last_save,
            env: self.entry.env.clone(),
            save_signal: self.entry.save_signal,
            stop_signal: self.entry.stop_signal,
            terminal: self.entry.terminal,
        }
    }

    /// The entry to save: it starts with the session next time if it runs now.
    pub fn to_entry(&self) -> ClientEntry {
        let mut e = self.entry.clone();
        e.autostart = self.running();
        if self.gui.is_some() {
            e.gui_visible = self.gui;
        }
        e
    }

    pub fn send(&self, m: &Msg) {
        if let Some((sock, addr)) = &self.out {
            let _ = sock.send_to(&osc::encode(m), addr);
        }
    }

    pub fn launch(&mut self, url: &str, dir: &Path, session: &str) -> Result<(), String> {
        let logs = dir.join(".patchbox");
        let _ = std::fs::create_dir_all(&logs);
        // This run's output; the previous run's is kept beside it (.1), nothing older. Append
        // mode, so the file can be cut back while the client writes (cap_log).
        let path = logs.join(format!("{}.log", self.entry.id));
        rotate(&path, 0);
        let log = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        self.log = log.is_some().then_some(path);
        // $SESSION_NAME, $SESSION_PATH, $CLIENT_ID, $JACK_CLIENT_NAME in its arguments (and its
        // environment's values) become what they name.
        let jack_name = if self.entry.label.is_empty() { self.entry.id.clone() } else { self.entry.label.clone() };
        let dir_s = dir.display().to_string();
        let vars = [("SESSION_NAME", session), ("SESSION_PATH", dir_s.as_str()), ("CLIENT_ID", self.entry.id.as_str()), ("JACK_CLIENT_NAME", jack_name.as_str())];
        let args: Vec<String> = self.entry.args.iter().map(|a| expand(a, &vars)).collect();
        let mut cmd = if self.entry.terminal { in_terminal(&self.entry.exe, &args) } else {
            let mut c = Command::new(&self.entry.exe);
            c.args(&args);
            c
        };
        cmd.current_dir(dir).stdin(Stdio::null());
        for (k, v) in &self.entry.env {
            cmd.env(k, expand(v, &vars));
        }
        match log.as_ref().and_then(|f| Some((f.try_clone().ok()?, f.try_clone().ok()?))) {
            Some((a, b)) => {
                cmd.stdout(a).stderr(b);
            }
            None => {
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        if self.entry.nsm {
            cmd.env("NSM_URL", url);
        } else {
            cmd.env_remove("NSM_URL")
                .env("PATCHBOX_SESSION_NAME", session)
                .env("PATCHBOX_SESSION_PATH", dir)
                .env("PATCHBOX_CLIENT_ID", &self.entry.id);
        }
        unsafe {
            cmd.pre_exec(|| {
                // Its own session (our terminal's signals aren't its), and SIGCHLD unblocked
                // (the daemon blocks it for its signalfd; the mask survives exec).
                libc::setsid();
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGCHLD);
                libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| format!("{}: {e}", self.entry.exe))?;
        // Reaped by the daemon's waitpid(-1); the Child handle is not needed.
        self.pid = Some(child.id());
        drop(child);
        self.out = None;
        self.caps.clear();
        self.dirty = None;
        self.gui = None;
        self.progress = 0.0;
        self.message.clear();
        self.external = false;
        self.kill_at = None;
        self.reply_by = None;
        if self.entry.nsm && !self.entry.no_announce {
            self.status = Status::Launching;
            self.announce_by = Some(Instant::now() + ANNOUNCE_WAIT);
        } else if self.entry.nsm {
            // It never announced before: don't hold the session up for it (it may still).
            self.status = Status::NoAnnounce;
            self.announce_by = None;
        } else {
            self.status = Status::Running;
            self.announce_by = None;
        }
        Ok(())
    }

    /// Announced: answer, then tell it to open its files under `dir`.
    pub fn announced(&mut self, out: (UdpSocket, SocketAddr), name: &str, caps: &str, dir: &Path, session: &str) {
        self.out = Some(out);
        self.caps = caps.to_string();
        if self.entry.name.is_empty() || self.entry.name != name {
            self.entry.name = name.to_string();
        }
        self.entry.no_announce = false;
        self.announce_by = None;
        self.dirty = self.has("dirty").then_some(false);
        self.gui = if self.has("optional-gui") { Some(self.entry.gui_visible.unwrap_or(true)) } else { None };
        self.send(&Msg::new(
            "/reply",
            vec![
                Arg::S("/nsm/server/announce".into()),
                Arg::S("Welcome to PatchBox.".into()),
                Arg::S("PatchBox".into()),
                Arg::S(":server-control:broadcast:optional-gui:".into()),
            ],
        ));
        self.send_open(dir, session);
    }

    /// Tell it to open its files for the session in `dir` (at announce, and when it is kept
    /// across a session switch).
    pub fn send_open(&mut self, dir: &Path, session: &str) {
        let prefix: String = self.entry.name.chars().map(|c| if c == '/' { '_' } else { c }).collect();
        let full = format!("{prefix}.{}", self.entry.id);
        let path = dir.join(&full);
        self.jack_name = full.clone();
        self.status = Status::Opening;
        self.open_sent = Some(Instant::now());
        self.reply_by = Some((Pending::Open, Instant::now() + Duration::from_secs(20)));
        self.send(&Msg::new(
            "/nsm/client/open",
            vec![Arg::S(path.display().to_string()), Arg::S(session.to_string()), Arg::S(full)],
        ));
    }

    /// Its answer to open or save.
    pub fn replied(&mut self, to: &str, ok: bool, msg: &str) -> Option<(String, bool)> {
        let id = self.entry.id.clone();
        match (to, ok) {
            ("/nsm/client/open", true) => {
                self.status = Status::Ready;
                self.reply_by = None;
                self.message = msg.to_string();
                // Bring its GUI back the way it was saved.
                if let (Some(want), true) = (self.entry.gui_visible, self.has("optional-gui")) {
                    self.show_gui(want);
                }
                if self.late {
                    self.late = false;
                    self.send(&Msg::new("/nsm/client/session_is_loaded", vec![]));
                }
                let took = self.open_sent.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
                Some((format!("{id} ready ({took:.1} s)"), false))
            }
            ("/nsm/client/open", false) => {
                self.status = Status::Error;
                self.reply_by = None;
                self.message = msg.to_string();
                Some((format!("{id} failed to open: {msg}"), true))
            }
            ("/nsm/client/save", true) => {
                self.status = Status::Ready;
                self.reply_by = None;
                self.entry.last_save = Some(crate::store::now());
                if self.dirty.is_some() {
                    self.dirty = Some(false);
                }
                if std::mem::take(&mut self.stop_after_save) {
                    self.stop();
                    return Some((format!("{id} saved; stopping"), false));
                }
                None
            }
            ("/nsm/client/save", false) => {
                self.status = Status::Ready;
                self.reply_by = None;
                self.message = format!("save failed: {msg}");
                let left = if std::mem::take(&mut self.stop_after_save) { "; left running" } else { "" };
                self.remove_after_stop = false;
                Some((format!("{id}: save failed: {msg}{left}"), true))
            }
            _ => None,
        }
    }

    /// Ask it to save. False if it can't now (not an NSM client, or not ready).
    pub fn save(&mut self) -> bool {
        // A plain program with a save signal: sent, and taken as done after 300 ms (RaySession's).
        if !self.entry.nsm && self.entry.save_signal > 0 && self.status == Status::Running
            && let Some(pid) = self.pid
        {
            unsafe { libc::kill(pid as i32, self.entry.save_signal) };
            self.status = Status::Saving;
            self.reply_by = Some((Pending::Save, Instant::now() + Duration::from_millis(300)));
            return true;
        }
        if !(self.entry.nsm && self.out.is_some() && self.status == Status::Ready) {
            return false;
        }
        self.status = Status::Saving;
        self.reply_by = Some((Pending::Save, Instant::now() + Duration::from_secs(10)));
        self.send(&Msg::new("/nsm/client/save", vec![]));
        true
    }

    pub fn show_gui(&mut self, show: bool) -> bool {
        if !self.has("optional-gui") || self.out.is_none() {
            return false;
        }
        self.send(&Msg::new(if show { "/nsm/client/show_optional_gui" } else { "/nsm/client/hide_optional_gui" }, vec![]));
        true
    }

    pub fn session_loaded(&mut self) {
        if self.entry.nsm && self.status == Status::Ready {
            self.send(&Msg::new("/nsm/client/session_is_loaded", vec![]));
        }
        self.late = false;
    }

    pub fn stop(&mut self) {
        let Some(pid) = self.pid else { return };
        // Out of the JACK graph first: stopped while in it, a client can miss a cycle (an xrun)
        crate::jackcfg::detach_process(pid);
        let sig = if !self.entry.nsm && self.entry.stop_signal > 0 { self.entry.stop_signal } else { libc::SIGTERM };
        unsafe { libc::kill(pid as i32, sig) };
        self.status = Status::Stopping;
        self.kill_at = Some(Instant::now() + STOP_WAIT);
    }

    pub fn kill(&mut self) {
        let Some(pid) = self.pid else { return };
        if !matches!(self.status, Status::Stopping) {
            crate::jackcfg::detach_process(pid);   // stop() did it already
        }
        unsafe {
            // Its whole process group (it is a session leader), then itself if it left the group.
            libc::kill(-(pid as i32), libc::SIGKILL);
            libc::kill(pid as i32, libc::SIGKILL);
        }
        self.kill_at = None;
    }

    /// Its process ended (waitpid, or an external one vanished).
    pub fn exited(&mut self, how: &str) -> String {
        let expected = matches!(self.status, Status::Stopping | Status::Stopped);
        self.pid = None;
        self.out = None;
        self.kill_at = None;
        self.reply_by = None;
        self.announce_by = None;
        self.progress = 0.0;
        if expected {
            self.status = Status::Stopped;
            format!("{} stopped", self.entry.id)
        } else {
            self.status = if matches!(self.status, Status::Launching | Status::Opening) { Status::Error } else { Status::Stopped };
            self.message = format!("exited ({how})");
            format!("{} exited on its own ({how})", self.entry.id)
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        [self.announce_by, self.reply_by.map(|r| r.1), self.kill_at, self.alive_check].into_iter().flatten().min()
    }

    /// Deadlines: never announced, no reply, didn't quit. Returns a log line.
    pub fn timers(&mut self, now: Instant) -> Option<(String, bool)> {
        let id = self.entry.id.clone();
        if self.announce_by.is_some_and(|t| now >= t) && self.status == Status::Launching {
            self.announce_by = None;
            self.status = Status::NoAnnounce;
            self.entry.no_announce = true;
            self.message = "never announced: run as a plain program".into();
            return Some((format!("{id} never announced (not an NSM client?): running it as a plain program"), true));
        }
        if let Some((p, t)) = self.reply_by
            && now >= t
        {
            self.reply_by = None;
            return Some(match p {
                Pending::Open => {
                    self.status = Status::Error;
                    self.message = "no answer to open".into();
                    (format!("{id} did not answer /nsm/client/open"), true)
                }
                Pending::Save if !self.entry.nsm => {
                    // A plain program told by signal: no confirmation comes.
                    self.status = Status::Running;
                    if std::mem::take(&mut self.stop_after_save) {
                        self.stop();
                    }
                    return None;
                }
                Pending::Save => {
                    self.status = Status::Ready;
                    self.message = "save not confirmed".into();
                    let left = if std::mem::take(&mut self.stop_after_save) { "; left running" } else { "" };
                self.remove_after_stop = false;
                    (format!("{id} did not confirm its save{left}"), true)
                }
            });
        }
        if self.kill_at.is_some_and(|t| now >= t) && self.running() {
            self.kill();
            return Some((format!("{id} did not quit: killed"), true));
        }
        if self.external {
            if self.alive_check.is_none_or(|t| now >= t) {
                self.alive_check = Some(now + Duration::from_secs(1));
                if let Some(pid) = self.pid
                    && unsafe { libc::kill(pid as i32, 0) } != 0
                {
                    self.alive_check = None;
                    return Some((self.exited("gone"), false));
                }
            }
        } else {
            self.alive_check = None;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_and_terminals() {
        let vars = [("CLIENT_ID", "carla_1"), ("SESSION_NAME", "band")];
        assert_eq!(expand("-n $CLIENT_ID ${SESSION_NAME}.x", &vars), "-n carla_1 band.x");
        // SAFETY: only this test reads TERMINAL.
        unsafe { std::env::set_var("TERMINAL", "/usr/bin/konsole") };
        let c = in_terminal("htop", &["-d".into(), "5".into()]);
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(c.get_program(), "/usr/bin/konsole");
        assert_eq!(args, ["--separate", "--nofork", "-e", "htop", "-d", "5"]);
        unsafe { std::env::set_var("TERMINAL", "foot") };
        let c = in_terminal("htop", &[]);
        assert_eq!(c.get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>(), ["htop"]);
        unsafe { std::env::remove_var("TERMINAL") };
    }

    #[test]
    fn logs_rotate_past_their_size() {
        let d = std::env::temp_dir().join(format!("pbx-rotate-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("c.log");
        std::fs::write(&p, "small").unwrap();
        rotate(&p, 100);
        assert!(p.exists(), "under the size: kept");
        std::fs::write(&p, vec![b'x'; 200]).unwrap();
        rotate(&p, 100);
        assert!(!p.exists() && d.join("c.log.1").exists(), "past it: put aside");
        std::fs::write(&p, "new run").unwrap();
        rotate(&p, 0);
        assert_eq!(std::fs::read_to_string(d.join("c.log.1")).unwrap(), "new run", "one older file only");
        let _ = std::fs::remove_dir_all(&d);
    }
}

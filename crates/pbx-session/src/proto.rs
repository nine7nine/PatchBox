//! The control protocol between patchboxd and its front ends (the GUI, patchbox-ctl): JSON lines
//! over a Unix stream socket. A front end sends `Request`s; the daemon answers each with a
//! `Reply`, and pushes `Push::State` to subscribers whenever anything changes (the state is
//! small: one session, a handful of clients).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Get pushed the state now and on every change, and log lines.
    Subscribe,
    State,
    ListSessions,
    New { name: String, #[serde(default)] template: Option<String> },
    /// Save the open session as a template (overwrites one of that name).
    SaveTemplate { name: String },
    ListTemplates,
    Open { name: String },
    Save,
    Close,
    /// Close without saving.
    Abort,
    Duplicate { name: String },
    Rename { name: String },
    /// Move a session that is not open to the desktop's Trash.
    Delete { name: String },
    SetNotes { notes: String },
    /// Add a client and start it. `nsm`: speaks NSM (else a plain program, stopped with SIGTERM).
    Add { exe: String, #[serde(default)] args: Vec<String>, nsm: bool, #[serde(default)] label: String },
    Start { id: String },
    Stop { id: String },
    /// Save it, then stop it once the save is confirmed (a failed save leaves it running).
    SaveStop { id: String },
    Kill { id: String },
    SaveClient { id: String },
    ShowGui { id: String },
    HideGui { id: String },
    /// Move a stopped client to the trash.
    Trash { id: String },
    Restore { id: String },
    /// Forget a removed client: its files stay, unless `files` (then they go to the desktop's Trash).
    Forget {
        id: String,
        #[serde(default)]
        files: bool,
    },
    /// Remove a client from the session (to its trash, where Restore brings it back). A running
    /// one is stopped first (saved first if `save`; a failed save leaves it running and in place).
    Remove {
        id: String,
        #[serde(default)]
        save: bool,
    },
    /// Move a session template (or, `client`, a client template) to the desktop's Trash.
    DeleteTemplate {
        name: String,
        #[serde(default)]
        client: bool,
    },
    SetLabel { id: String, label: String },
    /// JACK connections, through the daemon's own client (scripts, tests).
    Connect { src: String, dst: String },
    Disconnect { src: String, dst: String },
    /// The current connections, one "src -> dst" per line.
    Graph,
    /// Take a named snapshot of the session folder (git).
    Snapshot { label: String },
    ListSnapshots,
    /// Go back to a snapshot (after taking one of the present state).
    Rewind { id: String },
    /// Automatic snapshots at save and close, for this session.
    SetAutoSnapshot { on: bool },
    /// Remember JACK's settings with this session (jackdbus): kept at save, restored at open.
    SetJackMemory { on: bool },
    /// Change a client's properties (from its next start).
    SetClientProps { id: String, props: ClientProps },
    /// Keep a client (its command, properties and files) as a client template.
    SaveClientTemplate { id: String, name: String },
    ListClientTemplates,
    /// Add a client from a template to the open session, with its own copy of the files.
    AddClientTemplate { name: String },
    /// The connection rules (crate::rules).
    Rules,
    /// Replace the connection rules (saved; applied at once).
    SetRules { rules: Vec<crate::rules::Rule> },
    Quit,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Ok { msg: String },
    Error { msg: String },
    Sessions {
        root: String,
        sessions: Vec<SessionEntry>,
        /// The sessions last open for a while (over 30 s), newest first, at most 7.
        #[serde(default)]
        recent: Vec<String>,
    },
    Snapshots { snapshots: Vec<SnapshotEntry> },
    Templates { templates: Vec<String> },
    Rules { rules: Vec<crate::rules::Rule> },
    ClientTemplates { templates: Vec<String> },
    State(State),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "push", rename_all = "snake_case")]
pub enum Push {
    State(State),
    Log { text: String, bad: bool },
}

/// One line from the daemon: a reply or a push.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum FromDaemon {
    Push(Push),
    Reply(Reply),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SnapshotEntry {
    /// The git tag.
    pub id: String,
    /// Seconds since the epoch.
    pub time: u64,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SessionEntry {
    /// Relative to the sessions root.
    pub name: String,
    /// Seconds since the epoch the session file was written.
    pub modified: u64,
    pub clients: usize,
    pub notes: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Stopped,
    /// Started, not announced yet.
    Launching,
    /// Announced; /nsm/client/open sent.
    Opening,
    Ready,
    Saving,
    Stopping,
    /// A plain program, running.
    Running,
    /// Never announced: run as a plain program.
    NoAnnounce,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ClientState {
    pub id: String,
    /// The application's name (from its announce, else the executable).
    pub name: String,
    pub label: String,
    pub exe: String,
    pub args: Vec<String>,
    pub nsm: bool,
    pub status: Status,
    pub pid: Option<u32>,
    /// None: the client doesn't report it (no :dirty:).
    pub dirty: Option<bool>,
    /// None: no optional GUI.
    pub gui: Option<bool>,
    pub progress: f32,
    pub message: String,
    pub capabilities: String,
    /// The JACK client name it was given (NSM clients).
    pub jack_name: String,
    /// Started when the session opens.
    pub autostart: bool,
    pub last_save: Option<u64>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub save_signal: i32,
    #[serde(default)]
    pub stop_signal: i32,
    #[serde(default)]
    pub terminal: bool,
}

/// What a client's properties dialog changes (applies from its next start).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ClientProps {
    pub label: String,
    pub exe: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub save_signal: i32,
    pub stop_signal: i32,
    pub terminal: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct State {
    pub root: String,
    /// The open session (name relative to the root) and its folder.
    pub session: Option<String>,
    pub path: Option<String>,
    pub notes: String,
    pub clients: Vec<ClientState>,
    pub trash: Vec<ClientState>,
    /// An operation in progress ("Saving…", "Opening…").
    pub busy: Option<String>,
    /// The NSM_URL clients get.
    pub nsm_url: String,
    /// Saved connections and how many are present.
    pub connections_saved: usize,
    pub connections_present: usize,
    pub auto_snapshot: bool,
    /// The session restores JACK's settings when it opens.
    pub jack_memory: bool,
    /// The daemon's protocol version (PROTO); 0 from a daemon older than the field.
    #[serde(default)]
    pub proto: u32,
    /// The daemon's executable was replaced on disk (an update) since it started.
    #[serde(default)]
    pub outdated: bool,
}

/// The control protocol's version, bumped when requests or the state change: a GUI talking to an
/// older daemon (one left running across an update, with a session open) can tell, and replace it.
/// 2: delete, remove, save_stop, delete_template, forget's files.
pub const PROTO: u32 = 2;

/// $XDG_RUNTIME_DIR/patchbox/daemon.sock (PATCHBOX_SOCKET overrides).
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("PATCHBOX_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(format!("/tmp/patchbox-{}", unsafe { libc::getuid() }))
    });
    base.join("patchbox").join("daemon.sock")
}

/// Session templates: PATCHBOX_TEMPLATES, else $XDG_CONFIG_HOME/patchbox/templates.
pub fn templates_root() -> PathBuf {
    if let Some(p) = std::env::var_os("PATCHBOX_TEMPLATES") {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("patchbox")
        .join("templates")
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

/// Client templates: the configuration directory's client-templates/<name>/ (client.toml and
/// files/, the client's files with its prefix replaced by __CLIENT__).
pub fn client_templates_root() -> Option<PathBuf> {
    Some(config_dir()?.join("client-templates"))
}

/// Where sessions live: PATCHBOX_SESSIONS, else ~/PatchBox Sessions.
pub fn sessions_root() -> PathBuf {
    if let Some(p) = std::env::var_os("PATCHBOX_SESSIONS") {
        return PathBuf::from(p);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join("PatchBox Sessions")
}

/// A connected front end (blocking): requests and their replies, pushes in between.
pub struct Conn {
    stream: std::os::unix::net::UnixStream,
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
}

impl Conn {
    pub fn connect() -> std::io::Result<Conn> {
        let stream = std::os::unix::net::UnixStream::connect(socket_path())?;
        let reader = std::io::BufReader::new(stream.try_clone()?);
        Ok(Conn { stream, reader })
    }

    pub fn send(&mut self, r: &Request) -> std::io::Result<()> {
        use std::io::Write;
        let mut line = serde_json::to_vec(r).map_err(std::io::Error::other)?;
        line.push(b'\n');
        self.stream.write_all(&line)
    }

    /// The next line from the daemon.
    pub fn recv(&mut self) -> std::io::Result<FromDaemon> {
        use std::io::BufRead;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "daemon closed the connection"));
        }
        serde_json::from_str(&line).map_err(std::io::Error::other)
    }

    /// Send a request and wait for its reply (pushes before it are dropped).
    pub fn call(&mut self, r: &Request) -> std::io::Result<Reply> {
        self.send(r)?;
        loop {
            if let FromDaemon::Reply(rep) = self.recv()? {
                return Ok(rep);
            }
        }
    }

    pub fn try_clone_stream(&self) -> std::io::Result<std::os::unix::net::UnixStream> {
        self.stream.try_clone()
    }

    pub fn into_parts(self) -> (std::os::unix::net::UnixStream, std::io::BufReader<std::os::unix::net::UnixStream>) {
        (self.stream, self.reader)
    }
}

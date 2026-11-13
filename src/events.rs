//! The event log (qjackctl's Messages / Status): xruns with when they happened, how late and the
//! DSP load then; the server's changes (started, stopped, buffer size, rate, freewheel, shut
//! down); clients coming and going; connections; PatchBox's own messages. And the tail of
//! jackdbus's own log, which never rotates (it grows without end): only its end is read.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Xrun,
    Server,
    Client,
    Connection,
    Message,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Xrun => "Xrun",
            Kind::Server => "Server",
            Kind::Client => "Client",
            Kind::Connection => "Connection",
            Kind::Message => "PatchBox",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub at: SystemTime,
    pub kind: Kind,
    pub text: String,
    pub bad: bool,
}

/// Which entries the page shows.
pub const FILTERS: [(&str, &str); 6] = [
    ("useful", "Xruns, server, clients and messages"),
    ("xruns", "Xruns"),
    ("server", "Server"),
    ("clients", "Clients"),
    ("connections", "Connections"),
    ("all", "Everything"),
];

pub fn shows(filter: &str, k: Kind) -> bool {
    match filter {
        "xruns" => k == Kind::Xrun,
        "server" => k == Kind::Server,
        "clients" => k == Kind::Client,
        "connections" => k == Kind::Connection,
        "all" => true,
        _ => k != Kind::Connection,
    }
}

pub struct Events {
    pub list: VecDeque<Entry>,
    /// Xruns since `since` (the count is reset with the toolbar's).
    pub xruns: u32,
    pub since: SystemTime,
    pub filter: String,
    /// Changed since the page last showed it.
    pub dirty: bool,
}

impl Default for Events {
    fn default() -> Events {
        Events { list: VecDeque::new(), xruns: 0, since: SystemTime::now(), filter: "useful".into(), dirty: false }
    }
}

const KEEP: usize = 5000;

impl Events {
    pub fn push(&mut self, kind: Kind, text: impl Into<String>, bad: bool) {
        if kind == Kind::Xrun {
            self.xruns += 1;
        }
        self.list.push_back(Entry { at: SystemTime::now(), kind, text: text.into(), bad });
        while self.list.len() > KEEP {
            self.list.pop_front();
        }
        self.dirty = true;
    }

    pub fn reset_xruns(&mut self) {
        self.xruns = 0;
        self.since = SystemTime::now();
        self.dirty = true;
    }
}

/// Local wall-clock time, "HH:MM:SS.mmm".
pub fn clock(t: SystemTime) -> String {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:02}:{:02}:{:02}.{:03}", tm.tm_hour, tm.tm_min, tm.tm_sec, d.subsec_millis())
}

/// jackdbus's log: ~/.log/jack/jackdbus.log.
pub fn jackdbus_log() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".log/jack/jackdbus.log"))
}

/// The last `lines` lines of a file, reading at most its last `bytes` bytes; and its size.
pub fn tail(path: &std::path::Path, bytes: u64, lines: usize) -> std::io::Result<(Vec<String>, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let from = size.saturating_sub(bytes);
    f.seek(SeekFrom::Start(from))?;
    let mut buf = Vec::new();
    f.take(bytes).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut v: Vec<&str> = text.lines().collect();
    // A read from the middle starts mid-line: drop that part.
    if from > 0 && !v.is_empty() {
        v.remove(0);
    }
    let start = v.len().saturating_sub(lines);
    Ok((v[start..].iter().map(|l| l.to_string()).collect(), size))
}

/// A jackdbus log line ("Fri Nov 13 00:08:19 2026: Client 'x' is out") as (when, what).
pub fn split_jackdbus(line: &str) -> (String, String) {
    // "Www Mmm dd HH:MM:SS yyyy: " is 26 characters.
    match line.get(..26) {
        Some(head) if head.ends_with(": ") && head.as_bytes().get(13) == Some(&b':') => {
            (format!("{} {}", &head[4..10], &head[11..19]), line[26..].to_string())
        }
        _ => (String::new(), line.to_string()),
    }
}

/// A size for people: "287 MB".
pub fn size(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{} MB", n >> 20),
        n if n >= 1 << 10 => format!("{} KB", n >> 10),
        n => format!("{n} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jackdbus_lines() {
        let (t, m) = split_jackdbus("Fri Nov 13 00:08:19 2026: Client 'PatchBox' with PID 384746 is out");
        assert_eq!(t, "Nov 13 00:08:19");
        assert_eq!(m, "Client 'PatchBox' with PID 384746 is out");
        assert_eq!(split_jackdbus("odd line").1, "odd line");
    }

    #[test]
    fn tail_drops_the_partial_line() {
        let p = std::env::temp_dir().join(format!("pbx-tail-{}", std::process::id()));
        std::fs::write(&p, "one\ntwo\nthree\nfour\n").unwrap();
        let (v, size) = tail(&p, 12, 10).unwrap();
        assert_eq!(size, 19);
        assert_eq!(v, vec!["three", "four"]);
        let (v, _) = tail(&p, 100, 2).unwrap();
        assert_eq!(v, vec!["three", "four"]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn filters() {
        assert!(!shows("useful", Kind::Connection));
        assert!(shows("useful", Kind::Xrun));
        assert!(shows("all", Kind::Connection));
        assert!(!shows("xruns", Kind::Server));
    }
}

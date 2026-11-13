//! A snapshot of the server's ports and connections, read on the caller's thread.

use crate::{Client, cstr, ffi, keys, property};
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PortKind {
    Audio,
    Midi,
    /// Audio-rate control signal (metadata signal-type "CV").
    Cv,
    /// MIDI-type port carrying OSC (metadata event-types mentions OSC).
    Osc,
    /// Any other port type string (video, ...).
    Other,
}

impl PortKind {
    pub const ALL: [PortKind; 4] = [PortKind::Audio, PortKind::Midi, PortKind::Cv, PortKind::Osc];

    pub fn label(self) -> &'static str {
        match self {
            PortKind::Audio => "AUDIO",
            PortKind::Midi => "MIDI",
            PortKind::Cv => "CV",
            PortKind::Osc => "OSC",
            PortKind::Other => "OTHER",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Port {
    /// Full name, "client:port".
    pub name: String,
    pub client: String,
    /// The part after the first ':'.
    pub short: String,
    pub uuid: u64,
    pub flags: u32,
    pub kind: PortKind,
    /// The raw JACK type string.
    pub type_name: String,
    pub pretty: Option<String>,
    pub order: Option<i32>,
    /// metadata port-group (e.g. an LV2 group URI or "stereo").
    pub group: Option<String>,
    pub aliases: Vec<String>,
}

impl Port {
    pub fn is_input(&self) -> bool {
        self.flags & ffi::JACK_PORT_IS_INPUT != 0
    }
    pub fn is_output(&self) -> bool {
        self.flags & ffi::JACK_PORT_IS_OUTPUT != 0
    }
    pub fn is_physical(&self) -> bool {
        self.flags & ffi::JACK_PORT_IS_PHYSICAL != 0
    }
    pub fn is_terminal(&self) -> bool {
        self.flags & ffi::JACK_PORT_IS_TERMINAL != 0
    }
    /// What a person reads: the pretty name, else the short name.
    pub fn display(&self) -> &str {
        self.pretty.as_deref().unwrap_or(&self.short)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub ports: Vec<Port>,
    /// (output port, input port) indices into `ports`.
    pub conns: Vec<(usize, usize)>,
    pub by_name: HashMap<String, usize>,
    /// Client name -> pretty name (metadata on the client's UUID).
    pub client_pretty: HashMap<String, String>,
    /// Client name -> icon name (metadata).
    pub client_icon: HashMap<String, String>,
}

impl Graph {
    pub fn port(&self, name: &str) -> Option<&Port> {
        self.by_name.get(name).map(|&i| &self.ports[i])
    }
    pub fn connected(&self, out: usize, inp: usize) -> bool {
        self.conns.contains(&(out, inp))
    }
}

/// A NULL-terminated array of C strings libjack allocated; freed with jack_free.
fn take_names(arr: *mut *const c_char) -> Vec<String> {
    let mut out = Vec::new();
    if arr.is_null() {
        return out;
    }
    unsafe {
        let mut i = 0;
        loop {
            let p = *arr.add(i);
            if p.is_null() {
                break;
            }
            out.push(cstr(p));
            i += 1;
        }
        ffi::jack_free(arr as *mut c_void);
    }
    out
}

fn kind_of(type_name: &str, uuid: u64) -> PortKind {
    let base = match type_name {
        ffi::DEFAULT_AUDIO_TYPE => PortKind::Audio,
        ffi::DEFAULT_MIDI_TYPE => PortKind::Midi,
        _ => return PortKind::Other,
    };
    let signal = property(uuid, keys::SIGNAL_TYPE).or_else(|| property(uuid, keys::OLD_SIGNAL_TYPE));
    if base == PortKind::Audio && signal.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("CV")) {
        return PortKind::Cv;
    }
    if base == PortKind::Midi {
        let events = property(uuid, keys::EVENT_TYPES).or_else(|| property(uuid, keys::OLD_EVENT_TYPES));
        if let Some(e) = events {
            let e = e.to_ascii_uppercase();
            if e.contains("OSC") && !e.contains("MIDI") {
                return PortKind::Osc;
            }
        }
    }
    base
}

fn aliases(port: *const ffi::jack_port_t) -> Vec<String> {
    let size = unsafe { ffi::jack_port_name_size() }.max(64) as usize;
    let mut a = vec![0u8; size];
    let mut b = vec![0u8; size];
    let mut ptrs = [a.as_mut_ptr() as *mut c_char, b.as_mut_ptr() as *mut c_char];
    let n = unsafe { ffi::jack_port_get_aliases(port, ptrs.as_mut_ptr()) }.clamp(0, 2) as usize;
    ptrs[..n].iter().map(|&p| cstr(p)).filter(|s| !s.is_empty()).collect()
}

pub(crate) fn snapshot(c: &Client) -> Graph {
    let raw = c.raw();
    let mut g = Graph::default();
    let names = take_names(unsafe { ffi::jack_get_ports(raw, std::ptr::null(), std::ptr::null(), 0) });
    let mut handles = Vec::with_capacity(names.len());
    for name in names {
        let Ok(cn) = CString::new(name.as_str()) else { continue };
        let p = unsafe { ffi::jack_port_by_name(raw, cn.as_ptr()) };
        if p.is_null() {
            continue; // gone since the listing
        }
        let flags = unsafe { ffi::jack_port_flags(p) } as u32;
        let type_name = cstr(unsafe { ffi::jack_port_type(p) });
        let uuid = unsafe { ffi::jack_port_uuid(p) };
        let (client, short) = match name.split_once(':') {
            Some((c, s)) => (c.to_owned(), s.to_owned()),
            None => (String::new(), name.clone()),
        };
        let kind = kind_of(&type_name, uuid);
        let pretty = property(uuid, keys::PRETTY_NAME).or_else(|| property(uuid, keys::OLD_PRETTY_NAME)).filter(|s| !s.is_empty());
        let order = property(uuid, keys::ORDER).or_else(|| property(uuid, keys::OLD_ORDER)).and_then(|s| s.trim().parse().ok());
        let group = property(uuid, keys::PORT_GROUP);
        let al = aliases(p);
        g.by_name.insert(name.clone(), g.ports.len());
        g.ports.push(Port { name, client, short, uuid, flags, kind, type_name, pretty, order, group, aliases: al });
        handles.push(p);
    }
    for (i, &p) in handles.iter().enumerate() {
        if !g.ports[i].is_output() {
            continue;
        }
        for other in take_names(unsafe { ffi::jack_port_get_all_connections(raw, p) }) {
            if let Some(&j) = g.by_name.get(&other) {
                g.conns.push((i, j));
            }
        }
    }
    let mut clients: Vec<&str> = g.ports.iter().map(|p| p.client.as_str()).collect();
    clients.sort_unstable();
    clients.dedup();
    let clients: Vec<String> = clients.into_iter().map(str::to_owned).collect();
    for name in clients {
        if let Some(uuid) = c.client_uuid(&name) {
            if let Some(p) = property(uuid, keys::PRETTY_NAME).filter(|s| !s.is_empty()) {
                g.client_pretty.insert(name.clone(), p);
            }
            if let Some(i) = property(uuid, keys::ICON_NAME).filter(|s| !s.is_empty()) {
                g.client_icon.insert(name.clone(), i);
            }
        }
    }
    g
}

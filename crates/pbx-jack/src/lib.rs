//! PatchBox's JACK side: a non-realtime client that watches the graph and edits connections.
//!
//! JACK delivers its notifications on the client's notification thread. The callbacks here only
//! queue an [`Event`] and call the waker (the GUI pushes an SDL event, so the main loop wakes);
//! everything else (reading ports, connecting) happens on the caller's thread. Calling back into
//! the server from a notification callback can deadlock jack2, and none of it needs to be fast.

pub mod ffi;
mod graph;

pub use graph::{Graph, Port, PortKind};

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::{Arc, Mutex};

/// What the server told us, in arrival order.
#[derive(Debug, Clone)]
pub enum Event {
    ClientRegistered { name: String, registered: bool },
    PortRegistered { id: u32, registered: bool },
    PortConnect { a: u32, b: u32, connected: bool },
    PortRename { old: String, new: String },
    GraphOrder,
    Xrun,
    BufferSize(u32),
    SampleRate(u32),
    Freewheel(bool),
    Property { subject: u64, key: String, change: i32 },
    /// The server went away (or threw us out). The client must be closed and reopened.
    Shutdown { reason: String },
}

impl Event {
    /// Events after which the port/connection snapshot is stale.
    pub fn changes_graph(&self) -> bool {
        matches!(
            self,
            Event::ClientRegistered { .. }
                | Event::PortRegistered { .. }
                | Event::PortConnect { .. }
                | Event::PortRename { .. }
                | Event::Property { .. }
                | Event::Shutdown { .. }
        )
    }
}

pub type Waker = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    events: Mutex<Vec<Event>>,
    waker: Waker,
}

impl Shared {
    fn push(&self, e: Event) {
        if let Ok(mut q) = self.events.lock() {
            q.push(e);
        }
        (self.waker)();
    }
}

/// An open client on a running server.
pub struct Client {
    raw: *mut ffi::jack_client_t,
    shared: Arc<Shared>,
    name: String,
    /// Client name -> UUID, as the server last answered (a request per lookup otherwise).
    uuids: Mutex<HashMap<String, Option<u64>>>,
}

// The handle is used from one thread at a time (the GUI's main thread); libjack itself is
// thread-safe for the calls we make.
unsafe impl Send for Client {}

#[derive(Debug)]
pub enum OpenError {
    /// No server is running (we never start one: JackNoStartServer).
    NoServer,
    Failed(i32),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NoServer => write!(f, "no JACK server running"),
            OpenError::Failed(s) => write!(f, "jack_client_open failed (status 0x{s:x})"),
        }
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

/// libjack prints to stderr by default (every failed connect attempt while no server runs).
/// Route its messages to ours, quiet unless PATCHBOX_JACK_DEBUG is set.
extern "C" fn jack_log(msg: *const c_char) {
    if std::env::var_os("PATCHBOX_JACK_DEBUG").is_some() {
        eprintln!("[libjack] {}", cstr(msg));
    }
}

extern "C" fn on_shutdown(_code: ffi::jack_status_t, reason: *const c_char, arg: *mut c_void) {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::Shutdown { reason: cstr(reason) });
}
extern "C" fn on_client_reg(name: *const c_char, reg: c_int, arg: *mut c_void) {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::ClientRegistered { name: cstr(name), registered: reg != 0 });
}
extern "C" fn on_port_reg(id: ffi::jack_port_id_t, reg: c_int, arg: *mut c_void) {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::PortRegistered { id, registered: reg != 0 });
}
extern "C" fn on_port_connect(a: ffi::jack_port_id_t, b: ffi::jack_port_id_t, c: c_int, arg: *mut c_void) {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::PortConnect { a, b, connected: c != 0 });
}
extern "C" fn on_port_rename(_id: ffi::jack_port_id_t, old: *const c_char, new: *const c_char, arg: *mut c_void) {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::PortRename { old: cstr(old), new: cstr(new) });
}
extern "C" fn on_graph_order(arg: *mut c_void) -> c_int {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::GraphOrder);
    0
}
extern "C" fn on_xrun(arg: *mut c_void) -> c_int {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::Xrun);
    0
}
extern "C" fn on_buffer_size(n: ffi::jack_nframes_t, arg: *mut c_void) -> c_int {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::BufferSize(n));
    0
}
extern "C" fn on_sample_rate(n: ffi::jack_nframes_t, arg: *mut c_void) -> c_int {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::SampleRate(n));
    0
}
extern "C" fn on_freewheel(starting: c_int, arg: *mut c_void) {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::Freewheel(starting != 0));
}
extern "C" fn on_property(subject: ffi::jack_uuid_t, key: *const c_char, change: ffi::jack_property_change_t, arg: *mut c_void) {
    let s = unsafe { &*(arg as *const Shared) };
    s.push(Event::Property { subject, key: cstr(key), change });
}

impl Client {
    /// Open and activate a client named `name` on the default server. Never starts a server.
    pub fn open(name: &str, waker: Waker) -> Result<Client, OpenError> {
        unsafe {
            ffi::jack_set_error_function(jack_log);
            ffi::jack_set_info_function(jack_log);
        }
        let cname = CString::new(name).unwrap_or_default();
        let mut status: ffi::jack_status_t = 0;
        let raw = unsafe { ffi::jack_client_open(cname.as_ptr(), ffi::JACK_NO_START_SERVER, &mut status) };
        if raw.is_null() {
            return Err(if status & ffi::JACK_SERVER_FAILED != 0 { OpenError::NoServer } else { OpenError::Failed(status) });
        }
        let shared = Arc::new(Shared { events: Mutex::new(Vec::new()), waker });
        let arg = Arc::as_ptr(&shared) as *mut c_void;
        unsafe {
            ffi::jack_on_info_shutdown(raw, on_shutdown, arg);
            ffi::jack_set_client_registration_callback(raw, on_client_reg, arg);
            ffi::jack_set_port_registration_callback(raw, on_port_reg, arg);
            ffi::jack_set_port_connect_callback(raw, on_port_connect, arg);
            ffi::jack_set_port_rename_callback(raw, on_port_rename, arg);
            ffi::jack_set_graph_order_callback(raw, on_graph_order, arg);
            ffi::jack_set_xrun_callback(raw, on_xrun, arg);
            ffi::jack_set_buffer_size_callback(raw, on_buffer_size, arg);
            ffi::jack_set_sample_rate_callback(raw, on_sample_rate, arg);
            ffi::jack_set_freewheel_callback(raw, on_freewheel, arg);
            ffi::jack_set_property_change_callback(raw, on_property, arg);
        }
        let actual = cstr(unsafe { ffi::jack_get_client_name(raw) });
        let client = Client { raw, shared, name: actual, uuids: Mutex::new(HashMap::new()) };
        if unsafe { ffi::jack_activate(raw) } != 0 {
            return Err(OpenError::Failed(-1));
        }
        Ok(client)
    }

    /// Our name on the server (JACK may have suffixed it).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Everything queued since the last call.
    pub fn take_events(&self) -> Vec<Event> {
        self.shared.events.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe { ffi::jack_get_sample_rate(self.raw) }
    }
    pub fn buffer_size(&self) -> u32 {
        unsafe { ffi::jack_get_buffer_size(self.raw) }
    }
    pub fn set_buffer_size(&self, frames: u32) -> bool {
        unsafe { ffi::jack_set_buffer_size(self.raw, frames) == 0 }
    }
    /// The server's DSP load, percent.
    pub fn cpu_load(&self) -> f32 {
        unsafe { ffi::jack_cpu_load(self.raw) }
    }
    pub fn is_realtime(&self) -> bool {
        unsafe { ffi::jack_is_realtime(self.raw) != 0 }
    }
    pub fn set_freewheel(&self, on: bool) -> bool {
        unsafe { ffi::jack_set_freewheel(self.raw, on as c_int) == 0 }
    }

    pub fn connect(&self, source: &str, destination: &str) -> bool {
        let (Ok(s), Ok(d)) = (CString::new(source), CString::new(destination)) else { return false };
        let r = unsafe { ffi::jack_connect(self.raw, s.as_ptr(), d.as_ptr()) };
        r == 0 || r == libc_eexist()
    }
    pub fn disconnect(&self, source: &str, destination: &str) -> bool {
        let (Ok(s), Ok(d)) = (CString::new(source), CString::new(destination)) else { return false };
        unsafe { ffi::jack_disconnect(self.raw, s.as_ptr(), d.as_ptr()) == 0 }
    }

    /// The UUID of a client, cached (it is a server request).
    pub fn client_uuid(&self, client: &str) -> Option<u64> {
        if let Some(u) = self.uuids.lock().ok().and_then(|m| m.get(client).copied()) {
            return u;
        }
        let c = CString::new(client).ok()?;
        let p = unsafe { ffi::jack_get_uuid_for_client_name(self.raw, c.as_ptr()) };
        let uuid = if p.is_null() {
            None
        } else {
            let mut u: ffi::jack_uuid_t = 0;
            let ok = unsafe { ffi::jack_uuid_parse(p, &mut u) } == 0;
            unsafe { ffi::jack_free(p as *mut c_void) };
            ok.then_some(u)
        };
        if let Ok(mut m) = self.uuids.lock() {
            m.insert(client.to_owned(), uuid);
        }
        uuid
    }

    /// Forget cached client UUIDs (a client of that name may come back as another one).
    pub fn forget_client(&self, client: &str) {
        if let Ok(mut m) = self.uuids.lock() {
            m.remove(client);
        }
    }

    /// A metadata property's value.
    pub fn property(&self, subject: u64, key: &str) -> Option<String> {
        property(subject, key)
    }

    /// Read every port and connection.
    pub fn snapshot(&self) -> Graph {
        graph::snapshot(self)
    }

    pub(crate) fn raw(&self) -> *mut ffi::jack_client_t {
        self.raw
    }

    /// Register a port (test tools only: PatchBox itself owns no ports).
    pub fn register_port(&self, name: &str, audio: bool, input: bool) -> bool {
        let Ok(n) = CString::new(name) else { return false };
        let t = CString::new(if audio { ffi::DEFAULT_AUDIO_TYPE } else { ffi::DEFAULT_MIDI_TYPE }).unwrap();
        let flags = if input { ffi::JACK_PORT_IS_INPUT } else { ffi::JACK_PORT_IS_OUTPUT };
        !unsafe { ffi::jack_port_register(self.raw, n.as_ptr(), t.as_ptr(), flags as _, 0) }.is_null()
    }

    /// Set a metadata property on one of this process's subjects (test tools: pretty names).
    pub fn set_property(&self, subject: u64, key: &str, value: &str) -> bool {
        let (Ok(k), Ok(v)) = (CString::new(key), CString::new(value)) else { return false };
        unsafe { ffi::jack_set_property(self.raw, subject, k.as_ptr(), v.as_ptr(), ptr::null()) == 0 }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Closing deactivates; callbacks stop before it returns, so `shared` may go after.
        unsafe { ffi::jack_client_close(self.raw) };
    }
}

fn libc_eexist() -> c_int {
    17 // EEXIST: the connection already exists, which is what we wanted
}

pub(crate) fn property(subject: u64, key: &str) -> Option<String> {
    let k = CString::new(key).ok()?;
    let mut value: *mut c_char = ptr::null_mut();
    let mut ty: *mut c_char = ptr::null_mut();
    let r = unsafe { ffi::jack_get_property(subject, k.as_ptr(), &mut value, &mut ty) };
    let out = (r == 0 && !value.is_null()).then(|| cstr(value));
    unsafe {
        if !value.is_null() {
            ffi::jack_free(value as *mut c_void);
        }
        if !ty.is_null() {
            ffi::jack_free(ty as *mut c_void);
        }
    }
    out
}

/// Metadata keys (jack/metadata.h, and the older jackaudio.org namespace still in use).
pub mod keys {
    pub const PRETTY_NAME: &str = "http://jack.audio/metadata/pretty-name";
    pub const PORT_GROUP: &str = "http://jack.audio/metadata/port-group";
    pub const ICON_NAME: &str = "http://jack.audio/metadata/icon-name";
    pub const ORDER: &str = "http://jack.audio/metadata/order";
    pub const SIGNAL_TYPE: &str = "http://jack.audio/metadata/signal-type";
    pub const EVENT_TYPES: &str = "http://jack.audio/metadata/event-types";
    pub const HARDWARE: &str = "http://jack.audio/metadata/hardware";
    pub const OLD_ORDER: &str = "http://jackaudio.org/metadata/order";
    pub const OLD_SIGNAL_TYPE: &str = "http://jackaudio.org/metadata/signal-type";
    pub const OLD_EVENT_TYPES: &str = "http://jackaudio.org/metadata/event-types";
    pub const OLD_PRETTY_NAME: &str = "http://jackaudio.org/metadata/pretty-name";
}

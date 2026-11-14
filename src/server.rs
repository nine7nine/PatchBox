//! The JACK server itself, through jackdbus (org.jackaudio.service): start, stop, restart, and
//! its settings (engine, driver and the driver's parameters) — qjackctl's job. Never by spawning
//! jackd: the server it's made for is jackdbus (jack2dbus-nspa), often with PipeWire on top.
//!
//! D-Bus calls can take seconds (StartServer opens the device), so a worker thread owns the
//! connection; the GUI sends it commands and gets replies through the main loop's waker.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value as ZValue};

const SERVICE: &str = "org.jackaudio.service";
const PATH: &str = "/org/jackaudio/Controller";
const CONTROL: &str = "org.jackaudio.JackControl";
const CONFIGURE: &str = "org.jackaudio.Configure";

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    I32(i32),
    U32(u32),
    Char(u8),
    Str(String),
}

impl Value {
    pub fn show(&self) -> String {
        match self {
            Value::Bool(b) => if *b { "on" } else { "off" }.into(),
            Value::I32(v) => v.to_string(),
            Value::U32(v) => v.to_string(),
            Value::Char(c) => (*c as char).to_string(),
            Value::Str(s) => s.clone(),
        }
    }

    /// Parse `s` as a value of the same type as `self`.
    pub fn parse_like(&self, s: &str) -> Option<Value> {
        let s = s.trim();
        Some(match self {
            Value::Bool(_) => Value::Bool(matches!(s, "1" | "on" | "true" | "yes")),
            Value::I32(_) => Value::I32(s.parse().ok()?),
            Value::U32(_) => Value::U32(s.parse().ok()?),
            Value::Char(_) => Value::Char(*s.as_bytes().first()?),
            Value::Str(_) => Value::Str(s.to_string()),
        })
    }

    fn to_z(&self) -> ZValue<'static> {
        match self {
            Value::Bool(b) => ZValue::from(*b),
            Value::I32(v) => ZValue::from(*v),
            Value::U32(v) => ZValue::from(*v),
            Value::Char(c) => ZValue::from(*c),
            Value::Str(s) => ZValue::from(s.clone()),
        }
    }

    fn from_z(v: &OwnedValue) -> Option<Value> {
        Some(match &**v {
            ZValue::Bool(b) => Value::Bool(*b),
            ZValue::I32(x) => Value::I32(*x),
            ZValue::U32(x) => Value::U32(*x),
            ZValue::U8(c) => Value::Char(*c),
            ZValue::Str(s) => Value::Str(s.to_string()),
            ZValue::Value(inner) => return Value::from_z(&OwnedValue::try_from(&**inner).ok()?),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Constraint {
    /// Two values: min and max.
    pub range: bool,
    /// Only the listed values work.
    pub strict: bool,
    /// The values mean nothing to a person (show the labels only).
    pub fake: bool,
    pub values: Vec<(Value, String)>,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub path: Vec<String>,
    pub name: String,
    pub short: String,
    pub long: String,
    pub value: Value,
    pub default: Value,
    pub is_set: bool,
    pub constraint: Option<Constraint>,
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub driver: String,
    pub drivers: Vec<String>,
    pub engine: Vec<Param>,
    pub driver_params: Vec<Param>,
    /// Slave drivers in use (the engine's slave-drivers), and each one's parameters.
    pub slaves: Vec<String>,
    pub slave_params: std::collections::BTreeMap<String, Vec<Param>>,
    /// Internal clients jackdbus offers, and each one's parameters.
    pub internals: Vec<String>,
    pub internal_params: std::collections::BTreeMap<String, Vec<Param>>,
}

impl Config {
    /// Every parameter, wherever it is in the tree.
    pub fn all_params(&self) -> impl Iterator<Item = &Param> {
        self.engine.iter().chain(&self.driver_params).chain(self.slave_params.values().flatten()).chain(self.internal_params.values().flatten())
    }
}

/// The running server's figures, as jackdbus reports them.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub load: f64,
    pub latency_ms: f64,
    pub xruns: u32,
    pub realtime: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    /// jackdbus answered on the session bus.
    pub available: bool,
    pub started: bool,
}

pub enum Cmd {
    Refresh,
    Start,
    Stop,
    Restart,
    LoadConfig,
    Set(Vec<String>, Value),
    Reset(Vec<String>),
    /// Make the selected driver the running server's master (no restart).
    SwitchMaster,
    /// Add or remove a slave driver (live when the server runs, else the engine's list).
    Slave(String, bool),
    /// Load or unload an internal client.
    Internal(String, bool),
    ResetXruns,
    /// The server's figures (load, latency, xruns).
    Stats,
    /// Quit the jackdbus service (it stops the server).
    Exit,
    /// Read the driver and the parameters set (for presets).
    ReadSettings,
    /// Put these settings in place and (re)start the server: a preset, by name.
    ApplySettings(Box<pbx_session::jackcfg::JackSettings>, String),
}

pub enum Reply {
    Status(Status),
    Stats(Stats),
    Config(Box<Config>),
    /// The driver and the parameters set, as a preset keeps them.
    Settings(Box<pbx_session::jackcfg::JackSettings>),
    /// A finished action, for the log.
    Done(String),
    Error(String),
}

pub struct Server {
    tx: Sender<Cmd>,
    replies: Arc<Mutex<Vec<Reply>>>,
    pub status: Status,
    pub config: Option<Config>,
    /// A command is running (Start can take seconds).
    pub busy: bool,
    pub stats: Option<Stats>,
    /// Internal clients loaded from here (jackdbus can't be asked which are).
    pub internals_loaded: std::collections::BTreeSet<String>,
}

impl Server {
    pub fn spawn(waker: pbx_jack::Waker) -> Server {
        let (tx, rx) = channel();
        let replies = Arc::new(Mutex::new(Vec::new()));
        let r = replies.clone();
        std::thread::Builder::new()
            .name("jackdbus".into())
            .spawn(move || worker(rx, r, waker))
            .expect("jackdbus thread");
        let t2 = tx.clone();
        let _ = std::thread::Builder::new().name("jackdbus-signals".into()).spawn(move || watch_signals(t2));
        let s = Server { tx, replies, status: Status::default(), config: None, busy: false, stats: None, internals_loaded: Default::default() };
        s.send(Cmd::Refresh);
        s
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    /// Run a command, marking the server busy until its reply.
    pub fn run(&mut self, c: Cmd) {
        self.busy = true;
        self.send(c);
    }

    pub fn take(&mut self) -> Vec<Reply> {
        let v = self.replies.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        if !v.is_empty() {
            self.busy = false;
        }
        v
    }
}

fn worker(rx: Receiver<Cmd>, out: Arc<Mutex<Vec<Reply>>>, waker: pbx_jack::Waker) {
    let push = |r: Reply| {
        if let Ok(mut q) = out.lock() {
            q.push(r);
        }
        waker();
    };
    let mut conn: Option<Connection> = None;
    while let Ok(cmd) = rx.recv() {
        if conn.is_none() {
            conn = Connection::session().map_err(|e| push(Reply::Error(format!("D-Bus session bus: {e}")))).ok();
        }
        let Some(c) = &conn else {
            push(Reply::Status(Status::default()));
            continue;
        };
        let res = match cmd {
            Cmd::Refresh => Ok(None),
            Cmd::Stats => {
                if let Some(st) = stats(c) {
                    push(Reply::Stats(st));
                }
                Ok(None)
            }
            Cmd::SwitchMaster => control(c, "SwitchMaster").map(|_| Some("Switched the master driver".to_string())),
            Cmd::ResetXruns => control(c, "ResetXruns").map(|_| Some("Server xruns reset".to_string())),
            Cmd::Exit => control(c, "Exit").map(|_| Some("jackdbus quit".to_string())),
            Cmd::ReadSettings => pbx_session::jackcfg::read(c).map(|s| {
                push(Reply::Settings(Box::new(s)));
                None
            }),
            Cmd::ApplySettings(s, name) => pbx_session::jackcfg::apply(c, &s).and_then(|done| {
                use pbx_session::jackcfg::Applied;
                push(Reply::Config(Box::new(load_config(c)?)));
                push(Reply::Settings(s));
                Ok(Some(match done {
                    Applied::Restarted => format!("Preset {name}: in place, the server restarted"),
                    Applied::Live(Some(frames)) => format!("Preset {name}: in place, buffer size {frames} without a restart"),
                    Applied::Live(None) => format!("Preset {name}: in place, without a restart"),
                    Applied::Unchanged => format!("Preset {name}: already in place"),
                }))
            }),
            Cmd::Slave(name, on) => slave(c, &name, on).and_then(|msg| {
                let cfg = load_config(c)?;
                push(Reply::Config(Box::new(cfg)));
                Ok(Some(msg))
            }),
            Cmd::Internal(name, on) => proxy(c, CONTROL)
                .and_then(|p| p.call_method(if on { "LoadInternal" } else { "UnloadInternal" }, &(name.clone(),)).map_err(err))
                .map(|_| Some(format!("{} the internal client {name}", if on { "Loaded" } else { "Unloaded" }))),
            Cmd::Start => control(c, "StartServer").map(|_| Some("JACK server started".to_string())),
            Cmd::Stop => control(c, "StopServer").map(|_| Some("JACK server stopped".to_string())),
            Cmd::Restart => control(c, "StopServer")
                .or_else(|e| if e.contains("ServerNotRunning") { Ok(()) } else { Err(e) })
                .and_then(|_| control(c, "StartServer"))
                .map(|_| Some("JACK server restarted".to_string())),
            Cmd::LoadConfig => match load_config(c) {
                Ok(cfg) => {
                    push(Reply::Config(Box::new(cfg)));
                    Ok(None)
                }
                Err(e) => Err(e),
            },
            Cmd::Set(path, v) => set_param(c, &path, &v).and_then(|_| {
                let cfg = load_config(c)?;
                push(Reply::Config(Box::new(cfg)));
                Ok(Some(format!("{} = {}", path.join("/"), v.show())))
            }),
            Cmd::Reset(path) => configure(c)
                .and_then(|p| p.call_method("ResetParameterValue", &(path.clone(),)).map_err(err))
                .and_then(|_| {
                    let cfg = load_config(c)?;
                    push(Reply::Config(Box::new(cfg)));
                    Ok(Some(format!("{} reset", path.join("/"))))
                }),
        };
        match res {
            Ok(Some(msg)) => push(Reply::Done(msg)),
            Ok(None) => {}
            Err(e) => push(Reply::Error(e)),
        }
        push(Reply::Status(status(c)));
    }
}

fn err(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, msg, _) => format!("{}: {}", name.as_str(), msg.unwrap_or_default()),
        e => e.to_string(),
    }
}

fn proxy<'a>(c: &'a Connection, iface: &'static str) -> Result<Proxy<'a>, String> {
    Proxy::new(c, SERVICE, PATH, iface).map_err(err)
}

fn configure(c: &Connection) -> Result<Proxy<'_>, String> {
    proxy(c, CONFIGURE)
}

fn control(c: &Connection, method: &str) -> Result<(), String> {
    proxy(c, CONTROL)?.call_method(method, &()).map(|_| ()).map_err(err)
}

fn status(c: &Connection) -> Status {
    match proxy(c, CONTROL).and_then(|p| p.call::<_, _, bool>("IsStarted", &()).map_err(err)) {
        Ok(started) => Status { available: true, started },
        Err(_) => Status { available: false, started: false },
    }
}

fn set_param(c: &Connection, path: &[String], v: &Value) -> Result<(), String> {
    configure(c)?.call_method("SetParameterValue", &(path.to_vec(), v.to_z())).map(|_| ()).map_err(err)
}

fn param(p: &Proxy<'_>, path: Vec<String>, info: (u8, String, String, String)) -> Result<Param, String> {
    let (is_set, default, value): (bool, OwnedValue, OwnedValue) = p.call("GetParameterValue", &(path.clone(),)).map_err(err)?;
    let (range, strict, fake, values): (bool, bool, bool, Vec<(OwnedValue, String)>) =
        p.call("GetParameterConstraint", &(path.clone(),)).map_err(err)?;
    let constraint = (!values.is_empty()).then(|| Constraint {
        range,
        strict,
        fake,
        values: values.iter().filter_map(|(v, l)| Some((Value::from_z(v)?, l.clone()))).collect(),
    });
    let value = Value::from_z(&value).ok_or_else(|| format!("{}: unsupported value type", path.join("/")))?;
    let default = Value::from_z(&default).unwrap_or_else(|| value.clone());
    Ok(Param { name: info.1, short: info.2, long: info.3, path, value, default, is_set, constraint })
}

fn params(p: &Proxy<'_>, parent: &str) -> Result<Vec<Param>, String> {
    let infos: Vec<(u8, String, String, String)> = p.call("GetParametersInfo", &(vec![parent.to_string()],)).map_err(err)?;
    infos.into_iter().map(|i| param(p, vec![parent.to_string(), i.1.clone()], i)).collect()
}

fn load_config(c: &Connection) -> Result<Config, String> {
    let p = configure(c)?;
    let (_, drivers): (bool, Vec<String>) = p.call("ReadContainer", &(vec!["drivers".to_string()],)).map_err(err)?;
    let engine = params(&p, "engine")?;
    let driver = engine.iter().find(|e| e.name == "driver").map(|e| e.value.show()).unwrap_or_default();
    let driver_params = params(&p, "driver")?;
    let slaves: Vec<String> = engine
        .iter()
        .find(|e| e.name == "slave-drivers")
        .map(|e| e.value.show().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let mut slave_params = std::collections::BTreeMap::new();
    for s in &slaves {
        if let Ok(ps) = params_at(&p, &["drivers", s]) {
            slave_params.insert(s.clone(), ps);
        }
    }
    let (_, internals): (bool, Vec<String>) = p.call("ReadContainer", &(vec!["internals".to_string()],)).map_err(err).unwrap_or((true, Vec::new()));
    let mut internal_params = std::collections::BTreeMap::new();
    for i in &internals {
        if let Ok(ps) = params_at(&p, &["internals", i]) {
            internal_params.insert(i.clone(), ps);
        }
    }
    Ok(Config { driver, drivers, engine, driver_params, slaves, slave_params, internals, internal_params })
}

/// Parameters under any container (["drivers", "alsarawmidi"], ["internals", "netmanager"]).
fn params_at(p: &Proxy<'_>, parent: &[&str]) -> Result<Vec<Param>, String> {
    let base: Vec<String> = parent.iter().map(|s| s.to_string()).collect();
    let infos: Vec<(u8, String, String, String)> = p.call("GetParametersInfo", &(base.clone(),)).map_err(err)?;
    infos
        .into_iter()
        .map(|i| {
            let mut path = base.clone();
            path.push(i.1.clone());
            param(p, path, i)
        })
        .collect()
}

/// Add or remove a slave driver. jackdbus takes both AddSlaveDriver and the engine's
/// slave-drivers list only while the server is stopped, so a running server is stopped, changed
/// and started again.
fn slave(c: &Connection, name: &str, on: bool) -> Result<String, String> {
    let started = status(c).started;
    if started {
        control(c, "StopServer")?;
    }
    let r = proxy(c, CONTROL)?.call_method(if on { "AddSlaveDriver" } else { "RemoveSlaveDriver" }, &(name.to_string(),)).map_err(err);
    if started {
        control(c, "StartServer")?;
    }
    r?;
    Ok(format!(
        "{} the slave driver {name}{}",
        if on { "Added" } else { "Removed" },
        if started { " (the server restarted for it)" } else { "" }
    ))
}

fn stats(c: &Connection) -> Option<Stats> {
    let p = proxy(c, CONTROL).ok()?;
    let started: bool = p.call("IsStarted", &()).ok()?;
    if !started {
        return None;
    }
    Some(Stats {
        load: p.call("GetLoad", &()).unwrap_or(0.0),
        latency_ms: p.call("GetLatency", &()).unwrap_or(0.0),
        xruns: p.call("GetXruns", &()).unwrap_or(0),
        realtime: p.call("IsRealtime", &()).unwrap_or(false),
    })
}

/// jackdbus's ServerStarted / ServerStopped, so a start or stop from elsewhere (jack_control,
/// another program) shows at once: each one triggers a refresh.
fn watch_signals(tx: Sender<Cmd>) {
    let Ok(c) = Connection::session() else { return };
    let Ok(p) = Proxy::new(&c, SERVICE, PATH, CONTROL) else { return };
    let Ok(sigs) = p.receive_all_signals() else { return };
    for _ in sigs {
        if tx.send(Cmd::Refresh).is_err() {
            break;
        }
    }
}

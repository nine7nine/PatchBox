//! The JACK server itself, through jackdbus (org.jackaudio.service): start, stop, restart, and
//! its settings (engine, driver and the driver's parameters) — qjackctl's job. Never by spawning
//! jackd: the owner's server is jackdbus (jack2dbus-nspa), with PipeWire on top.
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
}

pub enum Reply {
    Status(Status),
    Config(Box<Config>),
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
        let s = Server { tx, replies, status: Status::default(), config: None, busy: false };
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
    Ok(Config { driver, drivers, engine, driver_params })
}

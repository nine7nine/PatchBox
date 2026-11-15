//! A session's JACK settings, through jackdbus (org.jackaudio.service): read the configuration
//! (the driver, and every engine and driver parameter that is set rather than defaulted), and
//! put one back: stop the server, set, start. RaySession does this with shell scripts around
//! jack_control; here it is the D-Bus Configure interface directly. What a running server takes
//! as it is (the buffer size, jackdbus' own log settings) is applied without the restart, which
//! would throw every client off the server.
//!
//! Values are kept as typed strings ("u:48000", "b:true", "s:hw:PCH", "i:-1", "y:99"), so the
//! session file stays plain TOML.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

const SERVICE: &str = "org.jackaudio.service";
const PATH: &str = "/org/jackaudio/Controller";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct JackSettings {
    /// Restore these when the session opens.
    pub remember: bool,
    pub driver: String,
    /// "engine/<name>" or "driver/<name>" -> typed value.
    pub params: BTreeMap<String, String>,
}

fn err(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(n, m, _) => format!("{}: {}", n.as_str(), m.unwrap_or_default()),
        e => e.to_string(),
    }
}

fn typed(v: &OwnedValue) -> Option<String> {
    Some(match &**v {
        Value::Bool(b) => format!("b:{b}"),
        Value::I32(x) => format!("i:{x}"),
        Value::U32(x) => format!("u:{x}"),
        Value::U8(x) => format!("y:{x}"),
        Value::Str(s) => format!("s:{s}"),
        Value::Value(inner) => return typed(&OwnedValue::try_from(&**inner).ok()?),
        _ => return None,
    })
}

fn untyped(s: &str) -> Option<Value<'static>> {
    let (t, v) = s.split_once(':')?;
    Some(match t {
        "b" => Value::from(v == "true"),
        "i" => Value::from(v.parse::<i32>().ok()?),
        "u" => Value::from(v.parse::<u32>().ok()?),
        "y" => Value::from(v.parse::<u8>().ok()?),
        "s" => Value::from(v.to_string()),
        _ => return None,
    })
}

/// The session bus, if jackdbus is on it (never auto-started just to look).
pub fn connect() -> Option<Connection> {
    let c = Connection::session().ok()?;
    let dbus = Proxy::new(&c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").ok()?;
    let has: bool = dbus.call("NameHasOwner", &(SERVICE,)).ok()?;
    has.then_some(c)
}

/// Before a client's process is stopped: take its JACK clients out of the graph
/// (org.jackaudio.JackPatchbay.DetachProcess, jack2dbus-nspa), so that its departure costs the
/// graph nothing, whatever its libjack and even if it is killed in the middle of its process
/// callback. Returns how many clients were taken out; None if jackdbus is not running (never
/// started just for this), its server is stopped, or it has no such method (another JACK2): the
/// process is stopped as before either way.
///
/// Called from the daemon's loop, so bounded: a jackdbus that does not answer within a second
/// is not asked again for 10 s (a session of stuck clients is not held up once per client).
pub fn detach_process(pid: u32) -> Option<u32> {
    static UNANSWERED: Mutex<Option<Instant>> = Mutex::new(None);
    if UNANSWERED.lock().ok()?.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
        return None;
    }
    let c = zbus::blocking::connection::Builder::session().ok()?.method_timeout(Duration::from_secs(1)).build().ok()?;
    let dbus = Proxy::new(&c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus").ok()?;
    let has: bool = dbus.call("NameHasOwner", &(SERVICE,)).ok()?;
    if !has {
        return None;
    }
    let patchbay = Proxy::new(&c, SERVICE, PATH, "org.jackaudio.JackPatchbay").ok()?;
    match patchbay.call::<_, _, u32>("DetachProcess", &(pid as i64,)) {
        Ok(n) => Some(n),
        Err(zbus::Error::InputOutput(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
            if let Ok(mut t) = UNANSWERED.lock() {
                *t = Some(Instant::now());
            }
            None
        }
        Err(_) => None,
    }
}

fn configure(c: &Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(c, SERVICE, PATH, "org.jackaudio.Configure").map_err(err)
}

fn control(c: &Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(c, SERVICE, PATH, "org.jackaudio.JackControl").map_err(err)
}

/// The current configuration: the driver and the parameters that are set.
pub fn read(c: &Connection) -> Result<JackSettings, String> {
    let p = configure(c)?;
    let (_, _, d): (bool, OwnedValue, OwnedValue) = p.call("GetParameterValue", &(vec!["engine".to_string(), "driver".to_string()],)).map_err(err)?;
    let driver = typed(&d).and_then(|s| s.strip_prefix("s:").map(str::to_string)).unwrap_or_default();
    let mut params = BTreeMap::new();
    for parent in ["engine", "driver"] {
        let infos: Vec<(u8, String, String, String)> = p.call("GetParametersInfo", &(vec![parent.to_string()],)).map_err(err)?;
        for (_, name, _, _) in infos {
            if parent == "engine" && name == "driver" {
                continue;
            }
            let path = vec![parent.to_string(), name.clone()];
            let (is_set, _, v): (bool, OwnedValue, OwnedValue) = p.call("GetParameterValue", &(path,)).map_err(err)?;
            if is_set && let Some(t) = typed(&v) {
                params.insert(format!("{parent}/{name}"), t);
            }
        }
    }
    Ok(JackSettings { remember: true, driver, params })
}

/// What `apply` did.
#[derive(Debug, PartialEq)]
pub enum Applied {
    /// It was already so (the server is left alone, started if it wasn't).
    Unchanged,
    /// Changed on the running server, no restart: the buffer size (Some(frames)) and/or
    /// jackdbus' own settings.
    Live(Option<u32>),
    /// The server was stopped, set and started.
    Restarted,
}

/// The parameters set differently in `a` and `b`, or set in only one of them.
fn changed<'a>(a: &'a BTreeMap<String, String>, b: &'a BTreeMap<String, String>) -> BTreeSet<&'a str> {
    a.keys().chain(b.keys()).filter(|k| a.get(*k) != b.get(*k)).map(String::as_str).collect()
}

/// Whether a running server takes these parameters as they are: the buffer size (its period,
/// org.jackaudio.JackControl.SetBufferSize) with the ALSA and dummy drivers, jackdbus' own log
/// settings (jack2dbus-nspa, read at each line), and the CPU wake-up latency it holds
/// (jack2dbus-nspa: applied when set; checked, since an older jackdbus only stores it).
/// Anything else needs a restart.
fn live(driver: &str, changed: &BTreeSet<&str>) -> bool {
    !changed.is_empty()
        && changed.iter().all(|k| match *k {
            "driver/period" => matches!(driver, "alsa" | "dummy"),
            "engine/log-level" | "engine/log-max-size" | "engine/cpu-dma-latency" => true,
            // jack2dbus-nspa applies its list of sound card bridges at once
            "engine/bridges" => true,
            _ => false,
        })
}

/// The CPU wake-up latency limit in effect system-wide (the smallest of all requests), µs;
/// None if /dev/cpu_dma_latency cannot be read. Reading it holds no request.
fn cpu_dma_latency_now() -> Option<i32> {
    let b = std::fs::read("/dev/cpu_dma_latency").ok()?;
    Some(i32::from_ne_bytes(b.get(..4)?.try_into().ok()?))
}

/// Set (or reset to its default) each changed parameter, as for a restart, then give the running
/// server its new buffer size. Err: the server did not take it (a restart follows).
fn apply_live(c: &Connection, ctl: &Proxy<'_>, cur: &JackSettings, want: &JackSettings, changed: &BTreeSet<&str>) -> Result<Option<u32>, String> {
    let p = configure(c)?;
    for k in changed {
        let path: Vec<String> = k.split('/').map(str::to_string).collect();
        match want.params.get(*k).and_then(|v| untyped(v)) {
            Some(v) => p.call_method("SetParameterValue", &(path, v)).map(|_| ()).map_err(err)?,
            None => p.call_method("ResetParameterValue", &(path,)).map(|_| ()).map_err(err)?,
        }
    }
    // A jackdbus that applies the request only at its next start leaves the limit in effect as it
    // was: above a new limit asked for, or still at (or below) the one released. Then it needs the
    // restart (a needless one only if another program holds a limit that low).
    if changed.contains("engine/cpu-dma-latency") {
        let usec = |s: &JackSettings| {
            s.params.get("engine/cpu-dma-latency").and_then(|v| v.strip_prefix("i:")).and_then(|n| n.parse::<i32>().ok()).filter(|n| *n >= 0)
        };
        if let Some(now) = cpu_dma_latency_now() {
            match (usec(cur), usec(want)) {
                (_, Some(asked)) if now > asked => {
                    return Err(format!("the CPU wake-up latency limit is {now} us, not {asked}: this jackdbus applies it at the next start"));
                }
                (Some(held), None) if now <= held => {
                    return Err(format!("the CPU wake-up latency limit is still {now} us: this jackdbus releases it at the next start"));
                }
                _ => {}
            }
        }
    }
    if !changed.contains("driver/period") {
        return Ok(None);
    }
    // The value now in effect for the next start: the session's, or the default it was reset to
    let path = vec!["driver".to_string(), "period".to_string()];
    let (_, _, v): (bool, OwnedValue, OwnedValue) = p.call("GetParameterValue", &(path,)).map_err(err)?;
    let frames = typed(&v)
        .and_then(|s| s.strip_prefix("u:").and_then(|n| n.parse::<u32>().ok()))
        .ok_or("the driver's period is not a number")?;
    ctl.call_method("SetBufferSize", &(frames,)).map_err(err)?;
    let now: u32 = ctl.call("GetBufferSize", &()).map_err(err)?;
    if now != frames {
        return Err(format!("the buffer size is {now}, not {frames}"));
    }
    Ok(Some(frames))
}

/// Make jackdbus's configuration `want`, on the running server where it can be (`Applied::Live`),
/// else by (re)starting the server with it.
pub fn apply(c: &Connection, want: &JackSettings) -> Result<Applied, String> {
    let cur = read(c)?;
    let ctl = control(c)?;
    let started: bool = ctl.call("IsStarted", &()).map_err(err)?;
    if cur.driver == want.driver && cur.params == want.params {
        if !started {
            ctl.call_method("StartServer", &()).map_err(err)?;
        }
        return Ok(Applied::Unchanged);
    }
    let diff = changed(&cur.params, &want.params);
    if started && cur.driver == want.driver && live(&want.driver, &diff) {
        // A failure leaves the parameters set: the restart below starts with them
        if let Ok(frames) = apply_live(c, &ctl, &cur, want, &diff) {
            return Ok(Applied::Live(frames));
        }
    }
    if started {
        ctl.call_method("StopServer", &()).map_err(err)?;
    }
    let p = configure(c)?;
    let set = |path: &str, v: Value<'static>| -> Result<(), String> {
        let path: Vec<String> = path.split('/').map(str::to_string).collect();
        p.call_method("SetParameterValue", &(path, v)).map(|_| ()).map_err(err)
    };
    if !want.driver.is_empty() && want.driver != cur.driver {
        set("engine/driver", Value::from(want.driver.clone()))?;
    }
    for (k, v) in &want.params {
        if let Some(v) = untyped(v) {
            set(k, v)?;
        }
    }
    // Parameters set now but not in the session go back to their defaults.
    for k in cur.params.keys().filter(|k| !want.params.contains_key(*k)) {
        let path: Vec<String> = k.split('/').map(str::to_string).collect();
        p.call_method("ResetParameterValue", &(path,)).map_err(err)?;
    }
    ctl.call_method("StartServer", &()).map_err(err)?;
    Ok(Applied::Restarted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn changed_parameters() {
        let a = params(&[("driver/period", "u:512"), ("driver/rate", "u:48000"), ("engine/realtime", "b:true")]);
        let b = params(&[("driver/period", "u:256"), ("driver/rate", "u:48000"), ("driver/nperiods", "u:2")]);
        let d = changed(&a, &b);
        assert_eq!(d.into_iter().collect::<Vec<_>>(), vec!["driver/nperiods", "driver/period", "engine/realtime"]);
        assert!(changed(&a, &a).is_empty());
    }

    #[test]
    fn live_or_restart() {
        let set = |ks: &[&'static str]| ks.iter().copied().collect::<BTreeSet<&str>>();
        assert!(live("alsa", &set(&["driver/period"])));
        assert!(live("dummy", &set(&["driver/period"])));
        assert!(live("alsa", &set(&["driver/period", "engine/log-level"])));
        assert!(live("alsa", &set(&["engine/cpu-dma-latency"])));
        assert!(live("alsa", &set(&["driver/period", "engine/cpu-dma-latency"])));
        assert!(!live("alsa", &set(&["engine/cpu-dma-latency", "engine/realtime"])));
        assert!(live("firewire", &set(&["engine/log-max-size"])));
        assert!(!live("firewire", &set(&["driver/period"])));
        assert!(!live("alsa", &set(&["driver/period", "driver/rate"])));
        assert!(!live("alsa", &set(&["driver/nperiods"])));
        assert!(!live("alsa", &set(&["engine/realtime-priority"])));
        assert!(!live("alsa", &set(&[])));
        assert!(live("alsa", &set(&["engine/bridges"])));
        assert!(live("dummy", &set(&["engine/bridges", "driver/period"])));
        assert!(!live("alsa", &set(&["engine/bridges", "driver/rate"])));
    }

    #[test]
    fn typed_roundtrip() {
        for s in ["b:true", "i:-1", "u:48000", "y:99", "s:hw:PCH,0"] {
            let v = OwnedValue::try_from(untyped(s).unwrap()).unwrap();
            assert_eq!(typed(&v).as_deref(), Some(s));
        }
        assert!(untyped("x:1").is_none());
    }
}

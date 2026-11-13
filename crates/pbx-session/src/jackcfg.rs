//! A session's JACK settings, through jackdbus (org.jackaudio.service): read the configuration
//! (the driver, and every engine and driver parameter that is set rather than defaulted), and
//! put one back: stop the server, set, start. RaySession does this with shell scripts around
//! jack_control; here it is the D-Bus Configure interface directly.
//!
//! Values are kept as typed strings ("u:48000", "b:true", "s:hw:PCH", "i:-1", "y:99"), so the
//! session file stays plain TOML.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

/// Make jackdbus's configuration `want` and (re)start the server with it. Ok(false): it was
/// already so (the server is left alone, started if it wasn't).
pub fn apply(c: &Connection, want: &JackSettings) -> Result<bool, String> {
    let cur = read(c)?;
    let ctl = control(c)?;
    let started: bool = ctl.call("IsStarted", &()).map_err(err)?;
    if cur.driver == want.driver && cur.params == want.params {
        if !started {
            ctl.call_method("StartServer", &()).map_err(err)?;
        }
        return Ok(false);
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
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_roundtrip() {
        for s in ["b:true", "i:-1", "u:48000", "y:99", "s:hw:PCH,0"] {
            let v = OwnedValue::try_from(untyped(s).unwrap()).unwrap();
            assert_eq!(typed(&v).as_deref(), Some(s));
        }
        assert!(untyped("x:1").is_none());
    }
}

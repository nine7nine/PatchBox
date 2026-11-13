//! PipeWire's own graph, for the drill-down from the `PipeWire` JACK client (PipeWire running on
//! top of jackdbus): its nodes (devices, streams, the JACK bridge), ports and links, drawn by the
//! same canvas as the JACK graph.
//!
//! No libpipewire bindings: `pw-dump --monitor` streams every object as JSON (and sits idle
//! between changes); `pw-link` makes and removes links by port id. Objects become a
//! `pbx_jack::Graph`, so the model, matrices, expanded nodes and undo work unchanged.
//!
//! The same stream feeds the PipeWire settings page (`PwState`): the clock (the `settings`
//! metadata), the default devices (`default`), WirePlumber's settings and their schema
//! (`sm-settings`, `schema-sm-settings`), device profiles, and the JACK bridge if PipeWire runs
//! on top of JACK. Changes go back through `pw-metadata` and `pw-cli`.

use pbx_jack::{Graph, Port, PortKind};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// What the reader thread hands over.
#[derive(Default)]
struct Shared {
    graph: Option<Graph>,
    ids: HashMap<String, u32>,
    state: Option<PwState>,
    error: Option<String>,
    log: Vec<(String, bool)>,
}

/// What the settings page shows of PipeWire.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct PwState {
    pub version: String,
    /// The `settings` metadata: clock.rate, clock.quantum, clock.force-quantum, log.level…
    pub settings: BTreeMap<String, String>,
    /// The `default` metadata (a session manager's), key -> node name: default.audio.sink,
    /// default.configured.audio.sink…; None when there is no such metadata.
    pub defaults: Option<BTreeMap<String, String>>,
    /// WirePlumber's settings, from its schema, with their values (JSON text).
    pub sm: Vec<SmSetting>,
    /// `persistent-sm-settings` exists: WirePlumber keeps what is written there.
    pub sm_persistent: bool,
    /// Audio sinks and sources: (node.name, description).
    pub sinks: Vec<(String, String)>,
    pub sources: Vec<(String, String)>,
    pub devices: Vec<Device>,
    /// PipeWire runs on top of JACK (module-jackdbus-detect or module-jack-tunnel).
    pub bridge: Option<Bridge>,
    /// Our own JACK client is one of PipeWire's (pipewire-jack): PipeWire is the JACK server.
    pub serves_jack: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SmSetting {
    pub key: String,
    pub name: String,
    pub description: String,
    /// "bool", "int", "float", "string".
    pub ty: String,
    /// JSON text.
    pub default: String,
    pub value: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Device {
    pub id: u32,
    pub name: String,
    pub description: String,
    /// (index, name, description, available)
    pub profiles: Vec<(i64, String, String, bool)>,
    pub current: Option<i64>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Bridge {
    /// The module: "jackdbus-detect" or "jack-tunnel".
    pub module: String,
    /// Its nodes: (node.name, description, audio ports, MIDI ports, drives the graph).
    pub nodes: Vec<(String, String, usize, usize, bool)>,
}

pub struct PwLink {
    child: Option<Child>,
    shared: Arc<Mutex<Shared>>,
    waker: pbx_jack::Waker,
    /// Full port name -> PipeWire port id (for pw-link).
    pub ids: HashMap<String, u32>,
}

/// The parts of the objects we use.
#[derive(Default, Clone)]
struct Objects {
    nodes: BTreeMap<u32, Value>,
    ports: BTreeMap<u32, Value>,
    links: BTreeMap<u32, Value>,
    devices: BTreeMap<u32, Value>,
    modules: BTreeMap<u32, Value>,
    clients: BTreeMap<u32, Value>,
    core: Option<Value>,
    /// Metadata objects: id -> (metadata.name, key -> (type, value)), subject 0 only.
    meta: BTreeMap<u32, (String, BTreeMap<String, (String, Value)>)>,
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

impl Objects {
    fn apply(&mut self, batch: &Value) {
        let Some(arr) = batch.as_array() else { return };
        for o in arr {
            let Some(id) = o.get("id").and_then(Value::as_u64).map(|i| i as u32) else { continue };
            // Metadata carries entries, not info: {"props": {"metadata.name"}, "metadata": [{subject,
            // key, type, value}]}; a null value removes the key.
            if let Some(entries) = o.get("metadata").and_then(Value::as_array) {
                let name = o.get("props").and_then(|p| s(p, "metadata.name")).map(str::to_string);
                let m = self.meta.entry(id).or_insert_with(|| (String::new(), BTreeMap::new()));
                if let Some(n) = name {
                    m.0 = n;
                }
                for e in entries {
                    if e.get("subject").and_then(Value::as_u64).unwrap_or(0) != 0 {
                        continue;
                    }
                    let Some(key) = s(e, "key") else { continue };
                    match e.get("value") {
                        None | Some(Value::Null) => {
                            m.1.remove(key);
                        }
                        Some(v) => {
                            m.1.insert(key.to_string(), (s(e, "type").unwrap_or("").to_string(), v.clone()));
                        }
                    }
                }
                continue;
            }
            let info = o.get("info");
            if info.is_none_or(Value::is_null) {
                // Removed (the monitor sends {"id": N, "info": null}).
                self.nodes.remove(&id);
                self.ports.remove(&id);
                self.links.remove(&id);
                self.devices.remove(&id);
                self.modules.remove(&id);
                self.clients.remove(&id);
                self.meta.remove(&id);
                continue;
            }
            let ty = s(o, "type").unwrap_or("");
            let map = match ty.rsplit(':').next() {
                Some("Node") => &mut self.nodes,
                Some("Port") => &mut self.ports,
                Some("Link") => &mut self.links,
                Some("Device") => &mut self.devices,
                Some("Module") => &mut self.modules,
                Some("Client") => &mut self.clients,
                Some("Core") => {
                    self.core = info.cloned();
                    continue;
                }
                _ => continue,
            };
            // Updates carry only what changed: merge the info.
            let entry = map.entry(id).or_insert_with(|| Value::Object(Default::default()));
            if let (Some(dst), Some(src)) = (entry.as_object_mut(), info.and_then(Value::as_object)) {
                for (k, v) in src {
                    // Props and params update key by key (a profile change sends only Profile).
                    if k == "props" || k == "params" {
                        let props = dst.entry(k.as_str()).or_insert_with(|| Value::Object(Default::default()));
                        if let (Some(p), Some(n)) = (props.as_object_mut(), v.as_object()) {
                            for (pk, pv) in n {
                                p.insert(pk.clone(), pv.clone());
                            }
                        }
                    } else {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    fn meta_named(&self, name: &str) -> Option<&BTreeMap<String, (String, Value)>> {
        self.meta.values().find(|(n, _)| n == name).map(|(_, m)| m)
    }

    /// What the settings page needs.
    fn state(&self) -> PwState {
        let text = |v: &Value| match v {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        let mut st = PwState { version: self.core.as_ref().and_then(|c| s(c, "version")).unwrap_or("").to_string(), ..Default::default() };
        if let Some(m) = self.meta_named("settings") {
            st.settings = m.iter().map(|(k, (_, v))| (k.clone(), text(v))).collect();
        }
        // Defaults hold {"name": "<node.name>"} (a JSON string or, in pw-dump, an object).
        st.defaults = self.meta_named("default").map(|m| {
            m.iter()
                .filter_map(|(k, (_, v))| {
                    let v = match v {
                        Value::String(t) => serde_json::from_str::<Value>(t).ok()?,
                        v => v.clone(),
                    };
                    Some((k.clone(), s(&v, "name")?.to_string()))
                })
                .collect()
        });
        let values = self.meta_named("sm-settings");
        if let Some(schema) = self.meta_named("schema-sm-settings") {
            for (key, (_, v)) in schema {
                let v = match v {
                    Value::String(t) => serde_json::from_str::<Value>(t).unwrap_or(Value::Null),
                    v => v.clone(),
                };
                st.sm.push(SmSetting {
                    key: key.clone(),
                    name: s(&v, "name").unwrap_or(key).to_string(),
                    description: s(&v, "description").unwrap_or("").to_string(),
                    ty: s(&v, "type").unwrap_or("string").to_string(),
                    default: v.get("default").map(|d| d.to_string()).unwrap_or_default(),
                    value: values.and_then(|m| m.get(key)).map(|(_, x)| x.to_string()),
                    min: v.get("min").and_then(Value::as_f64),
                    max: v.get("max").and_then(Value::as_f64),
                });
            }
        }
        st.sm_persistent = self.meta_named("persistent-sm-settings").is_some();
        // Each node's ports facing JACK, (audio, MIDI): a sink's inputs, a source's outputs
        // (monitors aside).
        let (mut ins_of, mut outs_of): (HashMap<u64, (usize, usize)>, HashMap<u64, (usize, usize)>) = Default::default();
        for p in self.ports.values() {
            let pp = &p["props"];
            let Some(n) = pp.get("node.id").and_then(Value::as_u64) else { continue };
            let monitor = pp.get("port.monitor").is_some_and(|m| m.as_bool() == Some(true) || m.as_str() == Some("true"));
            let dsp = s(pp, "format.dsp").unwrap_or("");
            let midi = dsp.contains("midi") || dsp.contains("UMP");
            let e = if s(p, "direction") == Some("input") || s(pp, "port.direction") == Some("in") {
                ins_of.entry(n).or_default()
            } else if !monitor {
                outs_of.entry(n).or_default()
            } else {
                continue;
            };
            if midi {
                e.1 += 1;
            } else {
                e.0 += 1;
            }
        }
        let bridge_module = self.modules.values().filter_map(|m| s(m, "name")).find_map(|n| {
            if n.contains("jackdbus-detect") {
                Some("jackdbus-detect")
            } else if n.contains("jack-tunnel") {
                Some("jack-tunnel")
            } else {
                None
            }
        });
        let mut bridge_nodes = Vec::new();
        for (id, n) in &self.nodes {
            let p = &n["props"];
            let name = s(p, "node.name").unwrap_or("").to_string();
            let desc = s(p, "node.description").filter(|d| !d.is_empty()).unwrap_or(&name).to_string();
            let class = s(p, "media.class").unwrap_or("");
            if class == "Audio/Sink" {
                st.sinks.push((name.clone(), desc.clone()));
            } else if class.starts_with("Audio/Source") {
                st.sources.push((name.clone(), desc.clone()));
            }
            if s(p, "node.group") == Some("jack-group") {
                let (a, m) = if class == "Audio/Sink" { ins_of.get(&(*id as u64)) } else { outs_of.get(&(*id as u64)) }.copied().unwrap_or((0, 0));
                let drives = p.get("node.driver").and_then(Value::as_bool).unwrap_or(false);
                bridge_nodes.push((name, desc, a, m, drives));
            }
        }
        if bridge_module.is_some() || !bridge_nodes.is_empty() {
            st.bridge = Some(Bridge { module: bridge_module.unwrap_or("jack-tunnel").to_string(), nodes: bridge_nodes });
        }
        for (id, d) in &self.devices {
            let p = &d["props"];
            let params = &d["params"];
            let profiles = params
                .get("EnumProfile")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| {
                            let avail = s(x, "available").is_none_or(|a| a != "no");
                            Some((x.get("index")?.as_i64()?, s(x, "name").unwrap_or("").to_string(), s(x, "description").unwrap_or("").to_string(), avail))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let current = params.get("Profile").and_then(Value::as_array).and_then(|a| a.first()).and_then(|x| x.get("index")).and_then(Value::as_i64);
            st.devices.push(Device {
                id: *id,
                name: s(p, "device.name").unwrap_or("").to_string(),
                description: s(p, "device.description").or(s(p, "device.nick")).or(s(p, "device.name")).unwrap_or("device").to_string(),
                profiles,
                current,
            });
        }
        st.serves_jack = self.clients.values().any(|c| {
            let p = &c["props"];
            s(p, "client.api") == Some("jack") && s(p, "application.name").is_some_and(|a| a.starts_with("PatchBox"))
        });
        st
    }

    /// The graph, with nodes named stably (node.name, numbered when repeated) and a map from
    /// our port names to PipeWire port ids.
    fn graph(&self) -> (Graph, HashMap<String, u32>) {
        let mut g = Graph::default();
        let mut ids = HashMap::new();
        let mut node_name: HashMap<u32, String> = HashMap::new();
        let mut node_video: HashMap<u32, bool> = HashMap::new();
        let mut used: HashMap<String, usize> = HashMap::new();
        for (id, n) in &self.nodes {
            let p = &n["props"];
            let base = s(p, "node.name").unwrap_or("node").to_string();
            let k = used.entry(base.clone()).or_insert(0);
            *k += 1;
            let name = if *k == 1 { base } else { format!("{base}#{k}") };
            // qpwgraph's naming: description, else nick, else node.name; the application in
            // front when it says something else ("Firefox/…").
            let nonempty = |k: &str| s(p, k).filter(|v| !v.is_empty());
            let base_title = nonempty("node.description").or(nonempty("node.nick")).unwrap_or(&name).to_string();
            let title = match nonempty("application.name") {
                Some(app) if app != base_title => format!("{app}/{base_title}"),
                _ => base_title,
            };
            let class = s(p, "media.class").unwrap_or("");
            node_video.insert(*id, class.contains("Video"));
            g.client_pretty.insert(name.clone(), title);
            if !class.is_empty() {
                g.client_class.insert(name.clone(), class.to_lowercase().replace('/', " "));
            }
            node_name.insert(*id, name);
        }
        let mut by_id: HashMap<u32, usize> = HashMap::new();
        for (id, port) in &self.ports {
            let p = &port["props"];
            let Some(nid) = p.get("node.id").and_then(Value::as_u64).map(|x| x as u32) else { continue };
            let Some(client) = node_name.get(&nid) else { continue };
            let short = s(p, "port.name").unwrap_or("port").to_string();
            let dir_out = s(&port, "direction") == Some("output") || s(p, "port.direction") == Some("out");
            let dsp = s(p, "format.dsp").unwrap_or("");
            // "32 bit raw UMP" (MIDI 2.0) counts as MIDI, as in qpwgraph.
            let kind = if dsp.contains("audio") {
                PortKind::Audio
            } else if dsp.contains("midi") || dsp.contains("UMP") {
                PortKind::Midi
            } else if dsp.contains("video") || node_video.get(&nid).copied().unwrap_or(false) {
                PortKind::Video
            } else {
                PortKind::Other
            };
            let mut flags = if dir_out { pbx_jack::ffi::JACK_PORT_IS_OUTPUT } else { pbx_jack::ffi::JACK_PORT_IS_INPUT };
            if p.get("port.physical").and_then(Value::as_bool).unwrap_or(false) {
                flags |= pbx_jack::ffi::JACK_PORT_IS_PHYSICAL;
            }
            if p.get("port.terminal").and_then(Value::as_bool).unwrap_or(false) {
                flags |= pbx_jack::ffi::JACK_PORT_IS_TERMINAL;
            }
            let mut full = format!("{client}:{short}");
            if g.by_name.contains_key(&full) {
                full = format!("{full}#{id}");
            }
            let pretty = s(p, "port.alias").map(|a| a.rsplit(':').next().unwrap_or(a).to_string()).filter(|a| a != &short);
            g.by_name.insert(full.clone(), g.ports.len());
            by_id.insert(*id, g.ports.len());
            ids.insert(full.clone(), *id);
            g.ports.push(Port {
                name: full,
                client: client.clone(),
                short,
                uuid: *id as u64,
                flags,
                kind,
                type_name: if dsp.is_empty() { kind.label().to_lowercase() } else { dsp.to_string() },
                pretty,
                order: p.get("port.id").and_then(Value::as_i64).map(|x| x as i32),
                group: s(p, "port.group").map(str::to_string),
                aliases: s(p, "port.alias").map(|a| vec![a.to_string()]).unwrap_or_default(),
            });
        }
        for l in self.links.values() {
            let (o, i) = (l.get("output-port-id").and_then(Value::as_u64), l.get("input-port-id").and_then(Value::as_u64));
            if let (Some(o), Some(i)) = (o.and_then(|o| by_id.get(&(o as u32))), i.and_then(|i| by_id.get(&(i as u32)))) {
                g.conns.push((*o, *i));
            }
        }
        (g, ids)
    }
}

impl PwLink {
    pub fn start(waker: pbx_jack::Waker) -> Result<PwLink, String> {
        let mut child = Command::new("pw-dump")
            .args(["--monitor", "--no-colors"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("pw-dump: {e}"))?;
        let out = child.stdout.take().ok_or("pw-dump: no output")?;
        let err = child.stderr.take();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (sh, w) = (shared.clone(), waker.clone());
        std::thread::Builder::new()
            .name("pw-dump".into())
            .spawn(move || {
                let mut objs = Objects::default();
                let stream = serde_json::Deserializer::from_reader(BufReader::new(out)).into_iter::<Value>();
                for batch in stream {
                    let Ok(batch) = batch else { break };
                    objs.apply(&batch);
                    let (g, ids) = objs.graph();
                    let st = objs.state();
                    if let Ok(mut s) = sh.lock() {
                        s.graph = Some(g);
                        s.ids = ids;
                        s.state = Some(st);
                    }
                    w();
                }
                let mut why = String::new();
                if let Some(mut e) = err {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut why);
                }
                if let Ok(mut s) = sh.lock() {
                    s.error = Some(if why.trim().is_empty() { "pw-dump ended".into() } else { why.trim().lines().last().unwrap_or("").to_string() });
                }
                w();
            })
            .map_err(|e| e.to_string())?;
        Ok(PwLink { child: Some(child), shared, waker, ids: HashMap::new() })
    }

    /// A newer graph and state, if they came; an error if pw-dump ended; log lines from the
    /// tools.
    pub fn take(&mut self) -> Taken {
        let Ok(mut s) = self.shared.lock() else { return Taken::default() };
        let graph = s.graph.take();
        if graph.is_some() {
            self.ids = std::mem::take(&mut s.ids);
        }
        Taken { graph, state: s.state.take(), error: s.error.take(), log: std::mem::take(&mut s.log) }
    }

    /// Run a PipeWire tool on a thread; a failure comes back through `take`, success as `ok`.
    fn run(&self, program: &str, args: Vec<String>, ok: Option<String>) {
        let (sh, w) = (self.shared.clone(), self.waker.clone());
        let program = program.to_string();
        std::thread::spawn(move || {
            let r = Command::new(&program).args(&args).stdin(Stdio::null()).output();
            let msg = match r {
                Ok(o) if o.status.success() => ok.map(|m| (m, false)),
                Ok(o) => Some((format!("{program}: {}", String::from_utf8_lossy(&o.stderr).trim()), true)),
                Err(e) => Some((format!("{program}: {e}"), true)),
            };
            if let (Some(m), Ok(mut s)) = (msg, sh.lock()) {
                s.log.push(m);
            }
            w();
        });
    }

    /// Set (or, None, remove) a key of a metadata object (subject 0).
    pub fn set_meta(&self, metadata: &str, key: &str, value: Option<&str>, ty: Option<&str>) {
        let mut a = vec!["-n".to_string(), metadata.to_string()];
        match value {
            Some(v) => {
                a.extend(["0".to_string(), key.to_string(), v.to_string()]);
                if let Some(t) = ty {
                    a.push(t.to_string());
                }
            }
            None => a.extend(["-d".to_string(), "0".to_string(), key.to_string()]),
        }
        self.run("pw-metadata", a, None);
    }

    /// Switch a device to profile `index` (kept by the session manager).
    pub fn set_profile(&self, device: u32, index: i64) {
        self.run("pw-cli", vec!["set-param".into(), device.to_string(), "Profile".into(), format!("{{ index: {index}, save: true }}")], None);
    }
}

/// What `PwLink::take` hands over.
#[derive(Default)]
pub struct Taken {
    pub graph: Option<Graph>,
    pub state: Option<PwState>,
    pub error: Option<String>,
    pub log: Vec<(String, bool)>,
}

impl PwLink {

    /// Link (or unlink) two ports, by our names. pw-link runs on a thread; failures come back
    /// through `take`.
    pub fn link(&self, out: &str, inp: &str, on: bool) {
        let (Some(&o), Some(&i)) = (self.ids.get(out), self.ids.get(inp)) else { return };
        let (sh, w) = (self.shared.clone(), self.waker.clone());
        let what = format!("{out} → {inp}");
        std::thread::spawn(move || {
            let mut cmd = Command::new("pw-link");
            if !on {
                cmd.arg("-d");
            }
            let r = cmd.arg(o.to_string()).arg(i.to_string()).stdin(Stdio::null()).output();
            let msg = match r {
                Ok(o) if o.status.success() => None,
                Ok(o) => Some(format!("pw-link {what}: {}", String::from_utf8_lossy(&o.stderr).trim())),
                Err(e) => Some(format!("pw-link: {e}")),
            };
            if let (Some(m), Ok(mut s)) = (msg, sh.lock()) {
                s.log.push((m, true));
            }
            w();
        });
    }
}

impl Drop for PwLink {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_to_graph() {
        let mut o = Objects::default();
        let batch: Value = serde_json::json!([
            {"id": 44, "type": "PipeWire:Interface:Node", "info": {"props": {"node.name": "jack_sink", "node.description": "JACK Sink", "media.class": "Audio/Sink"}}},
            {"id": 82, "type": "PipeWire:Interface:Node", "info": {"props": {"node.name": "skype", "media.class": "Stream/Output/Audio"}}},
            {"id": 53, "type": "PipeWire:Interface:Port", "info": {"direction": "input", "props": {"node.id": 44, "port.name": "playback_FL", "format.dsp": "32 bit float mono audio"}}},
            {"id": 85, "type": "PipeWire:Interface:Port", "info": {"direction": "output", "props": {"node.id": 82, "port.name": "output_FL", "format.dsp": "32 bit float mono audio"}}},
            {"id": 99, "type": "PipeWire:Interface:Link", "info": {"output-port-id": 85, "input-port-id": 53, "state": "active"}}
        ]);
        o.apply(&batch);
        let (g, ids) = o.graph();
        assert_eq!(g.ports.len(), 2);
        assert_eq!(g.conns.len(), 1);
        assert_eq!(ids["skype:output_FL"], 85);
        assert!(g.port("jack_sink:playback_FL").unwrap().is_input());
        assert_eq!(g.client_pretty["jack_sink"], "JACK Sink");
        assert_eq!(g.client_class["jack_sink"], "audio sink");
        // The link goes away, then the stream.
        o.apply(&serde_json::json!([{"id": 99, "info": null}, {"id": 82, "info": null}, {"id": 85, "info": null}]));
        let (g, _) = o.graph();
        assert_eq!(g.conns.len(), 0);
        assert_eq!(g.ports.len(), 1);
    }
}

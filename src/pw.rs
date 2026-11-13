//! PipeWire's own graph, for the drill-down from the `PipeWire` JACK client (PipeWire running on
//! top of jackdbus): its nodes (devices, streams, the JACK bridge), ports and links, drawn by the
//! same canvas as the JACK graph.
//!
//! No libpipewire bindings: `pw-dump --monitor` streams every object as JSON (and sits idle
//! between changes); `pw-link` makes and removes links by port id. Objects become a
//! `pbx_jack::Graph`, so the model, matrices, expanded nodes and undo work unchanged.

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
    error: Option<String>,
    log: Vec<(String, bool)>,
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
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

impl Objects {
    fn apply(&mut self, batch: &Value) {
        let Some(arr) = batch.as_array() else { return };
        for o in arr {
            let Some(id) = o.get("id").and_then(Value::as_u64).map(|i| i as u32) else { continue };
            let info = o.get("info");
            if info.is_none_or(Value::is_null) {
                // Removed (the monitor sends {"id": N, "info": null}).
                self.nodes.remove(&id);
                self.ports.remove(&id);
                self.links.remove(&id);
                continue;
            }
            let ty = s(o, "type").unwrap_or("");
            let map = match ty.rsplit(':').next() {
                Some("Node") => &mut self.nodes,
                Some("Port") => &mut self.ports,
                Some("Link") => &mut self.links,
                _ => continue,
            };
            // Updates carry only what changed: merge the info.
            let entry = map.entry(id).or_insert_with(|| Value::Object(Default::default()));
            if let (Some(dst), Some(src)) = (entry.as_object_mut(), info.and_then(Value::as_object)) {
                for (k, v) in src {
                    if k == "props" {
                        let props = dst.entry("props").or_insert_with(|| Value::Object(Default::default()));
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
                    if let Ok(mut s) = sh.lock() {
                        s.graph = Some(g);
                        s.ids = ids;
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

    /// A newer graph, if one came; an error if pw-dump ended; log lines from pw-link.
    pub fn take(&mut self) -> (Option<Graph>, Option<String>, Vec<(String, bool)>) {
        let Ok(mut s) = self.shared.lock() else { return (None, None, Vec::new()) };
        let g = s.graph.take();
        if g.is_some() {
            self.ids = std::mem::take(&mut s.ids);
        }
        (g, s.error.take(), std::mem::take(&mut s.log))
    }

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

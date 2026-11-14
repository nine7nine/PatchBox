//! Connection memory: the session's JACK connections, saved with it and restored as ports
//! appear. RaySession's rules:
//!
//! - save: every current connection is kept; a saved one is dropped only when both its ports
//!   exist and it is not connected (the user removed it); one whose port is missing stays;
//! - restore: a saved connection is made when both ports exist, it is absent, and at least one
//!   of its ports is new (appeared since the last look): a connection the user removed during
//!   the session doesn't come back on its own;
//! - once the session is loaded, a connection that appears on a new port, between two ports the
//!   session knew at its last save, and is not saved, is undone (an application auto-connecting
//!   on start, against what the session says).

use crate::store::Connections;
use pbx_jack::Graph;
use std::collections::{BTreeSet, HashSet};
use std::time::Instant;

#[derive(Default)]
pub struct PatchMem {
    pub saved: BTreeSet<(String, String)>,
    pub ports_at_save: HashSet<String>,
    /// Ports seen at the last look.
    known: HashSet<String>,
    /// Treat every present port as new at the next look (session just loaded, JACK reconnected).
    pub all_new: bool,
    pub loaded: bool,
    /// Restore at this time (debounced after port registrations).
    pub due: Option<Instant>,
    /// Saved connections present at the last look.
    pub present: usize,
}

fn names(g: &Graph) -> (HashSet<String>, HashSet<(String, String)>) {
    let ports = g.ports.iter().map(|p| p.name.clone()).collect();
    let conns = g.conns.iter().map(|&(o, i)| (g.ports[o].name.clone(), g.ports[i].name.clone())).collect();
    (ports, conns)
}

impl PatchMem {
    pub fn from_file(c: &Connections) -> PatchMem {
        PatchMem { saved: c.list.iter().cloned().collect(), ports_at_save: c.ports.iter().cloned().collect(), ..Default::default() }
    }

    pub fn to_file(&self) -> Connections {
        let mut ports: Vec<String> = self.ports_at_save.iter().cloned().collect();
        ports.sort();
        Connections { list: self.saved.iter().cloned().collect(), ports }
    }

    pub fn update_on_save(&mut self, g: &Graph) {
        let (ports, conns) = names(g);
        let keep: Vec<(String, String)> = self.saved.iter().filter(|(o, i)| !ports.contains(o) || !ports.contains(i)).cloned().collect();
        self.saved = conns.into_iter().chain(keep).collect();
        self.ports_at_save = ports;
    }

    /// What to connect and disconnect now.
    pub fn plan(&mut self, g: &Graph) -> (Vec<(String, String)>, Vec<(String, String)>) {
        let (ports, conns) = names(g);
        let new: HashSet<&String> = if self.all_new { ports.iter().collect() } else { ports.iter().filter(|p| !self.known.contains(*p)).collect() };
        let mut connect = Vec::new();
        let mut disconnect = Vec::new();
        if self.loaded {
            for (o, i) in &self.saved {
                if ports.contains(o) && ports.contains(i) && !conns.contains(&(o.clone(), i.clone())) && (new.contains(o) || new.contains(i)) {
                    connect.push((o.clone(), i.clone()));
                }
            }
            for (o, i) in &conns {
                let on_new = new.contains(o) || new.contains(i);
                let known_ports = self.ports_at_save.contains(o) && self.ports_at_save.contains(i);
                if on_new && known_ports && !self.saved.contains(&(o.clone(), i.clone())) {
                    disconnect.push((o.clone(), i.clone()));
                }
            }
            self.all_new = false;
            self.known = ports.clone();
        }
        self.present = self.saved.iter().filter(|c| conns.contains(*c)).count() + connect.len();
        (connect, disconnect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbx_jack::{Port, PortKind};

    fn graph(ports: &[&str], conns: &[(&str, &str)]) -> Graph {
        let mut g = Graph::default();
        for (k, p) in ports.iter().enumerate() {
            let (client, short) = p.split_once(':').unwrap();
            g.by_name.insert(p.to_string(), k);
            g.ports.push(Port {
                name: p.to_string(),
                client: client.into(),
                short: short.into(),
                uuid: 0,
                flags: 0,
                kind: PortKind::Audio,
                type_name: String::new(),
                pretty: None,
                order: None,
                group: None,
                aliases: Vec::new(),
            });
        }
        for (o, i) in conns {
            g.conns.push((g.by_name[*o], g.by_name[*i]));
        }
        g
    }

    #[test]
    fn save_keeps_missing_and_drops_removed() {
        let mut m = PatchMem::default();
        m.saved.insert(("a:out".into(), "b:in".into())); // b is gone: kept
        m.saved.insert(("a:out".into(), "c:in".into())); // both present, not connected: removed by the user
        let g = graph(&["a:out", "c:in", "d:in"], &[("a:out", "d:in")]);
        m.update_on_save(&g);
        let v: Vec<_> = m.saved.iter().cloned().collect();
        assert_eq!(v, vec![("a:out".to_string(), "b:in".to_string()), ("a:out".to_string(), "d:in".to_string())]);
        assert!(m.ports_at_save.contains("c:in"));
    }

    #[test]
    fn restore_on_new_ports_only() {
        let mut m = PatchMem::default();
        m.saved.insert(("a:out".into(), "b:in".into()));
        m.ports_at_save = ["a:out", "b:in", "x:in"].iter().map(|s| s.to_string()).collect();
        m.loaded = true;
        m.all_new = true;
        // b not there yet.
        let (c, d) = m.plan(&graph(&["a:out"], &[]));
        assert!(c.is_empty() && d.is_empty());
        // b appears and auto-connects a→x: restore a→b, undo a→x.
        let (c, d) = m.plan(&graph(&["a:out", "b:in", "x:in"], &[("a:out", "x:in")]));
        assert_eq!(c, vec![("a:out".to_string(), "b:in".to_string())]);
        assert_eq!(d, vec![("a:out".to_string(), "x:in".to_string())]);
        // The user disconnects a→b later: nothing new, nothing done.
        let (c, d) = m.plan(&graph(&["a:out", "b:in", "x:in"], &[]));
        assert!(c.is_empty() && d.is_empty());
    }
}

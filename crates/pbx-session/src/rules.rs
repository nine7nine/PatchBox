//! Connection rules (qjackctl's persistent patchbay): pairs of port names, output then input,
//! that patchboxd connects whenever both ports are there, session or not. A name may hold `*`
//! for any run of characters ("PipeWire*:capture_FL"): a client whose name changes, or a port
//! of any number. Kept in ~/.config/patchbox/rules.json.
//!
//! Like the session's connection memory, a rule acts when one of its ports appears (or when the
//! rules change): a connection removed by hand stays removed until then. And a session never
//! undoes a connection a rule asks for.

use pbx_jack::Graph;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rule {
    pub out: String,
    pub inp: String,
}

#[derive(Default)]
pub struct Rules {
    pub list: Vec<Rule>,
    /// Ports seen at the last look; None: treat every port as new at the next one.
    known: Option<HashSet<String>>,
}

/// `*` matches any run of characters, everything else itself.
pub fn glob(pat: &str, name: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || name.len() < first.len() + last.len() || !name.ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

pub fn path() -> Option<PathBuf> {
    let dir = std::env::var_os("PATCHBOX_CONFIG_DIR").map(PathBuf::from).or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("patchbox"))
    })?;
    Some(dir.join("rules.json"))
}

impl Rules {
    pub fn load() -> Rules {
        let list = path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Rules { list, known: None }
    }

    /// Replace the rules (saved), and look at every port again at the next plan.
    pub fn set(&mut self, mut list: Vec<Rule>) -> Result<(), String> {
        list.sort();
        list.dedup();
        list.retain(|r| !r.out.trim().is_empty() && !r.inp.trim().is_empty());
        let p = path().ok_or("no configuration directory")?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = p.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &p)).map_err(|e| e.to_string())?;
        self.list = list;
        self.known = None;
        Ok(())
    }

    /// Does a rule ask for this connection?
    pub fn wants(&self, o: &str, i: &str) -> bool {
        self.list.iter().any(|r| glob(&r.out, o) && glob(&r.inp, i))
    }

    /// The connections to make now: those rules ask for, between an output and an input that
    /// both exist, not connected yet, at least one of them new since the last look.
    pub fn plan(&mut self, g: &Graph) -> Vec<(String, String)> {
        let names: HashSet<String> = g.ports.iter().map(|p| p.name.clone()).collect();
        let is_new = |p: &str| self.known.as_ref().is_none_or(|k| !k.contains(p));
        let conns: HashSet<(usize, usize)> = g.conns.iter().copied().collect();
        let mut out = Vec::new();
        if !self.list.is_empty() {
            for (a, pa) in g.ports.iter().enumerate().filter(|(_, p)| p.is_output()) {
                for r in self.list.iter().filter(|r| glob(&r.out, &pa.name)) {
                    for (b, pb) in g.ports.iter().enumerate().filter(|(_, p)| p.is_input() && glob(&r.inp, &p.name)) {
                        if pa.kind == pb.kind && !conns.contains(&(a, b)) && (is_new(&pa.name) || is_new(&pb.name)) && !out.contains(&(pa.name.clone(), pb.name.clone())) {
                            out.push((pa.name.clone(), pb.name.clone()));
                        }
                    }
                }
            }
        }
        self.known = Some(names);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbx_jack::{Port, PortKind};

    fn graph(ports: &[(&str, bool)], conns: &[(&str, &str)]) -> Graph {
        let mut g = Graph::default();
        for (k, (name, out)) in ports.iter().enumerate() {
            let (client, short) = name.split_once(':').unwrap();
            g.by_name.insert(name.to_string(), k);
            g.ports.push(Port {
                name: name.to_string(),
                client: client.into(),
                short: short.into(),
                uuid: 0,
                flags: if *out { pbx_jack::ffi::JACK_PORT_IS_OUTPUT } else { pbx_jack::ffi::JACK_PORT_IS_INPUT },
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
    fn globs() {
        assert!(glob("a:b", "a:b") && !glob("a:b", "a:bc"));
        assert!(glob("PipeWire*:capture_FL", "PipeWire-01:capture_FL"));
        assert!(glob("*:playback_*", "system:playback_2"));
        assert!(!glob("PipeWire*:capture_FL", "PipeWire:capture_FR"));
        assert!(glob("*", "x:y") && glob("a*b*c", "a-b-c") && !glob("a*b*c", "a-c-b"));
    }

    #[test]
    fn rules_act_on_new_ports_only() {
        let mut r = Rules { list: vec![Rule { out: "PipeWire*:capture_FL".into(), inp: "system:playback_1".into() }], known: None };
        let g = graph(&[("PipeWire:capture_FL", true), ("system:playback_1", false)], &[]);
        assert_eq!(r.plan(&g), vec![("PipeWire:capture_FL".to_string(), "system:playback_1".to_string())]);
        // Removed by hand, nothing new: stays removed.
        assert!(r.plan(&g).is_empty());
        // The client comes back under another name: connected again.
        let g2 = graph(&[("PipeWire-01:capture_FL", true), ("system:playback_1", false)], &[]);
        assert_eq!(r.plan(&g2).len(), 1);
        assert!(r.wants("PipeWire-01:capture_FL", "system:playback_1"));
        assert!(!r.wants("system:playback_1", "PipeWire-01:capture_FL"));
    }
}

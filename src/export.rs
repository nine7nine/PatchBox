//! Our names for blocks and ports exported to JACK's metadata (pretty-name), so other JACK
//! programs (Carla, Ardour, qjackctl…) show them too. RaySession's rules:
//!
//! - one exporter at a time: a lock key on our own client; when another client holds one,
//!   nothing is exported;
//! - a pretty-name set by someone else is never overwritten; ours are remembered, so a name
//!   removed here is removed there, and all of them go when PatchBox quits;
//! - a new port is left alone for 200 ms (programs often name their ports just after making
//!   them).
//!
//! Blocks: a joined client's name goes on the client. A split block (capture / playback) has no
//! subject of its own, so its name stays ours.

use crate::app::{App, Source};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const LOCK: &str = "https://github.com/nine7nine/PatchBox/pretty-name-export";
const YOUNG: Duration = Duration::from_millis(200);
/// The key jack2 1.9.22 defines as JACK_METADATA_PRETTY_NAME (jackaudio.org namespace): what
/// other programs on it read. Our own reading takes the newer jack.audio one too.
const PRETTY: &str = pbx_jack::keys::OLD_PRETTY_NAME;

#[derive(Default)]
pub struct Export {
    /// Subjects (client and port UUIDs) we set a pretty-name on, and to what.
    written: HashMap<u64, String>,
    /// Some(false): another program exports; Some(true): we hold the lock.
    lock: Option<bool>,
    pub due: Option<Instant>,
    /// When each port was first seen.
    seen: HashMap<String, Instant>,
}

impl App {
    /// Export after the graph or the names settle.
    pub fn export_soon(&mut self) {
        self.export.due = Some(Instant::now() + Duration::from_millis(250));
    }

    /// Make JACK's pretty-names match ours. Returns when to look again (young ports).
    pub fn export_names(&mut self) -> Option<Instant> {
        let c = self.jack.as_ref()?;
        if self.source != Source::Jack {
            return None;
        }
        let now = Instant::now();
        // Ports' ages.
        let present: std::collections::HashSet<&str> = self.graph.ports.iter().map(|p| p.name.as_str()).collect();
        self.export.seen.retain(|n, _| present.contains(n.as_str()));
        for p in &self.graph.ports {
            self.export.seen.entry(p.name.clone()).or_insert(now);
        }
        // The lock: someone else exporting?
        let own = c.name().to_string();
        if self.export.lock.is_none() {
            let mut clients: Vec<&str> = self.graph.ports.iter().map(|p| p.client.as_str()).filter(|n| *n != own).collect();
            clients.sort();
            clients.dedup();
            let other = clients.into_iter().find(|n| c.client_uuid(n).is_some_and(|u| c.property(u, LOCK).is_some()));
            match other {
                Some(n) => {
                    self.export.lock = Some(false);
                    let msg = format!("{n} exports names to JACK already: PatchBox doesn't");
                    self.say(msg, false);
                    return None;
                }
                None => {
                    if let Some(u) = c.client_uuid(&own) {
                        c.set_property(u, LOCK, "1");
                    }
                    self.export.lock = Some(true);
                }
            }
        }
        let c = self.jack.as_ref()?;
        let mut want: HashMap<u64, String> = HashMap::new();
        let mut again: Option<Instant> = None;
        if self.export.lock == Some(true) && self.layout.view.export_names {
            for (key, name) in &self.layout.names {
                if let Some(port) = key.strip_prefix("port:") {
                    let Some(&p) = self.graph.by_name.get(port) else { continue };
                    let born = self.export.seen.get(port).copied().unwrap_or(now);
                    if now < born + YOUNG {
                        again = Some(again.map_or(born + YOUNG, |a: Instant| a.min(born + YOUNG)));
                        continue;
                    }
                    want.insert(self.graph.ports[p].uuid, name.clone());
                } else if let Some(client) = key.strip_prefix("M:")
                    && let Some(u) = c.client_uuid(client)
                {
                    want.insert(u, name.clone());
                }
            }
        }
        let key = PRETTY;
        for (&u, name) in &want {
            let cur = c.property(u, key);
            if cur.as_deref() == Some(name.as_str()) {
                self.export.written.insert(u, name.clone());
            } else if cur.is_none() || self.export.written.get(&u) == cur.as_ref() {
                if c.set_property(u, key, name) {
                    self.export.written.insert(u, name.clone());
                }
            }
            // Else someone else named it: theirs stays.
        }
        let gone: Vec<(u64, String)> = self.export.written.iter().filter(|(u, _)| !want.contains_key(u)).map(|(u, v)| (*u, v.clone())).collect();
        for (u, v) in gone {
            if c.property(u, key).as_deref() == Some(v.as_str()) {
                c.remove_property(u, key);
            }
            self.export.written.remove(&u);
        }
        again
    }

    /// Take our names and the lock back out of JACK (quitting).
    pub fn export_withdraw(&mut self) {
        let Some(c) = &self.jack else { return };
        let key = PRETTY;
        for (u, v) in std::mem::take(&mut self.export.written) {
            if c.property(u, key).as_deref() == Some(v.as_str()) {
                c.remove_property(u, key);
            }
        }
        if self.export.lock == Some(true)
            && let Some(u) = c.client_uuid(c.name())
        {
            c.remove_property(u, LOCK);
        }
    }

    /// The server went away: nothing of ours is there any more.
    pub fn export_reset(&mut self) {
        self.export = Export::default();
    }
}

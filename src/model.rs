//! From a JACK snapshot to what the canvas shows: nodes (PatchMatrix's clients, split by role)
//! and, per port type, connection matrices between a source node and a sink node.
//!
//! Splitting: physical ports (and a2j hardware) become a Source node (capture) and a Sink node
//! (playback), PatchMatrix's and RaySession's default; other clients are one Main node with both
//! directions. The user can split or join any client (layout `split`).
//!
//! Naming and stereo pairs follow RaySession's rules (docs/ref-raysession-patchbay.md §1-3).

use crate::layout::Layout;
use pbx_jack::{Graph, PortKind};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Role {
    Main,
    Source,
    Sink,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct NodeKey {
    /// The JACK client name, or `a2j/<device>` for a2j's per-device groups.
    pub group: String,
    pub role: Role,
}

impl NodeKey {
    /// The key used in the layout file.
    pub fn id(&self) -> String {
        let r = match self.role {
            Role::Main => 'M',
            Role::Source => 'S',
            Role::Sink => 'K',
        };
        format!("{r}:{}", self.group)
    }
}

pub struct Node {
    pub key: NodeKey,
    pub title: String,
    /// A small second label: "a2j", "capture", "playback".
    pub sub: Option<String>,
    /// Input ports (graph indices), display order.
    pub ins: Vec<usize>,
    pub outs: Vec<usize>,
    pub physical: bool,
}

impl Node {
    pub fn ins_of(&self, g: &Graph, k: PortKind) -> Vec<usize> {
        self.ins.iter().copied().filter(|&p| g.ports[p].kind == k).collect()
    }
    pub fn outs_of(&self, g: &Graph, k: PortKind) -> Vec<usize> {
        self.outs.iter().copied().filter(|&p| g.ports[p].kind == k).collect()
    }
    pub fn has_kind(&self, g: &Graph, k: PortKind) -> bool {
        self.ins.iter().chain(&self.outs).any(|&p| g.ports[p].kind == k)
    }
}

pub struct Matrix {
    /// Node indices.
    pub src: usize,
    pub dst: usize,
    /// Columns: the source node's output ports of this kind; rows: the sink's input ports.
    pub cols: Vec<usize>,
    pub rows: Vec<usize>,
    pub conns: HashSet<(usize, usize)>,
    /// Layout key ("<src id>><dst id>"), shared by every port type.
    pub key: String,
}

#[derive(Default)]
pub struct Model {
    pub nodes: Vec<Node>,
    pub index: HashMap<NodeKey, usize>,
    /// Graph port index -> node index.
    pub port_node: HashMap<usize, usize>,
    /// Graph port index -> label shown.
    pub labels: Vec<String>,
    /// Graph port index -> its stereo partner (both directions).
    pub partner: HashMap<usize, usize>,
    /// The user's port groups present now: (name, graph port indices in order).
    pub groups: Vec<(String, Vec<usize>)>,
    /// Graph port index -> (its group in `groups`, its place in it).
    pub group_of: HashMap<usize, (usize, usize)>,
}

/// a2j's port names: "<Client> [<id>] (capture|playback): <port>".
pub fn a2j_split(short: &str) -> Option<(&str, &str)> {
    let (head, port) = short.split_once("): ")?;
    let dev = head.rsplit_once(" [").map(|(d, _)| d).or_else(|| head.rsplit_once(" (").map(|(d, _)| d))?;
    Some((dev, port))
}

/// RaySession's graceful port names (default rules).
pub fn graceful_port(client: &str, short: &str) -> String {
    let mut s = short.to_string();
    if client == "a2j"
        && let Some((_, p)) = a2j_split(short) {
            s = p.to_string();
        }
    for pat in ["/audio_in ", "/audio_out ", "/midi_in ", "/midi_out "] {
        if let Some(i) = s.find(pat) {
            s = format!("{} {}", &s[..i], &s[i + pat.len()..]);
        }
    }
    s = s.replace('_', " ");
    for (from, to) in [("-left", " L"), (" left", " L"), ("-right", " R"), (" right", " R")] {
        if let Some(stripped) = s.strip_suffix(from) {
            s = format!("{stripped}{to}");
            break;
        }
    }
    s
}

/// Is `right` the right channel of `left`? (RaySession's stereo detection, plus FL/FR.)
pub fn is_pair(left: &str, right: &str) -> bool {
    // Trailing digits: "x 1"/"x 2", "xR3"/"xL3", "x2" with bare "x".
    let digits = right.len() - right.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 {
        let (base, num) = right.split_at(right.len() - digits);
        if let Ok(n) = num.parse::<u32>() {
            if let Some(b) = base.strip_suffix('R')
                && left == format!("{b}L{num}") {
                    return true;
                }
            if n >= 1 {
                // Callers test consecutive ports and skip both once paired, so (1,2), (3,4)...
                let prev = format!("{:0width$}", n - 1, width = digits);
                if left == format!("{base}{prev}") {
                    return true;
                }
                if n <= 2 {
                    let bare = base.trim_end_matches([' ', '_']);
                    if !bare.is_empty() && left == bare {
                        return true;
                    }
                }
            }
        }
    }
    for (l, r) in [
        ("left", "right"),
        ("Left", "Right"),
        ("(Left)", "(Right)"),
        (".l", ".r"),
        ("_l", "_r"),
        ("_FL", "_FR"),
        ("FL", "FR"),
        (" L", " R"),
        ("-L", "-R"),
        ("_L", "_R"),
        ("L", "R"),
    ] {
        if let Some(b) = right.strip_suffix(r)
            && left.strip_suffix(l) == Some(b) {
                return true;
            }
    }
    for x in ["out", "Out", "output", "Output", "in", "In", "input", "Input", "audio input", "audio output"] {
        for (l, r) in [("L", "R"), ("left", "right"), ("Left", "Right")] {
            if let Some(b) = right.strip_suffix(&format!("{r} {x}"))
                && left.strip_suffix(&format!("{l} {x}")) == Some(b) {
                    return true;
                }
        }
    }
    false
}

/// Pair up a node's ports of one direction (audio only): consecutive ports that match `is_pair`
/// (physical ports pair by position), or share a metadata port-group of two.
fn find_pairs(g: &Graph, ports: &[usize], physical: bool, out: &mut HashMap<usize, usize>) {
    let audio: Vec<usize> = ports.iter().copied().filter(|&p| g.ports[p].kind == PortKind::Audio).collect();
    let mut i = 0;
    while i + 1 < audio.len() {
        let (a, b) = (&g.ports[audio[i]], &g.ports[audio[i + 1]]);
        let grouped = a.group.is_some() && a.group == b.group && audio.get(i + 2).is_none_or(|&c| g.ports[c].group != a.group);
        if physical || grouped || is_pair(&a.short, &b.short) {
            out.insert(audio[i], audio[i + 1]);
            out.insert(audio[i + 1], audio[i]);
            i += 2;
        } else {
            i += 1;
        }
    }
}

/// Matrix layout keys pinned for a kind are stored as "<KIND>|<key>".
pub fn pin_id(kind: PortKind, key: &str) -> String {
    format!("{}|{key}", kind.label())
}

impl Model {
    pub fn build(g: &Graph, layout: &Layout, own_client: &str) -> Model {
        let mut m = Model { labels: vec![String::new(); g.ports.len()], ..Default::default() };
        // Port order: metadata order first (when set), then the server's listing order.
        let mut order: Vec<usize> = (0..g.ports.len()).collect();
        order.sort_by_key(|&i| (g.ports[i].client.clone(), g.ports[i].order.unwrap_or(i32::MAX), i));
        for i in order {
            let p = &g.ports[i];
            if p.client == own_client {
                continue;
            }
            let (group, title, sub, hw) = if p.client == "a2j" {
                match a2j_split(&p.short) {
                    Some((dev, _)) => (format!("a2j/{dev}"), dev.to_string(), Some("a2j".to_string()), true),
                    None => ("a2j".to_string(), "a2j".to_string(), None, true),
                }
            } else {
                let title = g.client_pretty.get(&p.client).cloned().unwrap_or_else(|| p.client.clone());
                (p.client.clone(), title, None, p.is_physical())
            };
            let split = layout.split.get(&group).copied().unwrap_or(hw);
            let role = match (split, p.is_output()) {
                (false, _) => Role::Main,
                (true, true) => Role::Source,
                (true, false) => Role::Sink,
            };
            let key = NodeKey { group, role };
            let ni = match m.index.get(&key) {
                Some(&n) => n,
                None => {
                    let sub = sub.or_else(|| g.client_class.get(&p.client).cloned()).or_else(|| {
                        (hw && role != Role::Main).then(|| if role == Role::Source { "capture" } else { "playback" }.to_string())
                    });
                    let title = layout.names.get(&key.id()).cloned().unwrap_or(title);
                    m.nodes.push(Node { key: key.clone(), title, sub, ins: Vec::new(), outs: Vec::new(), physical: hw });
                    m.index.insert(key, m.nodes.len() - 1);
                    m.nodes.len() - 1
                }
            };
            if p.is_output() {
                m.nodes[ni].outs.push(i);
            } else {
                m.nodes[ni].ins.push(i);
            }
            m.port_node.insert(i, ni);
            // The user's name first, then the application's pretty name, then a graceful one.
            m.labels[i] = layout
                .names
                .get(&format!("port:{}", p.name))
                .cloned()
                .or_else(|| p.pretty.clone())
                .unwrap_or_else(|| graceful_port(&p.client, &p.short));
        }
        for n in &m.nodes {
            find_pairs(g, &n.outs, n.physical, &mut m.partner);
            find_pairs(g, &n.ins, n.physical, &mut m.partner);
        }
        // The user's groups (their ports that exist); one of two is the pair, whatever was
        // guessed.
        for pg in &layout.port_groups {
            let ports: Vec<usize> = pg.ports.iter().filter_map(|n| g.by_name.get(n).copied()).collect();
            if ports.len() < 2 {
                continue;
            }
            for p in &ports {
                if let Some(q) = m.partner.remove(p) {
                    m.partner.remove(&q);
                }
            }
            if let [a, b] = ports[..] {
                m.partner.insert(a, b);
                m.partner.insert(b, a);
            }
            let gi = m.groups.len();
            for (k, &p) in ports.iter().enumerate() {
                m.group_of.insert(p, (gi, k));
            }
            m.groups.push((pg.name.clone(), ports));
        }
        m
    }

    /// The matrices to show for `kind`: every (source node, sink node) pair with a connection of
    /// that kind, plus the pinned ones whose nodes still have ports of that kind.
    pub fn matrices(&self, g: &Graph, kind: PortKind, layout: &Layout) -> Vec<Matrix> {
        let mut pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
        for &(o, i) in &g.conns {
            if g.ports[o].kind != kind {
                continue;
            }
            if let (Some(&a), Some(&b)) = (self.port_node.get(&o), self.port_node.get(&i)) {
                pairs.insert((a, b));
            }
        }
        let prefix = format!("{}|", kind.label());
        for pin in layout.pinned.iter().filter_map(|p| p.strip_prefix(&prefix)) {
            let Some((a, b)) = pin.split_once('>') else { continue };
            if let (Some(a), Some(b)) = (self.node_by_id(a), self.node_by_id(b)) {
                pairs.insert((a, b));
            }
        }
        let mut out = Vec::new();
        for (a, b) in pairs {
            if layout.hidden.contains(&self.nodes[a].key.id()) || layout.hidden.contains(&self.nodes[b].key.id()) {
                continue;
            }
            let cols = self.nodes[a].outs_of(g, kind);
            let rows = self.nodes[b].ins_of(g, kind);
            if cols.is_empty() || rows.is_empty() {
                continue;
            }
            let conns = g
                .conns
                .iter()
                .copied()
                .filter(|&(o, i)| self.port_node.get(&o) == Some(&a) && self.port_node.get(&i) == Some(&b))
                .collect();
            let key = format!("{}>{}", self.nodes[a].key.id(), self.nodes[b].key.id());
            out.push(Matrix { src: a, dst: b, cols, rows, conns, key });
        }
        out
    }

    pub fn node_by_id(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.key.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs() {
        assert!(is_pair("capture_1", "capture_2"));
        assert!(is_pair("ch0", "ch1"));
        assert!(is_pair("Output L", "Output R"));
        assert!(is_pair("out_L", "out_R"));
        assert!(is_pair("front-left", "front-right"));
        assert!(is_pair("output_FL", "output_FR"));
        assert!(is_pair("Master/audio_out 1", "Master/audio_out 2"));
        assert!(is_pair("in", "in2"));
        assert!(is_pair("L out", "R out"));
        assert!(!is_pair("Kick/audio_in 1", "Snare/audio_in 1"));
        assert!(!is_pair("input_1", "output_2"));
    }

    #[test]
    fn names() {
        assert_eq!(graceful_port("ardour", "Master/audio_out 1"), "Master 1");
        assert_eq!(graceful_port("system", "capture_1"), "capture 1");
        assert_eq!(graceful_port("PulseAudio JACK Sink", "front-left"), "front L");
        assert_eq!(
            graceful_port("a2j", "USB Keystation 61es [20] (capture): USB Keystation 61es MIDI 1"),
            "USB Keystation 61es MIDI 1"
        );
        assert_eq!(a2j_split("Midi Through [14] (capture): Midi Through Port-0"), Some(("Midi Through", "Midi Through Port-0")));
    }
}

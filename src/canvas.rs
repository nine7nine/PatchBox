//! The patchbay canvas, PatchMatrix style: client nodes, and between a source and a sink node a
//! matrix of their ports (columns: the source's outputs, rows: the sink's inputs) with a dot
//! where they are connected. Wires run from the source's output handle into the matrix's top and
//! out of its right side into the sink's input handle.
//!
//! Input (PatchMatrix's, made a little more direct):
//! - click a cell: connect/disconnect; drag across cells: paint the same state; with Shift the
//!   stereo partner cell follows (pairs from model.rs);
//! - drag a node: move it (its matrices follow halfway, as in PatchMatrix);
//! - drag from a node's output handle (or Shift+drag the node) onto another node: open a matrix
//!   between them; with Ctrl held at the drop, connect their ports one to one;
//! - Ctrl+drag a matrix: move it; right-click anything: its menu;
//! - drag empty canvas or middle-drag: pan; wheel: scroll; Ctrl+wheel: zoom at the pointer.

use crate::app::{Action, App};
use crate::gfx::{Gfx, Rect, alpha, mix};
use crate::model::Matrix;
use crate::text::Face;
use crate::theme::*;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Default)]
pub enum Drag {
    #[default]
    None,
    Pan {
        last: [f32; 2],
    },
    Node {
        id: String,
        last: [f32; 2],
        /// Where the drag puts the node's centre before snapping (world).
        raw: [f32; 2],
    },
    Matrix {
        key: String,
        last: [f32; 2],
        raw: [f32; 2],
    },
    Link {
        src: String,
    },
    /// Ctrl+drag on empty canvas: blocks touching the rectangle join `base` in the selection.
    Band {
        from: [f32; 2],
        base: std::collections::BTreeSet<String>,
    },
    /// A wire of the ports view picked up by one end (`moving_in`: the input end), from `from`
    /// (window pixels): dropped on another port or block it moves there, on empty canvas it
    /// is disconnected.
    Rewire {
        out: String,
        inp: String,
        moving_in: bool,
        from: [f32; 2],
    },
    /// A matrix view wire pulled off its block (`sink`: the block the matrix feeds): onto
    /// another block, the matrix's connections move there; onto empty canvas, they go.
    MatWire {
        key: String,
        sink: bool,
        from: [f32; 2],
    },
    Paint {
        key: String,
        connect: bool,
        pair: bool,
        done: HashSet<(usize, usize)>,
    },
    /// From one port row of an expanded node (full JACK name) to another port.
    Port {
        name: String,
    },
}

#[derive(Default, Clone, PartialEq, Debug)]
pub enum Hover {
    #[default]
    None,
    Node(usize),
    Out(usize),
    In(usize),
    /// (matrix index, column, row)
    Cell(usize, usize, usize),
    /// (node index, graph port index): a row of an expanded node.
    Port(usize, usize),
    /// A wire of the ports view: (output port, input port).
    Wire(usize, usize),
    /// A wire of the matrix view: (matrix, the one to its sink block).
    MatWire(usize, bool),
}

#[derive(Default)]
pub struct CanvasState {
    /// Where the canvas is drawn, window pixels.
    pub area: Rect,
    pub drag: Drag,
    pub hover: Hover,
    pub mouse: [f32; 2],
    /// A node to flash after "find in canvas".
    pub flash: Option<(String, Instant)>,
    /// Selected blocks (node ids): they move together and can be aligned.
    pub selected: std::collections::BTreeSet<String>,
    /// Where the left button went down (a click on empty canvas clears the selection).
    pub pressed_at: [f32; 2],
    /// A port found by the search, ringed for a moment.
    pub flash_port: Option<(String, Instant)>,
}

/// The find bar (Ctrl+F): what is typed and the result picked.
#[derive(Default)]
pub struct Search {
    pub text: String,
    pub sel: usize,
}

impl App {
    /// Blocks then ports whose names (JACK's, pretty, aliases, ours) hold the text, at most 12:
    /// (what, where, the action that goes there).
    pub fn search_results(&self) -> Vec<(String, String, Action)> {
        let Some(s) = &self.search else { return Vec::new() };
        let q = s.text.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let has = |t: &str| t.to_lowercase().contains(&q);
        let mut out = Vec::new();
        for n in &self.model.nodes {
            if has(&n.title) || has(&n.key.group) || n.sub.as_deref().is_some_and(has) {
                let what = match &n.sub {
                    Some(sub) => format!("block · {sub}"),
                    None => "block".to_string(),
                };
                out.push((n.title.clone(), what, Action::CenterNode(n.key.id())));
            }
        }
        for (pi, p) in self.graph.ports.iter().enumerate() {
            let label = self.model.labels.get(pi).cloned().unwrap_or_default();
            if has(&p.name) || has(&label) || p.aliases.iter().any(|a| has(a)) || p.pretty.as_deref().is_some_and(has) {
                out.push((p.name.clone(), format!("{} {}", p.kind.label(), if p.is_input() { "input" } else { "output" }), Action::FindPort(p.name.clone())));
            }
        }
        out.truncate(12);
        out
    }
}

/// World -> screen transform.
#[derive(Clone, Copy)]
pub struct Xf {
    pub o: [f32; 2],
    pub cam: [f32; 2],
    pub z: f32,
}

impl Xf {
    pub fn s(&self, p: [f32; 2]) -> [f32; 2] {
        [self.o[0] + (p[0] - self.cam[0]) * self.z, self.o[1] + (p[1] - self.cam[1]) * self.z]
    }
    pub fn w(&self, s: [f32; 2]) -> [f32; 2] {
        [(s[0] - self.o[0]) / self.z + self.cam[0], (s[1] - self.o[1]) / self.z + self.cam[1]]
    }
}

impl App {
    pub fn xf(&self) -> Xf {
        Xf { o: [self.canvas.area.x, self.canvas.area.y], cam: self.layout.view.cam, z: self.z() }
    }

    fn node_rect(&self, xf: &Xf, i: usize) -> Rect {
        let c = xf.s(self.node_pos(i));
        let [w, h] = self.node_size(i);
        Rect::centered(c[0], c[1], w * xf.z, h * xf.z)
    }

    /// A node with its port list, window pixels.
    fn node_extent_px(&self, xf: &Xf, i: usize) -> Rect {
        let e = self.node_extent(i);
        let p = xf.s([e.x, e.y]);
        Rect::new(p[0], p[1], e.w * xf.z, e.h * xf.z)
    }

    fn matrix_rect(&self, xf: &Xf, m: &Matrix) -> Rect {
        let c = xf.s(self.matrix_pos(m));
        let s = App::matrix_size(m);
        Rect::centered(c[0], c[1], s[0] * xf.z, s[1] * xf.z)
    }

    /// An expanded node's port list, screen pixels.
    fn body_rect(&self, xf: &Xf, i: usize) -> Option<Rect> {
        let h = self.body_h(i);
        if h <= 0.0 {
            return None;
        }
        let r = self.node_rect(xf, i);
        Some(Rect::new(r.x, r.bottom(), r.w, h * xf.z))
    }

    /// Each port row of a node's body: (graph port index, row rectangle in pixels). Vertical
    /// layout: the square of each port on the block's top (inputs) or bottom (outputs) edge.
    fn port_rows(&self, xf: &Xf, i: usize) -> Vec<(usize, Rect)> {
        if self.vertical() {
            let r = self.node_rect(xf, i);
            let step = crate::app::VPORT * xf.z;
            let q = 2.4 * HANDLE_R * xf.z;
            return self
                .body_layout(i)
                .into_iter()
                .map(|(p, k, side)| {
                    let cx = r.x + 12.0 * xf.z + (k as f32 + 0.5) * step;
                    let cy = if side == 0 { r.y } else { r.bottom() };
                    (p, Rect::centered(cx, cy, step.max(q), q * 1.6))
                })
                .collect();
        }
        let Some(b) = self.body_rect(xf, i) else { return Vec::new() };
        let rh = PORT_ROW * xf.z;
        let top = b.y + BODY_PAD * xf.z;
        let half = b.w / 2.0;
        self.body_layout(i)
            .into_iter()
            .map(|(p, r, side)| {
                let y = top + r as f32 * rh;
                let rect = match side {
                    0 => Rect::new(b.x, y, half, rh),
                    1 => Rect::new(b.x + half, y, half, rh),
                    _ => Rect::new(b.x, y, b.w, rh),
                };
                (p, rect)
            })
            .collect()
    }

    /// Where each port's wire attaches (ports view): the edge of its row.
    fn anchors(&self, xf: &Xf) -> HashMap<usize, [f32; 2]> {
        let mut m = HashMap::new();
        for i in 0..self.model.nodes.len() {
            if !self.visible(i) {
                continue;
            }
            for (p, r) in self.port_rows(xf, i) {
                m.insert(p, self.anchor_of(xf, p, r));
            }
        }
        m
    }

    /// Where port `p`'s wire leaves its row (or, vertical, its square) `r`.
    fn anchor_of(&self, xf: &Xf, p: usize, r: Rect) -> [f32; 2] {
        let q = 1.2 * HANDLE_R * xf.z;
        match (self.vertical(), self.graph.ports[p].is_input()) {
            (true, true) => [r.cx(), r.cy() - q],
            (true, false) => [r.cx(), r.cy() + q],
            (false, true) => [r.x, r.cy()],
            (false, false) => [r.right(), r.cy()],
        }
    }

    /// One port's anchor (ports view), if its block shows it.
    fn port_anchor(&self, xf: &Xf, p: usize) -> Option<[f32; 2]> {
        let &ni = self.model.port_node.get(&p)?;
        if !self.visible(ni) {
            return None;
        }
        let (_, r) = self.port_rows(xf, ni).into_iter().find(|(q, _)| *q == p)?;
        Some(self.anchor_of(xf, p, r))
    }

    /// A curve from `a` to `b` where `a` is an output's end: for a wire whose output end is
    /// `fixed` (or, `out_fixed` false, whose input end is) and whose other end is at `free`.
    fn loose_curve(&self, fixed: [f32; 2], free: [f32; 2], out_fixed: bool, z: f32) -> [[f32; 2]; 4] {
        if out_fixed { self.curve(fixed, free, z) } else { self.curve(free, fixed, z) }
    }

    /// The matrix view's two wires of matrix `mi`, window pixels: source block to the matrix's
    /// centre, centre to sink block. They leave a block level and head straight for the
    /// centre (the matrix is drawn over the end), so they look right wherever the matrix is.
    /// `loose`: the end at a block of one of them follows the pointer instead (pulled off).
    fn mat_curves(&self, xf: &Xf, mi: usize, loose: Option<(bool, [f32; 2])>) -> [[[f32; 2]; 4]; 2] {
        let m = &self.mats[mi];
        let c = xf.s(self.matrix_pos(m));
        let bend = WIRE_BEND * xf.z;
        let (out, _) = self.handles(m.src);
        let (_, inp) = self.handles(m.dst);
        let mut l0 = xf.s(out);
        let mut l1 = xf.s(inp);
        match loose {
            Some((false, p)) => l0 = p,
            Some((true, p)) => l1 = p,
            None => {}
        }
        let b0 = ((c[0] - l0[0]).abs() * 0.5).clamp(bend * 0.4, bend * 2.5);
        let b1 = ((l1[0] - c[0]).abs() * 0.5).clamp(bend * 0.4, bend * 2.5);
        [[l0, [l0[0] + b0, l0[1]], c, c], [c, c, [l1[0] - b1, l1[1]], l1]]
    }

    /// The matrix view wire under (x, y), if any (outside the matrix and the blocks).
    fn mat_wire_at(&self, xf: &Xf, x: f32, y: f32) -> Option<(usize, bool)> {
        let tol = 5.0 * self.scale;
        let mut best: Option<(f32, (usize, bool))> = None;
        for mi in 0..self.mats.len() {
            let mr = self.matrix_rect(xf, &self.mats[mi]);
            for (k, cv) in self.mat_curves(xf, mi, None).into_iter().enumerate() {
                if let Some((d, _)) = curve_dist(cv, [x, y])
                    && d < tol
                    && !mr.contains(x, y)
                    && best.is_none_or(|(bd, _)| d < bd)
                {
                    best = Some((d, (mi, k == 1)));
                }
            }
        }
        best.map(|(_, w)| w)
    }

    /// A wire's curve (Luluda's: the bend grows with the distance, backwards wires loop round).
    fn curve(&self, a: [f32; 2], b: [f32; 2], z: f32) -> [[f32; 2]; 4] {
        if !self.vertical() {
            return Self::wire_curve(a, b, z);
        }
        let dy = b[1] - a[1];
        let bend = if dy > 0.0 { (dy * 0.45).max(30.0 * z) } else { (dy.abs() * 0.3 + (b[0] - a[0]).abs() * 0.25).clamp(60.0 * z, 260.0 * z) };
        [a, [a[0], a[1] + bend], [b[0], b[1] - bend], b]
    }

    fn wire_curve(a: [f32; 2], b: [f32; 2], z: f32) -> [[f32; 2]; 4] {
        let dx = b[0] - a[0];
        let bend = if dx > 0.0 { (dx * 0.45).max(30.0 * z) } else { (dx.abs() * 0.3 + (b[1] - a[1]).abs() * 0.25).clamp(60.0 * z, 260.0 * z) };
        [a, [a[0] + bend, a[1]], [b[0] - bend, b[1]], b]
    }

    /// The wire under (x, y), if any (ports view), and how far along it (0 at the output).
    fn wire_at(&self, xf: &Xf, x: f32, y: f32) -> Option<(usize, usize, f32)> {
        let anchors = self.anchors(xf);
        let tol = 5.0 * self.scale;
        let mut best: Option<(f32, (usize, usize, f32))> = None;
        for &(o, i) in &self.graph.conns {
            if self.graph.ports[o].kind != self.kind {
                continue;
            }
            let (Some(&a), Some(&b)) = (anchors.get(&o), anchors.get(&i)) else { continue };
            if let Some((d, t)) = curve_dist(self.curve(a, b, xf.z), [x, y])
                && d < tol
                && best.is_none_or(|(bd, _)| d < bd)
            {
                best = Some((d, (o, i, t)));
            }
        }
        best.map(|(_, w)| w)
    }

    /// A centre, snapped so the top-left corner (`centre - half`) sits on the grid, when snapping.
    fn snapped(&self, c: [f32; 2], half: [f32; 2]) -> [f32; 2] {
        if !self.layout.view.snap {
            return c;
        }
        let g = GRID;
        [((c[0] - half[0]) / g).round() * g + half[0], ((c[1] - half[1]) / g).round() * g + half[1]]
    }

    /// Recompute what is under the pointer (after the layout or the graph changed).
    pub fn refresh_hover(&mut self) {
        if !matches!(self.canvas.drag, Drag::None) {
            return;
        }
        let [x, y] = self.ui.mouse;
        self.canvas.hover = if self.canvas.area.contains(x, y) && self.ui.menu.is_none() { self.hit(x, y) } else { Hover::None };
    }

    /// A point `t` (0..1) along the ports view wire `out` → `inp` (tests).
    pub fn wire_point(&self, out: &str, inp: &str, t: f32) -> Option<[f32; 2]> {
        let xf = self.xf();
        let (&o, &i) = (self.graph.by_name.get(out)?, self.graph.by_name.get(inp)?);
        let (a, b) = (self.port_anchor(&xf, o)?, self.port_anchor(&xf, i)?);
        Some(bezier_at(self.curve(a, b, xf.z), t))
    }

    /// A point `t` along a matrix view wire (tests): the matrix whose key contains `key`.
    pub fn mat_wire_point(&self, key: &str, sink: bool, t: f32) -> Option<[f32; 2]> {
        let xf = self.xf();
        let mi = self.mats.iter().position(|m| m.key.contains(key))?;
        Some(bezier_at(self.mat_curves(&xf, mi, None)[sink as usize], t))
    }

    /// Centre of a port's row in its expanded node (tests).
    pub fn port_point(&self, name: &str) -> Option<[f32; 2]> {
        let &p = self.graph.by_name.get(name)?;
        let &ni = self.model.port_node.get(&p)?;
        let xf = self.xf();
        self.port_rows(&xf, ni).into_iter().find(|(q, _)| *q == p).map(|(_, r)| [r.cx(), r.cy()])
    }

    /// Can a drag from port `a` end on port `b`? (opposite directions, same type)
    fn compatible(&self, a: usize, b: usize) -> bool {
        let (pa, pb) = (&self.graph.ports[a], &self.graph.ports[b]);
        a != b && pa.kind == pb.kind && pa.type_name == pb.type_name && pa.is_output() != pb.is_output()
    }

    fn visible(&self, i: usize) -> bool {
        !self.layout.hidden.contains(&self.model.nodes[i].key.id())
    }

    fn hit(&self, x: f32, y: f32) -> Hover {
        let xf = self.xf();
        let matrices = self.layout.view.matrices;
        for (mi, m) in self.mats.iter().enumerate().rev().filter(|_| matrices) {
            let r = self.matrix_rect(&xf, m);
            if r.contains(x, y) {
                let cell = CELL * xf.z;
                let c = (((x - r.x) / cell) as usize).min(m.cols.len() - 1);
                let row = (((y - r.y) / cell) as usize).min(m.rows.len() - 1);
                return Hover::Cell(mi, c, row);
            }
        }
        let hr = HANDLE_R * xf.z * 2.5;
        for i in (0..self.model.nodes.len()).rev() {
            if !self.visible(i) {
                continue;
            }
            let n = &self.model.nodes[i];
            let (out, inp) = self.handles(i);
            let (so, si) = (xf.s(out), xf.s(inp));
            if matrices && !n.outs_of(&self.graph, self.kind).is_empty() && crate::gfx::dist(so, [x, y]) <= hr {
                return Hover::Out(i);
            }
            if matrices && !n.ins_of(&self.graph, self.kind).is_empty() && crate::gfx::dist(si, [x, y]) <= hr {
                return Hover::In(i);
            }
            if self.vertical()
                && let Some((p, _)) = self.port_rows(&xf, i).into_iter().find(|(_, r)| r.contains(x, y))
            {
                return Hover::Port(i, p);
            }
            if self.node_rect(&xf, i).contains(x, y) {
                return Hover::Node(i);
            }
            if let Some(b) = self.body_rect(&xf, i)
                && b.contains(x, y)
            {
                return match self.port_rows(&xf, i).into_iter().find(|(_, r)| r.contains(x, y)) {
                    Some((p, _)) => Hover::Port(i, p),
                    None => Hover::Node(i),
                };
            }
        }
        if matrices {
            if let Some((mi, sink)) = self.mat_wire_at(&xf, x, y) {
                return Hover::MatWire(mi, sink);
            }
        } else if let Some((o, i, _)) = self.wire_at(&xf, x, y) {
            return Hover::Wire(o, i);
        }
        Hover::None
    }

    // ---- input ----

    pub fn canvas_down(&mut self, button: sdl3::mouse::MouseButton, x: f32, y: f32, ctrl: bool, shift: bool, clicks: u8) {
        // A gesture is one undo step: it starts here and ends with the button (canvas_up).
        if button == sdl3::mouse::MouseButton::Left {
            self.history.begin(&self.layout);
            self.canvas_press(button, x, y, ctrl, shift, clicks);
            if matches!(self.canvas.drag, Drag::None) {
                self.history.end(&self.layout);
            }
        } else {
            self.canvas_press(button, x, y, ctrl, shift, clicks);
        }
    }

    /// Escape during a drag: it ends where it is.
    pub fn cancel_drag(&mut self) {
        if !matches!(std::mem::take(&mut self.canvas.drag), Drag::None) {
            self.history.end(&self.layout);
        }
    }

    fn canvas_press(&mut self, button: sdl3::mouse::MouseButton, x: f32, y: f32, ctrl: bool, shift: bool, clicks: u8) {
        use sdl3::mouse::MouseButton as B;
        self.canvas.mouse = [x, y];
        let h = self.hit(x, y);
        self.redraw = true;
        if button == sdl3::mouse::MouseButton::Left {
            self.canvas.pressed_at = [x, y];
        }
        // Selection: Ctrl+click toggles a block, Ctrl+drag on empty canvas draws a band.
        if button == sdl3::mouse::MouseButton::Left && ctrl && clicks < 2 {
            match h {
                Hover::Node(i) | Hover::In(i) | Hover::Out(i) => {
                    let id = self.model.nodes[i].key.id();
                    if !self.canvas.selected.remove(&id) {
                        self.canvas.selected.insert(id);
                    }
                    return;
                }
                Hover::None | Hover::Wire(..) | Hover::MatWire(..) => {
                    self.canvas.drag = Drag::Band { from: [x, y], base: self.canvas.selected.clone() };
                    return;
                }
                _ => {}
            }
        }
        match button {
            B::Middle => self.canvas.drag = Drag::Pan { last: [x, y] },
            B::Right => self.canvas_menu(h, x, y),
            B::Left => match h {
                Hover::Cell(mi, c, r) => {
                    let key = self.mats[mi].key.clone();
                    if ctrl {
                        let raw = self.matrix_pos(&self.mats[mi]);
                        self.canvas.drag = Drag::Matrix { key, last: [x, y], raw };
                    } else {
                        let (o, i) = (self.mats[mi].cols[c], self.mats[mi].rows[r]);
                        let connect = !self.mats[mi].conns.contains(&(o, i));
                        self.pin(self.mats[mi].src, self.mats[mi].dst);
                        let mut done = HashSet::new();
                        self.paint_cell(mi, c, r, connect, shift, &mut done);
                        self.canvas.drag = Drag::Paint { key, connect, pair: shift, done };
                    }
                }
                Hover::Port(_, p) => self.canvas.drag = Drag::Port { name: self.graph.ports[p].name.clone() },
                Hover::Node(i) if clicks >= 2 && self.source == crate::app::Source::Jack && is_pipewire(&self.model.nodes[i].key.group) => {
                    self.apply(Action::OpenPipeWire);
                }
                Hover::Node(i) if clicks >= 2 => {
                    let id = self.model.nodes[i].key.id();
                    let on = !self.is_expanded(i);
                    self.apply(Action::Expand(id, on));
                }
                Hover::Out(i) => self.canvas.drag = Drag::Link { src: self.model.nodes[i].key.id() },
                Hover::Node(i) if shift && !self.model.nodes[i].outs_of(&self.graph, self.kind).is_empty() => {
                    self.canvas.drag = Drag::Link { src: self.model.nodes[i].key.id() }
                }
                Hover::Node(i) | Hover::In(i) => {
                    let id = self.model.nodes[i].key.id();
                    // A block outside the selection moves alone (and the selection goes).
                    if !self.canvas.selected.contains(&id) {
                        self.canvas.selected.clear();
                    }
                    self.canvas.drag = Drag::Node { id, last: [x, y], raw: self.node_pos(i) }
                }
                Hover::Wire(..) => {
                    // Picked up by its nearer end.
                    let xf = self.xf();
                    self.canvas.drag = match self.wire_at(&xf, x, y) {
                        Some((o, i, t)) => Drag::Rewire {
                            out: self.graph.ports[o].name.clone(),
                            inp: self.graph.ports[i].name.clone(),
                            moving_in: t >= 0.5,
                            from: [x, y],
                        },
                        None => Drag::Pan { last: [x, y] },
                    };
                }
                Hover::MatWire(mi, sink) => self.canvas.drag = Drag::MatWire { key: self.mats[mi].key.clone(), sink, from: [x, y] },
                Hover::None => self.canvas.drag = Drag::Pan { last: [x, y] },
            },
            _ => {}
        }
    }

    /// Set one cell (and with `pair` its stereo partner) to `connect`, once per drag.
    fn paint_cell(&mut self, mi: usize, c: usize, r: usize, connect: bool, pair: bool, done: &mut HashSet<(usize, usize)>) {
        let m = &self.mats[mi];
        let (o, i) = (m.cols[c], m.rows[r]);
        let mut cells = vec![(o, i)];
        // Shift: both ports in groups of the user's: the whole group, in step from this cell;
        // else the stereo partners.
        if pair
            && let (Some(&(go, ko)), Some(&(gi, ki))) = (self.model.group_of.get(&o), self.model.group_of.get(&i))
        {
            let (outs, ins) = (&self.model.groups[go].1, &self.model.groups[gi].1);
            for (a, &po) in outs.iter().enumerate() {
                let b = a as isize - ko as isize + ki as isize;
                if let Some(&pi) = usize::try_from(b).ok().and_then(|b| ins.get(b))
                    && m.cols.contains(&po)
                    && m.rows.contains(&pi)
                {
                    cells.push((po, pi));
                }
            }
        } else if pair
            && let (Some(&po), Some(&pi)) = (self.model.partner.get(&o), self.model.partner.get(&i))
                && m.cols.contains(&po) && m.rows.contains(&pi) {
                    cells.push((po, pi));
                }
        for (o, i) in cells {
            if !done.insert((o, i)) {
                continue;
            }
            let is = self.mats[mi].conns.contains(&(o, i));
            if connect && !is {
                self.connect(o, i);
            } else if !connect && is {
                self.disconnect(o, i);
            }
        }
    }

    pub fn canvas_motion(&mut self, x: f32, y: f32) {
        let last_mouse = self.canvas.mouse;
        self.canvas.mouse = [x, y];
        let z = self.z();
        let mut drag = std::mem::take(&mut self.canvas.drag);
        match &mut drag {
            Drag::None => {
                let h = self.hit(x, y);
                if h != self.canvas.hover {
                    self.canvas.hover = h;
                    self.redraw = true;
                }
            }
            Drag::Pan { last } => {
                self.layout.view.cam[0] -= (x - last[0]) / z;
                self.layout.view.cam[1] -= (y - last[1]) / z;
                *last = [x, y];
                self.layout_changed();
            }
            Drag::Node { id, last, raw } => {
                raw[0] += (x - last[0]) / z;
                raw[1] += (y - last[1]) / z;
                *last = [x, y];
                let half = [NODE_W / 2.0, NODE_H / 2.0];
                let target = self.snapped(*raw, half);
                let cur = self.layout.nodes.get(id.as_str()).copied().unwrap_or(target);
                let (dx, dy) = (target[0] - cur[0], target[1] - cur[1]);
                // Matrices follow halfway (PatchMatrix): they stay between their two nodes. The
                // rest of the selection moves with it.
                self.move_node(id, dx, dy);
                if self.canvas.selected.contains(id.as_str()) && (dx != 0.0 || dy != 0.0) {
                    let others: Vec<String> = self.canvas.selected.iter().filter(|s| *s != id).cloned().collect();
                    for o in others {
                        self.move_node(&o, dx, dy);
                    }
                }
            }
            Drag::Band { from, base } => {
                let band = Rect::new(from[0].min(x), from[1].min(y), (x - from[0]).abs(), (y - from[1]).abs());
                let xf = self.xf();
                let mut sel = base.clone();
                for i in 0..self.model.nodes.len() {
                    if self.visible(i) && self.node_extent_px(&xf, i).overlaps(&band) {
                        sel.insert(self.model.nodes[i].key.id());
                    }
                }
                self.canvas.selected = sel;
                self.redraw = true;
            }
            Drag::Matrix { key, last, raw } => {
                raw[0] += (x - last[0]) / z;
                raw[1] += (y - last[1]) / z;
                *last = [x, y];
                let size = self.mats.iter().find(|m| &m.key == key).map(App::matrix_size).unwrap_or([0.0, 0.0]);
                let target = self.snapped(*raw, [size[0] / 2.0, size[1] / 2.0]);
                if let Some(p) = self.layout.matrices.get_mut(key.as_str()) {
                    *p = target;
                }
                self.layout_changed();
            }
            Drag::Link { .. } | Drag::Port { .. } | Drag::Rewire { .. } | Drag::MatWire { .. } => {
                self.canvas.hover = self.hit(x, y);
                self.redraw = true;
            }
            Drag::Paint { key, connect, pair, done } => {
                if let Hover::Cell(mi, c, r) = self.hit(x, y)
                    && &self.mats[mi].key == key {
                        // Visit every cell under the segment since the last event (fast drags).
                        let steps = (crate::gfx::dist(last_mouse, [x, y]) / (CELL * z * 0.5)).ceil().max(1.0) as usize;
                        for s in 1..=steps {
                            let t = s as f32 / steps as f32;
                            let p = [last_mouse[0] + (x - last_mouse[0]) * t, last_mouse[1] + (y - last_mouse[1]) * t];
                            if let Hover::Cell(mj, c2, r2) = self.hit(p[0], p[1])
                                && mj == mi {
                                    self.paint_cell(mi, c2, r2, *connect, *pair, done);
                                }
                        }
                        self.paint_cell(mi, c, r, *connect, *pair, done);
                    }
                self.canvas.hover = self.hit(x, y);
                self.redraw = true;
            }
        }
        self.canvas.drag = drag;
    }

    pub fn canvas_up(&mut self, x: f32, y: f32, ctrl: bool) {
        let drag = std::mem::take(&mut self.canvas.drag);
        let gesture = !matches!(drag, Drag::None | Drag::Pan { .. }) || matches!(drag, Drag::Pan { .. });
        self.canvas_release(drag, x, y, ctrl);
        if gesture {
            self.history.end(&self.layout);
        }
    }

    fn canvas_release(&mut self, drag: Drag, x: f32, y: f32, ctrl: bool) {
        if matches!(drag, Drag::Pan { .. }) && crate::gfx::dist(self.canvas.pressed_at, [x, y]) < 4.0 * self.scale {
            self.canvas.selected.clear();
        }
        if let Drag::Node { id, .. } = &drag
            && let Some(i) = self.model.node_by_id(id)
        {
            self.repulse(i);
        }
        if let Drag::Port { name } = &drag
            && let Some(&a) = self.graph.by_name.get(name)
        {
            self.drop_port(a, self.hit(x, y), ctrl);
        }
        if let Drag::Rewire { out, inp, moving_in, from } = &drag
            && let (Some(&o), Some(&i)) = (self.graph.by_name.get(out), self.graph.by_name.get(inp))
        {
            let moved = crate::gfx::dist(*from, [x, y]) > 6.0 * self.scale;
            self.drop_wire(o, i, *moving_in, moved, self.hit(x, y));
        }
        if let Drag::MatWire { key, sink, from } = &drag {
            let moved = crate::gfx::dist(*from, [x, y]) > 6.0 * self.scale;
            self.drop_mat_wire(key, *sink, moved, self.hit(x, y));
        }
        if let Drag::Link { src } = drag {
            let target = match self.hit(x, y) {
                Hover::Node(i) | Hover::In(i) | Hover::Out(i) => Some(i),
                _ => None,
            };
            if let (Some(s), Some(t)) = (self.model.node_by_id(&src), target) {
                let outs = self.model.nodes[s].outs_of(&self.graph, self.kind);
                let ins = self.model.nodes[t].ins_of(&self.graph, self.kind);
                if !outs.is_empty() && !ins.is_empty() {
                    self.pin(s, t);
                    if ctrl {
                        for (o, i) in outs.into_iter().zip(ins) {
                            self.connect(o, i);
                        }
                    }
                    self.refresh_matrices();
                }
            }
        }
        self.canvas.hover = self.hit(x, y);
        self.redraw = true;
    }

    /// A wire picked up by one end was dropped: on a port that fits, or a block with one free,
    /// the connection moves there; on its own port or block, or a port that doesn't fit, it
    /// stays; anywhere else it is disconnected. A click without a drag changes nothing.
    fn drop_wire(&mut self, o: usize, i: usize, moving_in: bool, moved: bool, target: Hover) {
        let (fixed, old) = if moving_in { (o, i) } else { (i, o) };
        let old_node = self.model.port_node.get(&old).copied();
        let pair = |b: usize| if moving_in { (o, b) } else { (b, i) };
        let to = match target {
            _ if !moved => return,
            Hover::Port(_, b) if b == old => return,
            Hover::Port(_, b) if self.compatible(fixed, b) => b,
            Hover::Port(..) => return,
            Hover::Node(t) | Hover::In(t) | Hover::Out(t) => {
                if Some(t) == old_node {
                    return;
                }
                let n = &self.model.nodes[t];
                let cands = if moving_in { &n.ins } else { &n.outs };
                let free = cands.iter().copied().find(|&b| {
                    let (po, pi) = pair(b);
                    self.compatible(fixed, b) && !self.graph.connected(po, pi)
                });
                match free {
                    Some(b) => b,
                    None => return,
                }
            }
            _ => {
                self.disconnect(o, i);
                return;
            }
        };
        let (no, ni) = pair(to);
        self.disconnect(o, i);
        if !self.graph.connected(no, ni) {
            self.connect(no, ni);
        }
    }

    /// A matrix view wire pulled off its block was dropped: onto another block with ports of
    /// this type, the matrix's connections move to it (port for port, in order) and the matrix
    /// with them; onto its own block, nothing; anywhere else, they are disconnected and the
    /// matrix goes.
    fn drop_mat_wire(&mut self, key: &str, sink: bool, moved: bool, target: Hover) {
        let Some(m) = self.mats.iter().find(|m| m.key == key) else { return };
        if !moved {
            return;
        }
        let (src, dst) = (m.src, m.dst);
        let old = if sink { dst } else { src };
        let conns: Vec<(usize, usize)> = m.conns.iter().copied().collect();
        let to = match target {
            Hover::Node(t) | Hover::In(t) | Hover::Out(t) | Hover::Port(t, _) => Some(t),
            _ => None,
        };
        match to {
            Some(t) if t == old => {}
            Some(t) => {
                let ports = |n: usize| if sink { self.model.nodes[n].ins_of(&self.graph, self.kind) } else { self.model.nodes[n].outs_of(&self.graph, self.kind) };
                let (from_ports, to_ports) = (ports(old), ports(t));
                if to_ports.is_empty() || t == if sink { src } else { dst } {
                    return;
                }
                for (o, i) in conns {
                    let moving = if sink { i } else { o };
                    let Some(k) = from_ports.iter().position(|&p| p == moving) else { continue };
                    self.disconnect(o, i);
                    if let Some(&b) = to_ports.get(k) {
                        let (no, ni) = if sink { (o, b) } else { (b, i) };
                        if !self.graph.connected(no, ni) {
                            self.connect(no, ni);
                        }
                    }
                }
                self.apply(Action::Unpin(key.to_string()));
                if sink { self.pin(src, t) } else { self.pin(t, dst) };
                self.refresh_matrices();
            }
            None => {
                for (o, i) in conns {
                    self.disconnect(o, i);
                }
                self.apply(Action::Unpin(key.to_string()));
            }
        }
    }

    /// A port dragged from an expanded node was dropped: onto a port, toggle that connection;
    /// onto a node, connect to its first compatible port not yet connected to this one.
    /// `n` ports from `b` on, in its block's order, of its direction and type.
    fn following(&self, b: usize, n: usize) -> Vec<usize> {
        let Some(&ni) = self.model.port_node.get(&b) else { return vec![b] };
        let node = &self.model.nodes[ni];
        let pb = &self.graph.ports[b];
        let list: Vec<usize> = (if pb.is_input() { &node.ins } else { &node.outs }).iter().copied().filter(|&q| self.graph.ports[q].kind == pb.kind).collect();
        let at = list.iter().position(|&q| q == b).unwrap_or(0);
        list.into_iter().skip(at).take(n).collect()
    }

    /// A port of the user's group dropped (no Ctrl): the whole group connects 1:1, in order, to
    /// the group (or the ports from) the port it lands on, or a block's first ports; all
    /// connected already: they are disconnected. False: not a grouped port.
    fn drop_group(&mut self, a: usize, target: &Hover) -> bool {
        let Some(&(gi, _)) = self.model.group_of.get(&a) else { return false };
        let src = self.model.groups[gi].1.clone();
        let dst: Vec<usize> = match *target {
            Hover::Port(_, b) if self.compatible(a, b) => match self.model.group_of.get(&b) {
                Some(&(gj, _)) => self.model.groups[gj].1.clone(),
                None => self.following(b, src.len()),
            },
            Hover::Node(t) | Hover::In(t) | Hover::Out(t) => {
                let n = &self.model.nodes[t];
                let cands = if self.graph.ports[a].is_output() { &n.ins } else { &n.outs };
                cands.iter().copied().filter(|&b| self.compatible(a, b)).take(src.len()).collect()
            }
            _ => return true,
        };
        let pairs: Vec<(usize, usize)> = src
            .iter()
            .zip(&dst)
            .map(|(&s, &d)| if self.graph.ports[s].is_output() { (s, d) } else { (d, s) })
            .filter(|&(o, i)| self.compatible(o, i))
            .collect();
        let all = !pairs.is_empty() && pairs.iter().all(|&(o, i)| self.graph.connected(o, i));
        for (o, i) in pairs {
            if all {
                self.disconnect(o, i);
            } else if !self.graph.connected(o, i) {
                self.connect(o, i);
            }
        }
        true
    }

    fn drop_port(&mut self, a: usize, target: Hover, ctrl: bool) {
        if !ctrl && self.drop_group(a, &target) {
            return;
        }
        let b = match target {
            Hover::Port(_, b) if self.compatible(a, b) => Some(b),
            Hover::Node(t) | Hover::In(t) | Hover::Out(t) => {
                let n = &self.model.nodes[t];
                let cands = if self.graph.ports[a].is_output() { &n.ins } else { &n.outs };
                cands.iter().copied().find(|&b| {
                    let (o, i) = if self.graph.ports[a].is_output() { (a, b) } else { (b, a) };
                    self.compatible(a, b) && !self.graph.connected(o, i)
                })
            }
            _ => None,
        };
        let Some(b) = b else { return };
        let (o, i) = if self.graph.ports[a].is_output() { (a, b) } else { (b, a) };
        if self.graph.connected(o, i) {
            self.disconnect(o, i);
        } else {
            self.connect(o, i);
        }
    }

    pub fn canvas_wheel(&mut self, dx: f32, dy: f32, ctrl: bool) {
        let [mx, my] = self.canvas.mouse;
        if ctrl {
            self.zoom_at([mx, my], self.layout.view.zoom * 1.12f32.powf(dy));
        } else {
            let step = 40.0 / self.layout.view.zoom;
            self.layout.view.cam[0] -= dx * step;
            self.layout.view.cam[1] -= dy * step;
        }
        self.canvas.hover = self.hit(mx, my);
        self.layout_changed();
    }

    fn canvas_menu(&mut self, h: Hover, x: f32, y: f32) {
        use crate::ui::MenuItem as I;
        let mut items = Vec::new();
        match h {
            Hover::Cell(mi, ..) => {
                let m = &self.mats[mi];
                let title = format!("{} → {}", self.model.nodes[m.src].title, self.model.nodes[m.dst].title);
                items.push(I::header(title));
                items.push(I::new("Connect 1:1", Action::ConnectDiagonal(m.key.clone())));
                items.push(I::new("Disconnect all", Action::DisconnectMatrix(m.key.clone())).enabled(!m.conns.is_empty()));
                items.push(I::new("Remove matrix", Action::Unpin(m.key.clone())).enabled(m.conns.is_empty()));
            }
            Hover::MatWire(mi, _) => {
                let m = &self.mats[mi];
                let title = format!("{} → {}", self.model.nodes[m.src].title, self.model.nodes[m.dst].title);
                items.push(I::header(title));
                items.push(I::new("Disconnect all", Action::DisconnectMatrix(m.key.clone())).enabled(!m.conns.is_empty()));
                items.push(I::new("Remove matrix", Action::Unpin(m.key.clone())).enabled(m.conns.is_empty()));
            }
            Hover::Wire(o, i) => {
                let (on, inn) = (self.graph.ports[o].name.clone(), self.graph.ports[i].name.clone());
                items.push(I::header(format!("{} → {}", self.model.labels[o], self.model.labels[i])));
                if self.source == crate::app::Source::Jack {
                    let ruled = self.rule_wants(&on, &inn);
                    items.push(I::new(if ruled { "Stop keeping it (rule)" } else { "Always keep it (a rule)" }, Action::Rule(on.clone(), inn.clone(), !ruled)));
                }
                items.push(I::new("Disconnect", Action::Disconnect(on, inn)));
            }
            Hover::Port(i, p) => {
                let name = self.graph.ports[p].name.clone();
                items.push(I::header(self.model.labels[p].clone()));
                items.push(I::new("Rename this port…", Action::Rename(format!("port:{name}"))));
                match self.model.group_of.get(&p) {
                    Some(&(gi, _)) => {
                        items.push(I::new(format!("Rename the group \"{}\"…", self.model.groups[gi].0), Action::RenameGroup(name.clone())));
                        items.push(I::new(format!("Ungroup \"{}\"", self.model.groups[gi].0), Action::Ungroup(name.clone())));
                    }
                    None => items.push(I::new("Group ports from here…", Action::GroupPrompt(name.clone()))),
                }
                items.push(I::new("Copy its connections", Action::CopyConns(format!("port:{name}"), false)));
                items.push(I::new("Cut its connections", Action::CopyConns(format!("port:{name}"), true)));
                items.push(I::new("Paste connections", Action::PasteConns(format!("port:{name}"))).enabled(self.clip.is_some()));
                if self.layout.names.contains_key(&format!("port:{name}")) {
                    items.push(I::new("Its JACK name again", Action::SetName(format!("port:{name}"), String::new())));
                }
                items.extend(self.node_menu(i, false));
            }
            Hover::Node(i) | Hover::In(i) | Hover::Out(i) => items = self.node_menu(i, false),
            Hover::None => {
                items.push(I::new("Arrange (signal chain)", Action::Arrange));
                items.push(I::new("Fit to window", Action::FitAll));
                items.push(I::new("Reset zoom", Action::ResetZoom));
                items.push(I::new("Show hidden clients", Action::ShowAll).enabled(!self.layout.hidden.is_empty()));
                items.push(I::new("Port names on matrices", Action::ToggleLabels).checked(self.layout.view.labels));
                items.push(I::new("Matrices (PatchMatrix view)", Action::ToggleMatrices).checked(self.layout.view.matrices));
                items.push(I::new("Vertical flow (ports view)", Action::ToggleVertical).checked(self.vertical()));
                items.push(I::new("Show grid", Action::ToggleGrid).checked(self.layout.view.grid));
                items.push(I::new("Snap to grid", Action::ToggleSnap).checked(self.layout.view.snap));
                items.push(I::new("Prevent overlap", Action::ToggleNoOverlap).checked(self.layout.view.no_overlap));
            }
        }
        self.ui.open_menu([x, y], items);
    }

    /// A node's menu (canvas and sidebar). `find`: offer "Find in canvas" (from the sidebar).
    pub fn node_menu(&self, i: usize, find: bool) -> Vec<crate::ui::MenuItem> {
        use crate::ui::MenuItem as I;
        let n = &self.model.nodes[i];
        let id = n.key.id();
        let hidden = self.layout.hidden.contains(&id);
        let split = n.key.role != crate::model::Role::Main;
        let mut items = Vec::new();
        // Several blocks selected, this one among them: line them up.
        let sel = self.canvas.selected.len();
        if sel >= 2 && self.canvas.selected.contains(&id) {
            items.push(I::header(format!("{sel} blocks selected")));
            for (label, how) in [
                ("Align left edges", Align::Left),
                ("Align centres (column)", Align::CentreX),
                ("Align right edges", Align::Right),
                ("Align tops", Align::Top),
                ("Align middles (row)", Align::CentreY),
                ("Align bottoms", Align::Bottom),
                ("Space evenly across", Align::SpreadX),
                ("Space evenly down", Align::SpreadY),
            ] {
                items.push(I::new(label, Action::Align(how)));
            }
            items.push(I::new("Hide the selected", Action::HideSelected));
        }
        items.push(I::header(n.title.clone()));
        if find {
            items.push(I::new("Find in canvas", Action::CenterNode(id.clone())));
        }
        items.push(I::new(if split { "Join inputs and outputs" } else { "Split inputs and outputs" }, Action::Split(n.key.group.clone(), !split)));
        items.push(I::new(format!("Disconnect all {}", self.kind.label()), Action::DisconnectNode(id.clone())));
        items.push(I::new(format!("Copy its {} connections", self.kind.label()), Action::CopyConns(id.clone(), false)));
        items.push(I::new("Paste connections", Action::PasteConns(id.clone())).enabled(self.clip.as_ref().is_some_and(|c| c.kind == self.kind)));
        if self.source == crate::app::Source::Jack {
            items.push(I::new("Always keep its connections (rules)", Action::RulesFromNode(id.clone())));
        }
        if self.source == crate::app::Source::Jack && is_pipewire(&n.key.group) {
            items.push(I::new("Open PipeWire's graph", Action::OpenPipeWire));
        }
        // Ardour's tracks, Carla's plugins: a block each, or back into one.
        let client = n.key.group.split('/').next().unwrap_or(&n.key.group).to_string();
        if self.layout.tracks.contains(&client) {
            items.push(I::new("One block for the whole client", Action::Tracks(client, false)));
        } else if crate::model::track_count(&self.graph, &client) >= 2 {
            items.push(I::new("A block per track / plugin", Action::Tracks(client, true)));
        }
        items.push(I::new("Rename…", Action::Rename(id.clone())));
        if self.layout.names.contains_key(&id) {
            items.push(I::new("Its JACK name again", Action::SetName(id.clone(), String::new())));
        }
        let expanded = self.is_expanded(i);
        items.push(I::new(if expanded { "Fold ports" } else { "Show all ports" }, Action::Expand(id.clone(), !expanded)));
        items.push(if hidden { I::new("Show", Action::Show(id)) } else { I::new("Hide", Action::Hide(id)) });
        items
    }

    // ---- drawing ----

    pub fn draw_canvas(&mut self, g: &mut Gfx) {
        let a = self.canvas.area;
        let xf = self.xf();
        let z = xf.z;
        g.push_clip(a);
        g.rect(a, GRID_BG);
        let gs = GRID * z;
        if gs >= 6.0 && self.layout.view.grid {
            let ox = (a.x - xf.cam[0] * z).rem_euclid(gs);
            let oy = (a.y - xf.cam[1] * z).rem_euclid(gs);
            let lw = self.scale.max(1.0).floor();
            let mut x = a.x + ox - gs;
            while x < a.right() {
                g.rect(Rect::new(x.round(), a.y, lw, a.h), GRID_LINE);
                x += gs;
            }
            let mut y = a.y + oy - gs;
            while y < a.bottom() {
                g.rect(Rect::new(a.x, y.round(), a.w, lw), GRID_LINE);
                y += gs;
            }
        }

        if self.source == crate::app::Source::PipeWire {
            // Where we are, and the way back.
            let s = self.scale;
            let px = FONT * s;
            let lh = g.line_height(Face::Regular, px);
            let label = "← JACK  ›  PipeWire";
            let w = g.measure(Face::Bold, px, label) + 24.0 * s;
            let r = Rect::new(a.x + 12.0 * s, a.y + 10.0 * s, w, lh + 12.0 * s);
            let hov = r.contains(self.ui.mouse[0], self.ui.mouse[1]);
            g.rrect(r, 4.0 * s, alpha(BAR_BG, 0.92));
            g.rrect_stroke(r, 4.0 * s, s.max(1.0), if hov { HILIGHT } else { alpha(kind_color(pbx_jack::PortKind::Video), 0.8) });
            g.text(Face::Bold, px, r.x + 12.0 * s, r.y + 6.0 * s, TEXT_BRIGHT, label);
            self.ui.hits.push((r, Action::BackToJack));
        }
        if self.jack.is_none() && self.source == crate::app::Source::Jack {
            let msg = "Waiting for a JACK server…";
            g.text_center(Face::Regular, FONT * self.scale * 1.2, a, TEXT_DIM, msg);
            g.pop_clip();
            return;
        }

        // Which nodes and matrices light up: hovered ones and their neighbours.
        let hov_node = match self.canvas.hover {
            Hover::Node(i) | Hover::In(i) | Hover::Out(i) | Hover::Port(i, _) => Some(i),
            _ => None,
        };
        let hov_mat = match self.canvas.hover {
            Hover::Cell(mi, ..) | Hover::MatWire(mi, _) => Some(mi),
            _ => None,
        };
        let dragging_node = match &self.canvas.drag {
            Drag::Node { id, .. } => self.model.node_by_id(id),
            _ => None,
        };
        let dragging_mat = match &self.canvas.drag {
            Drag::Matrix { key, .. } | Drag::Paint { key, .. } | Drag::MatWire { key, .. } => self.mats.iter().position(|m| &m.key == key),
            _ => None,
        };
        let mut lit_nodes: HashSet<usize> = HashSet::new();
        let mut lit_mats: HashSet<usize> = HashSet::new();
        for (mi, m) in self.mats.iter().enumerate() {
            let on = Some(mi) == hov_mat
                || Some(mi) == dragging_mat
                || hov_node.is_some_and(|n| n == m.src || n == m.dst)
                || dragging_node.is_some_and(|n| n == m.src || n == m.dst);
            if on {
                lit_mats.insert(mi);
                lit_nodes.insert(m.src);
                lit_nodes.insert(m.dst);
            }
        }
        if let Some(n) = hov_node.or(dragging_node) {
            lit_nodes.insert(n);
        }

        // Wires (under everything): PatchMatrix's routing through the matrix (in at its top, out
        // at its right), Luluda's look: coloured by port type, curves that scale with the
        // distance, brighter when lit.
        let ww = (2.0 * self.scale).max(1.0);
        let bend = WIRE_BEND * z;
        let cs = HANDLE_R * z;
        let wire = kind_color(self.kind);
        if !self.layout.view.matrices {
            // Ports view: a wire per connection, port to port.
            let anchors = self.anchors(&xf);
            let hov_wire = match self.canvas.hover {
                Hover::Wire(o, i) => Some((o, i)),
                _ => None,
            };
            let hov_port = match self.canvas.hover {
                Hover::Port(_, p) => Some(p),
                _ => None,
            };
            let picked = match &self.canvas.drag {
                Drag::Rewire { out, inp, moving_in, .. } => self.graph.by_name.get(out).copied().zip(self.graph.by_name.get(inp).copied()).map(|w| (w, *moving_in)),
                _ => None,
            };
            for &(o, i) in &self.graph.conns {
                if self.graph.ports[o].kind != self.kind {
                    continue;
                }
                let (Some(&a), Some(&b)) = (anchors.get(&o), anchors.get(&i)) else { continue };
                if let Some(((po, pi), moving_in)) = picked
                    && (po, pi) == (o, i)
                {
                    let [p0, p1, p2, p3] = if moving_in { self.loose_curve(a, self.canvas.mouse, true, z) } else { self.loose_curve(b, self.canvas.mouse, false, z) };
                    g.bezier(p0, p1, p2, p3, ww * 1.4, mix(wire, [1.0, 1.0, 1.0, 1.0], 0.25));
                    continue;
                }
                let node_lit = |p: usize| self.model.port_node.get(&p).is_some_and(|n| lit_nodes.contains(n));
                let lit = hov_wire == Some((o, i)) || hov_port.is_some_and(|p| p == o || p == i) || node_lit(o) || node_lit(i);
                let col = if lit { mix(wire, [1.0, 1.0, 1.0, 1.0], 0.25) } else { alpha(wire, WIRE_ALPHA) };
                let [p0, p1, p2, p3] = self.curve(a, b, z);
                g.bezier(p0, p1, p2, p3, if lit { ww * 1.4 } else { ww }, col);
            }
        }
        // Matrix view: each matrix's wires meet at its centre, under it.
        let pulled = match &self.canvas.drag {
            Drag::MatWire { key, sink, .. } => self.mats.iter().position(|m| &m.key == key).map(|mi| (mi, *sink)),
            _ => None,
        };
        let hov_wire = match self.canvas.hover {
            Hover::MatWire(mi, sink) => Some((mi, sink)),
            _ => None,
        };
        for mi in (0..self.mats.len()).filter(|_| self.layout.view.matrices) {
            let lit = lit_mats.contains(&mi);
            let loose = pulled.filter(|(m, _)| *m == mi).map(|(_, sink)| (sink, self.canvas.mouse));
            for (k, [p0, p1, p2, p3]) in self.mat_curves(&xf, mi, loose).into_iter().enumerate() {
                let sink = k == 1;
                let hot = hov_wire == Some((mi, sink)) || loose.is_some_and(|(s, _)| s == sink);
                let col = if lit || hot { mix(wire, [1.0, 1.0, 1.0, 1.0], 0.25) } else { alpha(wire, WIRE_ALPHA) };
                g.bezier(p0, p1, p2, p3, if hot { ww * 1.5 } else { ww }, col);
            }
        }

        // Nodes.
        let flash = self.canvas.flash.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_millis(1500)).map(|(id, _)| id.clone());
        for i in 0..self.model.nodes.len() {
            if !self.visible(i) {
                continue;
            }
            let lit = lit_nodes.contains(&i) || flash.as_deref() == Some(&self.model.nodes[i].key.id());
            self.draw_node(g, &xf, i, lit);
            if self.canvas.selected.contains(&self.model.nodes[i].key.id()) {
                let r = self.node_extent_px(&xf, i);
                let s = self.scale;
                g.rrect_stroke(r.inset(-4.0 * s), 5.0 * s, (1.6 * s).max(1.0), alpha(LCD_TEXT, 0.9));
            }
        }
        // A port the search found: a ring round it for a moment.
        if let Some((name, t)) = &self.canvas.flash_port
            && t.elapsed() < Duration::from_millis(2500)
            && let Some(&p) = self.graph.by_name.get(name)
            && let Some(at) = self.port_anchor(&xf, p)
        {
            let s = self.scale;
            g.arc(at, 9.0 * s, 0.0, std::f32::consts::TAU, (2.0 * s).max(1.0), HILIGHT);
        }
        // The selection band.
        if let Drag::Band { from, .. } = &self.canvas.drag {
            let [x, y] = self.canvas.mouse;
            let band = Rect::new(from[0].min(x), from[1].min(y), (x - from[0]).abs(), (y - from[1]).abs());
            g.rect(band, alpha(LCD_TEXT, 0.08));
            g.rect_stroke(band, self.scale.max(1.0), alpha(LCD_TEXT, 0.7));
        }

        // Matrices.
        for mi in (0..self.mats.len()).filter(|_| self.layout.view.matrices) {
            self.draw_matrix(g, &xf, mi, lit_mats.contains(&mi));
        }

        // The link being dragged.
        if let Drag::Link { src } = &self.canvas.drag
            && let Some(s) = self.model.node_by_id(src) {
                let (out, _) = self.handles(s);
                let p0 = xf.s(out);
                let p1 = self.canvas.mouse;
                g.bezier(p0, [p0[0] + bend, p0[1]], [p1[0] - bend, p1[1]], p1, ww * 1.4, HILIGHT);
                g.arc(p0, cs * 2.0, 0.0, std::f32::consts::TAU, ww, HILIGHT);
            }

        // A port being dragged from an expanded node.
        if let Drag::Port { name } = &self.canvas.drag
            && let Some(&a) = self.graph.by_name.get(name)
            && let Some(p0) = self.port_anchor(&xf, a)
        {
            let [q0, q1, q2, q3] = self.loose_curve(p0, self.canvas.mouse, self.graph.ports[a].is_output(), z);
            g.bezier(q0, q1, q2, q3, ww * 1.4, kind_color(self.graph.ports[a].kind));
        }

        // Port names around matrices: the hovered one, or all of them.
        for mi in (0..self.mats.len()).filter(|_| self.layout.view.matrices) {
            if self.layout.view.labels || Some(mi) == hov_mat {
                self.draw_matrix_labels(g, &xf, mi);
            }
        }
        if let Hover::Cell(mi, c, r) = self.canvas.hover
            && matches!(self.canvas.drag, Drag::None | Drag::Paint { .. }) {
                self.draw_cell_tip(g, &xf, mi, c, r);
            }
        if let Hover::Wire(o, i) = self.canvas.hover
            && matches!(self.canvas.drag, Drag::None)
        {
            let mut lines = vec![
                (format!("{}  →  {}", self.graph.ports[o].name, self.graph.ports[i].name), TEXT_BRIGHT),
                ("drag an end off: disconnect, or onto another port: move".to_string(), TEXT_DIM),
                (if self.rule_wants(&self.graph.ports[o].name, &self.graph.ports[i].name) { "kept by a connection rule".to_string() } else { String::new() }, LCD_TEXT),
                ("right-click: disconnect".to_string(), TEXT_DIM),
            ];
            lines.retain(|(t, _)| !t.is_empty());
            self.draw_tooltip(g, self.canvas.mouse, &lines);
        }
        if let Hover::MatWire(mi, _) = self.canvas.hover
            && matches!(self.canvas.drag, Drag::None)
        {
            let m = &self.mats[mi];
            let n = m.conns.len();
            let lines = vec![
                (format!("{}  →  {}", self.model.nodes[m.src].title, self.model.nodes[m.dst].title), TEXT_BRIGHT),
                (format!("{n} connection{}", if n == 1 { "" } else { "s" }), TEXT),
                ("drag off the block: disconnect, or onto another block: move".to_string(), TEXT_DIM),
            ];
            self.draw_tooltip(g, self.canvas.mouse, &lines);
        }
        if let Hover::Port(_, p) = self.canvas.hover
            && matches!(self.canvas.drag, Drag::None)
        {
            self.draw_port_tip(g, p);
        }

        if self.model.nodes.is_empty() {
            g.text_center(Face::Regular, FONT * self.scale, a, TEXT_DIM, "No ports on the server");
        } else if !self.model.nodes.iter().any(|n| n.has_kind(&self.graph, self.kind)) {
            let msg = format!("No {} ports", self.kind.label());
            g.text_center(Face::Regular, FONT * self.scale, Rect::new(a.x, a.bottom() - 40.0 * self.scale, a.w, 30.0 * self.scale), TEXT_DIM, &msg);
        }
        g.pop_clip();
    }

    fn draw_node(&self, g: &mut Gfx, xf: &Xf, i: usize, lit: bool) {
        let n = &self.model.nodes[i];
        let z = xf.z;
        let r = self.node_rect(xf, i);
        if !r.overlaps(&self.canvas.area.inset(-40.0 * z)) {
            return;
        }
        let nin = n.ins_of(&self.graph, self.kind).len();
        let nout = n.outs_of(&self.graph, self.kind).len();
        let body = self.body_rect(xf, i);
        let editable = nin + nout > 0 || body.is_some();
        let f = if editable { 1.0 } else { 0.33 };
        // Luluda's block: light fill, border in the node type's colour, dark inset line; lit
        // (hovered, linked, dragged) borders go PatchMatrix orange.
        let stroke = if lit { HILIGHT } else { self.node_color(i) };
        let lw = (1.5 * self.scale).max(1.0);
        let rad = 2.4 * z;
        let fill = if lit { BLOCK_FILL_HOT } else { BLOCK_FILL };
        let full = match body {
            Some(b) => Rect::new(r.x, r.y, r.w, b.bottom() - r.y),
            None => r,
        };
        if body.is_some() {
            g.rrect(full, rad, BLOCK_BODY);
            g.rect(Rect::new(r.x + lw, r.y + lw, r.w - 2.0 * lw, r.h - lw), fill);
            g.rect(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), BLOCK_INSET);
        } else {
            g.rrect(full, rad, alpha(fill, f));
        }
        g.rrect_stroke(full, rad, lw, alpha(stroke, f));
        g.rrect_stroke(full.inset(lw), (rad - lw).max(0.5), 1.0, alpha(BLOCK_INSET, f));
        let px = FONT * z;
        let lh = g.line_height(Face::Regular, px);
        let dx = 4.0 * z;
        // PatchMatrix's port counts either side, in the matrix view only: the ports view shows
        // the ports themselves.
        let counts = self.layout.view.matrices;
        let dw = if counts { 1.5 * lh } else { 0.0 };
        let ww = r.w - 2.0 * (dx + dw);
        let ty = r.cy() - lh / 2.0;
        let tw = g.measure(Face::Regular, px, &n.title).min(ww);
        g.text_fit(Face::Regular, px, r.cx() - tw / 2.0, ty, ww, alpha(BLOCK_TEXT, f), &n.title);
        if counts && nout > 0 {
            let s = format!("{nout:02}");
            let w = g.measure(Face::Regular, px, &s);
            g.text(Face::Regular, px, r.right() - w - dx - lw, ty, alpha(BLOCK_TEXT, f * 0.85), &s);
        }
        if counts && nin > 0 {
            let s = format!("{nin:02}");
            g.text(Face::Regular, px, r.x + dx + lw, ty, alpha(BLOCK_TEXT, f * 0.85), &s);
        }
        // "capture" / "playback": on the side no wire comes from. Vertical: above a block whose
        // ports are below (outputs only), below one whose ports are on top (inputs only), beside
        // one with both; horizontal and matrix views: above.
        if let Some(sub) = &n.sub {
            let spx = FONT_SMALL * z;
            let sh = g.line_height(Face::Regular, spx);
            let (x, y) = match (self.vertical(), nin > 0, nout > 0) {
                (true, true, false) => (r.x + 2.0 * z, r.bottom() + 2.0 * z),
                (true, true, true) => (r.right() + 6.0 * z, r.cy() - sh / 2.0),
                _ => (r.x + 2.0 * z, r.y - sh - 1.0 * z),
            };
            g.text(Face::Regular, spx, x, y, alpha(TEXT, f), sub);
        }
        // Ports view: the ports are on the body (or, vertical, on the top and bottom edges); no
        // node handles.
        if !self.layout.view.matrices {
            if self.vertical() {
                self.draw_vports(g, xf, i, f);
            } else if body.is_some() {
                self.draw_body(g, xf, i);
            }
            return;
        }
        // Handles: Luluda's port squares, in the current type's colour.
        let cw = HANDLE_R * z;
        let col = if lit { HILIGHT } else { kind_color(self.kind) };
        let (out, inp) = self.handles(i);
        let port_sq = |g: &mut Gfx, p: [f32; 2]| {
            let q = Rect::centered(p[0], p[1], 2.4 * cw, 2.4 * cw);
            g.rrect(q, 0.5 * cw, col);
            g.rrect_stroke(q, 0.5 * cw, 1.0, [0.0, 0.0, 0.0, 0.75]);
        };
        if nout > 0 {
            let p = xf.s(out);
            port_sq(g, p);
            let hot = self.canvas.hover == crate::canvas::Hover::Out(i) && matches!(self.canvas.drag, Drag::None)
                || matches!(&self.canvas.drag, Drag::Link { src } if *src == n.key.id());
            if hot {
                g.arc(p, 2.0 * cw, 0.0, std::f32::consts::TAU, self.scale.max(1.0), HILIGHT);
            }
        }
        if nin > 0 {
            let p = xf.s(inp);
            port_sq(g, p);
            let linking = matches!(self.canvas.drag, Drag::Link { .. });
            let hot = linking && matches!(self.canvas.hover, Hover::Node(h) | Hover::In(h) | Hover::Out(h) if h == i);
            if hot {
                g.arc(p, 2.0 * cw, 0.0, std::f32::consts::TAU, self.scale.max(1.0), HILIGHT);
            }
        }
        if body.is_some() {
            self.draw_body(g, xf, i);
        }
    }

    /// The colour of a node's type: hardware audio green, MIDI devices orange, the PipeWire
    /// bridge cyan, applications purple (Luluda's block borders). In PipeWire's graph, devices
    /// (sinks, sources) count as hardware.
    fn node_color(&self, i: usize) -> crate::gfx::Color {
        let n = &self.model.nodes[i];
        let midi_only = n.ins.iter().chain(&n.outs).all(|&p| self.graph.ports[p].kind == pbx_jack::PortKind::Midi);
        if is_pipewire(&n.key.group) && self.source == crate::app::Source::Jack {
            return NODE_BRIDGE;
        }
        if let Some(class) = self.graph.client_class.get(&n.key.group) {
            return if class.contains("stream") { NODE_APP } else if class.contains("midi") { NODE_MIDI } else { NODE_HW };
        }
        if n.physical {
            return if midi_only { NODE_MIDI } else { NODE_HW };
        }
        NODE_APP
    }

    /// An expanded node's ports: inputs (strip on the left), then outputs (strip on the right),
    /// coloured by type; the current tab's type bright, the others dimmer. A dot: connected.
    fn draw_body(&self, g: &mut Gfx, xf: &Xf, i: usize) {
        let z = xf.z;
        let px = FONT_SMALL * z;
        if px < 4.0 {
            return;
        }
        let lh = g.line_height(Face::Regular, px);
        let rows = self.port_rows(xf, i);
        let dragged = match &self.canvas.drag {
            Drag::Port { name } => self.graph.by_name.get(name).copied(),
            _ => None,
        };
        let n_in = self.model.nodes[i].ins.len();
        for (k, &(p, row)) in rows.iter().enumerate() {
            if !row.overlaps(&self.canvas.area) {
                continue;
            }
            let port = &self.graph.ports[p];
            let input = port.is_input();
            let fa = if port.kind == self.kind { 1.0 } else { 0.55 };
            let ports_view = !self.layout.view.matrices;
            if !ports_view && k == n_in && k > 0 {
                g.rect(Rect::new(row.x + 6.0 * z, row.y.round(), row.w - 12.0 * z, self.scale.max(1.0)), PANEL_LINE);
            }
            let hovered = self.canvas.hover == Hover::Port(i, p);
            if dragged.is_some_and(|a| self.compatible(a, p)) {
                g.rect(row.inset(1.0), alpha(HILIGHT, if hovered { 0.30 } else { 0.12 }));
            } else if hovered {
                g.rect(row.inset(1.0), [1.0, 1.0, 1.0, 0.07]);
            }
            if ports_view {
                // Luluda's port square on the block's edge: filled when connected.
                let kc = alpha(kind_color(port.kind), fa);
                let q = 2.4 * HANDLE_R * z;
                let (ex, tx) = if input { (row.x, row.x + q * 0.5 + 4.0 * z) } else { (row.right(), row.right() - q * 0.5 - 4.0 * z) };
                let sq = Rect::centered(ex, row.cy(), q, q);
                let connected = self.graph.conns.iter().any(|&(o, i2)| o == p || i2 == p);
                if connected {
                    g.rrect(sq, 0.5 * HANDLE_R * z, kc);
                } else {
                    g.rrect(sq, 0.5 * HANDLE_R * z, alpha(BLOCK_BODY, fa));
                    g.rrect_stroke(sq, 0.5 * HANDLE_R * z, (1.2 * self.scale).max(1.0), kc);
                }
                g.rrect_stroke(sq.inset(-1.0), 0.5 * HANDLE_R * z + 1.0, 1.0, [0.0, 0.0, 0.0, 0.6 * fa]);
                // The user's group: a bracket along all its squares, from its first one.
                if let Some(&(gi, 0)) = self.model.group_of.get(&p)
                    && let Some(&last) = self.model.groups[gi].1.last()
                    && let Some((_, lr)) = rows.iter().find(|(n, _)| *n == last)
                {
                    let bx = if input { sq.x - 4.5 * z } else { sq.right() + 2.0 * z };
                    let bw = (2.5 * z).max(1.5);
                    let col = alpha(mix(kc, [1.0, 1.0, 1.0, 1.0], 0.25), 1.0);
                    g.rect(Rect::new(bx, row.cy() - bw / 2.0, bw, lr.cy() - row.cy() + bw), col);
                }
                if let Some(&q2) = self.model.partner.get(&p)
                    && rows.get(k + 1).is_some_and(|(n, _)| *n == q2)
                    && !self.model.group_of.contains_key(&p)
                {
                    // Stereo pair: a bracket joining the two squares.
                    let bx = if input { sq.right() + 1.5 * z } else { sq.x - 2.5 * z };
                    g.rect(Rect::new(bx, row.cy(), 1.0f32.max(z), row.h), alpha(kc, 0.6));
                }
                let label = &self.model.labels[p];
                let max_w = row.w - q - 8.0 * z;
                let col = alpha(BLOCK_TEXT, if hovered { fa } else { fa * 0.92 });
                if input {
                    g.text_fit(Face::Regular, px, tx, row.cy() - lh / 2.0, max_w, col, label);
                } else {
                    let w = g.measure(Face::Regular, px, label).min(max_w);
                    g.text_fit(Face::Regular, px, tx - w, row.cy() - lh / 2.0, max_w, col, label);
                }
                continue;
            }
            let sw = (3.0 * z).max(2.0);
            let strip = if input {
                Rect::new(row.x + 2.0 * z, row.y + 2.0 * z, sw, row.h - 4.0 * z)
            } else {
                Rect::new(row.right() - 2.0 * z - sw, row.y + 2.0 * z, sw, row.h - 4.0 * z)
            };
            g.rect(strip, alpha(kind_color(port.kind), fa));
            // Stereo pairs: the strip continues across both rows; a group's, across all of them.
            if let Some(&(gi, 0)) = self.model.group_of.get(&p)
                && let Some(&last) = self.model.groups[gi].1.last()
                && let Some((_, lr)) = rows.iter().find(|(n, _)| *n == last)
            {
                g.rect(Rect::new(strip.x, strip.bottom(), sw, lr.bottom() - 2.0 * z - strip.bottom()), alpha(mix(kind_color(port.kind), [1.0, 1.0, 1.0, 1.0], 0.25), fa));
            } else if let Some(&q) = self.model.partner.get(&p)
                && rows.get(k + 1).is_some_and(|(n, _)| *n == q)
                && !self.model.group_of.contains_key(&p)
            {
                g.rect(Rect::new(strip.x, strip.bottom(), sw, 4.0 * z), alpha(kind_color(port.kind), fa * 0.6));
            }
            let connected = self.graph.conns.iter().any(|&(o, i2)| o == p || i2 == p);
            let dot = if input { [strip.right() + 6.0 * z, row.cy()] } else { [strip.x - 6.0 * z, row.cy()] };
            if connected {
                g.circle(dot, 2.4 * z, alpha(TOGGLE, fa));
            } else {
                g.arc(dot, 2.4 * z, 0.0, std::f32::consts::TAU, self.scale.max(1.0) * 0.8, alpha(TEXT_DIM, fa));
            }
            let label = &self.model.labels[p];
            let max_w = row.w - 22.0 * z;
            let col = alpha(BLOCK_TEXT, if hovered { fa } else { fa * 0.92 });
            if input {
                g.text_fit(Face::Regular, px, dot[0] + 6.0 * z, row.cy() - lh / 2.0, max_w, col, label);
            } else {
                let w = g.measure(Face::Regular, px, label).min(max_w);
                g.text_fit(Face::Regular, px, dot[0] - 6.0 * z - w, row.cy() - lh / 2.0, max_w, col, label);
            }
        }
    }

    /// Vertical layout: port squares along the top (inputs) and bottom (outputs) edges, filled
    /// when connected; names on hover (the tooltip), as in Luluda.
    fn draw_vports(&self, g: &mut Gfx, xf: &Xf, i: usize, f: f32) {
        let z = xf.z;
        let q = 2.4 * HANDLE_R * z;
        let dragged = match &self.canvas.drag {
            Drag::Port { name } => self.graph.by_name.get(name).copied(),
            _ => None,
        };
        for (p, r) in self.port_rows(xf, i) {
            let port = &self.graph.ports[p];
            let fa = if port.kind == self.kind { f } else { f * 0.55 };
            let kc = alpha(kind_color(port.kind), fa);
            let sq = Rect::centered(r.cx(), r.cy(), q, q);
            let hovered = self.canvas.hover == Hover::Port(i, p);
            if dragged.is_some_and(|a| self.compatible(a, p)) || hovered {
                g.rrect(sq.inset(-3.0 * self.scale), 0.5 * HANDLE_R * z + 3.0, alpha(HILIGHT, 0.45));
            }
            let connected = self.graph.conns.iter().any(|&(o, i2)| o == p || i2 == p);
            if connected {
                g.rrect(sq, 0.5 * HANDLE_R * z, kc);
            } else {
                g.rrect(sq, 0.5 * HANDLE_R * z, alpha(BLOCK_BODY, fa));
                g.rrect_stroke(sq, 0.5 * HANDLE_R * z, (1.2 * self.scale).max(1.0), kc);
            }
            g.rrect_stroke(sq.inset(-1.0), 0.5 * HANDLE_R * z + 1.0, 1.0, [0.0, 0.0, 0.0, 0.6 * fa]);
        }
        // Groups (and stereo pairs): a bar along their squares, on the side away from the
        // block (above the inputs on top, below the outputs at the bottom).
        let rows = self.port_rows(xf, i);
        let at = |p: usize| rows.iter().find(|(q, _)| *q == p).map(|(_, r)| *r);
        let mut spans: Vec<(usize, usize, bool)> = Vec::new();
        for (_, ports) in &self.model.groups {
            if let (Some(&a), Some(&b)) = (ports.first(), ports.last()) {
                spans.push((a, b, true));
            }
        }
        for (&a, &b) in &self.model.partner {
            if a < b && !self.model.group_of.contains_key(&a) {
                spans.push((a, b, false));
            }
        }
        for (a, b, user) in spans {
            let (Some(ra), Some(rb)) = (at(a), at(b)) else { continue };
            let port = &self.graph.ports[a];
            let fa = if port.kind == self.kind { f } else { f * 0.55 };
            let col = if user { alpha(mix(kind_color(port.kind), [1.0, 1.0, 1.0, 1.0], 0.25), fa) } else { alpha(kind_color(port.kind), fa * 0.6) };
            let bw = if user { (2.5 * z).max(1.5) } else { (1.2 * z).max(1.0) };
            let y = if port.is_input() { ra.cy() - q / 2.0 - 3.5 * z - bw } else { ra.cy() + q / 2.0 + 3.5 * z };
            let (x0, x1) = (ra.cx().min(rb.cx()) - q / 2.0, ra.cx().max(rb.cx()) + q / 2.0);
            g.rect(Rect::new(x0, y, x1 - x0, bw), col);
        }
    }

    /// Everything about one port: its full name, type, aliases, connections.
    fn draw_port_tip(&self, g: &mut Gfx, p: usize) {
        let port = &self.graph.ports[p];
        let mut lines: Vec<(String, crate::gfx::Color)> = vec![(port.name.clone(), TEXT_BRIGHT)];
        let dir = if port.is_input() { "input" } else { "output" };
        let mut info = format!("{} {dir}", port.kind.label());
        if port.is_physical() {
            info.push_str(", physical");
        }
        if let Some(pretty) = &port.pretty {
            info.push_str(&format!(", pretty name \"{pretty}\""));
        }
        lines.push((info, kind_color(port.kind)));
        for a in &port.aliases {
            lines.push((format!("alias {a}"), TEXT_DIM));
        }
        if let Some(&(gi, k)) = self.model.group_of.get(&p) {
            let (name, ports) = &self.model.groups[gi];
            lines.push((format!("group \"{name}\", {} of {} (drag: all of them; Ctrl+drag: this one)", k + 1, ports.len()), LCD_CAPTION));
        }
        // Its latency: how long since the signal was captured, how long until it is played.
        if self.source == crate::app::Source::Jack
            && let Some((cap, play)) = self.jack.as_ref().and_then(|c| c.port_latency(&port.name))
        {
            let rate = self.stats.sample_rate.max(1) as f32;
            let show = |r: pbx_jack::ffi::jack_latency_range_t| {
                let f = if r.min == r.max { format!("{}", r.max) } else { format!("{}–{}", r.min, r.max) };
                format!("{f} frames ({:.2} ms)", r.max as f32 * 1000.0 / rate)
            };
            let mut parts = Vec::new();
            if cap.max > 0 {
                parts.push(format!("capture {}", show(cap)));
            }
            if play.max > 0 {
                parts.push(format!("playback {}", show(play)));
            }
            if !parts.is_empty() {
                lines.push((format!("latency: {}", parts.join(" · ")), LCD_TEXT));
            }
        }
        let others: Vec<&str> = self
            .graph
            .conns
            .iter()
            .filter_map(|&(o, i)| if o == p { Some(i) } else if i == p { Some(o) } else { None })
            .map(|q| self.graph.ports[q].name.as_str())
            .collect();
        if others.is_empty() {
            lines.push(("not connected".into(), TEXT_DIM));
        }
        let arrow = if port.is_input() { "←" } else { "→" };
        for (k, o) in others.iter().enumerate() {
            if k == 8 {
                lines.push((format!("… {} more", others.len() - 8), TEXT_DIM));
                break;
            }
            lines.push((format!("{arrow} {o}"), TOGGLE));
        }
        self.draw_tooltip(g, self.canvas.mouse, &lines);
    }

    fn draw_matrix(&self, g: &mut Gfx, xf: &Xf, mi: usize, lit: bool) {
        let m = &self.mats[mi];
        let r = self.matrix_rect(xf, m);
        if !r.overlaps(&self.canvas.area.inset(-20.0)) {
            return;
        }
        let z = xf.z;
        let cell = CELL * z;
        let lw = self.scale.max(1.0);
        g.rect(r, MATRIX_FILL);
        // Hovered row and column.
        if let Hover::Cell(hm, hc, hr) = self.canvas.hover
            && hm == mi {
                let tint = [1.0, 1.0, 1.0, 0.07];
                g.rect(Rect::new(r.x + hc as f32 * cell, r.y, cell, r.h), tint);
                g.rect(Rect::new(r.x, r.y + hr as f32 * cell, r.w, cell), tint);
            }
        // Grid: lines inside a stereo pair are fainter, so pairs read as one.
        let in_pair = |a: usize, b: usize| self.model.partner.get(&a) == Some(&b);
        for c in 1..m.cols.len() {
            let x = (r.x + c as f32 * cell).round();
            let col = if in_pair(m.cols[c - 1], m.cols[c]) { alpha(BORDER, 0.35) } else { BORDER };
            g.rect(Rect::new(x, r.y, lw, r.h), col);
        }
        for row in 1..m.rows.len() {
            let y = (r.y + row as f32 * cell).round();
            let col = if in_pair(m.rows[row - 1], m.rows[row]) { alpha(BORDER, 0.35) } else { BORDER };
            g.rect(Rect::new(r.x, y, r.w, lw), col);
        }
        g.rect_stroke(r, lw, if lit { HILIGHT } else { BORDER });
        let cs = HANDLE_R * z;
        for (c, &o) in m.cols.iter().enumerate() {
            for (row, &i) in m.rows.iter().enumerate() {
                if m.conns.contains(&(o, i)) {
                    let p = [r.x + (c as f32 + 0.5) * cell, r.y + (row as f32 + 0.5) * cell];
                    g.circle(p, cs, TOGGLE);
                }
            }
        }
        if let Hover::Cell(hm, hc, hr) = self.canvas.hover
            && hm == mi {
                let cr = Rect::new(r.x + hc as f32 * cell, r.y + hr as f32 * cell, cell, cell);
                g.rect_stroke(cr, lw, HILIGHT);
                // Where Shift would also connect: the stereo partner cell.
                let (o, i) = (m.cols[hc], m.rows[hr]);
                if let (Some(po), Some(pi)) = (self.model.partner.get(&o), self.model.partner.get(&i))
                    && let (Some(pc), Some(pr)) = (m.cols.iter().position(|x| x == po), m.rows.iter().position(|x| x == pi)) {
                        let pr = Rect::new(r.x + pc as f32 * cell, r.y + pr as f32 * cell, cell, cell);
                        g.rect_stroke(pr, lw, alpha(HILIGHT, 0.35));
                    }
            }
    }

    /// Port names: sink ports to the left of the rows, source ports above the columns (reading
    /// upward), on a dark backing so they read over wires.
    fn draw_matrix_labels(&self, g: &mut Gfx, xf: &Xf, mi: usize) {
        let m = &self.mats[mi];
        let r = self.matrix_rect(xf, m);
        let z = xf.z;
        let px = FONT_SMALL * z;
        if px < 6.0 {
            return;
        }
        let cell = CELL * z;
        let lh = g.line_height(Face::Regular, px);
        let pad = 4.0 * z;
        let max_w = 160.0 * z;
        let hover = match self.canvas.hover {
            Hover::Cell(hm, c, row) if hm == mi => Some((c, row)),
            _ => None,
        };
        let rows_w = m.rows.iter().map(|&p| g.measure(Face::Regular, px, &self.model.labels[p])).fold(0.0, f32::max).min(max_w);
        let cols_h = m.cols.iter().map(|&p| g.measure(Face::Regular, px, &self.model.labels[p])).fold(0.0, f32::max).min(max_w);
        let back = alpha(TOOLTIP_BG, 0.88);
        g.rect(Rect::new(r.x - rows_w - 2.0 * pad, r.y, rows_w + 2.0 * pad, r.h), back);
        g.rect(Rect::new(r.x, r.y - cols_h - 2.0 * pad, r.w, cols_h + 2.0 * pad), back);
        for (row, &p) in m.rows.iter().enumerate() {
            let s = &self.model.labels[p];
            let w = g.measure(Face::Regular, px, s).min(max_w);
            let y = r.y + (row as f32 + 0.5) * cell - lh / 2.0;
            let col = if hover.is_some_and(|h| h.1 == row) { HILIGHT } else { TEXT_BRIGHT };
            g.text_fit(Face::Regular, px, r.x - pad - w, y, max_w, col, s);
        }
        for (c, &p) in m.cols.iter().enumerate() {
            let s = &self.model.labels[p];
            let x = r.x + (c as f32 + 0.5) * cell - lh / 2.0;
            let col = if hover.is_some_and(|h| h.0 == c) { HILIGHT } else { TEXT_BRIGHT };
            let cut = cut_to(g, px, s, max_w);
            g.text_up(Face::Regular, px, x, r.y - pad, col, &cut);
        }
        // The two clients, in the corner the labels leave free.
        let cpx = FONT_SMALL * z;
        let src = &self.model.nodes[m.src].title;
        let dst = &self.model.nodes[m.dst].title;
        let corner = Rect::new(r.x - rows_w - 2.0 * pad, r.y - cols_h - 2.0 * pad, rows_w + 2.0 * pad, cols_h + 2.0 * pad);
        if corner.w > 30.0 && corner.h > 2.0 * lh {
            g.rect(corner, alpha(TOOLTIP_BG, 0.6));
            g.text_fit(Face::Bold, cpx, corner.x + pad, corner.bottom() - 2.0 * lh - pad, corner.w - 2.0 * pad, mix(TEXT, HANDLE, 0.4), &format!("{src} →"));
            g.text_fit(Face::Bold, cpx, corner.x + pad, corner.bottom() - lh - pad, corner.w - 2.0 * pad, TEXT, &format!("→ {dst}"));
        }
    }

    fn draw_cell_tip(&self, g: &mut Gfx, xf: &Xf, mi: usize, c: usize, row: usize) {
        let m = &self.mats[mi];
        let r = self.matrix_rect(xf, m);
        let (o, i) = (m.cols[c], m.rows[row]);
        let on = m.conns.contains(&(o, i));
        let s = format!("{}  {}  {}", self.model.labels[o], if on { "●→" } else { "→" }, self.model.labels[i]);
        let px = FONT * self.scale;
        let lh = g.line_height(Face::Regular, px);
        let w = g.measure(Face::Regular, px, &s);
        let pad = 6.0 * self.scale;
        let tip = Rect::new(r.cx() - w / 2.0 - pad, r.bottom() + 6.0 * self.scale, w + 2.0 * pad, lh + pad);
        g.rrect(tip, 3.0 * self.scale, alpha(TOOLTIP_BG, 0.92));
        g.rrect_stroke(tip, 3.0 * self.scale, 1.0, alpha(if on { TOGGLE } else { BORDER }, 0.6));
        g.text(Face::Regular, px, tip.x + pad, tip.y + pad / 2.0, TEXT_BRIGHT, &s);
    }
}

/// Distance from p to the segment a-b.
fn bezier_at([p0, p1, p2, p3]: [[f32; 2]; 4], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    [
        u * u * u * p0[0] + 3.0 * u * u * t * p1[0] + 3.0 * u * t * t * p2[0] + t * t * t * p3[0],
        u * u * u * p0[1] + 3.0 * u * u * t * p1[1] + 3.0 * u * t * t * p2[1] + t * t * t * p3[1],
    ]
}

/// Distance from `p` to a cubic curve (sampled), and where along it (0..1) that is closest.
fn curve_dist([p0, p1, p2, p3]: [[f32; 2]; 4], p: [f32; 2]) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    let mut prev = p0;
    for k in 1..=24 {
        let t = k as f32 / 24.0;
        let q = bezier_at([p0, p1, p2, p3], t);
        let d = seg_dist(p, prev, q);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, t - 0.5 / 24.0));
        }
        prev = q;
    }
    best
}

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-6 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    crate::gfx::dist(p, [a[0] + t * dx, a[1] + t * dy])
}

/// The JACK client PipeWire runs as (on top of jackdbus): "PipeWire", "PipeWire-…".
pub fn is_pipewire(group: &str) -> bool {
    group.to_ascii_lowercase().starts_with("pipewire")
}

fn cut_to(g: &Gfx, px: f32, s: &str, max_w: f32) -> String {
    if g.measure(Face::Regular, px, s) <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        out.push(ch);
        if g.measure(Face::Regular, px, &out) + g.measure(Face::Regular, px, "…") > max_w {
            out.pop();
            break;
        }
    }
    out.push('…');
    out
}

/// Lining up selected blocks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    CentreX,
    Right,
    Top,
    CentreY,
    Bottom,
    /// Equal gaps between them, left to right / top to bottom.
    SpreadX,
    SpreadY,
}

impl App {
    /// Line up the selected blocks (their extents: header and ports). One undo step.
    pub fn align(&mut self, how: Align) {
        let mut items: Vec<(String, Rect)> = self
            .canvas
            .selected
            .iter()
            .filter_map(|id| self.model.node_by_id(id).map(|i| (id.clone(), self.node_extent(i))))
            .collect();
        if items.len() < 2 {
            return;
        }
        self.history.begin(&self.layout);
        // Spreading keeps their order along the axis.
        match how {
            Align::SpreadX => items.sort_by(|a, b| a.1.x.total_cmp(&b.1.x)),
            Align::SpreadY => items.sort_by(|a, b| a.1.y.total_cmp(&b.1.y)),
            _ => {}
        }
        let min = |f: fn(&Rect) -> f32| items.iter().map(|(_, r)| f(r)).fold(f32::MAX, f32::min);
        let max = |f: fn(&Rect) -> f32| items.iter().map(|(_, r)| f(r)).fold(f32::MIN, f32::max);
        let mut moves: Vec<(String, f32, f32)> = Vec::new();
        match how {
            Align::Left => {
                let x = min(|r| r.x);
                moves = items.iter().map(|(id, r)| (id.clone(), x - r.x, 0.0)).collect();
            }
            Align::Right => {
                let x = max(|r| r.right());
                moves = items.iter().map(|(id, r)| (id.clone(), x - r.right(), 0.0)).collect();
            }
            Align::CentreX => {
                let x = items.iter().map(|(_, r)| r.cx()).sum::<f32>() / items.len() as f32;
                moves = items.iter().map(|(id, r)| (id.clone(), x - r.cx(), 0.0)).collect();
            }
            Align::Top => {
                let y = min(|r| r.y);
                moves = items.iter().map(|(id, r)| (id.clone(), 0.0, y - r.y)).collect();
            }
            Align::Bottom => {
                let y = max(|r| r.bottom());
                moves = items.iter().map(|(id, r)| (id.clone(), 0.0, y - r.bottom())).collect();
            }
            Align::CentreY => {
                let y = items.iter().map(|(_, r)| r.cy()).sum::<f32>() / items.len() as f32;
                moves = items.iter().map(|(id, r)| (id.clone(), 0.0, y - r.cy())).collect();
            }
            Align::SpreadX | Align::SpreadY => {
                let across = how == Align::SpreadX;
                let (start, end) = if across { (min(|r| r.x), max(|r| r.right())) } else { (min(|r| r.y), max(|r| r.bottom())) };
                let sizes: f32 = items.iter().map(|(_, r)| if across { r.w } else { r.h }).sum();
                let gap = ((end - start - sizes) / (items.len() - 1) as f32).max(8.0);
                let mut at = start;
                for (id, r) in &items {
                    let (from, size) = if across { (r.x, r.w) } else { (r.y, r.h) };
                    moves.push(if across { (id.clone(), at - from, 0.0) } else { (id.clone(), 0.0, at - from) });
                    at += size + gap;
                }
            }
        }
        for (id, dx, dy) in moves {
            if dx != 0.0 || dy != 0.0 {
                self.move_node(&id, dx, dy);
            }
        }
        self.history.end(&self.layout);
        self.redraw = true;
    }
}

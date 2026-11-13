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
use std::collections::HashSet;
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
        Rect::centered(c[0], c[1], NODE_W * xf.z, NODE_H * xf.z)
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

    /// Each port row of an expanded node: (graph port index, row rectangle in pixels).
    fn port_rows(&self, xf: &Xf, i: usize) -> Vec<(usize, Rect)> {
        let Some(b) = self.body_rect(xf, i) else { return Vec::new() };
        let rh = PORT_ROW * xf.z;
        let top = b.y + BODY_PAD * xf.z;
        self.body_ports(i).into_iter().enumerate().map(|(k, p)| (p, Rect::new(b.x, top + k as f32 * rh, b.w, rh))).collect()
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
        for (mi, m) in self.mats.iter().enumerate().rev() {
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
            if !n.outs_of(&self.graph, self.kind).is_empty() && crate::gfx::dist(so, [x, y]) <= hr {
                return Hover::Out(i);
            }
            if !n.ins_of(&self.graph, self.kind).is_empty() && crate::gfx::dist(si, [x, y]) <= hr {
                return Hover::In(i);
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
                    self.canvas.drag = Drag::Node { id: self.model.nodes[i].key.id(), last: [x, y], raw: self.node_pos(i) }
                }
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
        if pair
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
                // Matrices follow halfway (PatchMatrix): they stay between their two nodes.
                self.move_node(id, target[0] - cur[0], target[1] - cur[1]);
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
            Drag::Link { .. } | Drag::Port { .. } => {
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
        if let Drag::Port { name } = &drag
            && let Some(&a) = self.graph.by_name.get(name)
        {
            self.drop_port(a, self.hit(x, y));
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

    /// A port dragged from an expanded node was dropped: onto a port, toggle that connection;
    /// onto a node, connect to its first compatible port not yet connected to this one.
    fn drop_port(&mut self, a: usize, target: Hover) {
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
            Hover::Port(i, p) => {
                let name = self.graph.ports[p].name.clone();
                items.push(I::header(self.model.labels[p].clone()));
                items.push(I::new("Rename this port…", Action::Rename(format!("port:{name}"))));
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
                items.push(I::new("Show grid", Action::ToggleGrid).checked(self.layout.view.grid));
                items.push(I::new("Snap to grid", Action::ToggleSnap).checked(self.layout.view.snap));
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
        let mut items = vec![I::header(n.title.clone())];
        if find {
            items.push(I::new("Find in canvas", Action::CenterNode(id.clone())));
        }
        items.push(I::new(if split { "Join inputs and outputs" } else { "Split inputs and outputs" }, Action::Split(n.key.group.clone(), !split)));
        items.push(I::new(format!("Disconnect all {}", self.kind.label()), Action::DisconnectNode(id.clone())));
        if self.source == crate::app::Source::Jack && is_pipewire(&n.key.group) {
            items.push(I::new("Open PipeWire's graph", Action::OpenPipeWire));
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
            Hover::Cell(mi, ..) => Some(mi),
            _ => None,
        };
        let dragging_node = match &self.canvas.drag {
            Drag::Node { id, .. } => self.model.node_by_id(id),
            _ => None,
        };
        let dragging_mat = match &self.canvas.drag {
            Drag::Matrix { key, .. } | Drag::Paint { key, .. } => self.mats.iter().position(|m| &m.key == key),
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
        for (mi, m) in self.mats.iter().enumerate() {
            let r = self.matrix_rect(&xf, m);
            let col = if lit_mats.contains(&mi) { mix(wire, [1.0, 1.0, 1.0, 1.0], 0.25) } else { alpha(wire, WIRE_ALPHA) };
            let (out, _) = self.handles(m.src);
            let (_, inp) = self.handles(m.dst);
            let (l0, l1) = (xf.s(out), xf.s(inp));
            let top = [r.cx(), r.y];
            let right = [r.right(), r.cy()];
            let b0 = ((top[0] - l0[0]).abs().max((top[1] - l0[1]).abs()) * 0.5).clamp(bend * 0.4, bend * 2.5);
            let b1 = ((l1[0] - right[0]).abs() * 0.5).clamp(bend * 0.4, bend * 2.5);
            g.bezier(l0, [l0[0] + b0, l0[1]], [top[0], top[1] - b0], top, ww, col);
            g.bezier(right, [right[0] + b1, right[1]], [l1[0] - b1, l1[1]], l1, ww, col);
            g.pie(top, cs, std::f32::consts::PI, std::f32::consts::TAU, col);
            g.pie(right, cs, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2, col);
        }

        // Nodes.
        let flash = self.canvas.flash.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_millis(1500)).map(|(id, _)| id.clone());
        for i in 0..self.model.nodes.len() {
            if !self.visible(i) {
                continue;
            }
            let lit = lit_nodes.contains(&i) || flash.as_deref() == Some(&self.model.nodes[i].key.id());
            self.draw_node(g, &xf, i, lit);
        }

        // Matrices.
        for mi in 0..self.mats.len() {
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
            && let Some(&ni) = self.model.port_node.get(&a)
            && let Some((_, row)) = self.port_rows(&xf, ni).into_iter().find(|(p, _)| *p == a)
        {
            let out = self.graph.ports[a].is_output();
            let p0 = if out { [row.right(), row.cy()] } else { [row.x, row.cy()] };
            let p1 = self.canvas.mouse;
            let b = if out { bend } else { -bend };
            g.bezier(p0, [p0[0] + b, p0[1]], [p1[0] - b, p1[1]], p1, ww * 1.4, kind_color(self.graph.ports[a].kind));
        }

        // Port names around matrices: the hovered one, or all of them.
        for mi in 0..self.mats.len() {
            if self.layout.view.labels || Some(mi) == hov_mat {
                self.draw_matrix_labels(g, &xf, mi);
            }
        }
        if let Hover::Cell(mi, c, r) = self.canvas.hover
            && matches!(self.canvas.drag, Drag::None | Drag::Paint { .. }) {
                self.draw_cell_tip(g, &xf, mi, c, r);
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
        let dw = 1.5 * lh;
        let ww = r.w - 2.0 * (dx + dw);
        let ty = r.cy() - lh / 2.0;
        let tw = g.measure(Face::Regular, px, &n.title).min(ww);
        g.text_fit(Face::Regular, px, r.cx() - tw / 2.0, ty, ww, alpha(BLOCK_TEXT, f), &n.title);
        // PatchMatrix's port counts for the current type, either side.
        if nout > 0 {
            let s = format!("{nout:02}");
            let w = g.measure(Face::Regular, px, &s);
            g.text(Face::Regular, px, r.right() - w - dx - lw, ty, alpha(BLOCK_TEXT, f * 0.7), &s);
        }
        if nin > 0 {
            let s = format!("{nin:02}");
            g.text(Face::Regular, px, r.x + dx + lw, ty, alpha(BLOCK_TEXT, f * 0.7), &s);
        }
        if let Some(sub) = &n.sub {
            let spx = FONT_SMALL * z;
            let sh = g.line_height(Face::Regular, spx);
            g.text(Face::Regular, spx, r.x + 2.0 * z, r.y - sh - 1.0 * z, alpha(TEXT_DIM, f), sub);
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
            if k == n_in && k > 0 {
                g.rect(Rect::new(row.x + 6.0 * z, row.y.round(), row.w - 12.0 * z, self.scale.max(1.0)), PANEL_LINE);
            }
            let hovered = self.canvas.hover == Hover::Port(i, p);
            if dragged.is_some_and(|a| self.compatible(a, p)) {
                g.rect(row.inset(1.0), alpha(HILIGHT, if hovered { 0.30 } else { 0.12 }));
            } else if hovered {
                g.rect(row.inset(1.0), [1.0, 1.0, 1.0, 0.07]);
            }
            let sw = (3.0 * z).max(2.0);
            let strip = if input {
                Rect::new(row.x + 2.0 * z, row.y + 2.0 * z, sw, row.h - 4.0 * z)
            } else {
                Rect::new(row.right() - 2.0 * z - sw, row.y + 2.0 * z, sw, row.h - 4.0 * z)
            };
            g.rect(strip, alpha(kind_color(port.kind), fa));
            // Stereo pairs: the strip continues across both rows.
            if let Some(&q) = self.model.partner.get(&p)
                && rows.get(k + 1).is_some_and(|(n, _)| *n == q)
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
            let col = alpha(if hovered { BLOCK_TEXT } else { mix(BLOCK_TEXT, TEXT_BRIGHT, 0.15) }, fa);
            if input {
                g.text_fit(Face::Regular, px, dot[0] + 6.0 * z, row.cy() - lh / 2.0, max_w, col, label);
            } else {
                let w = g.measure(Face::Regular, px, label).min(max_w);
                g.text_fit(Face::Regular, px, dot[0] - 6.0 * z - w, row.cy() - lh / 2.0, max_w, col, label);
            }
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

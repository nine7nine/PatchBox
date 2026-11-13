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
    },
    Matrix {
        key: String,
        last: [f32; 2],
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
        }
        Hover::None
    }

    // ---- input ----

    pub fn canvas_down(&mut self, button: sdl3::mouse::MouseButton, x: f32, y: f32, ctrl: bool, shift: bool) {
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
                        self.canvas.drag = Drag::Matrix { key, last: [x, y] };
                    } else {
                        let (o, i) = (self.mats[mi].cols[c], self.mats[mi].rows[r]);
                        let connect = !self.mats[mi].conns.contains(&(o, i));
                        self.pin(self.mats[mi].src, self.mats[mi].dst);
                        let mut done = HashSet::new();
                        self.paint_cell(mi, c, r, connect, shift, &mut done);
                        self.canvas.drag = Drag::Paint { key, connect, pair: shift, done };
                    }
                }
                Hover::Out(i) => self.canvas.drag = Drag::Link { src: self.model.nodes[i].key.id() },
                Hover::Node(i) if shift && !self.model.nodes[i].outs_of(&self.graph, self.kind).is_empty() => {
                    self.canvas.drag = Drag::Link { src: self.model.nodes[i].key.id() }
                }
                Hover::Node(i) | Hover::In(i) => self.canvas.drag = Drag::Node { id: self.model.nodes[i].key.id(), last: [x, y] },
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
            Drag::Node { id, last } => {
                let (dx, dy) = ((x - last[0]) / z, (y - last[1]) / z);
                *last = [x, y];
                if let Some(p) = self.layout.nodes.get_mut(id.as_str()) {
                    p[0] += dx;
                    p[1] += dy;
                }
                // Matrices follow halfway (PatchMatrix): they stay between their two nodes.
                let src_pre = format!("{id}>");
                let dst_suf = format!(">{id}");
                for (k, p) in self.layout.matrices.iter_mut() {
                    let (a, b) = (k.starts_with(&src_pre), k.ends_with(&dst_suf));
                    let f = if a && b { 1.0 } else if a || b { 0.5 } else { 0.0 };
                    p[0] += dx * f;
                    p[1] += dy * f;
                }
                self.layout_changed();
            }
            Drag::Matrix { key, last } => {
                if let Some(p) = self.layout.matrices.get_mut(key.as_str()) {
                    p[0] += (x - last[0]) / z;
                    p[1] += (y - last[1]) / z;
                }
                *last = [x, y];
                self.layout_changed();
            }
            Drag::Link { .. } => {
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

    pub fn canvas_wheel(&mut self, dx: f32, dy: f32, ctrl: bool) {
        let [mx, my] = self.canvas.mouse;
        if ctrl {
            let before = self.xf().w([mx, my]);
            let zoom = (self.layout.view.zoom * 1.12f32.powf(dy)).clamp(0.25, 3.0);
            self.layout.view.zoom = zoom;
            let z = self.z();
            self.layout.view.cam = [before[0] - (mx - self.canvas.area.x) / z, before[1] - (my - self.canvas.area.y) / z];
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
            Hover::Node(i) | Hover::In(i) | Hover::Out(i) => items = self.node_menu(i, false),
            Hover::None => {
                items.push(I::new("Arrange (signal chain)", Action::Arrange));
                items.push(I::new("Fit to window", Action::FitAll));
                items.push(I::new("Reset zoom", Action::ResetZoom));
                items.push(I::new("Show hidden clients", Action::ShowAll).enabled(!self.layout.hidden.is_empty()));
                items.push(I::new("Port names on matrices", Action::ToggleLabels).checked(self.layout.view.labels));
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
        if gs >= 6.0 {
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

        if self.jack.is_none() {
            let msg = "Waiting for a JACK server…";
            g.text_center(Face::Regular, FONT * self.scale * 1.2, a, TEXT_DIM, msg);
            g.pop_clip();
            return;
        }

        // Which nodes and matrices light up: hovered ones and their neighbours.
        let hov_node = match self.canvas.hover {
            Hover::Node(i) | Hover::In(i) | Hover::Out(i) => Some(i),
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

        // Wires (under everything).
        let ww = (1.25 * self.scale).max(1.0);
        let bend = WIRE_BEND * z;
        let cs = HANDLE_R * z;
        for (mi, m) in self.mats.iter().enumerate() {
            let r = self.matrix_rect(&xf, m);
            let col = if lit_mats.contains(&mi) { HILIGHT } else { HANDLE };
            let (out, _) = self.handles(m.src);
            let (_, inp) = self.handles(m.dst);
            let (l0, l1) = (xf.s(out), xf.s(inp));
            let top = [r.cx(), r.y];
            let right = [r.right(), r.cy()];
            g.bezier(l0, [l0[0] + bend, l0[1]], [top[0], top[1] - bend], top, ww, col);
            g.bezier(right, [right[0] + bend, right[1]], [l1[0] - bend, l1[1]], l1, ww, col);
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
        let editable = nin + nout > 0;
        let f = if editable { 1.0 } else { 0.33 };
        let stroke = if lit { HILIGHT } else { BORDER };
        g.rrect(r, NODE_RADIUS * z, alpha(NODE_FILL, f));
        g.rrect_stroke(r, NODE_RADIUS * z, self.scale.max(1.0), alpha(stroke, f));
        let px = FONT * z;
        let lh = g.line_height(Face::Regular, px);
        let dx = 4.0 * z;
        let dw = 1.5 * lh;
        let ww = r.w - 2.0 * (dx + dw);
        let ty = r.cy() - lh / 2.0;
        let tw = g.measure(Face::Regular, px, &n.title).min(ww);
        g.text_fit(Face::Regular, px, r.cx() - tw / 2.0, ty, ww, alpha(TEXT, f), &n.title);
        if nout > 0 {
            let s = format!("{nout:02}");
            let w = g.measure(Face::Regular, px, &s);
            g.text(Face::Regular, px, r.right() - w - dx, ty, alpha(TEXT, f), &s);
        }
        if nin > 0 {
            let s = format!("{nin:02}");
            g.text(Face::Regular, px, r.x + dx, ty, alpha(TEXT, f), &s);
        }
        if let Some(sub) = &n.sub {
            let spx = FONT_SMALL * z;
            let sh = g.line_height(Face::Regular, spx);
            g.text(Face::Regular, spx, r.x + 2.0 * z, r.y - sh - 1.0 * z, alpha(TEXT_DIM, f), sub);
        }
        // Handles.
        let cw = HANDLE_R * z;
        let col = if lit { HILIGHT } else { HANDLE };
        let (out, inp) = self.handles(i);
        if nout > 0 {
            let p = xf.s(out);
            g.circle(p, cw, col);
            let hot = self.canvas.hover == crate::canvas::Hover::Out(i) && matches!(self.canvas.drag, Drag::None)
                || matches!(&self.canvas.drag, Drag::Link { src } if *src == n.key.id());
            if hot {
                g.arc(p, 2.0 * cw, 0.0, std::f32::consts::TAU, self.scale.max(1.0), HILIGHT);
            }
        }
        if nin > 0 {
            let p = xf.s(inp);
            g.circle(p, cw, col);
            let linking = matches!(self.canvas.drag, Drag::Link { .. });
            let hot = linking && matches!(self.canvas.hover, Hover::Node(h) | Hover::In(h) | Hover::Out(h) if h == i);
            if hot {
                g.arc(p, 2.0 * cw, 0.0, std::f32::consts::TAU, self.scale.max(1.0), HILIGHT);
            }
        }
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

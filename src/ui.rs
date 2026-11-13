//! The window's chrome around the canvas: the top bar (it is also the titlebar), the sidebar,
//! popup menus. Immediate mode: each frame draws and records the clickable rectangles with the
//! action each one maps to.

use crate::app::{Action, App};
use crate::csd::{Button, Csd};
use crate::gfx::{Gfx, Rect, alpha};
use crate::text::Face;
use crate::theme::*;
use pbx_jack::PortKind;

pub struct MenuItem {
    pub label: String,
    pub action: Option<Action>,
    pub enabled: bool,
    pub checked: Option<bool>,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, action: Action) -> MenuItem {
        MenuItem { label: label.into(), action: Some(action), enabled: true, checked: None }
    }
    pub fn header(label: impl Into<String>) -> MenuItem {
        MenuItem { label: label.into(), action: None, enabled: false, checked: None }
    }
    pub fn enabled(mut self, on: bool) -> MenuItem {
        self.enabled = on;
        self
    }
    pub fn checked(mut self, on: bool) -> MenuItem {
        self.checked = Some(on);
        self
    }
}

pub struct Menu {
    pub at: [f32; 2],
    pub items: Vec<MenuItem>,
    pub rect: Rect,
    /// Each item's row, as last drawn.
    pub rows: Vec<Rect>,
    pub hover: Option<usize>,
}

pub struct UiState {
    pub csd: Csd,
    pub csd_hover: Option<Button>,
    pub menu: Option<Menu>,
    /// Clickable rectangles of this frame, front-most last.
    pub hits: Vec<(Rect, Action)>,
    pub hover_hit: Option<usize>,
    pub mouse: [f32; 2],
    pub sidebar_scroll: f32,
    pub sidebar_rect: Rect,
    pub bar_h: f32,
    /// The zoom slider's track while it is being dragged.
    pub slider: Option<Rect>,
}

impl UiState {
    pub fn new() -> UiState {
        UiState {
            csd: Csd::from_system(),
            csd_hover: None,
            menu: None,
            hits: Vec::new(),
            hover_hit: None,
            mouse: [0.0, 0.0],
            sidebar_scroll: 0.0,
            sidebar_rect: Rect::default(),
            bar_h: 0.0,
            slider: None,
        }
    }

    pub fn open_menu(&mut self, at: [f32; 2], items: Vec<MenuItem>) {
        self.menu = Some(Menu { at, items, rect: Rect::default(), rows: Vec::new(), hover: None });
    }

    pub fn open_buffer_menu(&mut self, under: Rect, current: u32) {
        let items = [16u32, 32, 64, 128, 256, 512, 1024, 2048, 4096]
            .into_iter()
            .map(|n| MenuItem::new(format!("{n} frames"), Action::SetBufferSize(n)).checked(n == current))
            .collect();
        self.open_menu([under.x, under.bottom() + 2.0], items);
    }

    pub fn open_scale_menu(&mut self, at: Rect, current: f32) {
        let items = [1.0f32, 1.1, 1.25, 1.5, 1.75, 2.0]
            .into_iter()
            .map(|f| MenuItem::new(format!("{:.0} %", f * 100.0), Action::SetUiScale(f)).checked((f - current).abs() < 0.01))
            .collect();
        self.open_menu([at.x, at.y], items);
    }

    pub fn open_app_menu(&mut self, under: Rect, labels: bool, sidebar: bool, ui_scale: f32, grid: bool, snap: bool) {
        let items = vec![
            MenuItem::new(format!("Interface scale: {:.0} % …", ui_scale * 100.0), Action::ScaleMenu(Rect::new(under.x, under.bottom() + 2.0, 0.0, 0.0))),
            MenuItem::new("Undo (Ctrl+Z)", Action::Undo),
            MenuItem::new("Redo (Ctrl+Shift+Z)", Action::Redo),
            MenuItem::new("Show sidebar", Action::ToggleSidebar).checked(sidebar),
            MenuItem::new("Views…", Action::ViewsPage),
            MenuItem::new("Matrices (PatchMatrix view, M)", Action::ToggleMatrices),
            MenuItem::new("Vertical flow (V)", Action::ToggleVertical),
            MenuItem::new("Port names on matrices", Action::ToggleLabels).checked(labels),
            MenuItem::new("Show grid (G)", Action::ToggleGrid).checked(grid),
            MenuItem::new("Snap to grid (Shift+G)", Action::ToggleSnap).checked(snap),
            MenuItem::new("Arrange (signal chain)", Action::Arrange),
            MenuItem::new("Fit to window", Action::FitAll),
            MenuItem::new("Show hidden clients", Action::ShowAll),
            MenuItem::new("Freewheel", Action::ToggleFreewheel),
            MenuItem::new("Quit", Action::Quit),
        ];
        self.open_menu([under.x, under.bottom() + 2.0], items);
    }
}

impl App {
    /// Lay out the frame's regions (pixels).
    pub fn layout_regions(&mut self) {
        let s = self.scale;
        let bar_h = (TOPBAR_H * s).round();
        self.ui.bar_h = bar_h;
        let side_w = if self.layout.view.sidebar { (SIDEBAR_W * s).round() } else { 0.0 };
        self.ui.sidebar_rect = Rect::new(0.0, bar_h, side_w, self.win[1] - bar_h);
        self.canvas.area = Rect::new(side_w, bar_h, self.win[0] - side_w, self.win[1] - bar_h);
    }

    pub fn add_hit_pub(&mut self, r: Rect, a: Action) {
        self.add_hit(r, a);
    }

    fn add_hit(&mut self, r: Rect, a: Action) {
        self.ui.hits.push((r, a));
    }

    fn hovered(&self, r: &Rect) -> bool {
        self.ui.menu.is_none() && r.contains(self.ui.mouse[0], self.ui.mouse[1])
    }

    fn button(&mut self, g: &mut Gfx, r: Rect, label: &str, active: bool, action: Action) {
        let s = self.scale;
        let hov = self.hovered(&r);
        g.rrect(r, 3.0 * s, if hov { BUTTON_HOVER } else { BUTTON });
        g.rrect_stroke(r, 3.0 * s, s.max(1.0), if active { HILIGHT } else { alpha(BORDER, 0.8) });
        g.text_center(Face::Regular, FONT * s, r, if active { TEXT_BRIGHT } else { TEXT }, label);
        self.add_hit(r, action);
    }

    /// The chrome; the frame's clickable rectangles were cleared before the canvas drew (it adds
    /// its own: the PipeWire breadcrumb).
    pub fn draw_ui(&mut self, g: &mut Gfx) {
        self.draw_topbar(g);
        if self.layout.view.sidebar {
            self.draw_sidebar(g);
        }
        if self.jack.is_some() {
            self.draw_zoom(g);
            self.draw_transport(g);
        }
    }

    /// Transport, bottom-left of the canvas (the toolbar row takes it in the UI pass):
    /// rewind, play/pause, stop; time; bars|beats|ticks and tempo when a timebase master gives them.
    fn draw_transport(&mut self, g: &mut Gfx) {
        let s = self.scale;
        let a = self.canvas.area;
        let t = self.stats.transport;
        let h = 26.0 * s;
        let px = FONT * s;
        let secs = t.frame as f64 / t.rate.max(1) as f64;
        let time = format!("{}:{:02}:{:04.1}", (secs / 3600.0) as u32, ((secs / 60.0) as u32) % 60, secs % 60.0);
        let bbt = t.bbt.map(|(bar, beat, tick, bpm, bpb, bt)| format!("{bar:>3}|{beat}|{tick:04}  {bpm:.1} BPM  {bpb}/{bt}"));
        let text = match &bbt {
            Some(b) => format!("{time}   {b}"),
            None => time,
        };
        let tw = g.measure(Face::Regular, px, &text);
        let bw = h;
        let w = 3.0 * bw + 3.0 * 4.0 * s + tw + 22.0 * s;
        let panel = Rect::new(a.x + 12.0 * s, a.bottom() - h - 12.0 * s, w, h);
        g.rrect(panel, 4.0 * s, alpha(BAR_BG, 0.9));
        g.rrect_stroke(panel, 4.0 * s, s.max(1.0), alpha(BORDER, 0.5));
        let mut x = panel.x + 4.0 * s;
        let playing = t.rolling || t.starting;
        for (i, act) in [Action::TransportRewind, Action::TransportToggle, Action::TransportStop].into_iter().enumerate() {
            let r = Rect::new(x, panel.y, bw, h);
            x += bw + 4.0 * s;
            if self.hovered(&r) {
                g.rrect(r.inset(2.0 * s), 3.0 * s, BUTTON_HOVER);
            }
            let (cx, cy, k) = (r.cx(), r.cy(), 5.5 * s);
            let col = if i == 1 && playing { OK } else { TEXT_BRIGHT };
            match i {
                0 => {
                    g.rect(Rect::new(cx - k, cy - k, 1.8 * s, 2.0 * k), col);
                    g.triangle([cx + k, cy - k], [cx + k, cy + k], [cx - k + 2.0 * s, cy], col);
                }
                1 if playing => {
                    g.rect(Rect::new(cx - k * 0.8, cy - k, k * 0.6, 2.0 * k), col);
                    g.rect(Rect::new(cx + k * 0.2, cy - k, k * 0.6, 2.0 * k), col);
                }
                1 => g.triangle([cx - k * 0.7, cy - k], [cx - k * 0.7, cy + k], [cx + k, cy], col),
                _ => g.rect(Rect::new(cx - k * 0.85, cy - k * 0.85, k * 1.7, k * 1.7), col),
            }
            self.add_hit(r, act);
        }
        let lh = g.line_height(Face::Regular, px);
        g.text(Face::Regular, px, x + 6.0 * s, panel.cy() - lh / 2.0, if playing { TEXT_BRIGHT } else { TEXT }, &text);
    }

    /// Zoom controls, bottom-right of the canvas: − [slider] + and the zoom (click: 100 %).
    fn draw_zoom(&mut self, g: &mut Gfx) {
        let s = self.scale;
        let a = self.canvas.area;
        let h = 26.0 * s;
        let bw = h;
        let track_w = 120.0 * s;
        let pct = format!("{:.0} %", self.layout.view.zoom * 100.0);
        let pw = g.measure(Face::Regular, FONT * s, "400 %") + 12.0 * s;
        let w = bw * 2.0 + track_w + pw + 4.0 * 6.0 * s;
        let panel = Rect::new(a.right() - w - 12.0 * s, a.bottom() - h - 12.0 * s, w, h);
        g.rrect(panel, 4.0 * s, alpha(BAR_BG, 0.9));
        g.rrect_stroke(panel, 4.0 * s, s.max(1.0), alpha(BORDER, 0.5));
        let mut x = panel.x + 6.0 * s;
        let minus = Rect::new(x, panel.y, bw, h);
        x += bw + 6.0 * s;
        let track = Rect::new(x, panel.y, track_w, h);
        x += track_w + 6.0 * s;
        let plus = Rect::new(x, panel.y, bw, h);
        x += bw + 6.0 * s;
        let pr = Rect::new(x, panel.y, pw, h);
        for (r, sign) in [(minus, false), (plus, true)] {
            if self.hovered(&r) {
                g.rrect(r.inset(3.0 * s), 3.0 * s, BUTTON_HOVER);
            }
            let (cx, cy, l) = (r.cx().round(), r.cy().round(), (5.0 * s).round());
            let t = (1.6 * s).max(1.0).round();
            g.rect(Rect::new(cx - l, cy - (t / 2.0).floor(), 2.0 * l, t), TEXT_BRIGHT);
            if sign {
                g.rect(Rect::new(cx - (t / 2.0).floor(), cy - l, t, 2.0 * l), TEXT_BRIGHT);
            }
        }
        self.add_hit(minus, Action::ZoomBy(1.0 / 1.2));
        self.add_hit(plus, Action::ZoomBy(1.2));
        // Track: a line, a tick at 100 %, the knob.
        let ty = track.cy().round();
        let lw = (2.0 * s).max(1.0).round();
        g.rect(Rect::new(track.x, ty - lw / 2.0, track.w, lw), PANEL_LINE);
        let one = track.x + App::zoom_to_t(1.0) * track.w;
        g.rect(Rect::new(one.round(), ty - 5.0 * s, s.max(1.0), 10.0 * s), TEXT_DIM);
        let kx = track.x + App::zoom_to_t(self.layout.view.zoom) * track.w;
        let active = self.ui.slider.is_some() || self.hovered(&track);
        g.circle([kx, ty], 6.0 * s, if active { HILIGHT } else { BORDER });
        self.add_hit(track, Action::ZoomSlider(track));
        g.text_center(Face::Regular, FONT * s, pr, if self.hovered(&pr) { TEXT_BRIGHT } else { TEXT }, &pct);
        self.add_hit(pr, Action::ZoomTo(1.0));
    }

    fn draw_topbar(&mut self, g: &mut Gfx) {
        let s = self.scale;
        let bar = Rect::new(0.0, 0.0, self.win[0], self.ui.bar_h);
        g.rect(bar, BAR_BG);
        g.rect(Rect::new(0.0, bar.bottom() - s.max(1.0), bar.w, s.max(1.0)), PANEL_LINE);
        let strip = self.ui.csd.strip_w(self.chrome);
        let (mut x, right_edge) = if self.ui.csd.left { (strip + 6.0 * s, bar.w - 8.0 * s) } else { (8.0 * s, bar.w - strip - 6.0 * s) };
        let bh = (TOPBAR_H - 10.0) * s;
        let by = (bar.h - bh) / 2.0;

        // App menu (three bars).
        let mr = Rect::new(x, by, bh, bh);
        let hov = self.hovered(&mr);
        g.rrect(mr, 3.0 * s, if hov { BUTTON_HOVER } else { BAR_BG });
        for k in -1..=1 {
            let y = (mr.cy() + k as f32 * 4.0 * s - 0.8 * s).round();
            g.rect(Rect::new((mr.cx() - 6.0 * s).round(), y, (12.0 * s).round(), (1.6 * s).max(1.0).round()), TEXT);
        }
        self.add_hit(mr, Action::AppMenu(mr));
        x += bh + 8.0 * s;

        // Port-type tabs (PatchMatrix's top row).
        let tab_w = 74.0 * s;
        for k in self.kinds() {
            let r = Rect::new(x, by, tab_w, bh);
            let active = self.kind == k;
            let hov = self.hovered(&r);
            g.rrect(r, 3.0 * s, if hov { BUTTON_HOVER } else { BUTTON });
            g.rrect_stroke(r, 3.0 * s, s.max(1.0), if active { HILIGHT } else { alpha(BORDER, 0.8) });
            g.circle([r.x + 10.0 * s, r.cy()], 3.0 * s, kind_color(k));
            g.text_center(Face::Regular, FONT * s, Rect::new(r.x + 14.0 * s, r.y, r.w - 14.0 * s, r.h), if active { TEXT_BRIGHT } else { TEXT }, k.label());
            self.add_hit(r, Action::SetKind(k));
            x += tab_w + 4.0 * s;
        }
        let tabs_end = x;

        // Right side, right to left: DSP, xruns, buffer, sample rate.
        let mut rx = right_edge;
        let st = self.stats;
        let connected = self.jack.is_some();
        // DSP meter.
        let dw = 92.0 * s;
        let dr = Rect::new(rx - dw, by, dw, bh);
        g.rrect(dr, 3.0 * s, BUTTON);
        if connected {
            let frac = (st.dsp / 100.0).clamp(0.0, 1.0);
            let col = if st.dsp < 50.0 { OK } else if st.dsp < 80.0 { WARN } else { BAD };
            g.rrect(Rect::new(dr.x, dr.y, (dr.w * frac).max(2.0 * s), dr.h), 3.0 * s, alpha(col, 0.45));
        }
        g.rrect_stroke(dr, 3.0 * s, s.max(1.0), alpha(BORDER, 0.8));
        let dsp = if connected { format!("DSP {:>4.1}%", st.dsp) } else { "DSP  –".into() };
        g.text_center(Face::Regular, FONT * s, dr, TEXT_BRIGHT, &dsp);
        rx -= dw + 6.0 * s;
        // Xruns (click: reset).
        let xs = format!("Xruns {}", st.xruns);
        let xw = g.measure(Face::Regular, FONT * s, &xs) + 18.0 * s;
        let xr = Rect::new(rx - xw, by, xw, bh);
        self.button(g, xr, &xs, st.xruns > 0, Action::ResetXruns);
        rx -= xw + 6.0 * s;
        // Buffer size (menu).
        let bs = if connected { format!("Buffer {} ▼", st.buffer_size) } else { "Buffer – ▼".into() };
        let bw = g.measure(Face::Regular, FONT * s, &bs) + 18.0 * s;
        let br = Rect::new(rx - bw, by, bw, bh);
        self.button(g, br, &bs, false, Action::BufferMenu(br));
        rx -= bw + 6.0 * s;
        // Sample rate, latency.
        let sr = if connected {
            let ms = st.buffer_size as f32 * 1000.0 / st.sample_rate.max(1) as f32;
            format!("{} Hz  {:.1} ms{}", st.sample_rate, ms, if st.freewheel { "  FREEWHEEL" } else { "" })
        } else {
            "JACK offline".into()
        };
        let sw = g.measure(Face::Regular, FONT * s, &sr);
        let col = if connected { if st.freewheel { WARN } else { TEXT } } else { BAD };
        let lh = g.line_height(Face::Regular, FONT * s);
        g.text(Face::Regular, FONT * s, rx - sw - 4.0 * s, bar.cy() - lh / 2.0, col, &sr);
        rx -= sw + 14.0 * s;

        // Title, centred in what's left.
        let title_r = Rect::new(tabs_end + 8.0 * s, 0.0, (rx - tabs_end - 16.0 * s).max(0.0), bar.h);
        if title_r.w > 60.0 * s {
            g.text_center(Face::Bold, FONT * s, title_r, TEXT_DIM, "PatchBox");
        }

        self.ui.csd.draw(g, bar.w, bar.h, self.chrome, self.ui.csd_hover);
    }

    fn draw_sidebar(&mut self, g: &mut Gfx) {
        let s = self.scale;
        let r = self.ui.sidebar_rect;
        g.rect(r, PANEL_BG);
        g.rect(Rect::new(r.right() - s.max(1.0), r.y, s.max(1.0), r.h), PANEL_LINE);
        let pad = 10.0 * s;
        let px = FONT * s;
        let lh = g.line_height(Face::Regular, px);
        let mut y = r.y + pad;

        // The JACK server (jackdbus).
        let st = self.server.status.clone();
        let (dot, state) = if !st.available {
            (TEXT_DIM, "jackdbus not available".to_string())
        } else if self.server.busy {
            (WARN, "working…".to_string())
        } else if st.started {
            (OK, format!("running · {} Hz · {}", self.stats.sample_rate, self.stats.buffer_size))
        } else {
            (BAD, "stopped".to_string())
        };
        g.text(Face::Bold, px, r.x + pad, y, TEXT_BRIGHT, "JACK");
        g.circle([r.x + pad + g.measure(Face::Bold, px, "JACK") + 10.0 * s, y + lh / 2.0], 4.0 * s, dot);
        g.text_fit(Face::Regular, FONT_SMALL * s, r.x + pad + g.measure(Face::Bold, px, "JACK") + 20.0 * s, y + 1.0 * s, r.w - 80.0 * s, TEXT, &state);
        y += lh + 8.0 * s;
        let bh = (lh + 10.0 * s).round();
        let bw = ((r.w - 2.0 * pad - 12.0 * s) / 3.0).floor();
        let mut bx = r.x + pad;
        if st.available {
            let (label, act) = if st.started { ("Stop", Action::ServerStop) } else { ("Start", Action::ServerStart) };
            self.button(g, Rect::new(bx, y, bw, bh), label, false, act);
            bx += bw + 6.0 * s;
            self.button(g, Rect::new(bx, y, bw, bh), "Restart", false, Action::ServerRestart);
            bx += bw + 6.0 * s;
        }
        self.button(g, Rect::new(bx, y, bw, bh), "Settings", false, Action::ServerSettings);
        y += bh + 12.0 * s;
        g.rect(Rect::new(r.x + pad, y, r.w - 2.0 * pad, s.max(1.0)), PANEL_LINE);
        y += 8.0 * s;

        // The session (patchboxd).
        y = self.draw_session_panel(g, r, y);
        y += 4.0 * s;
        g.rect(Rect::new(r.x + pad, y, r.w - 2.0 * pad, s.max(1.0)), PANEL_LINE);
        y += 8.0 * s;

        // Clients.
        let n_nodes = self.model.nodes.len();
        g.text(Face::Bold, px, r.x + pad, y, TEXT_BRIGHT, &format!("Clients ({n_nodes})"));
        y += lh + 6.0 * s;
        let log_h = (6.0 * (lh + 2.0 * s) + 2.0 * pad).min(r.h * 0.35);
        let list = Rect::new(r.x, y, r.w, r.bottom() - log_h - y);
        let row_h = (lh + 8.0 * s).round();
        let content_h = n_nodes as f32 * row_h;
        self.ui.sidebar_scroll = self.ui.sidebar_scroll.clamp(0.0, (content_h - list.h).max(0.0));
        g.push_clip(list);
        let mut ry = list.y - self.ui.sidebar_scroll;
        let mut order: Vec<usize> = (0..n_nodes).collect();
        order.sort_by_key(|&i| (self.model.nodes[i].title.to_lowercase(), self.model.nodes[i].key.role));
        for i in order {
            let n = &self.model.nodes[i];
            let rr = Rect::new(list.x + 4.0 * s, ry, list.w - 8.0 * s, row_h);
            ry += row_h;
            if rr.bottom() < list.y || rr.y > list.bottom() {
                continue;
            }
            let id = n.key.id();
            let hidden = self.layout.hidden.contains(&id);
            let hov = self.hovered(&rr) && list.contains(self.ui.mouse[0], self.ui.mouse[1]);
            if hov {
                g.rrect(rr, 3.0 * s, BUTTON_HOVER);
            }
            // Port types this node has.
            let mut dx = rr.x + 8.0 * s;
            for k in self.kinds() {
                if n.has_kind(&self.graph, k) {
                    g.circle([dx, rr.cy()], 2.6 * s, kind_color(k));
                    dx += 7.0 * s;
                }
            }
            let tx = rr.x + 8.0 * s + 4.0 * 7.0 * s + 2.0 * s;
            let label = match &n.sub {
                Some(sub) => format!("{} ({sub})", n.title),
                None => n.title.clone(),
            };
            let col = if hidden { TEXT_DIM } else { TEXT };
            let counts = format!("{}/{}", n.ins.len(), n.outs.len());
            let cw = g.measure(Face::Regular, FONT_SMALL * s, &counts);
            g.text_fit(Face::Regular, px, tx, rr.cy() - lh / 2.0, rr.right() - tx - cw - 10.0 * s, col, &label);
            let slh = g.line_height(Face::Regular, FONT_SMALL * s);
            g.text(Face::Regular, FONT_SMALL * s, rr.right() - cw - 6.0 * s, rr.cy() - slh / 2.0, TEXT_DIM, &counts);
            let vis = rr.intersect(&list);
            self.add_hit(vis, Action::CenterNode(id));
        }
        g.pop_clip();

        // Log.
        let lr = Rect::new(r.x, r.bottom() - log_h, r.w, log_h);
        g.rect(Rect::new(lr.x + pad, lr.y, lr.w - 2.0 * pad, s.max(1.0)), PANEL_LINE);
        let spx = FONT_SMALL * s;
        let slh = g.line_height(Face::Regular, spx) + 2.0 * s;
        let fit = ((lr.h - pad) / slh) as usize;
        let start = self.log.len().saturating_sub(fit);
        let mut ly = lr.y + pad * 0.6;
        for l in &self.log[start..] {
            let col = if l.bad { WARN } else { TEXT_DIM };
            g.text_fit(Face::Regular, spx, lr.x + pad, ly, lr.w - 2.0 * pad, col, &l.text);
            ly += slh;
        }
    }

    pub fn draw_menu(&mut self, g: &mut Gfx) {
        let s = self.scale;
        let Some(menu) = self.ui.menu.as_mut() else { return };
        let px = FONT * s;
        let lh = g.line_height(Face::Regular, px);
        let row_h = (lh + 10.0 * s).round();
        let pad = 6.0 * s;
        let check_w = 18.0 * s;
        let w = menu.items.iter().map(|i| g.measure(Face::Regular, px, &i.label)).fold(120.0 * s, f32::max) + 2.0 * pad + check_w + 10.0 * s;
        let h = menu.items.len() as f32 * row_h + 2.0 * pad;
        let mut x = menu.at[0];
        let mut y = menu.at[1];
        if x + w > self.win[0] {
            x = (self.win[0] - w - 4.0 * s).max(0.0);
        }
        if y + h > self.win[1] {
            y = (self.win[1] - h - 4.0 * s).max(0.0);
        }
        let rect = Rect::new(x, y, w, h);
        menu.rect = rect;
        let (mx, my) = (self.ui.mouse[0], self.ui.mouse[1]);
        menu.hover = None;
        menu.rows.clear();
        g.rrect(Rect::new(rect.x + 3.0 * s, rect.y + 4.0 * s, rect.w, rect.h), 5.0 * s, [0.0, 0.0, 0.0, 0.35]);
        g.rrect(rect, 5.0 * s, MENU_BG);
        g.rrect_stroke(rect, 5.0 * s, s.max(1.0), alpha(BORDER, 0.7));
        for (i, item) in menu.items.iter().enumerate() {
            let rr = Rect::new(rect.x + pad / 2.0, rect.y + pad + i as f32 * row_h, rect.w - pad, row_h);
            menu.rows.push(rr);
            let active = item.enabled && item.action.is_some();
            if active && rr.contains(mx, my) {
                menu.hover = Some(i);
                g.rrect(rr, 3.0 * s, BUTTON_HOVER);
            }
            let col = if item.action.is_none() {
                HILIGHT
            } else if item.enabled {
                TEXT_BRIGHT
            } else {
                TEXT_DIM
            };
            let face = if item.action.is_none() { Face::Bold } else { Face::Regular };
            if item.checked == Some(true) {
                g.circle([rr.x + pad + 5.0 * s, rr.cy()], 3.5 * s, TOGGLE);
            } else if item.checked == Some(false) {
                g.arc([rr.x + pad + 5.0 * s, rr.cy()], 3.5 * s, 0.0, std::f32::consts::TAU, s.max(1.0), TEXT_DIM);
            }
            g.text_fit(face, px, rr.x + pad + check_w, rr.cy() - lh / 2.0, rr.w - check_w - 2.0 * pad, col, &item.label);
        }
    }

    /// A box of text lines near `at` (window pixels), kept inside the window.
    pub fn draw_tooltip(&self, g: &mut Gfx, at: [f32; 2], lines: &[(String, crate::gfx::Color)]) {
        let s = self.scale;
        let px = FONT * s;
        let lh = g.line_height(Face::Regular, px) + 2.0 * s;
        let pad = 7.0 * s;
        let w = lines.iter().map(|(t, _)| g.measure(Face::Regular, px, t)).fold(0.0, f32::max).min(self.win[0] * 0.6) + 2.0 * pad;
        let h = lines.len() as f32 * lh + 2.0 * pad - 2.0 * s;
        let mut x = at[0] + 16.0 * s;
        let mut y = at[1] + 18.0 * s;
        if x + w > self.win[0] - 4.0 {
            x = (at[0] - w - 12.0 * s).max(4.0);
        }
        if y + h > self.win[1] - 4.0 {
            y = (at[1] - h - 12.0 * s).max(4.0);
        }
        let r = Rect::new(x, y, w, h);
        g.rrect(r, 4.0 * s, alpha(TOOLTIP_BG, 0.95));
        g.rrect_stroke(r, 4.0 * s, s.max(1.0), alpha(BORDER, 0.6));
        for (k, (t, c)) in lines.iter().enumerate() {
            g.text_fit(Face::Regular, px, r.x + pad, r.y + pad + k as f32 * lh, w - 2.0 * pad, *c, t);
        }
    }

    /// A click while a menu is open: run the item under the pointer, or close the menu.
    pub fn menu_click(&mut self, x: f32, y: f32) {
        let Some(menu) = self.ui.menu.take() else { return };
        if !menu.rect.contains(x, y) {
            self.redraw = true;
            return;
        }
        // The row under the click itself (the hover of the last frame may be stale).
        let idx = menu.rows.iter().position(|r| r.contains(x, y));
        let row = idx.and_then(|i| menu.items.into_iter().nth(i));
        if let Some(MenuItem { action: Some(a), enabled: true, .. }) = row {
            self.apply(a);
        }
        self.redraw = true;
    }

    /// The action under (x, y) among this frame's controls.
    pub fn hit_action(&self, x: f32, y: f32) -> Option<Action> {
        self.ui.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a.clone())
    }

    /// Centre of a top-bar control by name (tests): a port type, "Buffer", "Xruns", "menu".
    pub fn bar_point(&self, name: &str) -> Option<[f32; 2]> {
        self.ui.hits.iter().find_map(|(r, a)| {
            let hit = match a {
                Action::SetKind(k) => k.label() == name,
                Action::BufferMenu(_) => name == "Buffer",
                Action::ResetXruns => name == "Xruns",
                Action::AppMenu(_) => name == "menu",
                Action::ZoomBy(f) => (name == "zoom+" && *f > 1.0) || (name == "zoom-" && *f < 1.0),
                Action::ZoomSlider(_) => name == "zoomtrack",
                Action::ZoomTo(_) => name == "zoompct",
                Action::TransportToggle => name == "play",
                Action::TransportStop => name == "tstop",
                Action::TransportRewind => name == "rew",
                Action::Sess(a) => {
                    use crate::sessui::SessAction as S;
                    matches!((a, name), (S::New, "sess-new") | (S::Open, "sess-open") | (S::Save, "sess-save") | (S::Close, "sess-close") | (S::AddApp, "sess-add") | (S::Menu(_), "sess-menu"))
                        || matches!(a, S::ClientMenu(id, _) if name == format!("client:{id}"))
                        || matches!(a, S::Start(id) | S::Stop(id) if name == format!("startstop:{id}"))
                }
                Action::BackToJack => name == "back",
                Action::ServerStart => name == "start",
                Action::ServerStop => name == "stop",
                Action::ServerRestart => name == "restart",
                Action::ServerSettings => name == "settings",
                _ => false,
            };
            hit.then(|| [r.cx(), r.cy()])
        })
    }

    /// The top bar's controls, for the window's hit-test (clicks there are not drags).
    pub fn bar_controls(&self) -> Vec<Rect> {
        let mut v: Vec<Rect> = self.ui.hits.iter().map(|(r, _)| *r).filter(|r| r.y < self.ui.bar_h).collect();
        let strip = self.ui.csd.strip_w(self.chrome);
        v.push(if self.ui.csd.left { Rect::new(0.0, 0.0, strip, self.ui.bar_h) } else { Rect::new(self.win[0] - strip, 0.0, strip, self.ui.bar_h) });
        v
    }
}

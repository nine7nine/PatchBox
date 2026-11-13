//! The window's chrome around the canvas: the top bar (it is also the titlebar), the sidebar,
//! popup menus. Immediate mode: each frame draws and records the clickable rectangles with the
//! action each one maps to.

use crate::app::{Action, App};
use crate::csd::{Button, Csd};
use pbx_jack::PortKind;
use crate::gfx::{Gfx, Rect, alpha};
use crate::text::Face;
use crate::theme::*;

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
    /// The title bar's height (dragging moves the window) and the whole top's (with the toolbar).
    pub title_h: f32,
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
            title_h: 0.0,
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

    #[allow(clippy::too_many_arguments)]
    pub fn open_app_menu(&mut self, under: Rect, labels: bool, sidebar: bool, ui_scale: f32, grid: bool, snap: bool, undo: (bool, bool)) {
        let items = vec![
            MenuItem::new(format!("Interface scale: {:.0} % …", ui_scale * 100.0), Action::ScaleMenu(Rect::new(under.x, under.bottom() + 2.0, 0.0, 0.0))),
            MenuItem::new("Event log (xruns, server, clients)", Action::EventLog),
            MenuItem::new("Undo (Ctrl+Z)", Action::Undo).enabled(undo.0),
            MenuItem::new("Redo (Ctrl+Shift+Z)", Action::Redo).enabled(undo.1),
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

/// A value over a small caption (green, the LCD's), centred in `r`.
fn readout(g: &mut Gfx, r: Rect, s: f32, value: &str, caption: &str, col: crate::gfx::Color) {
    let (px, spx) = (FONT * s, FONT_SMALL * s);
    let (lh, slh) = (g.line_height(Face::Regular, px), g.line_height(Face::Regular, spx));
    let y0 = (r.cy() - (lh + slh) / 2.0).round();
    let w = g.measure(Face::Regular, px, value);
    g.text(Face::Regular, px, (r.cx() - w / 2.0).round(), y0, col, value);
    let cw = g.measure(Face::Regular, spx, caption);
    g.text(Face::Regular, spx, (r.cx() - cw / 2.0).round(), y0 + lh, LCD_CAPTION, caption);
}

/// The toolbar's icons, drawn (the fonts have no such glyphs).
#[derive(Clone, Copy)]
pub enum Icon {
    New,
    Open,
    Save,
    Audio,
    Midi,
    Cv,
    Osc,
    PortsH,
    PortsV,
    Matrix,
}

/// Draw `icon` centred on `c`, about 20 points across.
pub fn draw_icon(g: &mut Gfx, icon: Icon, c: [f32; 2], s: f32, col: crate::gfx::Color) {
    let lw = (1.5 * s).max(1.0);
    let p = |x: f32, y: f32| [c[0] + x * s, c[1] + y * s];
    match icon {
        Icon::New => {
            g.polyline(&[p(-6.0, -8.0), p(2.0, -8.0), p(6.0, -4.0), p(6.0, 8.0), p(-6.0, 8.0)], lw, col, true);
            g.polyline(&[p(2.0, -8.0), p(2.0, -4.0), p(6.0, -4.0)], lw, col, false);
            g.line(p(-3.0, 2.5), p(3.0, 2.5), lw, col);
            g.line(p(0.0, -0.5), p(0.0, 5.5), lw, col);
        }
        Icon::Open => {
            g.polyline(&[p(-9.0, -7.0), p(-3.0, -7.0), p(-1.0, -5.0), p(8.0, -5.0), p(8.0, 7.0), p(-9.0, 7.0)], lw, col, true);
            g.line(p(-9.0, -2.0), p(8.0, -2.0), lw, col);
        }
        Icon::Save => {
            g.polyline(&[p(-8.0, -8.0), p(5.0, -8.0), p(8.0, -5.0), p(8.0, 8.0), p(-8.0, 8.0)], lw, col, true);
            g.rect(Rect::new(c[0] - 4.0 * s, c[1] - 8.0 * s, 7.0 * s, 5.0 * s), col);
            g.rrect_stroke(Rect::new(c[0] - 5.0 * s, c[1] + 1.5 * s, 10.0 * s, 6.5 * s), 1.0 * s, lw, col);
        }
        Icon::Audio => {
            let pts: Vec<[f32; 2]> = (0..=40)
                .map(|i| {
                    let t = i as f32 / 40.0;
                    p(-10.0 + 20.0 * t, -6.0 * (t * std::f32::consts::TAU * 1.5).sin())
                })
                .collect();
            g.polyline(&pts, lw * 1.1, col, false);
        }
        Icon::Midi => {
            // A 5-pin DIN socket.
            g.arc(c, 8.5 * s, 0.0, std::f32::consts::TAU, lw, col);
            for a in [0.5f32, 0.625, 0.75, 0.875, 1.0] {
                let t = a * std::f32::consts::TAU;
                g.circle([c[0] + 5.0 * s * t.cos(), c[1] + 5.0 * s * t.sin()], 1.4 * s, col);
            }
            g.rect(Rect::new(c[0] - 1.5 * s, c[1] + 6.0 * s, 3.0 * s, 2.5 * s), col);
        }
        Icon::Cv => {
            // An envelope: attack, decay, sustain, release.
            g.polyline(&[p(-10.0, 7.0), p(-6.0, -7.0), p(-2.0, -1.0), p(5.0, -1.0), p(10.0, 7.0)], lw * 1.1, col, false);
        }
        Icon::Osc => {
            // Three hosts on a network.
            let (a, b, d) = (p(-7.0, 5.0), p(0.0, -6.0), p(7.0, 5.0));
            for (u, v) in [(a, b), (b, d), (a, d)] {
                g.line(u, v, lw, col);
            }
            for q in [a, b, d] {
                g.circle(q, 2.8 * s, col);
            }
        }
        Icon::PortsH | Icon::PortsV => {
            // A block with ports on its edges and wires leaving them: the flow.
            let v = matches!(icon, Icon::PortsV);
            let q = |x: f32, y: f32| if v { p(y, x) } else { p(x, y) };
            let (bw, bh) = if v { (14.0, 9.0) } else { (9.0, 14.0) };
            g.rrect_stroke(Rect::centered(c[0], c[1], bw * s, bh * s), 1.5 * s, lw, col);
            for y in [-3.0f32, 3.0] {
                for x in [-4.5f32, 4.5] {
                    let m = q(x, y);
                    g.rect(Rect::centered(m[0], m[1], 3.0 * s, 3.0 * s), col);
                }
                g.line(q(-10.0, y), q(-6.0, y), lw, col);
                g.line(q(6.0, y), q(10.0, y), lw, col);
            }
        }
        Icon::Matrix => {
            let k = 5.0 * s;
            let o = [c[0] - 1.5 * k, c[1] - 1.5 * k];
            for i in 0..=3 {
                let f = i as f32 * k;
                g.line([o[0] + f, o[1]], [o[0] + f, o[1] + 3.0 * k], lw * 0.8, col);
                g.line([o[0], o[1] + f], [o[0] + 3.0 * k, o[1] + f], lw * 0.8, col);
            }
            for (cx, cy) in [(0.0f32, 0.0f32), (1.0, 1.0), (2.0, 2.0)] {
                g.rect(Rect::new(o[0] + cx * k + 1.5 * s, o[1] + cy * k + 1.5 * s, k - 3.0 * s, k - 3.0 * s), col);
            }
        }
    }
}

/// Luluda's button: a dark face with a highlight along its top and a shade along its bottom,
/// the border tinted by what the button does; hovering and being active warm the face with the
/// tint and brighten the border.
pub fn lbutton(g: &mut Gfx, r: Rect, s: f32, tint: crate::gfx::Color, hover: bool, active: bool, enabled: bool) {
    use crate::gfx::mix;
    let rad = 4.0 * s;
    let face = if active {
        mix(BTN_FACE, tint, 0.13)
    } else if hover && enabled {
        mix(BTN_FACE, tint, 0.06)
    } else {
        BTN_FACE
    };
    g.rrect(r, rad, if enabled { face } else { alpha(face, 0.55) });
    let lw = s.max(1.0);
    g.rect(Rect::new(r.x + rad, r.y + lw, r.w - 2.0 * rad, lw), [1.0, 1.0, 1.0, if enabled { 0.06 } else { 0.02 }]);
    g.rect(Rect::new(r.x + rad, r.bottom() - 2.0 * lw, r.w - 2.0 * rad, lw), [0.0, 0.0, 0.0, 0.35]);
    // Luluda dims the border of an idle button to about three quarters of its tint; softer
    // still here, full only on the active one.
    let border = mix(BTN_FACE, tint, if !enabled { 0.14 } else if active { 0.72 } else if hover { 0.55 } else { 0.38 });
    g.rrect_stroke(r, rad, (1.2 * s).max(1.0), border);
}

/// The text colour on an `lbutton`.
pub fn lbutton_text(tint: crate::gfx::Color, hover: bool, active: bool, enabled: bool) -> crate::gfx::Color {
    use crate::gfx::mix;
    if !enabled {
        TEXT_DIM
    } else if active {
        mix(TEXT_BRIGHT, tint, 0.18)
    } else if hover {
        mix(TEXT, TEXT_BRIGHT, 0.6)
    } else {
        mix(TEXT, tint, 0.16)
    }
}

/// What a button does, as Luluda colours it.
pub fn tint_for(label: &str) -> crate::gfx::Color {
    match label {
        "Stop" | "Kill" | "Quit" => TINT_RED,
        "Start" | "Create" | "Add command" | "Save the current view" | "Take" => TINT_GREEN,
        "Restart" | "Save" | "Rename" | "Duplicate" => TINT_AMBER,
        "Settings" | "Open" => TINT_BLUE,
        "New" | "+ Application" => TINT_GREEN,
        l if l.starts_with("Trash") => TINT_RED,
        _ => TINT_GREY,
    }
}

/// Break `text` into lines no wider than `max_w`: at spaces, or (a long name with none) anywhere
/// after a ':' '/' '_' '-' or, failing that, mid-word.
pub fn wrap(g: &Gfx, px: f32, text: &str, max_w: f32) -> Vec<String> {
    if g.measure(Face::Regular, px, text) <= max_w {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_inclusive(' ') {
        let cand = format!("{line}{word}");
        if g.measure(Face::Regular, px, cand.trim_end()) <= max_w {
            line = cand;
            continue;
        }
        if !line.is_empty() {
            out.push(line.trim_end().to_string());
            line.clear();
        }
        // A word wider than the box: cut it, preferring a separator.
        let mut rest = word.to_string();
        while g.measure(Face::Regular, px, rest.trim_end()) > max_w {
            let chars: Vec<char> = rest.chars().collect();
            let mut fit = 0;
            for k in 1..=chars.len() {
                if g.measure(Face::Regular, px, &chars[..k].iter().collect::<String>()) > max_w {
                    break;
                }
                fit = k;
            }
            let fit = fit.max(1);
            let cut = chars[..fit].iter().rposition(|c| matches!(c, ':' | '/' | '_' | '-' | '.')).map(|p| p + 1).filter(|&p| p > fit / 3).unwrap_or(fit);
            out.push(chars[..cut].iter().collect());
            rest = chars[cut..].iter().collect();
        }
        line = rest;
    }
    if !line.trim().is_empty() {
        out.push(line.trim_end().to_string());
    }
    out
}

impl App {
    /// Lay out the frame's regions (pixels).
    pub fn layout_regions(&mut self) {
        let s = self.scale;
        // A slim title bar at the desktop's scale, a toolbar row at the interface's.
        self.ui.title_h = (TITLE_H * self.chrome).round().max((FONT * s * 1.9).round());
        let bar_h = self.ui.title_h + (TOOLBAR_H * s).round();
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
        let tint = tint_for(label);
        lbutton(g, r, s, tint, hov, active, true);
        g.text_center(Face::Regular, FONT * s, r, lbutton_text(tint, hov, active, true), label);
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
        }
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
        let w = bw * 3.0 + track_w + pw + 5.0 * 6.0 * s;
        let panel = Rect::new(a.right() - w - 12.0 * s, a.bottom() - h - 12.0 * s, w, h);
        g.rrect(panel, 4.0 * s, alpha(PANEL_BG, 0.92));
        g.rrect_stroke(panel, 4.0 * s, s.max(1.0), LINE_SOFT);
        let mut x = panel.x + 6.0 * s;
        // Centre: fit the whole graph in the window (F), as qjackctl's and RaySession's do.
        let fit = Rect::new(x, panel.y, bw, h);
        x += bw + 6.0 * s;
        {
            let hov = self.hovered(&fit);
            if hov {
                g.rrect(fit.inset(3.0 * s), 3.0 * s, BUTTON_HOVER);
            }
            let col = if hov { TEXT_BRIGHT } else { TEXT };
            let (cx, cy) = (fit.cx().round(), fit.cy().round());
            let (l, gap, t) = ((7.0 * s).round(), (3.0 * s).round(), (1.6 * s).max(1.0).round());
            // Four corner brackets around a dot.
            for (dx, dy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let (ex, ey) = (cx + dx * l, cy + dy * l);
                let hx = if dx < 0.0 { ex } else { ex - gap };
                let vy = if dy < 0.0 { ey } else { ey - gap };
                g.rect(Rect::new(hx, if dy < 0.0 { ey } else { ey - t }, gap, t), col);
                g.rect(Rect::new(if dx < 0.0 { ex } else { ex - t }, vy, t, gap), col);
            }
            g.circle([cx, cy], 2.0 * s, col);
            self.add_hit(fit, Action::FitAll);
        }
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

    /// The window's top: a slim title bar (window buttons, the menu, the title) and a toolbar
    /// row of grouped controls (Luluda's layout): session, transport, graph, JACK.
    fn draw_topbar(&mut self, g: &mut Gfx) {
        self.draw_titlebar(g);
        self.draw_toolbar(g);
    }

    fn draw_titlebar(&mut self, g: &mut Gfx) {
        let s = self.scale;
        let th = self.ui.title_h;
        let bar = Rect::new(0.0, 0.0, self.win[0], th);
        g.rect(bar, TITLE_BG);
        let strip = self.ui.csd.strip_w(self.chrome);
        let mx = if self.ui.csd.left { strip + 4.0 * s } else { 8.0 * s };
        let mh = th - 6.0 * self.chrome;
        let mr = Rect::new(mx, (th - mh) / 2.0, mh, mh);
        let hov = self.hovered(&mr);
        g.rrect(mr, 3.0 * s, if hov { BUTTON_HOVER } else { TITLE_BG });
        for k in -1..=1 {
            let y = (mr.cy() + k as f32 * 4.0 * s - 0.8 * s).round();
            g.rect(Rect::new((mr.cx() - 6.0 * s).round(), y, (12.0 * s).round(), (1.6 * s).max(1.0).round()), TEXT);
        }
        self.add_hit(mr, Action::AppMenu(mr));
        let mut title = "PatchBox".to_string();
        if let Some(n) = &self.sess.state.session {
            title.push_str(&format!("  —  {n}"));
        }
        if self.source == crate::app::Source::PipeWire {
            title.push_str("  ·  PipeWire");
        }
        g.text_center(Face::Bold, FONT * s, Rect::new(bar.w * 0.25, 0.0, bar.w * 0.5, th), TEXT_DIM, &title);
        self.ui.csd.draw(g, bar.w, th, self.chrome, self.ui.csd_hover);
    }

    /// A group panel of the toolbar.
    fn group(&self, g: &mut Gfx, r: Rect) {
        let s = self.scale;
        g.rrect(r, 4.0 * s, GROUP_BG);
        g.rrect_stroke(r, 4.0 * s, s.max(1.0), PANEL_LINE);
    }

    fn draw_toolbar(&mut self, g: &mut Gfx) {
        let s = self.scale;
        let top = self.ui.title_h;
        let bar = Rect::new(0.0, top, self.win[0], self.ui.bar_h - top);
        g.rect(bar, BAR_BG);
        g.rect(Rect::new(0.0, bar.bottom() - s.max(1.0), bar.w, s.max(1.0)), PANEL_LINE);
        let pad = 6.0 * s;
        let gh = bar.h - 2.0 * pad;
        let gy = bar.y + pad;
        let bh = gh - 2.0 * 3.0 * s;
        let by = gy + 3.0 * s;
        let px = FONT * s;
        let spx = FONT_SMALL * s;
        let st = self.stats;
        let connected = self.jack.is_some();
        // Luluda's tiles: an icon over a small label, all of a group the same width.
        let tile_w = |g: &Gfx, labels: &[&str]| labels.iter().map(|l| g.measure(Face::Regular, spx, l)).fold(0.0f32, f32::max).max(30.0 * s) + 18.0 * s;
        let bgap = 4.0 * s;
        let group_w = |n: usize, w: f32| n as f32 * w + (n as f32 - 1.0) * bgap + 8.0 * s;
        // A readout tile: a value over a caption.
        let read_w = |g: &Gfx, v: &str, c: &str| g.measure(Face::Regular, px, v).max(g.measure(Face::Regular, spx, c)) + 20.0 * s;

        // Widths first, so groups drop out right to left of priority when the window is narrow.
        let sess_tw = tile_w(g, &["New", "Open", "Save"]);
        let sess_w = group_w(3, sess_tw);
        let lcd_w = 136.0 * s;
        let trans_w = 3.0 * bh + 2.0 * bgap + 6.0 * s + lcd_w + 8.0 * s;
        let kinds = self.kinds();
        let kind_tw = tile_w(g, &["AUDIO", "MIDI", "CV", "OSC"]);
        let kinds_w = group_w(kinds.len(), kind_tw);
        let view_tw = tile_w(g, &["Ports", "Matrix"]);
        let views_w = group_w(2, view_tw);
        let ms = st.buffer_size as f32 * 1000.0 / st.sample_rate.max(1) as f32;
        let (rate_v, rate_c) = if connected {
            (format!("{} Hz", st.sample_rate), if st.freewheel { "FREEWHEEL".to_string() } else { format!("{ms:.1} ms") })
        } else {
            ("offline".to_string(), "JACK".to_string())
        };
        let bs = if connected { format!("{} ▼", st.buffer_size) } else { "– ▼".into() };
        let xs = st.xruns.to_string();
        let dsp = if connected { format!("{:.1} %", st.dsp) } else { "–".into() };
        let rate_w = read_w(g, &rate_v, &rate_c);
        let buf_w = read_w(g, &bs, "BUFFER");
        let xr_w = read_w(g, "0000", "XRUNS");
        let dsp_w = read_w(g, "100.0 %", "DSP LOAD").max(84.0 * s);
        let jack_w = rate_w + buf_w + xr_w + dsp_w + 3.0 * bgap + 8.0 * s;
        let gap = 8.0 * s;
        let avail = bar.w - 2.0 * pad;
        let need_all = sess_w + trans_w + kinds_w + views_w + jack_w + 4.0 * gap;
        let show_sess = need_all <= avail;
        let show_views = need_all - if show_sess { 0.0 } else { sess_w + gap } <= avail;

        let mut x = bar.x + pad;
        // Session.
        if show_sess {
            let r = Rect::new(x, gy, sess_w, gh);
            self.group(g, r);
            let mut bx = r.x + 4.0 * s;
            let open = self.sess.state.session.is_some();
            for (l, icon, a, on) in [
                ("New", Icon::New, crate::sessui::SessAction::New, true),
                ("Open", Icon::Open, crate::sessui::SessAction::Open, true),
                ("Save", Icon::Save, crate::sessui::SessAction::Save, open),
            ] {
                self.tile(g, Rect::new(bx, by, sess_tw, bh), l, icon, tint_for(l), None, false, on, Action::Sess(a));
                bx += sess_tw + bgap;
            }
            x += sess_w + gap;
        }
        // Transport: rewind, play/pause, stop, and the time display.
        {
            let r = Rect::new(x, gy, trans_w, gh);
            self.group(g, r);
            let t = st.transport;
            let playing = t.rolling || t.starting;
            let mut bx = r.x + 4.0 * s;
            for (i, act) in [Action::TransportRewind, Action::TransportToggle, Action::TransportStop].into_iter().enumerate() {
                let b = Rect::new(bx, by, bh, bh);
                bx += bh + bgap;
                let tint = [TINT_BLUE, TINT_GREEN, TINT_GREY][i];
                let hov = connected && self.hovered(&b);
                lbutton(g, b, s, tint, hov, i == 1 && playing, connected);
                let (cx, cy, k) = (b.cx(), b.cy(), 7.0 * s);
                let col = if !connected { TEXT_DIM } else { crate::gfx::mix(lbutton_text(tint, hov, i == 1 && playing, true), tint, 0.35) };
                match i {
                    0 => {
                        g.rect(Rect::new(cx - k, cy - k, 2.0 * s, 2.0 * k), col);
                        g.triangle([cx + k, cy - k], [cx + k, cy + k], [cx - k + 2.5 * s, cy], col);
                    }
                    1 if playing => {
                        g.rect(Rect::new(cx - k * 0.8, cy - k, k * 0.6, 2.0 * k), col);
                        g.rect(Rect::new(cx + k * 0.2, cy - k, k * 0.6, 2.0 * k), col);
                    }
                    1 => g.triangle([cx - k * 0.7, cy - k], [cx - k * 0.7, cy + k], [cx + k, cy], col),
                    _ => g.rect(Rect::new(cx - k * 0.85, cy - k * 0.85, k * 1.7, k * 1.7), col),
                }
                if connected {
                    self.add_hit(b, act);
                }
            }
            // The time display (Luluda's LCD): time, and bars|beats|ticks and tempo below.
            let lcd = Rect::new(bx + 2.0 * s, by, lcd_w, bh);
            g.rrect(lcd, 3.0 * s, LCD_BG);
            g.rrect_stroke(lcd, 3.0 * s, s.max(1.0), PANEL_LINE);
            let secs = t.frame as f64 / t.rate.max(1) as f64;
            let time = format!("{}:{:02}:{:04.1}", (secs / 3600.0) as u32, ((secs / 60.0) as u32) % 60, secs % 60.0);
            let tcol = if playing { LCD_TEXT } else { alpha(LCD_TEXT, 0.8) };
            let tpx = px * 1.2;
            let lh = g.line_height(Face::Regular, tpx);
            let slh = g.line_height(Face::Regular, spx);
            let sub = match t.bbt {
                Some((bar_n, beat, tick, bpm, _, _)) => format!("{bar_n}|{beat}|{tick:04}  {bpm:.1} BPM"),
                None => "TIME".to_string(),
            };
            let y0 = lcd.cy() - (lh + slh) / 2.0;
            let w = g.measure(Face::Regular, tpx, &time);
            g.text(Face::Regular, tpx, lcd.cx() - w / 2.0, y0, tcol, &time);
            let sw = g.measure(Face::Regular, spx, &sub);
            g.text(Face::Regular, spx, lcd.cx() - sw / 2.0, y0 + lh, LCD_CAPTION, &sub);
            x += trans_w + gap;
        }
        // Graph: the port types, then the view.
        {
            let r = Rect::new(x, gy, kinds_w, gh);
            self.group(g, r);
            let mut bx = r.x + 4.0 * s;
            for k in kinds {
                let icon = match k {
                    PortKind::Audio => Icon::Audio,
                    PortKind::Midi => Icon::Midi,
                    PortKind::Cv => Icon::Cv,
                    _ => Icon::Osc,
                };
                self.tile(g, Rect::new(bx, by, kind_tw, bh), k.label(), icon, kind_color(k), Some(kind_color(k)), self.kind == k, true, Action::SetKind(k));
                bx += kind_tw + bgap;
            }
            x += kinds_w + gap;
        }
        if show_views {
            let r = Rect::new(x, gy, views_w, gh);
            self.group(g, r);
            let bx = r.x + 4.0 * s;
            let (matrices, vertical) = (self.layout.view.matrices, self.layout.view.vertical);
            // Ports: its icon shows the flow; clicking it again switches between left to right
            // and top to bottom.
            let icon = if vertical { Icon::PortsV } else { Icon::PortsH };
            self.tile(g, Rect::new(bx, by, view_tw, bh), "Ports", icon, TINT_TEAL, None, !matrices, true, Action::PortsStyle);
            let mr = Rect::new(bx + view_tw + bgap, by, view_tw, bh);
            self.tile(g, mr, "Matrix", Icon::Matrix, TINT_TEAL, None, matrices, !matrices, Action::SetStyle(true, vertical));
        }
        // JACK, right-aligned: rate and latency, buffer size, xruns, DSP load.
        {
            let r = Rect::new(bar.right() - pad - jack_w, gy, jack_w, gh);
            self.group(g, r);
            let mut rx = r.right() - 4.0 * s;
            let dr = Rect::new(rx - dsp_w, by, dsp_w, bh);
            g.rrect(dr, 3.0 * s, LCD_BG);
            g.rrect_stroke(dr, 3.0 * s, s.max(1.0), PANEL_LINE);
            // The load as a level along the bottom: the face stays the LCD's, like the time's.
            if connected {
                let frac = (st.dsp / 100.0).clamp(0.0, 1.0);
                let col = if st.dsp < 50.0 { OK } else if st.dsp < 80.0 { WARN } else { BAD };
                let track = Rect::new(dr.x + 6.0 * s, dr.bottom() - 5.0 * s, dr.w - 12.0 * s, (2.0 * s).max(1.0));
                g.rect(track, alpha(PANEL_LINE, 0.8));
                if frac > 0.0 {
                    g.rect(Rect::new(track.x, track.y, (track.w * frac).max(s), track.h), alpha(col, 0.85));
                }
            }
            readout(g, dr, s, &dsp, "DSP LOAD", if connected { LCD_TEXT } else { TEXT_DIM });
            rx -= dsp_w + bgap;
            let xr = Rect::new(rx - xr_w, by, xr_w, bh);
            let xt = if st.xruns > 0 { TINT_RED } else { TINT_GREY };
            let hov = self.hovered(&xr);
            lbutton(g, xr, s, xt, hov, st.xruns > 0, true);
            readout(g, xr, s, &xs, "XRUNS", if st.xruns > 0 { crate::gfx::mix(BAD, TEXT_BRIGHT, 0.2) } else if hov { TEXT_BRIGHT } else { LCD_TEXT });
            self.add_hit(xr, Action::EventLog);
            rx -= xr_w + bgap;
            let br = Rect::new(rx - buf_w, by, buf_w, bh);
            let hov = connected && self.hovered(&br);
            lbutton(g, br, s, TINT_AMBER, hov, false, connected);
            readout(g, br, s, &bs, "BUFFER", if !connected { TEXT_DIM } else if hov { TEXT_BRIGHT } else { LCD_TEXT });
            if connected {
                self.add_hit(br, Action::BufferMenu(br));
            }
            rx -= buf_w + bgap;
            let rr = Rect::new(rx - rate_w, by, rate_w, bh);
            g.rrect(rr, 3.0 * s, LCD_BG);
            g.rrect_stroke(rr, 3.0 * s, s.max(1.0), PANEL_LINE);
            let col = if connected { if st.freewheel { WARN } else { LCD_TEXT } } else { BAD };
            readout(g, rr, s, &rate_v, &rate_c, col);
        }
    }

    /// A toolbar tile (Luluda): an icon over its label. `icon_col` colours the icon (else it
    /// takes the label's colour).
    #[allow(clippy::too_many_arguments)]
    fn tile(&mut self, g: &mut Gfx, r: Rect, label: &str, icon: Icon, tint: crate::gfx::Color, icon_col: Option<crate::gfx::Color>, active: bool, enabled: bool, a: Action) {
        let s = self.scale;
        let hov = enabled && self.hovered(&r);
        let shown = enabled || active;
        lbutton(g, r, s, tint, hov, active, shown);
        let col = lbutton_text(tint, hov, active, shown);
        let spx = FONT_SMALL * s;
        let slh = g.line_height(Face::Regular, spx);
        let ly = r.bottom() - slh - 4.0 * s;
        let ic = match icon_col {
            Some(c) if shown => crate::gfx::mix(c, col, if active || hov { 0.15 } else { 0.35 }),
            _ => col,
        };
        draw_icon(g, icon, [r.cx().round(), ((r.y + ly) / 2.0 + 1.0 * s).round()], s, ic);
        let w = g.measure(Face::Regular, spx, label);
        g.text(Face::Regular, spx, (r.cx() - w / 2.0).round(), ly, col, label);
        if enabled {
            self.add_hit(r, a);
        }
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
        // The server is jackdbus's (jack2dbus-nspa): start, stop and settings go through it.
        let head = "JACK · jackdbus";
        let hw = g.measure(Face::Bold, px, head);
        g.text(Face::Bold, px, r.x + pad, y, TEXT_BRIGHT, head);
        g.circle([r.x + pad + hw + 10.0 * s, y + lh / 2.0], 4.0 * s, dot);
        y += lh + 2.0 * s;
        g.text_fit(Face::Regular, FONT_SMALL * s, r.x + pad, y + 1.0 * s, r.w - 2.0 * pad, TEXT, &state);
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
        let open = self.dialog.as_ref().is_some_and(|d| matches!(d.kind, crate::dialog::Kind::Server | crate::dialog::Kind::PipeWire));
        self.button(g, Rect::new(bx, y, bw, bh), "Settings", open, Action::ServerSettings);
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
            g.text(Face::Regular, FONT_SMALL * s, rr.right() - cw - 6.0 * s, rr.cy() - slh / 2.0, if hidden { TEXT_DIM } else { TEXT_SOFT }, &counts);
            let vis = rr.intersect(&list);
            self.add_hit(vis, Action::CenterNode(id));
        }
        g.pop_clip();

        // Log: the latest messages; a click opens the event log.
        let lr = Rect::new(r.x, r.bottom() - log_h, r.w, log_h);
        self.add_hit(lr, Action::EventLog);
        g.rect(Rect::new(lr.x + pad, lr.y, lr.w - 2.0 * pad, s.max(1.0)), PANEL_LINE);
        let spx = FONT_SMALL * s;
        let slh = g.line_height(Face::Regular, spx) + 2.0 * s;
        let fit = ((lr.h - pad) / slh) as usize;
        let start = self.log.len().saturating_sub(fit);
        let mut ly = lr.y + pad * 0.6;
        for l in &self.log[start..] {
            let col = if l.bad { WARN } else { TEXT_SOFT };
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
        g.rrect_stroke(rect, 5.0 * s, s.max(1.0), LINE_SOFT);
        for (i, item) in menu.items.iter().enumerate() {
            let rr = Rect::new(rect.x + pad / 2.0, rect.y + pad + i as f32 * row_h, rect.w - pad, row_h);
            menu.rows.push(rr);
            let active = item.enabled && item.action.is_some();
            if active && rr.contains(mx, my) {
                menu.hover = Some(i);
                g.rrect(rr, 3.0 * s, MENU_HOVER);
            }
            let col = if item.action.is_none() {
                crate::gfx::mix(HILIGHT, TEXT, 0.3)
            } else if item.enabled {
                crate::gfx::mix(TEXT, TEXT_BRIGHT, 0.55)
            } else {
                TEXT_DIM
            };
            let face = if item.action.is_none() { Face::Bold } else { Face::Regular };
            if item.checked == Some(true) {
                g.circle([rr.x + pad + 5.0 * s, rr.cy()], 3.5 * s, crate::gfx::mix(TOGGLE, TEXT, 0.3));
            } else if item.checked == Some(false) {
                g.arc([rr.x + pad + 5.0 * s, rr.cy()], 3.5 * s, 0.0, std::f32::consts::TAU, s.max(1.0), TEXT_DIM);
            }
            g.text_fit(face, px, rr.x + pad + check_w, rr.cy() - lh / 2.0, rr.w - check_w - 2.0 * pad, col, &item.label);
        }
    }

    /// A box of text lines near `at` (window pixels), kept inside the window; long lines wrap at
    /// a compact width.
    pub fn draw_tooltip(&self, g: &mut Gfx, at: [f32; 2], lines: &[(String, crate::gfx::Color)]) {
        let s = self.scale;
        let px = FONT_SMALL * s * 1.08;
        let max_w = (340.0 * s).min(self.win[0] * 0.45);
        let mut wrapped: Vec<(String, crate::gfx::Color)> = Vec::new();
        for (t, c) in lines {
            for l in wrap(g, px, t, max_w) {
                wrapped.push((l, *c));
            }
        }
        let lines = &wrapped;
        let lh = g.line_height(Face::Regular, px) + 2.0 * s;
        let pad = 7.0 * s;
        let w = lines.iter().map(|(t, _)| g.measure(Face::Regular, px, t)).fold(0.0, f32::max).min(max_w) + 2.0 * pad;
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
        g.rrect_stroke(r, 4.0 * s, s.max(1.0), LINE_SOFT);
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
                Action::EventLog => name == "log",
                Action::AppMenu(_) => name == "menu",
                Action::ZoomBy(f) => (name == "zoom+" && *f > 1.0) || (name == "zoom-" && *f < 1.0),
                Action::ZoomSlider(_) => name == "zoomtrack",
                Action::ZoomTo(_) => name == "zoompct",
                Action::FitAll => name == "zoomfit",
                Action::PortsStyle => name == "ports",
                Action::SetStyle(true, _) => name == "matrix",
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
        let mut v: Vec<Rect> = self.ui.hits.iter().map(|(r, _)| *r).filter(|r| r.y < self.ui.title_h).collect();
        let strip = self.ui.csd.strip_w(self.chrome);
        v.push(if self.ui.csd.left { Rect::new(0.0, 0.0, strip, self.ui.title_h) } else { Rect::new(self.win[0] - strip, 0.0, strip, self.ui.title_h) });
        v
    }
}

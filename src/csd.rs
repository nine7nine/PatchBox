//! Client-side decorations. The window is borderless; its top bar (ui.rs) is the titlebar, with
//! the window buttons on the side the desktop puts them (KDE kwinrc / GNOME button-layout; left by
//! default). Move and resize go through SDL's hit-test: the compositor runs the interactive
//! drag/resize. Ported from teams-sdl-cef's csd.rs (same owner), with a dynamic list of the
//! top bar's controls instead of fixed geometry.

use crate::gfx::{Gfx, Rect};
use sdl3::video::HitTestResult;
use std::sync::{Arc, Mutex};

const RESIZE_BORDER: f32 = 6.0;
const CORNER_GRAB: f32 = 20.0;
const BTN_R: f32 = 6.0;
const BTN_SPACING: f32 = 22.0;
const EDGE_MARGIN: f32 = 18.0;

// Paneless's traffic-light colours.
const CLOSE: [f32; 4] = [0xe0 as f32 / 255.0, 0x60 as f32 / 255.0, 0x6a as f32 / 255.0, 1.0];
const MIN: [f32; 4] = [0xeb as f32 / 255.0, 0xcb as f32 / 255.0, 0x8b as f32 / 255.0, 1.0];
const MAX: [f32; 4] = [0xa3 as f32 / 255.0, 0xbe as f32 / 255.0, 0x8c as f32 / 255.0, 1.0];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Button {
    Close,
    Minimize,
    Maximize,
}

/// What the hit-test closure needs, refreshed every frame (pixels).
#[derive(Default)]
pub struct HitGeom {
    pub w: f32,
    pub h: f32,
    pub bar_h: f32,
    /// Controls in the top bar: clicks, not drags.
    pub controls: Vec<Rect>,
    pub density: f32,
    pub scale: f32,
    pub maximized: bool,
}

pub type SharedGeom = Arc<Mutex<HitGeom>>;

pub struct Csd {
    pub buttons: Vec<Button>,
    pub left: bool,
}

impl Csd {
    pub fn from_system() -> Csd {
        let (left, buttons) = kde_layout().or_else(gnome_layout).unwrap_or((true, vec![Button::Close, Button::Minimize, Button::Maximize]));
        Csd { buttons, left }
    }

    /// Width the buttons take at the bar's edge, in pixels.
    pub fn strip_w(&self, scale: f32) -> f32 {
        if self.buttons.is_empty() {
            return 0.0;
        }
        (EDGE_MARGIN + (self.buttons.len() as f32 - 1.0) * BTN_SPACING + BTN_R + 10.0) * scale
    }

    fn centers(&self, win_w: f32, scale: f32) -> Vec<(Button, f32)> {
        let n = self.buttons.len();
        self.buttons
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let cx = if self.left {
                    (EDGE_MARGIN + i as f32 * BTN_SPACING) * scale
                } else {
                    win_w - (EDGE_MARGIN + (n - 1 - i) as f32 * BTN_SPACING) * scale
                };
                (b, cx)
            })
            .collect()
    }

    pub fn button_at(&self, x: f32, y: f32, win_w: f32, bar_h: f32, scale: f32) -> Option<Button> {
        if y < 0.0 || y >= bar_h {
            return None;
        }
        let cy = bar_h / 2.0;
        self.centers(win_w, scale)
            .into_iter()
            .find(|&(_, cx)| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() <= (BTN_R + 5.0) * scale)
            .map(|(b, _)| b)
    }

    pub fn draw(&self, g: &mut Gfx, win_w: f32, bar_h: f32, scale: f32, hover: Option<Button>) {
        let cy = bar_h / 2.0;
        for (b, cx) in self.centers(win_w, scale) {
            let c = match b {
                Button::Close => CLOSE,
                Button::Minimize => MIN,
                Button::Maximize => MAX,
            };
            g.circle([cx, cy], BTN_R * scale, c);
            if hover == Some(b) {
                g.circle([cx, cy], 2.0 * scale, [0.0, 0.0, 0.0, 0.55]);
            }
        }
    }
}

/// SDL hit-test: resize edges, a draggable bar, everything else normal. `p` is in points.
pub fn hit_test(geom: &SharedGeom, px: i32, py: i32) -> HitTestResult {
    let Ok(g) = geom.lock() else { return HitTestResult::Normal };
    let (x, y) = (px as f32 * g.density, py as f32 * g.density);
    if !g.maximized {
        let rb = RESIZE_BORDER * g.scale;
        let cg = CORNER_GRAB * g.scale;
        let (l, r, t, b) = (x < rb, x >= g.w - rb, y < rb, y >= g.h - rb);
        let (lc, rc, tc, bc) = (x < cg, x >= g.w - cg, y < cg, y >= g.h - cg);
        if (t && lc) || (l && tc) {
            return HitTestResult::ResizeTopLeft;
        }
        if (t && rc) || (r && tc) {
            return HitTestResult::ResizeTopRight;
        }
        if (b && lc) || (l && bc) {
            return HitTestResult::ResizeBottomLeft;
        }
        if (b && rc) || (r && bc) {
            return HitTestResult::ResizeBottomRight;
        }
        if l {
            return HitTestResult::ResizeLeft;
        }
        if r {
            return HitTestResult::ResizeRight;
        }
        if t {
            return HitTestResult::ResizeTop;
        }
        if b {
            return HitTestResult::ResizeBottom;
        }
    }
    if y < g.bar_h && !g.controls.iter().any(|c| c.contains(x, y)) {
        return HitTestResult::Draggable;
    }
    HitTestResult::Normal
}

/// KDE: ~/.config/kwinrc, [org.kde.kdecoration2] ButtonsOnLeft / ButtonsOnRight (X close,
/// I minimize, A maximize).
fn kde_layout() -> Option<(bool, Vec<Button>)> {
    let home = std::env::var_os("HOME")?;
    let text = std::fs::read_to_string(std::path::Path::new(&home).join(".config/kwinrc")).ok()?;
    let (mut in_section, mut left_s, mut right_s) = (false, None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[org.kde.kdecoration2]";
            continue;
        }
        if in_section {
            if let Some(v) = line.strip_prefix("ButtonsOnLeft=") {
                left_s = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("ButtonsOnRight=") {
                right_s = Some(v.to_string());
            }
        }
    }
    let parse = |s: &str| -> Vec<Button> {
        s.chars()
            .filter_map(|c| match c {
                'X' => Some(Button::Close),
                'I' => Some(Button::Minimize),
                'A' => Some(Button::Maximize),
                _ => None,
            })
            .collect()
    };
    let left = left_s.as_deref().map(parse).unwrap_or_default();
    let right = right_s.as_deref().map(parse).unwrap_or_default();
    if !left.is_empty() {
        Some((true, left))
    } else if !right.is_empty() {
        Some((false, right))
    } else {
        None
    }
}

/// GNOME: gsettings org.gnome.desktop.wm.preferences button-layout ("appmenu:minimize,maximize,close").
fn gnome_layout() -> Option<(bool, Vec<Button>)> {
    if std::env::var("XDG_CURRENT_DESKTOP").map(|d| !d.contains("GNOME")).unwrap_or(true) {
        return None;
    }
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.wm.preferences", "button-layout"]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let s = s.trim().trim_matches('\'');
    let (lhs, rhs) = s.split_once(':')?;
    let names = |part: &str| -> Vec<Button> {
        part.split(',')
            .filter_map(|t| match t.trim() {
                "close" => Some(Button::Close),
                "minimize" => Some(Button::Minimize),
                "maximize" => Some(Button::Maximize),
                _ => None,
            })
            .collect()
    };
    let (left, right) = (names(lhs), names(rhs));
    if !left.is_empty() {
        Some((true, left))
    } else if !right.is_empty() {
        Some((false, right))
    } else {
        None
    }
}

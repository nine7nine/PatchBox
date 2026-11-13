//! Scripted input for tests (PATCHBOX_TEST): synthetic SDL events fed through the same handlers
//! as real ones, with points named by what is under them, so scripts survive layout changes.
//!
//! Steps, separated by ';':
//!   wait <ms>                      pause
//!   move <pt>                      pointer motion
//!   click <pt> [right|middle] [ctrl] [shift] [double]
//!   drag <pt> <pt> [ctrl] [shift]  press, move in steps, release
//!   wheel <pt> <dy> [ctrl]
//!   key <name>                     a key: 1-4, l, f, e, space, enter, escape, f9
//!   type <text>                    typed text (`+` for a space)
//!   shot <path.png>                write the next frame
//!   quit
//! Points (`+` stands for a space): `x,y` (window pixels), `node:<id>`, `out:<id>`, `in:<id>` (node ids as in the layout
//! file, e.g. `M:ardour`, `S:system`), `cell:<matrix key substring>:<col>:<row>`, `empty` (a
//! spot on the canvas with nothing under it), `bar:<label>` (a control: AUDIO, MIDI, Buffer,
//! Xruns, menu, zoom-, zoom+, zoomtrack, zoompct), `port:<client:port>` (its row in an expanded
//! node), `menu:<item label>`.

use crate::app::App;
use sdl3::event::Event;
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::{MouseButton, MouseState};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct Script {
    steps: VecDeque<String>,
    next_at: Instant,
    pub shot: Option<String>,
    pub done: bool,
}

pub struct Synth {
    pub ev: Event,
    pub ctrl: bool,
    pub shift: bool,
}

fn motion(x: f32, y: f32, d: f32) -> Event {
    Event::MouseMotion {
        timestamp: 0,
        window_id: 0,
        which: 0,
        mousestate: MouseState::from_sdl_state(0),
        x: x / d,
        y: y / d,
        xrel: 0.0,
        yrel: 0.0,
    }
}

fn button(down: bool, b: MouseButton, x: f32, y: f32, d: f32, clicks: u8) -> Event {
    if down {
        Event::MouseButtonDown { timestamp: 0, window_id: 0, which: 0, mouse_btn: b, clicks, x: x / d, y: y / d }
    } else {
        Event::MouseButtonUp { timestamp: 0, window_id: 0, which: 0, mouse_btn: b, clicks, x: x / d, y: y / d }
    }
}

impl Script {
    pub fn from_env() -> Option<Script> {
        let s = std::env::var("PATCHBOX_TEST").ok()?;
        let steps = s.split(';').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        Some(Script { steps, next_at: Instant::now(), shot: None, done: false })
    }

    pub fn next_at(&self) -> Option<Instant> {
        (!self.done).then_some(self.next_at)
    }

    /// Run due steps; returns the events to inject now.
    pub fn run(&mut self, app: &App, density: f32) -> Vec<Synth> {
        let mut out = Vec::new();
        if self.done || Instant::now() < self.next_at || self.shot.is_some() {
            return out;
        }
        while let Some(step) = self.steps.pop_front() {
            let w: Vec<&str> = step.split_whitespace().collect();
            let flag = |f: &str| w.contains(&f);
            let (ctrl, shift) = (flag("ctrl"), flag("shift"));
            let push = |out: &mut Vec<Synth>, ev| out.push(Synth { ev, ctrl, shift });
            match w.first().copied() {
                Some("wait") => {
                    let ms = w.get(1).and_then(|v| v.parse().ok()).unwrap_or(100);
                    self.next_at = Instant::now() + Duration::from_millis(ms);
                    eprintln!("[test] {step}");
                    return out;
                }
                Some("move") => {
                    if let Some(p) = w.get(1).and_then(|p| resolve(app, p)) {
                        push(&mut out, motion(p[0], p[1], density));
                    } else {
                        eprintln!("[test] cannot resolve: {step}");
                    }
                }
                Some("click") => {
                    let Some(p) = w.get(1).and_then(|p| resolve(app, p)) else {
                        eprintln!("[test] cannot resolve: {step}");
                        continue;
                    };
                    let b = if flag("right") {
                        MouseButton::Right
                    } else if flag("middle") {
                        MouseButton::Middle
                    } else {
                        MouseButton::Left
                    };
                    let clicks = if flag("double") { 2 } else { 1 };
                    push(&mut out, motion(p[0], p[1], density));
                    push(&mut out, button(true, b, p[0], p[1], density, clicks));
                    push(&mut out, button(false, b, p[0], p[1], density, clicks));
                }
                Some("drag") => {
                    let (Some(a), Some(b)) = (w.get(1).and_then(|p| resolve(app, p)), w.get(2).and_then(|p| resolve(app, p))) else {
                        eprintln!("[test] cannot resolve: {step}");
                        continue;
                    };
                    push(&mut out, motion(a[0], a[1], density));
                    push(&mut out, button(true, MouseButton::Left, a[0], a[1], density, 1));
                    for i in 1..=12 {
                        let t = i as f32 / 12.0;
                        push(&mut out, motion(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, density));
                    }
                    push(&mut out, button(false, MouseButton::Left, b[0], b[1], density, 1));
                }
                Some("wheel") => {
                    let Some(p) = w.get(1).and_then(|p| resolve(app, p)) else { continue };
                    let dy: f32 = w.get(2).and_then(|v| v.parse().ok()).unwrap_or(1.0);
                    push(&mut out, motion(p[0], p[1], density));
                    push(
                        &mut out,
                        Event::MouseWheel {
                            timestamp: 0,
                            window_id: 0,
                            which: 0,
                            x: 0.0,
                            y: dy,
                            direction: sdl3::mouse::MouseWheelDirection::Normal,
                            mouse_x: p[0] / density,
                            mouse_y: p[1] / density,
                            integer_x: 0,
                            integer_y: dy as i32,
                        },
                    );
                }
                Some("key") => {
                    let k = match w.get(1).copied().unwrap_or("") {
                        "1" => Keycode::_1,
                        "2" => Keycode::_2,
                        "3" => Keycode::_3,
                        "4" => Keycode::_4,
                        "l" => Keycode::L,
                        "e" => Keycode::E,
                        "g" => Keycode::G,
                        "f" => Keycode::F,
                        "f9" => Keycode::F9,
                        "space" => Keycode::Space,
                        "enter" => Keycode::Return,
                        "z" => Keycode::Z,
                        "y" => Keycode::Y,
                        _ => Keycode::Escape,
                    };
                    push(
                        &mut out,
                        Event::KeyDown {
                            timestamp: 0,
                            window_id: 0,
                            keycode: Some(k),
                            scancode: None,
                            keymod: Mod::NOMOD,
                            repeat: false,
                            which: 0,
                            raw: 0,
                        },
                    );
                }
                Some("type") => {
                    let text = step.splitn(2, ' ').nth(1).unwrap_or("").replace('+', " ");
                    push(&mut out, Event::TextInput { timestamp: 0, window_id: 0, text });
                }
                Some("shot") => {
                    self.shot = w.get(1).map(|s| s.to_string());
                    eprintln!("[test] {step}");
                    return out;
                }
                Some("quit") => {
                    self.done = true;
                    out.push(Synth { ev: Event::Quit { timestamp: 0 }, ctrl: false, shift: false });
                    return out;
                }
                _ => eprintln!("[test] unknown step: {step}"),
            }
            eprintln!("[test] {step}");
        }
        self.done = true;
        out
    }
}

/// A point named by what is there (window pixels).
pub fn resolve(app: &App, p: &str) -> Option<[f32; 2]> {
    let p = &p.replace('+', " ");
    let xf = app.xf();
    if let Some((x, y)) = p.split_once(',')
        && let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
            return Some([x, y]);
        }
    if p == "empty" {
        let a = app.canvas.area;
        return Some([a.right() - 20.0, a.bottom() - 20.0]);
    }
    let (kind, rest) = p.split_once(':')?;
    match kind {
        "node" | "out" | "in" => {
            let i = app.model.node_by_id(rest)?;
            let (o, n) = app.handles(i);
            Some(xf.s(match kind {
                "node" => app.node_pos(i),
                "out" => o,
                _ => n,
            }))
        }
        "cell" => {
            let mut it = rest.rsplitn(3, ':');
            let row: usize = it.next()?.parse().ok()?;
            let col: usize = it.next()?.parse().ok()?;
            let key = it.next()?;
            let m = app.mats.iter().find(|m| m.key.contains(key))?;
            let c = xf.s(app.matrix_pos(m));
            let s = App::matrix_size(m);
            let cell = crate::theme::CELL * xf.z;
            Some([c[0] - s[0] * xf.z / 2.0 + (col as f32 + 0.5) * cell, c[1] - s[1] * xf.z / 2.0 + (row as f32 + 0.5) * cell])
        }
        "bar" => app.bar_point(rest),
        "port" => app.port_point(rest),
        "dlg" => app.dialog.as_ref()?.point(rest),
        "menu" => {
            let m = app.ui.menu.as_ref()?;
            let i = m.items.iter().position(|it| it.label.contains(rest))?;
            m.rows.get(i).map(|r| [r.cx(), r.cy()])
        }
        _ => None,
    }
}

//! PatchBox: a JACK patchbay (and, next, session manager) drawn with SDL3 + GL.
//!
//! The main loop sleeps until input, a JACK notification or a timer is due, and draws only when
//! something changed: an audio workstation's patchbay should cost nothing while idle. JACK's
//! notification thread wakes it with one SDL user event at a time (coalesced).

mod app;
mod canvas;
mod csd;
mod dialog;
mod gfx;
mod layout;
mod model;
mod server;
mod session;
mod sessui;
mod shadow;
mod text;
mod testscript;
mod theme;
mod ui;
mod undo;

use app::{Action, App, WindowCmd};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::MouseButton;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The registered SDL event type JACK wakes us with, and whether one is already queued.
static WAKE_TYPE: AtomicU32 = AtomicU32::new(0);
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);
/// Touchpad pinch: the scale accumulated since the main loop last looked (f32 bits). The sdl3
/// crate doesn't decode SDL 3.4's pinch events, so an event watch reads them raw.
static PINCH: AtomicU32 = AtomicU32::new(0x3f80_0000);

unsafe extern "C" fn pinch_watch(_: *mut std::ffi::c_void, ev: *mut sdl3::sys::events::SDL_Event) -> bool {
    unsafe {
        if (*ev).r#type == sdl3::sys::events::SDL_EVENT_PINCH_UPDATE.0 {
            let s = (*ev).pinch.scale;
            if s.is_finite() && s > 0.0 {
                let mut cur = PINCH.load(Ordering::Acquire);
                while let Err(now) = PINCH.compare_exchange_weak(cur, (f32::from_bits(cur) * s).to_bits(), Ordering::AcqRel, Ordering::Acquire) {
                    cur = now;
                }
            }
        }
    }
    true
}

fn wake() {
    if WAKE_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut ev: sdl3::sys::events::SDL_Event = unsafe { std::mem::zeroed() };
    ev.r#type = WAKE_TYPE.load(Ordering::Relaxed);
    unsafe { sdl3::sys::events::SDL_PushEvent(&mut ev) };
}

const CORNER_RADIUS: f32 = 6.0;

fn main() {
    let sdl = sdl3::init().expect("SDL init");
    let video = sdl.video().expect("SDL video");
    let wake_type = unsafe { sdl3::sys::events::SDL_RegisterEvents(1) };
    WAKE_TYPE.store(wake_type, Ordering::Relaxed);

    let mut app = App::new(Arc::new(wake));

    let msaa = !matches!(std::env::var("PATCHBOX_MSAA").as_deref(), Ok("0"));
    let [w, h] = app.layout.window.unwrap_or([1280, 760]);
    let make = |samples: u8| {
        let attr = video.gl_attr();
        attr.set_context_profile(sdl3::video::GLProfile::Core);
        attr.set_context_version(3, 3);
        attr.set_double_buffer(true);
        attr.set_alpha_size(8);
        attr.set_multisample_buffers(if samples > 0 { 1 } else { 0 });
        attr.set_multisample_samples(samples);
        let mut b = video.window("PatchBox", w, h);
        b.opengl().resizable().borderless().high_pixel_density();
        let flags = b.window_flags() | sdl3::sys::video::SDL_WINDOW_TRANSPARENT.0 as u32;
        b.set_window_flags(flags);
        b.build()
    };
    let mut window = match make(if msaa { 4 } else { 0 }) {
        Ok(w) => w,
        Err(e) if msaa => {
            eprintln!("[patchbox] no 4x MSAA window ({e}); without");
            make(0).expect("window")
        }
        Err(e) => panic!("window: {e}"),
    };
    let _ = window.set_minimum_size(640, 400);
    let gl_ctx = window.gl_create_context().expect("GL context");
    window.gl_make_current(&gl_ctx).expect("GL current");
    let _ = video.gl_set_swap_interval(sdl3::video::SwapInterval::VSync);
    let mut gfx = gfx::Gfx::new(|s| video.gl_get_proc_address(s).map_or(std::ptr::null(), |f| f as *const std::ffi::c_void));
    unsafe {
        use glow::HasContext;
        gfx.gl().enable(glow::MULTISAMPLE);
    }
    let _shadow = if matches!(std::env::var("PATCHBOX_SHADOW").as_deref(), Ok("0")) {
        None
    } else {
        shadow::attach(window.raw(), CORNER_RADIUS as i32)
    };

    let geom: csd::SharedGeom = Arc::new(Mutex::new(csd::HitGeom::default()));
    {
        let g = geom.clone();
        let _ = window.set_hit_test(move |p| csd::hit_test(&g, p.x, p.y));
    }

    let snapshot = std::env::var("PATCHBOX_SNAPSHOT").ok();
    let snapshot_delay = Duration::from_millis(std::env::var("PATCHBOX_SNAPSHOT_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500));
    let snapshot_exit = std::env::var_os("PATCHBOX_SNAPSHOT_EXIT").is_some();
    let started = Instant::now();
    let mut snapshot_done = false;
    if let Ok(k) = std::env::var("PATCHBOX_KIND") {
        app.kind = app::kind_from(&k);
    }

    let mut script = testscript::Script::from_env();
    let mut pump = sdl.event_pump().expect("event pump");
    let kb = sdl.keyboard();
    let mut density = window.pixel_density();
    app.chrome = window.display_scale().max(0.5);
    app.apply_scales();
    unsafe { sdl3::sys::events::SDL_AddEventWatch(Some(pinch_watch), std::ptr::null_mut()) };
    let (pw, ph) = window.size_in_pixels();
    app.win = [pw as f32, ph as f32];
    app.redraw = true;

    let stats = std::env::var_os("PATCHBOX_STATS").is_some();
    let mut text_on = false;
    // Test scripts start once JACK was there, and keep running if it goes away.
    let mut ever_connected = false;
    let (mut loops, mut frames) = (0u64, 0u64);
    let mut wakes: std::collections::BTreeMap<String, u64> = Default::default();
    'main: loop {
        loops += 1;
        let mut next = app.tick();
        ever_connected |= app.jack.is_some();
        if let Some(t) = script.as_ref().and_then(|s| s.next_at())
            && ever_connected {
                let t = t.max(started + Duration::from_millis(800));
                next = Some(next.map_or(t, |n| n.min(t)));
            }
        let mut pending: Vec<Event> = Vec::new();
        let snapshot_due = snapshot.is_some() && !snapshot_done;
        if app.redraw {
            pending.extend(pump.poll_iter());
        } else {
            let now = Instant::now();
            let mut until = next.map(|t| t.saturating_duration_since(now)).unwrap_or(Duration::from_secs(3600));
            if snapshot_due {
                until = until.min((started + snapshot_delay).saturating_duration_since(now));
            }
            match pump.wait_event_timeout(until.max(Duration::from_millis(1))) {
                Some(e) => {
                    if stats {
                        let name = format!("{e:?}");
                        let name = name.split([' ', '{', '(']).next().unwrap_or("?").to_string();
                        *wakes.entry(name).or_default() += 1;
                    }
                    pending.push(e);
                    pending.extend(pump.poll_iter());
                }
                None => {
                    if stats {
                        *wakes.entry(format!("timeout({}ms)", until.as_millis())).or_default() += 1;
                    }
                }
            }
        }
        let mut events: Vec<(Event, Option<(bool, bool)>)> = pending.into_iter().map(|e| (e, None)).collect();
        if let Some(s) = script.as_mut()
            && ever_connected && started.elapsed() >= Duration::from_millis(800) {
                for syn in s.run(&app, density) {
                    events.push((syn.ev, Some((syn.ctrl, syn.shift))));
                }
                if s.shot.is_some() {
                    app.redraw = true;
                }
            }
        for (ev, forced) in events {
            let (ctrl, shift) = forced.unwrap_or_else(|| {
                let mods = kb.mod_state();
                (mods.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD), mods.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD))
            });
            match ev {
                Event::Quit { .. } => break 'main,
                Event::Unknown { type_, .. } if type_ == sdl3::sys::events::SDL_EVENT_PINCH_UPDATE.0 => {
                    let f = f32::from_bits(PINCH.swap(0x3f80_0000, Ordering::AcqRel));
                    if (f - 1.0).abs() > 1e-4 && app.canvas.area.contains(app.ui.mouse[0], app.ui.mouse[1]) {
                        let z = app.layout.view.zoom * f;
                        app.zoom_at(app.ui.mouse, z);
                    }
                }
                Event::User { type_, .. } if type_ == wake_type => {
                    WAKE_PENDING.store(false, Ordering::Release);
                    app.on_wake();
                }
                Event::Window { win_event, .. } => match win_event {
                    WindowEvent::PixelSizeChanged(..) | WindowEvent::Resized(..) | WindowEvent::DisplayChanged(..) => {
                        let (pw, ph) = window.size_in_pixels();
                        app.win = [pw as f32, ph as f32];
                        density = window.pixel_density();
                        app.chrome = window.display_scale().max(0.5);
                        app.apply_scales();
                        if !app.maximized {
                            let (lw, lh) = window.size();
                            app.layout.window = Some([lw, lh]);
                        }
                        app.redraw = true;
                    }
                    WindowEvent::Maximized => {
                        app.maximized = true;
                        app.redraw = true;
                    }
                    WindowEvent::Restored => {
                        app.maximized = false;
                        app.redraw = true;
                    }
                    WindowEvent::CloseRequested => break 'main,
                    WindowEvent::MouseLeave => {
                        app.ui.mouse = [-1.0, -1.0];
                        app.ui.csd_hover = None;
                        app.canvas.hover = canvas::Hover::None;
                        app.redraw = true;
                    }
                    WindowEvent::Exposed | WindowEvent::Shown => app.redraw = true,
                    _ => {}
                },
                Event::MouseMotion { x, y, .. } => {
                    let (px, py) = (x * density, y * density);
                    app.ui.mouse = [px, py];
                    if app.ui.menu.is_some() || app.dialog.is_some() {
                        app.redraw = true;
                        continue;
                    }
                    if app.ui.slider.is_some() {
                        app.zoom_slider_drag(px);
                        continue;
                    }
                    let ch = app.ui.csd.button_at(px, py, app.win[0], app.ui.bar_h, app.chrome);
                    let hh = app.ui.hits.iter().rposition(|(r, _)| r.contains(px, py));
                    if ch != app.ui.csd_hover || hh != app.ui.hover_hit {
                        app.ui.csd_hover = ch;
                        app.ui.hover_hit = hh;
                        app.redraw = true;
                    }
                    let dragging = !matches!(app.canvas.drag, canvas::Drag::None);
                    if dragging || app.canvas.area.contains(px, py) {
                        app.canvas_motion(px, py);
                    } else if app.canvas.hover != canvas::Hover::None {
                        app.canvas.hover = canvas::Hover::None;
                        app.redraw = true;
                    }
                }
                Event::MouseButtonDown { mouse_btn, x, y, clicks, .. } => {
                    let (px, py) = (x * density, y * density);
                    app.ui.mouse = [px, py];
                    if app.ui.menu.is_some() {
                        app.menu_click(px, py);
                        continue;
                    }
                    // A dialog is modal.
                    if let Some(d) = app.dialog.as_mut() {
                        if mouse_btn == MouseButton::Left {
                            for ev in d.click(px, py) {
                                app.dialog_event(ev);
                            }
                        }
                        app.redraw = true;
                        continue;
                    }
                    if mouse_btn == MouseButton::Left
                        && let Some(b) = app.ui.csd.button_at(px, py, app.win[0], app.ui.bar_h, app.chrome)
                    {
                        app.apply(Action::Window(b));
                        continue;
                    }
                    // Controls drawn over the canvas (zoom) come before the canvas itself.
                    let over_canvas_control = app.canvas.area.contains(px, py) && app.hit_action(px, py).is_some();
                    if py < app.ui.bar_h || app.ui.sidebar_rect.contains(px, py) || over_canvas_control {
                        if mouse_btn == MouseButton::Left
                            && let Some(a) = app.hit_action(px, py)
                        {
                            let slider = matches!(a, Action::ZoomSlider(_));
                            app.apply(a);
                            if slider {
                                app.zoom_slider_drag(px);
                            }
                        }
                        // A client row's menu.
                        if mouse_btn == MouseButton::Right
                            && let Some(Action::CenterNode(id)) = app.hit_action(px, py)
                            && let Some(i) = app.model.node_by_id(&id)
                        {
                            let items = app.node_menu(i, true);
                            app.ui.open_menu([px, py], items);
                            app.redraw = true;
                        }
                        if py < app.ui.bar_h && clicks == 2 && app.hit_action(px, py).is_none() {
                            app.window_cmd = Some(WindowCmd::ToggleMaximize);
                        }
                    } else if app.canvas.area.contains(px, py) {
                        app.canvas_down(mouse_btn, px, py, ctrl, shift, clicks);
                    }
                }
                Event::MouseButtonUp { x, y, .. } => {
                    let (px, py) = (x * density, y * density);
                    if app.ui.slider.take().is_some() {
                        app.redraw = true;
                    }
                    if !matches!(app.canvas.drag, canvas::Drag::None) {
                        app.canvas_up(px, py, ctrl);
                    }
                }
                Event::MouseWheel { x, y, mouse_x, mouse_y, .. } => {
                    let (px, py) = (mouse_x * density, mouse_y * density);
                    if let Some(d) = app.dialog.as_mut() {
                        d.scroll_by(-y * 40.0 * app.scale);
                        app.redraw = true;
                    } else if app.ui.sidebar_rect.contains(px, py) {
                        app.ui.sidebar_scroll -= y * 40.0 * app.scale;
                        app.redraw = true;
                    } else if app.canvas.area.contains(px, py) {
                        app.canvas.mouse = [px, py];
                        app.canvas_wheel(x, y, ctrl);
                    }
                }
                Event::TextInput { text, .. } => {
                    if let Some(d) = app.dialog.as_mut() {
                        d.text_input(&text);
                        app.redraw = true;
                    }
                }
                Event::KeyDown { keycode: Some(k), .. } if app.dialog.is_some() => {
                    let d = app.dialog.as_mut().unwrap();
                    app.redraw = true;
                    match k {
                        Keycode::Escape if app.ui.menu.is_some() => app.ui.menu = None,
                        Keycode::Escape if d.editing.is_some() => d.editing = None,
                        Keycode::Escape => app.dialog = None,
                        Keycode::Return | Keycode::KpEnter => {
                            // Enter commits a field; in a one-field dialog it also confirms.
                            let one_field = matches!(d.kind, dialog::Kind::NewSession | dialog::Kind::Rename | dialog::Kind::Duplicate | dialog::Kind::Notes | dialog::Kind::Snapshot | dialog::Kind::SaveTemplate | dialog::Kind::RenameItem);
                            let last = d.buttons.len().saturating_sub(1);
                            if let Some(ev) = d.commit() {
                                app.dialog_event(ev);
                            }
                            if one_field && app.dialog.is_some() {
                                app.dialog_event(dialog::Event::Button(last));
                            }
                        }
                        Keycode::Backspace => d.backspace(),
                        _ => {}
                    }
                }
                Event::KeyDown { keycode: Some(k), .. } => {
                    let a = match k {
                        Keycode::Escape => {
                            app.ui.menu = None;
                            app.cancel_drag();
                            app.redraw = true;
                            None
                        }
                        Keycode::Z if ctrl && shift => Some(Action::Redo),
                        Keycode::Z if ctrl => Some(Action::Undo),
                        Keycode::Y if ctrl => Some(Action::Redo),
                        Keycode::_1 => Some(Action::SetKind(pbx_jack::PortKind::Audio)),
                        Keycode::_2 => Some(Action::SetKind(pbx_jack::PortKind::Midi)),
                        Keycode::_3 => Some(Action::SetKind(pbx_jack::PortKind::Cv)),
                        Keycode::_4 => Some(Action::SetKind(pbx_jack::PortKind::Osc)),
                        Keycode::_0 if ctrl => Some(Action::ZoomTo(1.0)),
                        Keycode::Equals | Keycode::Plus | Keycode::KpPlus if ctrl && shift => Some(Action::SetUiScale(app.layout.view.ui_scale + 0.25)),
                        Keycode::Minus | Keycode::KpMinus if ctrl && shift => Some(Action::SetUiScale(app.layout.view.ui_scale - 0.25)),
                        Keycode::Equals | Keycode::Plus | Keycode::KpPlus if ctrl => Some(Action::ZoomBy(1.2)),
                        Keycode::Minus | Keycode::KpMinus if ctrl => Some(Action::ZoomBy(1.0 / 1.2)),
                        Keycode::F => Some(Action::FitAll),
                        Keycode::L => Some(Action::ToggleLabels),
                        Keycode::E => Some(Action::ExpandAll),
                        Keycode::Space => Some(Action::TransportToggle),
                        Keycode::F9 => Some(Action::ToggleSidebar),
                        Keycode::Q if ctrl => Some(Action::Quit),
                        _ => None,
                    };
                    if let Some(a) = a {
                        app.apply(a);
                    }
                }
                _ => {}
            }
            if app.quit {
                break 'main;
            }
        }
        if app.quit {
            break;
        }
        // Typed text only while a field is being edited (an input method is a cost).
        let want_text = app.dialog.as_ref().is_some_and(|d| d.editing.is_some());
        if want_text != text_on {
            text_on = want_text;
            if want_text {
                video.text_input().start(&window);
            } else {
                video.text_input().stop(&window);
            }
        }
        if let Some(cmd) = app.window_cmd.take() {
            match cmd {
                WindowCmd::Minimize => {
                    window.minimize();
                }
                WindowCmd::ToggleMaximize => {
                    if app.maximized {
                        window.restore();
                    } else {
                        window.maximize();
                    }
                }
            }
        }
        let snap_now = snapshot_due && started.elapsed() >= snapshot_delay;
        if app.redraw || snap_now {
            app.redraw = false;
            frames += 1;
            app.layout_regions();
            if app.fit_pending {
                app.fit_pending = false;
                app.fit_all();
            }
            gfx.begin(app.win[0] as u32, app.win[1] as u32);
            app.draw_canvas(&mut gfx);
            app.draw_ui(&mut gfx);
            if let Some(mut d) = app.dialog.take() {
                d.draw(&mut gfx, app.win, app.scale, app.ui.mouse);
                let help = d.hover_help.clone();
                app.dialog = Some(d);
                if let Some((text, at)) = help
                    && app.ui.menu.is_none()
                {
                    app.draw_tooltip(&mut gfx, at, &[(text, theme::TEXT_BRIGHT)]);
                }
            }
            app.draw_menu(&mut gfx);
            if !app.maximized {
                gfx.erase_corners(CORNER_RADIUS * density);
            }
            gfx.end();
            if let Ok(mut g) = geom.lock() {
                g.w = app.win[0];
                g.h = app.win[1];
                g.bar_h = app.ui.bar_h;
                g.controls = app.bar_controls();
                g.density = density;
                g.scale = app.chrome;
                g.maximized = app.maximized;
            }
            if gfx.needs_redraw() {
                app.redraw = true;
            }
            if let Some(path) = script.as_mut().and_then(|s| if gfx.needs_redraw() { None } else { s.shot.take() }) {
                write_png(&path, &gfx);
                eprintln!("[test] wrote {path}");
            }
            if snap_now && !gfx.needs_redraw() {
                if let Some(path) = &snapshot {
                    write_png(path, &gfx);
                    eprintln!("[patchbox] snapshot written to {path}");
                }
                snapshot_done = true;
                if snapshot_exit {
                    window.gl_swap_window();
                    break;
                }
            }
            window.gl_swap_window();
        }
    }
    app.save_layout();
    if stats {
        let t = started.elapsed().as_secs_f32();
        eprintln!("[stats] wakes: {wakes:?}");
        eprintln!("[stats] {t:.1} s: {loops} loop passes ({:.1}/s), {frames} frames ({:.1}/s)", loops as f32 / t, frames as f32 / t);
    }
}

fn write_png(path: &str, gfx: &gfx::Gfx) {
    let px = gfx.read_rgba();
    let (w, h) = (gfx.w as u32, gfx.h as u32);
    let Ok(file) = std::fs::File::create(path) else {
        eprintln!("[patchbox] cannot write {path}");
        return;
    };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    if let Ok(mut wr) = enc.write_header() {
        let _ = wr.write_image_data(&px);
    }
}

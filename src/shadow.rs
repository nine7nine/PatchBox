//! Window shadows on KDE Plasma (KWin's `org_kde_kwin_shadow`).
//!
//! Our window is borderless (csd.rs draws the titlebar in GL), so the compositor draws no shadow
//! around it — unlike libdecor-decorated windows or KDE's own apps. From teams-sdl-cef (same owner).
//! KWin lets a client attach a shadow to its wl_surface: eight tiles (corners, plus edges it
//! stretches) and how far they reach past each edge. KWin draws it OUTSIDE the surface, so input,
//! maximize and snapping are unaffected. Other compositors don't offer the protocol: no shadow.
//!
//! Uses SDL's own wl_display (a foreign display for wayland-client) on a private event queue.

use std::os::fd::AsFd;
use wayland_client::{
    backend::{Backend, ObjectId},
    protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_plasma::shadow::client::{org_kde_kwin_shadow, org_kde_kwin_shadow_manager};

/// How far the shadow reaches past the window edges, in px (bottom more: light from above).
const SIDE: i32 = 26;
const TOP: i32 = 20;
const BOTTOM: i32 = 32;
/// Peak opacity at the window edge.
const MAX_ALPHA: f32 = 0.42;

#[derive(Default)]
struct Globals {
    shm: Option<(u32, u32)>,
    shadow_mgr: Option<(u32, u32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(s: &mut Self, _: &wl_registry::WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = e {
            match interface.as_str() {
                "wl_shm" => s.shm = Some((name, version)),
                "org_kde_kwin_shadow_manager" => s.shadow_mgr = Some((name, version)),
                _ => {}
            }
        }
    }
}
impl Dispatch<wl_shm::WlShm, ()> for Globals {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_shm_pool::WlShmPool, ()> for Globals {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_buffer::WlBuffer, ()> for Globals {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<org_kde_kwin_shadow_manager::OrgKdeKwinShadowManager, ()> for Globals {
    fn event(
        _: &mut Self, _: &org_kde_kwin_shadow_manager::OrgKdeKwinShadowManager, _: org_kde_kwin_shadow_manager::Event, _: &(),
        _: &Connection, _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<org_kde_kwin_shadow::OrgKdeKwinShadow, ()> for Globals {
    fn event(
        _: &mut Self, _: &org_kde_kwin_shadow::OrgKdeKwinShadow, _: org_kde_kwin_shadow::Event, _: &(), _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The shadow attached to one window; kept alive with it (its buffers back the shadow).
pub struct Shadow {
    _conn: Connection,
    _shadow: org_kde_kwin_shadow::OrgKdeKwinShadow,
    _buffers: Vec<wl_buffer::WlBuffer>,
    _pool: wl_shm_pool::WlShmPool,
}

/// Opacity of the shadow at `d` px outside the edge, for a shadow `size` px deep: a smooth falloff
/// to 0 at the far end.
fn falloff(d: f32, size: f32) -> f32 {
    let t = (d / size).clamp(0.0, 1.0);
    MAX_ALPHA * (1.0 - t).powf(2.2)
}

/// Attach a shadow to an SDL window's wl_surface. None off KWin, off Wayland, or on any failure.
/// `radius`: the window's rounded corners (desktop px; 0 = square), which the shadow follows.
pub fn attach(window: *mut sdl3::sys::video::SDL_Window, radius: i32) -> Option<Shadow> {
    use sdl3::sys::{properties::SDL_GetPointerProperty, video::*};
    let (display, surface) = unsafe {
        let props = SDL_GetWindowProperties(window);
        (
            SDL_GetPointerProperty(props, SDL_PROP_WINDOW_WAYLAND_DISPLAY_POINTER, std::ptr::null_mut()),
            SDL_GetPointerProperty(props, SDL_PROP_WINDOW_WAYLAND_SURFACE_POINTER, std::ptr::null_mut()),
        )
    };
    if display.is_null() || surface.is_null() {
        return None;
    }
    let backend = unsafe { Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue = conn.new_event_queue::<Globals>();
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    let mut g = Globals::default();
    queue.roundtrip(&mut g).ok()?;
    let (Some((mgr_name, mgr_ver)), Some((shm_name, _))) = (g.shadow_mgr, g.shm) else {
        return None; // not KWin
    };
    let shm: wl_shm::WlShm = registry.bind(shm_name, 1, &qh, ());
    let mgr: org_kde_kwin_shadow_manager::OrgKdeKwinShadowManager = registry.bind(mgr_name, mgr_ver.min(2), &qh, ());
    let sid = unsafe { ObjectId::from_ptr(wl_surface::WlSurface::interface(), surface.cast()) }.ok()?;
    let surface = wl_surface::WlSurface::from_id(&conn, sid).ok()?;

    // The tiles, in KWin's order: left, top-left, top, top-right, right, bottom-right, bottom,
    // bottom-left. Edges are 1 px long (KWin stretches them); corners span both depths, plus the
    // corner radius: KWin anchors them at the shadow's outer corners, so the extra `r` reaches under
    // the window's rounded-off pixels (as Breeze's shadows do) and the shadow follows the curve.
    let (s, t, b, r) = (SIDE, TOP, BOTTOM, radius.max(0));
    let tiles: [(i32, i32); 8] = [(s, 1), (s + r, t + r), (1, t), (s + r, t + r), (s, 1), (s + r, b + r), (1, b), (s + r, b + r)];
    let pixel = |tile: usize, x: i32, y: i32, w: i32, h: i32| -> f32 {
        // Distance outward from the window edge(s) this tile sits against.
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let (wf, hf, rf) = (w as f32, h as f32, r as f32);
        // A corner of the rounded window: the arc's centre (cx, cy) in the tile, and the outward
        // distance from that centre along each axis.
        let corner = |qx: f32, qy: f32, sx: f32, sy: f32| {
            let (kx, ky) = (qx.max(0.0), qy.max(0.0));
            let k = (kx * kx + ky * ky).sqrt();
            if k <= rf {
                return MAX_ALPHA; // under the window
            }
            // From the nearest point of the arc (or the straight side next to it) to here, with the
            // two depths blended elliptically.
            let (vx, vy) = (kx * (1.0 - rf / k), ky * (1.0 - rf / k));
            let d = ((vx / sx).powi(2) + (vy / sy).powi(2)).sqrt();
            MAX_ALPHA * (1.0 - d.clamp(0.0, 1.0)).powf(2.2)
        };
        let (sf, tf, bf) = (s as f32, t as f32, b as f32);
        match tile {
            0 => falloff(wf - fx, sf),
            1 => corner(wf - fx, hf - fy, sf, tf),
            2 => falloff(hf - fy, tf),
            3 => corner(fx - rf, hf - fy, sf, tf),
            4 => falloff(fx, sf),
            5 => corner(fx - rf, fy - rf, sf, bf),
            6 => falloff(fy, bf),
            _ => corner(wf - fx, fy - rf, sf, bf),
        }
    };
    let total: usize = tiles.iter().map(|(w, h)| (w * h * 4) as usize).sum();
    let fd = memfd()?;
    let file = std::fs::File::from(fd);
    file.set_len(total as u64).ok()?;
    let mut data = Vec::with_capacity(total);
    for (i, &(w, h)) in tiles.iter().enumerate() {
        for y in 0..h {
            for x in 0..w {
                // wl_shm ARGB8888 is premultiplied, little-endian B,G,R,A: black at alpha a.
                let a = (pixel(i, x, y, w, h) * 255.0).round() as u8;
                data.extend_from_slice(&[0, 0, 0, a]);
            }
        }
    }
    use std::io::Write;
    (&file).write_all(&data).ok()?;
    let pool = shm.create_pool(file.as_fd(), total as i32, &qh, ());
    let mut offset = 0;
    let mut buffers = Vec::with_capacity(8);
    for &(w, h) in &tiles {
        buffers.push(pool.create_buffer(offset, w, h, w * 4, wl_shm::Format::Argb8888, &qh, ()));
        offset += w * h * 4;
    }
    let shadow = mgr.create(&surface, &qh, ());
    shadow.attach_left(&buffers[0]);
    shadow.attach_top_left(&buffers[1]);
    shadow.attach_top(&buffers[2]);
    shadow.attach_top_right(&buffers[3]);
    shadow.attach_right(&buffers[4]);
    shadow.attach_bottom_right(&buffers[5]);
    shadow.attach_bottom(&buffers[6]);
    shadow.attach_bottom_left(&buffers[7]);
    shadow.set_left_offset(s as f64);
    shadow.set_top_offset(t as f64);
    shadow.set_right_offset(s as f64);
    shadow.set_bottom_offset(b as f64);
    shadow.commit();
    // Shadow state is double-buffered on the surface: SDL's next frame commits it.
    conn.flush().ok()?;
    Some(Shadow { _conn: conn, _shadow: shadow, _buffers: buffers, _pool: pool })
}

fn memfd() -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::memfd_create(c"patchbox-shadow".as_ptr(), libc::MFD_CLOEXEC) };
    (fd >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

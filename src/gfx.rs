//! The 2D renderer: everything PatchBox shows is triangles from here, in window pixels.
//!
//! One shader (colour × atlas coverage), one streamed vertex buffer, one texture (the glyph atlas,
//! whose white block serves solid shapes). Shapes are tessellated on the CPU each frame; edges are
//! smoothed by the window's MSAA. Clip rectangles split the frame into scissored batches.

use crate::text::{ATLAS, Face, Fonts, WHITE_UV};
use glow::HasContext;

pub type Color = [f32; 4];

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

pub fn alpha(c: Color, a: f32) -> Color {
    [c[0], c[1], c[2], c[3] * a]
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn centered(cx: f32, cy: f32, w: f32, h: f32) -> Rect {
        Rect { x: cx - w / 2.0, y: cy - h / 2.0, w, h }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect { x: self.x + d, y: self.y + d, w: (self.w - 2.0 * d).max(0.0), h: (self.h - 2.0 * d).max(0.0) }
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect { x, y, w: (r - x).max(0.0), h: (b - y).max(0.0) }
    }
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

struct Batch {
    clip: Option<Rect>,
    start: usize,
    /// Clears what it covers to transparent (the window's rounded corners).
    erase: bool,
}

pub struct Gfx {
    gl: glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    atlas: glow::Texture,
    u_screen: Option<glow::UniformLocation>,
    verts: Vec<Vertex>,
    batches: Vec<Batch>,
    clips: Vec<Rect>,
    pub w: f32,
    pub h: f32,
    pub fonts: Fonts,
}

const VS: &str = r#"#version 330 core
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_color;
uniform vec2 u_screen;
out vec2 v_uv;
out vec4 v_color;
void main() {
    v_uv = a_uv;
    v_color = a_color;
    gl_Position = vec4(a_pos.x / u_screen.x * 2.0 - 1.0, 1.0 - a_pos.y / u_screen.y * 2.0, 0.0, 1.0);
}
"#;

const FS: &str = r#"#version 330 core
in vec2 v_uv;
in vec4 v_color;
uniform sampler2D u_atlas;
out vec4 frag;
void main() {
    float a = texture(u_atlas, v_uv).r;
    // Premultiplied output: the window is translucent only outside its rounded corners.
    frag = vec4(v_color.rgb * v_color.a * a, v_color.a * a);
}
"#;

impl Gfx {
    pub fn new(loader: impl FnMut(&str) -> *const std::ffi::c_void) -> Gfx {
        let gl = unsafe { glow::Context::from_loader_function(loader) };
        unsafe {
            let program = link_program(&gl, VS, FS);
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_atlas").as_ref(), 0);
            let u_screen = gl.get_uniform_location(program, "u_screen");
            let vao = gl.create_vertex_array().expect("vao");
            gl.bind_vertex_array(Some(vao));
            let vbo = gl.create_buffer().expect("vbo");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = std::mem::size_of::<Vertex>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 16);
            let atlas = gl.create_texture().expect("atlas");
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                ATLAS as i32,
                ATLAS as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            Gfx {
                gl,
                program,
                vao,
                vbo,
                atlas,
                u_screen,
                verts: Vec::with_capacity(1 << 16),
                batches: Vec::new(),
                clips: Vec::new(),
                w: 1.0,
                h: 1.0,
                fonts: Fonts::new(),
            }
        }
    }

    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    /// Start a frame of `w`×`h` pixels, cleared to transparent.
    pub fn begin(&mut self, w: u32, h: u32) {
        self.w = w.max(1) as f32;
        self.h = h.max(1) as f32;
        self.verts.clear();
        self.batches.clear();
        self.clips.clear();
        self.batches.push(Batch { clip: None, start: 0, erase: false });
        self.fonts.overflowed = false;
    }

    /// Draw everything queued since `begin`.
    pub fn end(&mut self) {
        let gl = &self.gl;
        unsafe {
            gl.viewport(0, 0, self.w as i32, self.h as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(self.u_screen.as_ref(), self.w, self.h);
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            if let Some((y0, y1)) = self.fonts.dirty.take() {
                let rows = &self.fonts.pixels[y0 * ATLAS..y1 * ATLAS];
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    y0 as i32,
                    ATLAS as i32,
                    (y1 - y0) as i32,
                    glow::RED,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(rows)),
                );
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes = std::slice::from_raw_parts(
                self.verts.as_ptr() as *const u8,
                self.verts.len() * std::mem::size_of::<Vertex>(),
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            let n = self.verts.len();
            for (i, b) in self.batches.iter().enumerate() {
                let end = self.batches.get(i + 1).map_or(n, |nb| nb.start);
                if end <= b.start {
                    continue;
                }
                match b.clip {
                    Some(r) => {
                        gl.enable(glow::SCISSOR_TEST);
                        let x = r.x.floor().max(0.0);
                        let y = (self.h - r.bottom()).floor().max(0.0);
                        gl.scissor(x as i32, y as i32, r.w.ceil().max(0.0) as i32, r.h.ceil().max(0.0) as i32);
                    }
                    None => gl.disable(glow::SCISSOR_TEST),
                }
                if b.erase {
                    gl.blend_func(glow::ZERO, glow::ONE_MINUS_SRC_ALPHA);
                } else {
                    gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                }
                gl.draw_arrays(glow::TRIANGLES, b.start as i32, (end - b.start) as i32);
            }
        }
    }

    /// The frame drew glyphs that were evicted mid-frame: draw it again.
    pub fn needs_redraw(&self) -> bool {
        self.fonts.overflowed
    }

    /// Read the framebuffer (after `end`, before the swap) as RGBA rows, top first.
    pub fn read_rgba(&self) -> Vec<u8> {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut buf = vec![0u8; w * h * 4];
        unsafe {
            self.gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut buf)),
            );
        }
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            out[y * w * 4..(y + 1) * w * 4].copy_from_slice(&buf[(h - 1 - y) * w * 4..(h - y) * w * 4]);
        }
        out
    }

    // ---- clipping ----

    pub fn push_clip(&mut self, r: Rect) {
        let r = match self.clips.last() {
            Some(c) => c.intersect(&r),
            None => r,
        };
        self.clips.push(r);
        self.batches.push(Batch { clip: Some(r), start: self.verts.len(), erase: false });
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
        self.batches.push(Batch { clip: self.clips.last().copied(), start: self.verts.len(), erase: false });
    }

    pub fn clip(&self) -> Option<Rect> {
        self.clips.last().copied()
    }

    /// Round the window's corners: clear the area outside a radius-`r` arc in each corner.
    pub fn erase_corners(&mut self, r: f32) {
        if r < 0.5 {
            return;
        }
        let clip = self.clips.last().copied();
        self.batches.push(Batch { clip: None, start: self.verts.len(), erase: true });
        let (w, h) = (self.w, self.h);
        let n = 12;
        use std::f32::consts::{FRAC_PI_2, PI};
        for (corner, center, a0) in [
            ([0.0, 0.0], [r, r], PI),
            ([w, 0.0], [w - r, r], -FRAC_PI_2),
            ([w, h], [w - r, h - r], 0.0),
            ([0.0, h], [r, h - r], FRAC_PI_2),
        ] {
            let mut prev = [center[0] + r * a0.cos(), center[1] + r * a0.sin()];
            for i in 1..=n {
                let a = a0 + FRAC_PI_2 * i as f32 / n as f32;
                let p = [center[0] + r * a.cos(), center[1] + r * a.sin()];
                self.tri(corner, prev, p, [0.0, 0.0, 0.0, 1.0]);
                prev = p;
            }
        }
        self.batches.push(Batch { clip, start: self.verts.len(), erase: false });
    }

    // ---- primitives ----

    fn tri(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], color: Color) {
        for p in [a, b, c] {
            self.verts.push(Vertex { pos: p, uv: WHITE_UV, color });
        }
    }

    fn quad_uv(&mut self, r: Rect, uv: [f32; 4], color: Color) {
        let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
        let v = |x, y, u, w| Vertex { pos: [x, y], uv: [u, w], color };
        self.verts.extend_from_slice(&[v(x0, y0, u0, v0), v(x1, y0, u1, v0), v(x1, y1, u1, v1), v(x0, y0, u0, v0), v(x1, y1, u1, v1), v(x0, y1, u0, v1)]);
    }

    pub fn rect(&mut self, r: Rect, color: Color) {
        if r.w <= 0.0 || r.h <= 0.0 || color[3] <= 0.0 {
            return;
        }
        self.quad_uv(r, [WHITE_UV[0], WHITE_UV[1], WHITE_UV[0], WHITE_UV[1]], color);
    }

    /// A 1-pixel (or `w`) outline inside `r`, pixel-aligned.
    pub fn rect_stroke(&mut self, r: Rect, w: f32, color: Color) {
        self.rect(Rect::new(r.x, r.y, r.w, w), color);
        self.rect(Rect::new(r.x, r.bottom() - w, r.w, w), color);
        self.rect(Rect::new(r.x, r.y + w, w, r.h - 2.0 * w), color);
        self.rect(Rect::new(r.right() - w, r.y + w, w, r.h - 2.0 * w), color);
    }

    fn rrect_points(r: Rect, rad: f32) -> Vec<[f32; 2]> {
        let rad = rad.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        let n = ((rad / 1.5) as usize).clamp(2, 10);
        let mut pts = Vec::with_capacity(4 * (n + 1));
        let corners = [
            (r.right() - rad, r.y + rad, -std::f32::consts::FRAC_PI_2),
            (r.right() - rad, r.bottom() - rad, 0.0),
            (r.x + rad, r.bottom() - rad, std::f32::consts::FRAC_PI_2),
            (r.x + rad, r.y + rad, std::f32::consts::PI),
        ];
        for (cx, cy, a0) in corners {
            for i in 0..=n {
                let a = a0 + std::f32::consts::FRAC_PI_2 * i as f32 / n as f32;
                pts.push([cx + rad * a.cos(), cy + rad * a.sin()]);
            }
        }
        pts
    }

    pub fn rrect(&mut self, r: Rect, rad: f32, color: Color) {
        if rad < 0.5 {
            return self.rect(r, color);
        }
        if r.w <= 0.0 || r.h <= 0.0 || color[3] <= 0.0 {
            return;
        }
        let pts = Self::rrect_points(r, rad);
        let c = [r.cx(), r.cy()];
        for i in 0..pts.len() {
            self.tri(c, pts[i], pts[(i + 1) % pts.len()], color);
        }
    }

    pub fn rrect_stroke(&mut self, r: Rect, rad: f32, w: f32, color: Color) {
        if rad < 0.5 {
            return self.rect_stroke(r, w, color);
        }
        let h = w / 2.0;
        let pts = Self::rrect_points(r.inset(h), (rad - h).max(0.0));
        self.polyline(&pts, w, color, true);
    }

    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], w: f32, color: Color) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-3 {
            return;
        }
        let (nx, ny) = (-dy / len * w / 2.0, dx / len * w / 2.0);
        let p0 = [a[0] + nx, a[1] + ny];
        let p1 = [b[0] + nx, b[1] + ny];
        let p2 = [b[0] - nx, b[1] - ny];
        let p3 = [a[0] - nx, a[1] - ny];
        self.tri(p0, p1, p2, color);
        self.tri(p0, p2, p3, color);
    }

    /// A thick polyline with mitred joins (bevelled when sharp).
    pub fn polyline(&mut self, pts: &[[f32; 2]], w: f32, color: Color, closed: bool) {
        let n = pts.len();
        if n < 2 {
            return;
        }
        let hw = w / 2.0;
        let seg = |i: usize| -> [f32; 2] {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-6);
            [-dy / l, dx / l]
        };
        let nseg = if closed { n } else { n - 1 };
        // Offset of each vertex along its miter.
        let mut off = Vec::with_capacity(n);
        for i in 0..n {
            let has_prev = closed || i > 0;
            let has_next = closed || i < n - 1;
            let np = if has_prev { Some(seg((i + n - 1) % n)) } else { None };
            let nn = if has_next { Some(seg(i)) } else { None };
            let m = match (np, nn) {
                (Some(a), Some(b)) => {
                    let mx = a[0] + b[0];
                    let my = a[1] + b[1];
                    let ml = (mx * mx + my * my).sqrt();
                    if ml < 1e-3 {
                        [b[0] * hw, b[1] * hw]
                    } else {
                        let (mx, my) = (mx / ml, my / ml);
                        let d = (mx * b[0] + my * b[1]).max(0.35);
                        [mx * hw / d, my * hw / d]
                    }
                }
                (Some(a), None) => [a[0] * hw, a[1] * hw],
                (None, Some(b)) => [b[0] * hw, b[1] * hw],
                (None, None) => [0.0, 0.0],
            };
            off.push(m);
        }
        for i in 0..nseg {
            let j = (i + 1) % n;
            let (a, b) = (pts[i], pts[j]);
            let (oa, ob) = (off[i], off[j]);
            let p0 = [a[0] + oa[0], a[1] + oa[1]];
            let p1 = [b[0] + ob[0], b[1] + ob[1]];
            let p2 = [b[0] - ob[0], b[1] - ob[1]];
            let p3 = [a[0] - oa[0], a[1] - oa[1]];
            self.tri(p0, p1, p2, color);
            self.tri(p0, p2, p3, color);
        }
    }

    /// A cubic Bézier, as a polyline fine enough for its on-screen length.
    pub fn bezier(&mut self, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], w: f32, color: Color) {
        let approx = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
        let n = ((approx / 6.0) as usize).clamp(8, 96);
        let mut pts = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            pts.push([
                a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
            ]);
        }
        self.polyline(&pts, w, color, false);
    }

    fn segments(r: f32) -> usize {
        ((r * 1.2) as usize).clamp(10, 64)
    }

    pub fn circle(&mut self, c: [f32; 2], r: f32, color: Color) {
        self.pie(c, r, 0.0, std::f32::consts::TAU, color);
    }

    /// A filled sector from angle `a0` to `a1` (radians, clockwise on screen from +x).
    pub fn pie(&mut self, c: [f32; 2], r: f32, a0: f32, a1: f32, color: Color) {
        if r <= 0.0 {
            return;
        }
        let n = ((Self::segments(r) as f32 * (a1 - a0).abs() / std::f32::consts::TAU).ceil() as usize).max(3);
        let mut prev = [c[0] + r * a0.cos(), c[1] + r * a0.sin()];
        for i in 1..=n {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            let p = [c[0] + r * a.cos(), c[1] + r * a.sin()];
            self.tri(c, prev, p, color);
            prev = p;
        }
    }

    pub fn arc(&mut self, c: [f32; 2], r: f32, a0: f32, a1: f32, w: f32, color: Color) {
        let full = (a1 - a0).abs() >= std::f32::consts::TAU - 1e-3;
        let n = ((Self::segments(r) as f32 * (a1 - a0).abs() / std::f32::consts::TAU).ceil() as usize).max(3);
        let count = if full { n } else { n + 1 };
        let pts: Vec<[f32; 2]> = (0..count)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                [c[0] + r * a.cos(), c[1] + r * a.sin()]
            })
            .collect();
        self.polyline(&pts, w, color, full);
    }

    // ---- text ----

    pub fn line_height(&self, face: Face, px: f32) -> f32 {
        self.fonts.metrics(face, px).1
    }

    pub fn measure(&self, face: Face, px: f32, s: &str) -> f32 {
        self.fonts.measure(face, px, s)
    }

    /// Draw `s` with its line box's top-left at (x, y). Returns the advance width.
    pub fn text(&mut self, face: Face, px: f32, x: f32, y: f32, color: Color, s: &str) -> f32 {
        self.text_dir(face, px, x, y, color, s, false)
    }

    /// Text reading bottom to top (rotated 90° counter-clockwise), its line box's bottom-left at
    /// (x, y): the first character sits at (x, y) and the text goes up.
    pub fn text_up(&mut self, face: Face, px: f32, x: f32, y: f32, color: Color, s: &str) -> f32 {
        self.text_dir(face, px, x, y, color, s, true)
    }

    fn text_dir(&mut self, face: Face, px: f32, x: f32, y: f32, color: Color, s: &str, up: bool) -> f32 {
        if px < 3.0 || color[3] <= 0.0 {
            return self.measure(face, px, s);
        }
        let (ascent, _) = self.fonts.metrics(face, px);
        let base = (y + ascent).round();
        let mut pen = 0.0f32;
        let mut prev = None;
        let inv = 1.0 / ATLAS as f32;
        for ch in s.chars() {
            let (g, kern, adv) = self.fonts.glyph(face, px, prev, ch);
            pen += kern;
            if let Some(g) = g {
                let gx = (pen + g.off[0]).round();
                let gy = g.off[1];
                let (tw, th) = (g.tex[2] as f32, g.tex[3] as f32);
                let uv = [g.tex[0] as f32 * inv, g.tex[1] as f32 * inv, (g.tex[0] as f32 + tw) * inv, (g.tex[1] as f32 + th) * inv];
                if !up {
                    let r = Rect::new(x.round() + gx, base + gy, tw, th);
                    self.quad_uv(r, uv, color);
                } else {
                    // Rotate (gx, gy) relative to the pen origin by -90°: x' = gy, y' = -gx.
                    // The baseline runs upward along x = x + ascent.
                    let ox = (x + ascent).round();
                    let oy = y.round();
                    let p = |dx: f32, dy: f32| [ox + dy, oy - dx];
                    let (a, b, c, d) = (p(gx, gy), p(gx + tw, gy), p(gx + tw, gy + th), p(gx, gy + th));
                    let v = |pos: [f32; 2], u: f32, w: f32| Vertex { pos, uv: [u, w], color };
                    self.verts.extend_from_slice(&[
                        v(a, uv[0], uv[1]),
                        v(b, uv[2], uv[1]),
                        v(c, uv[2], uv[3]),
                        v(a, uv[0], uv[1]),
                        v(c, uv[2], uv[3]),
                        v(d, uv[0], uv[3]),
                    ]);
                }
            }
            pen += adv;
            prev = Some(ch);
        }
        pen
    }

    /// Text clipped to `max_w`, with "…" when cut.
    pub fn text_fit(&mut self, face: Face, px: f32, x: f32, y: f32, max_w: f32, color: Color, s: &str) -> f32 {
        let w = self.measure(face, px, s);
        if w <= max_w {
            return self.text(face, px, x, y, color, s);
        }
        let ell = "…";
        let ew = self.measure(face, px, ell);
        let mut cut = String::new();
        for ch in s.chars() {
            cut.push(ch);
            if self.measure(face, px, &cut) + ew > max_w {
                cut.pop();
                break;
            }
        }
        cut.push_str(ell);
        self.text(face, px, x, y, color, &cut)
    }

    /// Text centred in `r` (both axes), cut with "…" to fit.
    pub fn text_center(&mut self, face: Face, px: f32, r: Rect, color: Color, s: &str) {
        let w = self.measure(face, px, s).min(r.w);
        let lh = self.line_height(face, px);
        self.text_fit(face, px, r.cx() - w / 2.0, r.cy() - lh / 2.0, r.w, color, s);
    }
}

pub fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

unsafe fn link_program(gl: &glow::Context, vs: &str, fs: &str) -> glow::Program {
    unsafe {
        let program = gl.create_program().expect("gl program");
        for (ty, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let sh = gl.create_shader(ty).expect("gl shader");
            gl.shader_source(sh, src);
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                panic!("shader compile error: {}", gl.get_shader_info_log(sh));
            }
            gl.attach_shader(program, sh);
            gl.delete_shader(sh);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            panic!("program link error: {}", gl.get_program_info_log(program));
        }
        program
    }
}

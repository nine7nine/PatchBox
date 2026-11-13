//! Fonts and the glyph atlas.
//!
//! Glyphs are rasterised on demand (ab_glyph) at the exact pixel size asked for, quantised to half
//! pixels, into one R8 atlas the renderer samples. Positions snap to whole pixels, so text stays
//! sharp at any interface scale or canvas zoom. The atlas keeps a white block for solid shapes, so
//! everything draws with one texture and one shader.
//!
//! Font: Cousine (Apache-2.0, assets/fonts/LICENSE), the face PatchMatrix uses.

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};
use std::collections::HashMap;

pub const ATLAS: usize = 2048;
/// The white block for solid fills sits at the atlas origin.
pub const WHITE_UV: [f32; 2] = [1.5 / ATLAS as f32, 1.5 / ATLAS as f32];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    Regular,
    Bold,
}

#[derive(Clone, Copy)]
pub struct Glyph {
    /// Atlas rectangle, in texels (x, y, w, h).
    pub tex: [u16; 4],
    /// Offset of the bitmap's top-left from the pen position on the baseline, in pixels.
    pub off: [f32; 2],
    pub advance: f32,
}

pub struct Fonts {
    regular: FontRef<'static>,
    bold: FontRef<'static>,
    glyphs: HashMap<(Face, GlyphId, u16), Option<Glyph>>,
    pub pixels: Vec<u8>,
    /// Rows changed since the last upload (y0..y1), if any.
    pub dirty: Option<(usize, usize)>,
    shelf_x: usize,
    shelf_y: usize,
    shelf_h: usize,
    /// The atlas filled up and was cleared: what was drawn this frame may reference stale texels,
    /// so the caller draws the frame again.
    pub overflowed: bool,
}

const PAD: usize = 1;

impl Fonts {
    pub fn new() -> Fonts {
        let regular = FontRef::try_from_slice(include_bytes!("../assets/fonts/Cousine-Regular.ttf")).expect("embedded font");
        let bold = FontRef::try_from_slice(include_bytes!("../assets/fonts/Cousine-Bold.ttf")).expect("embedded font");
        let mut f = Fonts {
            regular,
            bold,
            glyphs: HashMap::new(),
            pixels: vec![0; ATLAS * ATLAS],
            dirty: None,
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            overflowed: false,
        };
        f.reset();
        f
    }

    fn reset(&mut self) {
        self.pixels.fill(0);
        for y in 0..4 {
            for x in 0..4 {
                self.pixels[y * ATLAS + x] = 255;
            }
        }
        self.glyphs.clear();
        self.shelf_x = 4 + PAD;
        self.shelf_y = 0;
        self.shelf_h = 4;
        self.dirty = Some((0, ATLAS));
    }

    fn font(&self, face: Face) -> &FontRef<'static> {
        match face {
            Face::Regular => &self.regular,
            Face::Bold => &self.bold,
        }
    }

    fn q(px: f32) -> u16 {
        (px * 2.0).round().clamp(2.0, 400.0) as u16
    }

    /// Line metrics at `px`: (ascent, height) in pixels.
    pub fn metrics(&self, face: Face, px: f32) -> (f32, f32) {
        let s = self.font(face).as_scaled(PxScale::from(Self::q(px) as f32 / 2.0));
        (s.ascent(), s.ascent() - s.descent())
    }

    /// Width of `text` at `px`, in pixels (as laid out by `layout`).
    pub fn measure(&self, face: Face, px: f32, text: &str) -> f32 {
        let s = self.font(face).as_scaled(PxScale::from(Self::q(px) as f32 / 2.0));
        let mut w = 0.0;
        let mut prev: Option<GlyphId> = None;
        for ch in text.chars() {
            let id = s.glyph_id(ch);
            if let Some(p) = prev {
                w += s.kern(p, id);
            }
            w += s.h_advance(id);
            prev = Some(id);
        }
        w
    }

    /// The glyph for `ch` (rasterised now if needed) and the kerning from `prev`.
    pub fn glyph(&mut self, face: Face, px: f32, prev: Option<char>, ch: char) -> (Option<Glyph>, f32, f32) {
        let q = Self::q(px);
        let scale = PxScale::from(q as f32 / 2.0);
        let (id, kern, advance) = {
            let s = self.font(face).as_scaled(scale);
            let id = s.glyph_id(ch);
            let kern = prev.map(|p| s.kern(s.glyph_id(p), id)).unwrap_or(0.0);
            (id, kern, s.h_advance(id))
        };
        let key = (face, id, q);
        if let Some(g) = self.glyphs.get(&key) {
            return (*g, kern, advance);
        }
        let g = self.rasterise(face, id, scale, advance);
        self.glyphs.insert(key, g);
        (g, kern, advance)
    }

    fn rasterise(&mut self, face: Face, id: GlyphId, scale: PxScale, advance: f32) -> Option<Glyph> {
        let outlined = self.font(face).outline_glyph(id.with_scale_and_position(scale, point(0.0, 0.0)))?;
        let b = outlined.px_bounds();
        let (w, h) = (b.width().ceil() as usize + 1, b.height().ceil() as usize + 1);
        if w + PAD >= ATLAS || h + PAD >= ATLAS {
            return None;
        }
        if self.shelf_x + w + PAD > ATLAS {
            self.shelf_y += self.shelf_h + PAD;
            self.shelf_x = 0;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + PAD > ATLAS {
            // Full: start over. Glyphs already emitted this frame point at cleared texels.
            self.reset();
            self.overflowed = true;
            return self.rasterise(face, id, scale, advance);
        }
        let (x0, y0) = (self.shelf_x, self.shelf_y);
        let pixels = &mut self.pixels;
        outlined.draw(|x, y, c| {
            let (x, y) = (x0 + x as usize, y0 + y as usize);
            if x < ATLAS && y < ATLAS {
                pixels[y * ATLAS + x] = (c.clamp(0.0, 1.0) * 255.0) as u8;
            }
        });
        self.shelf_x += w + PAD;
        self.shelf_h = self.shelf_h.max(h);
        self.dirty = Some(match self.dirty {
            Some((a, z)) => (a.min(y0), z.max(y0 + h)),
            None => (y0, y0 + h),
        });
        Some(Glyph { tex: [x0 as u16, y0 as u16, w as u16, h as u16], off: [b.min.x, b.min.y], advance })
    }
}

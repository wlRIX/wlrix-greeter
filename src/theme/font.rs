// SPDX-License-Identifier: GPL-3.0-or-later
//! Text, rasterized with fontdue.
//!
//! IRIX's `FontPalette` asks for Helvetica, which is proprietary. The nearest free match
//! is Nimbus Sans, so the greeter looks for that first, then Liberation Sans, then DejaVu
//! Sans -- resolving each family as a whole so a regular is never paired with another
//! family's bold. If none is found it says which paths it tried and gives up, because a
//! login screen that draws no text is worse than one that explains why.
//!
//! There is no shaping and no kerning: fontdue does neither, and the Latin labels and
//! usernames here do not need them. A name in a complex script would render as unshaped
//! glyphs, which is a known limitation, not a surprise.

use std::collections::HashMap;

use fontdue::{Font, FontSettings, Metrics};

use crate::theme::Rgb;
use crate::ui::canvas::Canvas;

/// The three faces the login screen uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    /// Field contents, user names, buttons.
    Regular,
    /// The `Login name:` label.
    Bold,
    /// The full name beside the field, and the `IRIX` wordmark.
    Italic,
}

/// One font family's three faces, resolved together.
struct Family {
    name: &'static str,
    regular: &'static str,
    bold: &'static str,
    italic: &'static str,
}

/// Where fonts live, and the families to try in order of preference.
const FONT_DIRS: &[&str] = &[
    "/usr/share/fonts/gsfonts",
    "/usr/share/fonts/liberation",
    "/usr/share/fonts/TTF",
    "/usr/share/fonts/truetype/liberation",
    "/usr/share/fonts/truetype/dejavu",
];

const FAMILIES: &[Family] = &[
    Family {
        name: "Nimbus Sans",
        regular: "NimbusSans-Regular.otf",
        bold: "NimbusSans-Bold.otf",
        italic: "NimbusSans-Italic.otf",
    },
    Family {
        name: "Liberation Sans",
        regular: "LiberationSans-Regular.ttf",
        bold: "LiberationSans-Bold.ttf",
        italic: "LiberationSans-Italic.ttf",
    },
    Family {
        name: "DejaVu Sans",
        regular: "DejaVuSans.ttf",
        bold: "DejaVuSans-Bold.ttf",
        italic: "DejaVuSans-Oblique.ttf",
    },
];

/// A rasterized glyph, cached.
struct Glyph {
    metrics: Metrics,
    coverage: Vec<u8>,
}

/// A text run's fixed parameters: which face and size, where, and in what color.
///
/// Grouped so [`Fonts::draw`] takes one description of the run rather than a long list
/// of positional arguments.
#[derive(Clone, Copy)]
pub struct Run {
    pub face: Face,
    pub px: f32,
    /// The run's left edge.
    pub x: i32,
    /// The baseline the glyphs sit on.
    pub baseline: i32,
    pub colour: Rgb,
}

/// The greeter's fonts, with a glyph cache.
pub struct Fonts {
    regular: Font,
    bold: Font,
    italic: Font,
    /// Keyed by face, character and pixel size (as its bit pattern, so it can be
    /// hashed). Glyphs are rasterized once and reused.
    cache: HashMap<(Face, char, u32), Glyph>,
    /// Which family was chosen, for a one-line note at startup.
    family: &'static str,
}

impl Fonts {
    /// Load the first available family. Errs, listing what it tried, if none is found.
    pub fn load() -> Result<Self, String> {
        for family in FAMILIES {
            let Some(regular) = load_face(family.regular) else {
                continue;
            };
            // A family missing its bold or italic is not used piecemeal; fall through
            // to the next rather than mixing families mid-word.
            let (Some(bold), Some(italic)) = (load_face(family.bold), load_face(family.italic))
            else {
                continue;
            };
            return Ok(Self {
                regular,
                bold,
                italic,
                cache: HashMap::new(),
                family: family.name,
            });
        }

        Err(format!(
            "no usable font family found; looked for {} in {}",
            FAMILIES
                .iter()
                .map(|f| f.name)
                .collect::<Vec<_>>()
                .join(", "),
            FONT_DIRS.join(", ")
        ))
    }

    /// The family in use, for logging.
    pub fn family(&self) -> &str {
        self.family
    }

    fn font(&self, face: Face) -> &Font {
        match face {
            Face::Regular => &self.regular,
            Face::Bold => &self.bold,
            Face::Italic => &self.italic,
        }
    }

    /// The distance from the top of a line to the baseline, so callers can turn a box
    /// top into a baseline for [`draw`].
    pub fn ascent(&self, face: Face, px: f32) -> i32 {
        self.font(face)
            .horizontal_line_metrics(px)
            .map(|m| m.ascent.ceil() as i32)
            .unwrap_or_else(|| px.ceil() as i32)
    }

    /// A line's natural height, for spacing rows.
    pub fn line_height(&self, face: Face, px: f32) -> i32 {
        self.font(face)
            .horizontal_line_metrics(px)
            .map(|m| m.new_line_size.ceil() as i32)
            .unwrap_or_else(|| (px * 1.2).ceil() as i32)
    }

    /// How wide `text` is at this face and size, in whole pixels.
    pub fn width(&mut self, face: Face, px: f32, text: &str) -> i32 {
        let mut pen = 0.0f32;
        for ch in text.chars() {
            pen += self.glyph(face, ch, px).metrics.advance_width;
        }
        pen.ceil() as i32
    }

    /// Draw `text` with the given face, size, position and color.
    pub fn draw(&mut self, canvas: &mut Canvas, run: Run, text: &str) {
        let Run {
            face,
            px,
            x,
            baseline,
            colour,
        } = run;
        let mut pen = x as f32;
        for ch in text.chars() {
            // Split the borrow: rasterize (which caches, so needs &mut self) before the
            // canvas write, then read the cached glyph back immutably.
            let advance = self.glyph(face, ch, px).metrics.advance_width;
            let glyph = self.glyph(face, ch, px);
            let m = glyph.metrics;

            let gx = pen.round() as i32 + m.xmin;
            // ymin is measured up from the baseline, so the bitmap's top is the baseline
            // minus the glyph's height and its below-baseline part.
            let gy = baseline - m.height as i32 - m.ymin;

            for row in 0..m.height {
                for col in 0..m.width {
                    let coverage = glyph.coverage[row * m.width + col];
                    canvas.blend(gx + col as i32, gy + row as i32, colour, coverage);
                }
            }
            pen += advance;
        }
    }

    /// Rasterize a glyph, caching it.
    fn glyph(&mut self, face: Face, ch: char, px: f32) -> &Glyph {
        let key = (face, ch, px.to_bits());
        if !self.cache.contains_key(&key) {
            let (metrics, coverage) = self.font(face).rasterize(ch, px);
            self.cache.insert(key, Glyph { metrics, coverage });
        }
        &self.cache[&key]
    }
}

/// Read one face file from the first directory that has it.
fn load_face(file: &str) -> Option<Font> {
    for dir in FONT_DIRS {
        let path = std::path::Path::new(dir).join(file);
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(font) = Font::from_bytes(bytes, FontSettings::default())
        {
            return Some(font);
        }
    }
    None
}

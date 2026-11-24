use crate::model::Point;
use ab_glyph::{Font, FontRef, FontVec, OutlinedGlyph, ScaleFont, point};
use std::{fs, path::Path, sync::OnceLock};

/// Font selection for labels, with a built-in bitmap fallback.
#[derive(Debug, Default)]
pub struct TextFont<'a>(Option<FontSource<'a>>);

#[derive(Debug)]
enum FontSource<'a> {
    Borrowed(Box<FontRef<'a>>),
    System(&'static FontVec),
}

impl<'a> TextFont<'a> {
    /// Uses the supplied font data, or the bitmap font if the data is invalid.
    pub fn from_bytes(bytes: &'a [u8]) -> Self {
        Self(
            FontRef::try_from_slice(bytes)
                .ok()
                .map(|font| FontSource::Borrowed(Box::new(font))),
        )
    }

    /// Returns whether labels will be rasterized from a scalable font.
    pub fn is_truetype(&self) -> bool {
        self.0.is_some()
    }

    pub(crate) fn layout(
        &self,
        origin: Point,
        text: &str,
        size: f32,
    ) -> Option<(f32, f32, Vec<OutlinedGlyph>)> {
        if !origin.is_finite() || !size.is_finite() || size <= 0.0 {
            return None;
        }
        let size = size.min(1024.0);
        Some(match self.0.as_ref()? {
            FontSource::Borrowed(font) => layout_line(font.as_ref(), origin, text, size),
            FontSource::System(font) => layout_line(*font, origin, text, size),
        })
    }
}

impl TextFont<'static> {
    /// Uses the first available system sans-serif font, cached for the process.
    pub fn system() -> Self {
        static FONT: OnceLock<Option<FontVec>> = OnceLock::new();
        Self(
            FONT.get_or_init(load_system_font)
                .as_ref()
                .map(FontSource::System),
        )
    }
}

fn load_system_font() -> Option<FontVec> {
    let paths = if cfg!(target_os = "windows") {
        let root = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
        ["segoeui.ttf", "arial.ttf"]
            .map(|name| Path::new(&root).join("Fonts").join(name))
            .to_vec()
    } else if cfg!(target_os = "macos") {
        [
            "/System/Library/Fonts/Helvetica.ttc",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
        .map(Into::into)
        .to_vec()
    } else {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ]
        .map(Into::into)
        .to_vec()
    };
    paths.into_iter().find_map(|path| {
        let bytes = fs::read(path).ok()?;
        FontVec::try_from_vec(bytes).ok()
    })
}

fn layout_line(
    font: &impl Font,
    origin: Point,
    text: &str,
    size: f32,
) -> (f32, f32, Vec<OutlinedGlyph>) {
    let scaled = font.as_scaled(size);
    let baseline = origin.y + scaled.ascent();
    let mut cursor = 0.0;
    let mut previous = None;
    let glyphs = text
        .chars()
        .filter_map(|character| {
            let id = scaled.glyph_id(character);
            if let Some(previous) = previous {
                cursor += scaled.kern(previous, id);
            }
            let glyph = id.with_scale_and_position(size, point(origin.x + cursor, baseline));
            cursor += scaled.h_advance(id);
            previous = Some(id);
            scaled.outline_glyph(glyph)
        })
        .collect();
    (cursor, scaled.height(), glyphs)
}

/// Returns a 5x7 monochrome glyph bitmap (each row uses the lowest 5 bits, MSB on the left).
pub(crate) fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        ' ' => [0; 7],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ',' => [0, 0, 0, 0, 0b00100, 0b00100, 0b01000],
        ':' => [0, 0b00100, 0b00100, 0, 0b00100, 0b00100, 0],
        ';' => [0, 0b00100, 0b00100, 0, 0b00100, 0b00100, 0b01000],
        '!' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00110, 0b00100, 0b00000, 0b00100,
        ],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '=' => [0, 0, 0b11111, 0, 0b11111, 0, 0],
        '/' => [
            0b00001, 0b00010, 0b00100, 0b00100, 0b01000, 0b10000, 0b00000,
        ],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '[' => [
            0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110,
        ],
        ']' => [
            0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110,
        ],
        _ => [
            0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111,
        ],
    }
}

//! The characters the screens may use, and what replaces the others.
//!
//! The Linux console shows only the glyphs of its font: 256 at most, to
//! keep 16 colours. The `crustates-console` font is Lat2-Terminus16 (SIL
//! Open Font License) with eight rarely used glyphs replaced by the eight
//! the screens need ([`ADDED`]). [`Glyphs::apply`] swaps any other
//! character for one the active font has.

use std::collections::HashSet;

use ratatui::buffer::Buffer;

use crate::psf::Psf;

/// Glyphs the screens need that Lat2-Terminus16 lacks, with their 8×16
/// bitmaps (rows top first, leftmost pixel in the highest bit).
pub const ADDED: [(char, [u8; 16]); 8] = [
    (
        '▀',
        [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    ),
    (
        '▄',
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ],
    ),
    ('▌', [0xF0; 16]),
    ('▐', [0x0F; 16]),
    // The same 4×4 circle as the font's ●.
    (
        '○',
        [0, 0, 0, 0, 0, 0, 0x18, 0x24, 0x24, 0x18, 0, 0, 0, 0, 0, 0],
    ),
    (
        '◐',
        [0, 0, 0, 0, 0, 0, 0x18, 0x34, 0x34, 0x18, 0, 0, 0, 0, 0, 0],
    ),
    (
        '✓',
        [
            0, 0, 0, 0, 0, 0x01, 0x03, 0x06, 0x8C, 0xD8, 0x70, 0x20, 0, 0, 0, 0,
        ],
    ),
    (
        '✗',
        [
            0, 0, 0, 0, 0x82, 0xC6, 0x6C, 0x38, 0x6C, 0xC6, 0x82, 0, 0, 0, 0, 0,
        ],
    ),
];

/// The Lat2-Terminus16 glyphs [`ADDED`] replaces, one each: double box
/// lines, the cedilla T that Romanian replaced with Ț, and Slovak Ŕ and Ĺ.
pub const REPLACED: [char; 8] = ['╬', '╪', 'Ţ', 'ţ', 'Ŕ', 'ŕ', 'Ĺ', 'ĺ'];

/// The characters of the `crustates-console` font, one `U+XXXX` per line;
/// `cargo xtask console-font` checks it against the font it makes.
pub const CONSOLE_FONT: &str = include_str!("console-font.txt");

/// Makes the `crustates-console` font from Lat2-Terminus16.
///
/// # Errors
///
/// Fails if `lat2` is not an 8×16 font or lacks a glyph to replace.
pub fn console_font(lat2: &Psf) -> Result<Psf, String> {
    if (lat2.width, lat2.height) != (8, 16) {
        return Err(format!(
            "expected an 8×16 font, got {}×{}",
            lat2.width, lat2.height
        ));
    }
    let mut font = lat2.clone();
    for (old, (new, bitmap)) in REPLACED.iter().zip(ADDED) {
        let glyph = font
            .glyph_of(*old)
            .ok_or_else(|| format!("the font has no {old}"))?;
        font.glyphs[glyph] = bitmap.to_vec();
        font.chars[glyph] = vec![new];
    }
    Ok(font)
}

/// The characters a font shows.
#[derive(Clone, Debug)]
pub struct Glyphs {
    /// `None`: every character, as in a terminal emulator.
    chars: Option<HashSet<char>>,
}

impl Glyphs {
    /// Every character: for terminal emulators with full fonts.
    #[must_use]
    pub fn any() -> Self {
        Self { chars: None }
    }

    /// The `crustates-console` font.
    #[must_use]
    pub fn console() -> Self {
        let chars = CONSOLE_FONT
            .lines()
            .filter_map(|l| l.strip_prefix("U+"))
            .filter_map(|hex| u32::from_str_radix(hex, 16).ok())
            .filter_map(char::from_u32)
            .collect();
        Self { chars: Some(chars) }
    }

    /// Printable ASCII only: when the active font is not known.
    #[must_use]
    pub fn ascii() -> Self {
        Self {
            chars: Some((' '..='~').collect()),
        }
    }

    /// The characters of `font`.
    #[must_use]
    pub fn of(font: &Psf) -> Self {
        Self {
            chars: Some(font.coverage().collect()),
        }
    }

    /// Whether the font shows `c`.
    #[must_use]
    pub fn has(&self, c: char) -> bool {
        self.chars.as_ref().is_none_or(|chars| chars.contains(&c))
    }

    /// Replaces every character in `buf` the font cannot show.
    pub fn apply(&self, buf: &mut Buffer) {
        if self.chars.is_none() {
            return;
        }
        for cell in &mut buf.content {
            let Some(c) = cell.symbol().chars().next() else {
                continue;
            };
            if self.has(c) {
                continue;
            }
            let fallback = fallback(c).filter(|f| self.has(*f)).unwrap_or('?');
            cell.set_char(fallback);
        }
    }
}

/// A look-alike that more fonts have.
fn fallback(c: char) -> Option<char> {
    Some(match c {
        '▀' | '▄' | '▌' | '▐' | '█' => '#',
        '░' | '·' | '…' => '.',
        '●' | '•' => '*',
        '○' => 'o',
        '◐' => '~',
        '✗' => 'x',
        '─' | '━' => '-',
        '│' => '|',
        '✓' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '╬' | '╪' => {
            '+'
        }
        '→' => '>',
        '←' => '<',
        'Ţ' => 'Ț',
        'ţ' => 'ț',
        'Ŕ' => 'R',
        'ŕ' => 'r',
        'Ĺ' => 'L',
        'ĺ' => 'l',
        '\u{a0}' => ' ',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;

    #[test]
    fn the_console_font_has_what_the_screens_draw_with() {
        let glyphs = Glyphs::console();
        for c in "▀▄▌▐●○◐✓✗─│┌┐└┘█░·…→ abcXYZ09".chars() {
            assert!(glyphs.has(c), "{c}");
        }
        for c in REPLACED {
            assert!(!glyphs.has(c), "{c} was replaced");
        }
    }

    #[test]
    fn missing_characters_fall_back() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 1));
        buf.set_string(0, 0, "a✓┌○Ŕ€", ratatui::style::Style::new());
        Glyphs::ascii().apply(&mut buf);
        let text: String = buf.content.iter().map(|c| c.symbol().to_owned()).collect();
        assert_eq!(text, "a++oR?");
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf.set_string(0, 0, "ŕ✓", ratatui::style::Style::new());
        Glyphs::console().apply(&mut buf);
        assert_eq!(buf.content[0].symbol(), "r");
        assert_eq!(buf.content[1].symbol(), "✓");
    }
}

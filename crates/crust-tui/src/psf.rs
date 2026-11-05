//! PC Screen Font version 2, the Linux console's font format: a header, the
//! glyph bitmaps, and a table of the Unicode characters each glyph shows.

use thiserror::Error;

const MAGIC: u32 = 0x864A_B572;
const HEADER_SIZE: u32 = 32;
const HAS_UNICODE_TABLE: u32 = 1;
/// Ends a glyph's entry in the Unicode table.
const END: u8 = 0xFF;
/// Starts a sequence of combining characters; these are not kept.
const SEQUENCE: u8 = 0xFE;

/// Why a file is not a font this module reads.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PsfError {
    /// Not PSF2.
    #[error("not a PSF2 font")]
    NotPsf2,
    /// The file ends early.
    #[error("the font is cut short")]
    Truncated,
    /// No Unicode table.
    #[error("the font has no Unicode table")]
    NoUnicode,
}

/// A console font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Psf {
    /// Glyph width in pixels.
    pub width: u32,
    /// Glyph height in pixels.
    pub height: u32,
    /// Each glyph's rows, top first, each row padded to whole bytes with
    /// the leftmost pixel in the highest bit.
    pub glyphs: Vec<Vec<u8>>,
    /// The characters each glyph shows.
    pub chars: Vec<Vec<char>>,
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, PsfError> {
    let b: [u8; 4] = bytes
        .get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .ok_or(PsfError::Truncated)?;
    Ok(u32::from_le_bytes(b))
}

fn usize_of(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

impl Psf {
    /// Reads a PSF2 font with a Unicode table.
    ///
    /// # Errors
    ///
    /// Fails if `bytes` is not such a font.
    pub fn parse(bytes: &[u8]) -> Result<Self, PsfError> {
        if u32_at(bytes, 0)? != MAGIC {
            return Err(PsfError::NotPsf2);
        }
        let header = usize_of(u32_at(bytes, 8)?);
        if u32_at(bytes, 12)? & HAS_UNICODE_TABLE == 0 {
            return Err(PsfError::NoUnicode);
        }
        let count = usize_of(u32_at(bytes, 16)?);
        let size = usize_of(u32_at(bytes, 20)?);
        let (height, width) = (u32_at(bytes, 24)?, u32_at(bytes, 28)?);
        let glyphs = (0..count)
            .map(|g| {
                let at = header + g * size;
                bytes.get(at..at + size).map(<[u8]>::to_vec)
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(PsfError::Truncated)?;
        let mut table = bytes
            .get(header + count * size..)
            .ok_or(PsfError::Truncated)?;
        let mut chars = Vec::with_capacity(count);
        for _ in 0..count {
            let end = table
                .iter()
                .position(|b| *b == END)
                .ok_or(PsfError::Truncated)?;
            let entry = &table[..end];
            // Single characters come first; sequences follow, each after 0xFE.
            let singles = entry
                .iter()
                .position(|b| *b == SEQUENCE)
                .map_or(entry, |s| &entry[..s]);
            chars.push(String::from_utf8_lossy(singles).chars().collect());
            table = &table[end + 1..];
        }
        Ok(Self {
            width,
            height,
            glyphs,
            chars,
        })
    }

    /// The font as PSF2, with its Unicode table.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let size = self.glyphs.first().map_or(0, Vec::len);
        let header = [
            MAGIC,
            0,
            HEADER_SIZE,
            HAS_UNICODE_TABLE,
            u32::try_from(self.glyphs.len()).unwrap_or(u32::MAX),
            u32::try_from(size).unwrap_or(u32::MAX),
            self.height,
            self.width,
        ];
        let mut out: Vec<u8> = header.iter().flat_map(|n| n.to_le_bytes()).collect();
        for glyph in &self.glyphs {
            out.extend_from_slice(glyph);
        }
        for chars in &self.chars {
            out.extend(chars.iter().collect::<String>().bytes());
            out.push(END);
        }
        out
    }

    /// Every character the font shows.
    pub fn coverage(&self) -> impl Iterator<Item = char> + '_ {
        self.chars.iter().flatten().copied()
    }

    /// The glyph that shows `c`.
    #[must_use]
    pub fn glyph_of(&self, c: char) -> Option<usize> {
        self.chars.iter().position(|chars| chars.contains(&c))
    }

    /// The same font with every pixel doubled: 8×16 becomes 16×32.
    #[must_use]
    pub fn doubled(&self) -> Self {
        let row_bytes = usize_of(self.width.div_ceil(8));
        let glyphs = self
            .glyphs
            .iter()
            .map(|glyph| {
                glyph
                    .chunks(row_bytes)
                    .flat_map(|row| {
                        let bits = row.iter().fold(0u128, |acc, b| (acc << 8) | u128::from(*b));
                        let wide = (0..self.width).fold(0u128, |acc, x| {
                            let on = (bits >> (row_bytes * 8 - 1 - usize_of(x))) & 1;
                            (acc << 2) | (on * 0b11)
                        });
                        let out_bytes = row_bytes * 2;
                        let padded = wide << (out_bytes * 8 - usize_of(self.width) * 2);
                        let bytes: Vec<u8> = (0..out_bytes)
                            .rev()
                            .map(|i| u8::try_from((padded >> (i * 8)) & 0xFF).unwrap_or(0))
                            .collect();
                        [bytes.clone(), bytes]
                    })
                    .flatten()
                    .collect()
            })
            .collect();
        Self {
            width: self.width * 2,
            height: self.height * 2,
            glyphs,
            chars: self.chars.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Psf {
        Psf {
            width: 8,
            height: 2,
            glyphs: vec![vec![0b1000_0001, 0b0100_0000], vec![0xFF, 0]],
            chars: vec![vec!['a', 'á'], vec!['▀']],
        }
    }

    #[test]
    fn fonts_survive_writing_and_reading() {
        let font = tiny();
        assert_eq!(Psf::parse(&font.to_bytes()).unwrap(), font);
        assert_eq!(font.glyph_of('▀'), Some(1));
        assert_eq!(font.coverage().collect::<String>(), "aá▀");
        assert_eq!(Psf::parse(b"no"), Err(PsfError::Truncated));
        assert_eq!(Psf::parse(&[0u8; 40]), Err(PsfError::NotPsf2));
    }

    #[test]
    fn sequences_are_skipped() {
        let mut bytes = tiny().to_bytes();
        // 'a' plus the sequence a + combining acute, before glyph 0's end.
        let table = usize_of(HEADER_SIZE) + 2 * 2;
        let end = table + bytes[table..].iter().position(|b| *b == END).unwrap();
        bytes.splice(end..end, [SEQUENCE, b'a', 0xCC, 0x81]);
        assert_eq!(Psf::parse(&bytes).unwrap().chars[0], ['a', 'á']);
    }

    #[test]
    fn doubling_doubles_every_pixel() {
        let big = tiny().doubled();
        assert_eq!((big.width, big.height), (16, 4));
        assert_eq!(
            big.glyphs[0],
            [
                0b1100_0000,
                0b0000_0011,
                0b1100_0000,
                0b0000_0011,
                0b0011_0000,
                0,
                0b0011_0000,
                0
            ]
        );
    }
}

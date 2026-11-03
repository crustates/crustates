//! `cargo xtask console-font <Lat2-Terminus16.psfu> <dir>` makes the
//! `crustates-console` font in 8×16 and 16×32 (see `crust_tui::glyphs`).
//! It checks the font's characters against
//! `crates/crust-tui/src/console-font.txt`, which the screen tests use;
//! `--update-list` rewrites that list instead.
//!
//! `cargo xtask splash <dir>` draws the boot splash's images: the crab and
//! the progress bar's parts.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, bail};
use crust_tui::glyphs::console_font;
use crust_tui::mark;
use crust_tui::palette::{PALETTE, Role};
use crust_tui::psf::Psf;

pub fn font(args: &[String]) -> anyhow::Result<()> {
    let update = args.iter().any(|a| a == "--update-list");
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let [src, dir] = paths[..] else {
        bail!("usage: cargo xtask console-font [--update-list] <Lat2-Terminus16.psfu> <dir>");
    };
    let bytes = std::fs::read(src).with_context(|| format!("reading {src}"))?;
    let lat2 = Psf::parse(&bytes).with_context(|| format!("reading {src}"))?;
    let font = console_font(&lat2).map_err(anyhow::Error::msg)?;

    let mut chars: Vec<char> = font.coverage().collect();
    chars.sort_unstable();
    chars.dedup();
    let list = chars.iter().fold(String::new(), |mut list, c| {
        let _ = writeln!(list, "U+{:04X}", u32::from(*c));
        list
    });
    let list_path = crate::root().join("crates/crust-tui/src/console-font.txt");
    if update {
        std::fs::write(&list_path, &list)
            .with_context(|| format!("writing {}", list_path.display()))?;
        println!("wrote {}", list_path.display());
    } else {
        let committed = std::fs::read_to_string(&list_path)
            .with_context(|| format!("reading {}", list_path.display()))?;
        if committed != list {
            bail!(
                "{src} makes a font whose characters differ from {}; check the screens and run with --update-list",
                list_path.display()
            );
        }
    }

    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for (font, name) in [
        (font.clone(), "crustates-console-16.psfu"),
        (font.doubled(), "crustates-console-32.psfu"),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, font.to_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {} ({} characters)", path.display(), chars.len());
    }
    Ok(())
}

/// The crab as pixel art, each of its pixels 4×4, and 1×1 images in the
/// splash's colours that the theme scales into the progress bar.
pub fn splash(args: &[String]) -> anyhow::Result<()> {
    const SCALE: usize = 4;
    let [dir] = args else {
        bail!("usage: cargo xtask splash <dir>");
    };
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let pixels = mark::pixels(36);
    let (width, height) = (pixels[0].len() * SCALE, pixels.len() * SCALE);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in &pixels {
        let line: Vec<u8> = row
            .iter()
            .flat_map(|p| {
                let color = p.map_or([0, 0, 0, 0], |role| {
                    let [_, r, g, b] = PALETTE[usize::from(role.index())].to_be_bytes();
                    [r, g, b, 255]
                });
                color.repeat(SCALE)
            })
            .collect();
        for _ in 0..SCALE {
            rgba.extend_from_slice(&line);
        }
    }
    let images = [
        ("mark.png", width, height, rgba),
        ("frame.png", 1, 1, rgba_of(Role::Text)),
        ("track.png", 1, 1, rgba_of(Role::Bar)),
        ("fill.png", 1, 1, rgba_of(Role::Shadow)),
    ];
    for (name, width, height, rgba) in images {
        let path = dir.join(name);
        std::fs::write(&path, png(width, height, &rgba, 4))
            .with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn rgba_of(role: Role) -> Vec<u8> {
    let [_, r, g, b] = PALETTE[usize::from(role.index())].to_be_bytes();
    vec![r, g, b, 255]
}

/// A PNG of `width`×`height` pixels with `channels` bytes each (3: RGB,
/// 4: RGBA), deflated without compression: the images are small, and this
/// needs no library.
pub fn png(width: usize, height: usize, pixels: &[u8], channels: usize) -> Vec<u8> {
    let stride = width * channels;
    // Each row starts with filter type 0, none.
    let raw: Vec<u8> = pixels
        .chunks(stride)
        .flat_map(|row| std::iter::once(0).chain(row.iter().copied()))
        .collect();
    let mut zlib = vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).collect::<Vec<_>>();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = u16::try_from(block.len()).unwrap_or(u16::MAX);
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend(((b << 16) | a).to_be_bytes());

    let size = |n: usize| u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes();
    let mut header = Vec::with_capacity(13);
    header.extend(size(width));
    header.extend(size(height));
    header.extend([8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(*b"IHDR", header), (*b"IDAT", zlib), (*b"IEND", Vec::new())] {
        out.extend(size(data.len()));
        let start = out.len();
        out.extend(kind);
        out.extend(&data);
        out.extend(crc32(&out[start..]).to_be_bytes());
    }
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |crc, byte| {
        (0..8).fold(crc ^ u32::from(*byte), |c, _| {
            if c & 1 == 1 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            }
        })
    })
}

/// A binary PPM (P6), as QEMU's `screendump` writes it, as a PNG.
pub fn ppm_to_png(ppm: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while ppm.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let start = at;
        while ppm.get(at).is_some_and(|b| !b.is_ascii_whitespace()) {
            at += 1;
        }
        if start == at {
            bail!("not a PPM image");
        }
        fields.push(String::from_utf8_lossy(&ppm[start..at]).into_owned());
    }
    let [magic, width, height, max] = &fields[..] else {
        bail!("not a PPM image")
    };
    if magic != "P6" || max != "255" {
        bail!("not an 8-bit binary PPM image");
    }
    let (width, height): (usize, usize) = (width.parse()?, height.parse()?);
    let pixels = ppm
        .get(at + 1..at + 1 + width * height * 3)
        .context("the PPM image is cut short")?;
    Ok(png(width, height, pixels, 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pngs_have_the_right_chunks_and_checksums() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        let image = png(2, 1, &[255, 0, 0, 0, 255, 0], 3);
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"));
        assert!(image.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
        let from_ppm = ppm_to_png(b"P6\n2 1\n255\n\xff\0\0\0\xff\0").unwrap();
        assert_eq!(from_ppm, image);
        assert!(ppm_to_png(b"P5\n1 1\n255\n\0").is_err());
    }
}

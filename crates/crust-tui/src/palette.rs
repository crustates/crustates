//! The ten colours of the design kit as Linux console palette entries.
//!
//! The Linux console has 16 colours that can be redefined (`ESC ] P`), but
//! only the first eight work as backgrounds, and bold text switches a
//! colour below 8 to the one 8 above it. So the colours used as backgrounds
//! sit in 0 to 7, and 10, 11 and 13 to 15 repeat 2, 3 and 5 to 7 for bold.

use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

/// What a colour is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The screen background.
    Bg,
    /// The rust of the crab and the QR code's shadow; a background too.
    Shadow,
    /// Healthy states, done steps, progress.
    Ok,
    /// The focused pane, the selection, warnings, codes.
    Focus,
    /// The top bar and the key-hint bar; text on the focus colour.
    Bar,
    /// URLs and addresses.
    Info,
    /// A second accent.
    Teal,
    /// Normal text, key caps, the QR code's light modules.
    Text,
    /// Secondary text, unfocused panes.
    Dim,
    /// Failures.
    Bad,
    /// Danger actions.
    Rust,
}

impl Role {
    /// The palette index.
    #[must_use]
    pub fn index(self) -> u8 {
        match self {
            Role::Bg => 0,
            Role::Shadow => 1,
            Role::Ok => 2,
            Role::Focus => 3,
            Role::Bar => 4,
            Role::Info => 5,
            Role::Teal => 6,
            Role::Text => 7,
            Role::Dim => 8,
            Role::Bad => 9,
            Role::Rust => 12,
        }
    }

    /// The colour, as a palette index.
    #[must_use]
    pub fn color(self) -> Color {
        Color::Indexed(self.index())
    }

    /// Text in this colour on the screen background.
    #[must_use]
    pub fn fg(self) -> Style {
        Style::new().fg(self.color()).bg(Role::Bg.color())
    }

    /// Bold text in this colour.
    #[must_use]
    pub fn bold(self) -> Style {
        self.fg().add_modifier(Modifier::BOLD)
    }
}

/// The 16 palette entries, `0xRRGGBB`.
pub const PALETTE: [u32; 16] = [
    0x14_1414, // 0 background
    0xE8_552D, // 1 rust shadow
    0x5B_E39A, // 2 ok
    0xFF_D23F, // 3 focus (Molt yellow)
    0x0B_0B0B, // 4 bar, ink
    0x8F_A2FF, // 5 info
    0x4F_D1C1, // 6 teal
    0xFF_F6E5, // 7 text (cream)
    0xB9_B2A5, // 8 dim
    0xFF_7A70, // 9 bad
    0x5B_E39A, // 10 bold ok
    0xFF_D23F, // 11 bold focus
    0xFF_8A65, // 12 rust text
    0x8F_A2FF, // 13 bold info
    0x4F_D1C1, // 14 bold teal
    0xFF_F6E5, // 15 bold text
];

/// The escape sequences that load [`PALETTE`] into the Linux console.
#[must_use]
pub fn vt_palette() -> String {
    PALETTE
        .iter()
        .enumerate()
        .fold(String::new(), |mut out, (index, rgb)| {
            let _ = write!(out, "\x1b]P{index:X}{rgb:06X}");
            out
        })
}

/// Turns palette indices in `buf` into true colour, for terminals whose
/// palette is not ours.
pub fn to_rgb(buf: &mut Buffer) {
    let rgb = |c: Color| match c {
        Color::Indexed(index) => {
            let value = PALETTE
                .get(usize::from(index))
                .copied()
                .unwrap_or(0xFF_F6E5);
            let [_, red, green, blue] = value.to_be_bytes();
            Color::Rgb(red, green, blue)
        }
        other => other,
    };
    for cell in &mut buf.content {
        cell.fg = rgb(cell.fg);
        cell.bg = rgb(cell.bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backgrounds_fit_the_first_eight_and_bold_keeps_its_colour() {
        for role in [Role::Bg, Role::Shadow, Role::Focus, Role::Bar, Role::Text] {
            assert!(role.index() < 8, "{role:?} is used as a background");
        }
        for i in [2, 3, 5, 6, 7] {
            assert_eq!(PALETTE[i], PALETTE[i + 8], "bold {i}");
        }
        assert!(vt_palette().starts_with("\x1b]P0141414\x1b]P1E8552D"));
        assert_eq!(vt_palette().len(), 16 * 10);
    }
}

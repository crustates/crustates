//! The design kit's pieces: the top bar, the key-hint bar, panes, status
//! and check lines, menus, buttons, fields, progress bars, QR codes and
//! modals.

use qrcode::{Color as Module, QrCode};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::palette::Role;

/// Fills `area` with the screen background.
pub fn background(buf: &mut Buffer, area: Rect) {
    buf.set_style(area, Role::Bg.fg());
    for pos in area.positions() {
        if let Some(cell) = buf.cell_mut(pos) {
            cell.set_symbol(" ");
        }
    }
}

/// The top bar: a bold title, then dim facts, and `right` at the right end.
pub fn top_bar(buf: &mut Buffer, area: Rect, title: &str, facts: &[&str], right: &str) {
    let bar = Style::new().fg(Role::Text.color()).bg(Role::Bar.color());
    buf.set_style(area, bar);
    let mut spans = vec![Span::styled(
        format!(" {title}"),
        bar.add_modifier(Modifier::BOLD),
    )];
    for fact in facts {
        spans.push(Span::styled(format!("  {fact}"), bar.fg(Role::Dim.color())));
    }
    let width = u16::try_from(right.chars().count() + 1).unwrap_or(area.width);
    let room = area.width.saturating_sub(width + 1);
    buf.set_line(area.x, area.y, &Line::from(spans), room);
    if width < area.width {
        buf.set_string(
            area.right() - width,
            area.y,
            right,
            bar.fg(Role::Dim.color()),
        );
    }
}

/// The key-hint bar: each key as a cap, then what it does.
pub fn key_bar(buf: &mut Buffer, area: Rect, keys: &[(&str, &str)]) {
    let bar = Style::new().fg(Role::Text.color()).bg(Role::Bar.color());
    buf.set_style(area, bar);
    let mut spans = vec![Span::styled(" ", bar)];
    for (key, what) in keys {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new().fg(Role::Bar.color()).bg(Role::Text.color()),
        ));
        spans.push(Span::styled(format!(" {what}  "), bar));
    }
    buf.set_line(area.x, area.y, &Line::from(spans), area.width);
}

/// A bordered pane with a title. `accent` colours the border and title of
/// a focused pane (or a failed one); `None` draws it dim.
#[must_use]
pub fn pane(title: &str, accent: Option<Role>) -> Block<'static> {
    let color = accent.unwrap_or(Role::Dim);
    let mut title_style = color.fg();
    if accent.is_some() {
        title_style = title_style.add_modifier(Modifier::BOLD);
    }
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(color.fg())
        .style(Role::Bg.fg())
        .title(Line::from(Span::styled(format!(" {title} "), title_style)))
}

/// How something is doing, for [`status`] lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Running or healthy: a green `●`.
    Ok,
    /// Needs a look: a yellow `●`.
    Warn,
    /// Starting or busy: a yellow `◐`.
    Busy,
    /// Failed: a red `✗`.
    Bad,
    /// Not started yet: a dim `○`.
    Waiting,
    /// No marker, for plain facts.
    None,
}

impl State {
    fn marker(self) -> Span<'static> {
        match self {
            State::Ok => Span::styled("●", Role::Ok.fg()),
            State::Warn => Span::styled("●", Role::Focus.fg()),
            State::Busy => Span::styled("◐", Role::Focus.fg()),
            State::Bad => Span::styled("✗", Role::Bad.fg()),
            State::Waiting => Span::styled("○", Role::Dim.fg()),
            State::None => Span::styled(" ", Role::Bg.fg()),
        }
    }
}

/// `● label        value  note`: a status line with the label padded to
/// `width`.
#[must_use]
pub fn status(state: State, label: &str, width: usize, value: &str, note: &str) -> Line<'static> {
    let label_style = if state == State::None {
        Role::Dim
    } else {
        Role::Text
    };
    let value_style = match state {
        State::Bad => Role::Bad,
        State::Waiting | State::None => Role::Dim,
        _ => Role::Text,
    };
    let mut spans = vec![
        state.marker(),
        Span::styled(format!(" {label:<width$} "), label_style.fg()),
        Span::styled(value.to_owned(), value_style.fg()),
    ];
    if !note.is_empty() {
        spans.push(Span::styled(format!("  {note}"), Role::Dim.fg()));
    }
    Line::from(spans)
}

/// The result of a check, for [`check`] lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// `✓`, green.
    Passed,
    /// `!`, yellow: it works, but not well.
    Slow,
    /// `✗`, red.
    Failed,
    /// `◐`, yellow: running now.
    Running,
    /// `○`, dim: not run yet.
    Pending,
}

/// `✓ label  value  note`: one check with the label padded to `width`.
#[must_use]
pub fn check(result: Check, label: &str, width: usize, value: &str, note: &str) -> Line<'static> {
    let (marker, color) = match result {
        Check::Passed => ("✓", Role::Ok),
        Check::Slow => ("!", Role::Focus),
        Check::Failed => ("✗", Role::Bad),
        Check::Running => ("◐", Role::Focus),
        Check::Pending => ("○", Role::Dim),
    };
    let value_color = match result {
        Check::Passed => Role::Text,
        Check::Pending | Check::Running => Role::Dim,
        _ => color,
    };
    let mut spans = vec![
        Span::styled(marker, color.fg()),
        Span::styled(format!(" {label:<width$} "), Role::Text.fg()),
        Span::styled(value.to_owned(), value_color.fg()),
    ];
    if !note.is_empty() {
        spans.push(Span::styled(format!("  {note}"), Role::Dim.fg()));
    }
    Line::from(spans)
}

/// A menu entry: its key, its text and a dim note.
#[derive(Clone, Debug)]
pub struct Item {
    /// The key that runs it, for example `1`.
    pub key: String,
    /// What it does.
    pub text: String,
    /// A dim note after the text, for example the version a rollback goes to.
    pub note: String,
}

/// Menu lines `width` wide; the `selected` one is highlighted.
#[must_use]
pub fn menu(items: &[Item], selected: usize, width: u16) -> Vec<Line<'static>> {
    let selected_style = Style::new().fg(Role::Bar.color()).bg(Role::Focus.color());
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mut text = format!(" {}  {}", item.key, item.text);
            if i == selected {
                if !item.note.is_empty() {
                    text = format!("{text} {}", item.note);
                }
                let pad = usize::from(width).saturating_sub(text.chars().count());
                return Line::from(Span::styled(
                    format!("{text}{}", " ".repeat(pad)),
                    selected_style,
                ));
            }
            let mut spans = vec![Span::styled(text, Role::Text.fg())];
            if !item.note.is_empty() {
                spans.push(Span::styled(format!(" {}", item.note), Role::Dim.fg()));
            }
            Line::from(spans)
        })
        .collect()
}

/// A button: highlighted when `selected`, else `[ label ]`.
#[must_use]
pub fn button(label: &str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(
            format!(" {label} "),
            Style::new().fg(Role::Bar.color()).bg(Role::Focus.color()),
        )
    } else {
        Span::styled(format!("[ {label} ]"), Role::Text.fg())
    }
}

/// A text field `width` wide, padded with `_`; highlighted when `focused`.
#[must_use]
pub fn field(value: &str, width: usize, focused: bool) -> Span<'static> {
    let pad = width.saturating_sub(value.chars().count());
    let text = format!("{value}{}", "_".repeat(pad));
    if focused {
        Span::styled(
            text,
            Style::new().fg(Role::Bar.color()).bg(Role::Focus.color()),
        )
    } else {
        Span::styled(text, Role::Text.fg())
    }
}

/// A progress bar `width` cells wide, then `done of total`.
#[must_use]
pub fn progress(done: usize, total: usize, width: usize) -> Line<'static> {
    let filled = (done * width).checked_div(total).unwrap_or(0).min(width);
    Line::from(vec![
        Span::styled("█".repeat(filled), Role::Ok.fg()),
        Span::styled("░".repeat(width - filled), Role::Dim.fg()),
        Span::styled(format!("   {done} of {total}"), Role::Text.fg()),
    ])
}

/// `text` as a QR code in half blocks: dark modules on cream, with a quiet
/// zone of two modules and a rust shadow to the right and below. `None` if
/// the text does not fit a QR code.
#[must_use]
pub fn qr(text: &str) -> Option<Vec<Line<'static>>> {
    const QUIET: usize = 2;
    let code = QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    let size = width + 2 * QUIET;
    let dark = |x: usize, y: usize| {
        x >= QUIET
            && y >= QUIET
            && x < width + QUIET
            && y < width + QUIET
            && colors[(y - QUIET) * width + (x - QUIET)] == Module::Dark
    };
    let paper = Style::new().fg(Role::Bar.color()).bg(Role::Text.color());
    let shadow = Style::new().bg(Role::Shadow.color());
    let mut lines: Vec<Line<'static>> = (0..size.div_ceil(2))
        .map(|row| {
            let mut spans: Vec<Span<'static>> = (0..size)
                .map(|x| {
                    let symbol = match (dark(x, row * 2), dark(x, row * 2 + 1)) {
                        (true, true) => "█",
                        (true, false) => "▀",
                        (false, true) => "▄",
                        (false, false) => " ",
                    };
                    Span::styled(symbol, paper)
                })
                .collect();
            spans.push(if row == 0 {
                Span::styled(" ", Role::Bg.fg())
            } else {
                Span::styled(" ", shadow)
            });
            Line::from(spans)
        })
        .collect();
    lines.push(Line::from(vec![
        Span::styled(" ", Role::Bg.fg()),
        Span::styled(" ".repeat(size), shadow),
    ]));
    Some(lines)
}

/// Lays `width`×`height` in the middle of `area`.
#[must_use]
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let (width, height) = (width.min(area.width), height.min(area.height));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Dims everything in `area` and draws an empty focused box of
/// `width`×`height` in its middle; returns the space inside the box.
pub fn modal(buf: &mut Buffer, area: Rect, width: u16, height: u16, accent: Role) -> Rect {
    for pos in area.positions() {
        if let Some(cell) = buf.cell_mut(pos) {
            cell.set_fg(Role::Dim.color());
            if cell.bg != Role::Bg.color() {
                cell.set_bg(Role::Bar.color());
            }
            cell.modifier = Modifier::empty();
        }
    }
    let outer = centered(area, width, height);
    Clear.render(outer, buf);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(accent.fg())
        .style(Role::Bg.fg());
    let inner = block.inner(outer);
    block.render(outer, buf);
    Rect::new(
        inner.x + 2,
        inner.y + 1,
        inner.width.saturating_sub(4),
        inner.height.saturating_sub(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn lines_read_like_the_design() {
        assert_eq!(
            text(&status(State::Ok, "API", 12, "running", "")),
            "● API          running"
        );
        assert_eq!(
            text(&check(Check::Slow, "NTP", 6, "pool.ntp.org", "slow")),
            "! NTP    pool.ntp.org  slow"
        );
        assert_eq!(text(&progress(6, 8, 8)), "██████░░   6 of 8");
        let items = [
            Item {
                key: "1".into(),
                text: "Show network settings".into(),
                note: String::new(),
            },
            Item {
                key: "3".into(),
                text: "Roll back".into(),
                note: "(Molt: 0.3.2)".into(),
            },
        ];
        let lines = menu(&items, 0, 30);
        assert_eq!(text(&lines[0]), " 1  Show network settings     ");
        assert_eq!(text(&lines[1]), " 3  Roll back (Molt: 0.3.2)");
    }

    #[test]
    fn qr_codes_have_a_quiet_zone_and_a_shadow() {
        let lines = qr("http://10.0.2.15:8080/docs").unwrap();
        let width = text(&lines[0]).chars().count();
        assert!(lines.iter().all(|l| text(l).chars().count() == width));
        assert_eq!(
            lines.len(),
            (width - 1).div_ceil(2) + 1,
            "half blocks plus the shadow row"
        );
        // The quiet zone: the first row and column are light.
        assert!(text(&lines[0]).starts_with("  "));
    }

    #[test]
    fn modals_sit_in_the_middle() {
        let area = Rect::new(0, 0, 80, 25);
        let mut buf = Buffer::empty(area);
        let inner = modal(&mut buf, area, 40, 10, Role::Focus);
        assert_eq!(inner, Rect::new(23, 9, 34, 7));
        assert_eq!(centered(area, 100, 30), area);
    }
}

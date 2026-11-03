//! The system log on tty2 (`crust-console --logs`), read-only: this boot's
//! journal, filtered by unit, level and a search, following new lines.

use std::collections::VecDeque;

use crust_tui::palette::Role;
use crust_tui::widgets::{background, field, key_bar, pane, top_bar};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use serde_json::Value;

use crate::app::Key;
use crate::system::LogLine;

/// Lines kept in memory.
const KEEP: usize = 5000;
/// The units the filter steps through; "kernel" is the kernel's log.
const UNITS: [&str; 7] = [
    "all",
    "crust-api",
    "crust-agent",
    "crust-console",
    "greenboot-healthcheck",
    "NetworkManager",
    "kernel",
];
/// The levels the filter steps through: the lowest priority shown.
const LEVELS: [(u8, &str); 5] = [
    (6, "info and up"),
    (5, "notice and up"),
    (4, "warnings and up"),
    (3, "errors and up"),
    (7, "everything"),
];

/// The log screen.
#[derive(Debug, Default)]
pub struct Logs {
    lines: VecDeque<LogLine>,
    unit: usize,
    level: usize,
    search: String,
    searching: bool,
    paused: bool,
    /// Lines scrolled up from the newest.
    scroll: usize,
}

impl Logs {
    /// The `journalctl` arguments for the current filter.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = ["-b", "-o", "json", "--no-pager", "-n", "2000", "-f"]
            .map(String::from)
            .to_vec();
        args.push(format!("--priority={}", LEVELS[self.level].0));
        match UNITS[self.unit] {
            "all" => {}
            "kernel" => args.push("-k".into()),
            unit => args.push(format!("--unit={unit}")),
        }
        args
    }

    /// Adds a line from the journal.
    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() == KEEP {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        if self.paused {
            self.scroll += 1;
        }
    }

    /// Handles a key; `true` when the journal must be read again.
    pub fn key(&mut self, key: Key) -> bool {
        if self.searching {
            match key {
                Key::Enter | Key::Esc => self.searching = false,
                Key::Backspace => {
                    self.search.pop();
                }
                Key::Char(c) => self.search.push(c),
                _ => {}
            }
            return false;
        }
        match key {
            Key::Char('f') => {
                self.paused = !self.paused;
                if !self.paused {
                    self.scroll = 0;
                }
            }
            Key::Char('/') => self.searching = true,
            Key::Char('u') => {
                self.unit = (self.unit + 1) % UNITS.len();
                self.lines.clear();
                return true;
            }
            Key::Char('l') => {
                self.level = (self.level + 1) % LEVELS.len();
                self.lines.clear();
                return true;
            }
            Key::PageUp => {
                self.paused = true;
                self.scroll = (self.scroll + 20).min(self.lines.len());
            }
            Key::PageDown => {
                self.scroll = self.scroll.saturating_sub(20);
                if self.scroll == 0 {
                    self.paused = false;
                }
            }
            _ => {}
        }
        false
    }

    /// Draws the log screen.
    #[allow(clippy::too_many_lines, reason = "a screen is drawn line by line")]
    pub fn draw(&self, buf: &mut Buffer, version: &str, now: &str) {
        let area = buf.area;
        background(buf, area);
        let version = format!("Crustates {version}");
        top_bar(
            buf,
            Rect::new(area.x, area.y, area.width, 1),
            "System log",
            &[&version, "tty2", "read only"],
            now,
        );
        key_bar(
            buf,
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
            &[
                ("f", "follow on/off"),
                ("/", "search"),
                ("u", "unit"),
                ("l", "level"),
                ("pgup/pgdn", "scroll"),
                ("alt+f1", "back to console"),
            ],
        );
        let body = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        let filter = Rect::new(body.x, body.y, body.width, 3);
        let selected = Style::new().fg(Role::Bar.color()).bg(Role::Focus.color());
        let follow = if self.paused {
            Span::styled("○ paused", Role::Dim.fg())
        } else {
            Span::styled("● following", Role::Ok.fg())
        };
        let line = Line::from(vec![
            Span::styled(" unit ", Role::Text.fg()),
            Span::styled(format!(" {} ", UNITS[self.unit]), selected),
            Span::styled("   level ", Role::Text.fg()),
            Span::styled(LEVELS[self.level].1, Role::bold(Role::Text)),
            Span::styled("   since ", Role::Text.fg()),
            Span::styled("this boot", Role::bold(Role::Text)),
            Span::styled("   search ", Role::Text.fg()),
            field(&self.search, 20, self.searching),
            Span::raw("   "),
            follow,
        ]);
        let block = pane("Filter", None);
        let inner = block.inner(filter);
        block.render(filter, buf);
        buf.set_line(inner.x, inner.y, &line, inner.width);

        let journal = Rect::new(
            body.x,
            body.y + 3,
            body.width,
            body.height.saturating_sub(3),
        );
        let needle = self.search.to_lowercase();
        let shown: Vec<&LogLine> = self
            .lines
            .iter()
            .filter(|l| {
                needle.is_empty()
                    || l.message.to_lowercase().contains(&needle)
                    || l.unit.to_lowercase().contains(&needle)
            })
            .collect();
        let block = pane(
            &format!("journal · {} lines", shown.len()),
            Some(Role::Focus),
        );
        let inner = block.inner(journal);
        block.render(journal, buf);
        let rows = usize::from(inner.height.saturating_sub(1));
        let end = shown.len().saturating_sub(self.scroll);
        let lines: Vec<Line<'static>> = shown[end.saturating_sub(rows)..end]
            .iter()
            .map(|l| {
                let color = match l.priority {
                    0..=3 => Role::Bad,
                    4 => Role::Focus,
                    7 => Role::Dim,
                    _ => Role::Text,
                };
                Line::from(vec![
                    Span::styled(format!("{} ", l.time), Role::Dim.fg()),
                    Span::styled(
                        format!("{:<16} ", l.unit.chars().take(16).collect::<String>()),
                        Role::Teal.fg(),
                    ),
                    Span::styled(l.message.clone(), color.fg()),
                ])
            })
            .collect();
        let text = Rect::new(
            inner.x + 1,
            inner.y + 1,
            inner.width.saturating_sub(2),
            inner.height.saturating_sub(1),
        );
        Paragraph::new(lines).render(text, buf);
    }
}

/// A journal entry from `journalctl -o json`.
#[must_use]
pub fn parse(json: &str) -> Option<LogLine> {
    let entry: Value = serde_json::from_str(json).ok()?;
    let micros: i64 = entry["__REALTIME_TIMESTAMP"].as_str()?.parse().ok()?;
    let time = jiff::Timestamp::from_microsecond(micros)
        .ok()?
        .to_zoned(jiff::tz::TimeZone::system())
        .strftime("%b %d %H:%M:%S")
        .to_string();
    let unit = entry["SYSLOG_IDENTIFIER"]
        .as_str()
        .or_else(|| entry["_SYSTEMD_UNIT"].as_str())
        .unwrap_or("-")
        .to_owned();
    // Messages that are not UTF-8 come as an array of bytes.
    let message = match &entry["MESSAGE"] {
        Value::String(s) => s.clone(),
        Value::Array(bytes) => String::from_utf8_lossy(
            &bytes
                .iter()
                .filter_map(|b| u8::try_from(b.as_u64()?).ok())
                .collect::<Vec<_>>(),
        )
        .into_owned(),
        _ => String::new(),
    };
    let priority = entry["PRIORITY"]
        .as_str()
        .and_then(|p| p.parse().ok())
        .unwrap_or(6);
    Some(LogLine {
        time,
        unit,
        priority,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_entries_become_lines() {
        let line = parse(r#"{"__REALTIME_TIMESTAMP":"1762206298000000","SYSLOG_IDENTIFIER":"crust-api","PRIORITY":"4","MESSAGE":"slow"}"#).unwrap();
        assert_eq!(
            (line.unit.as_str(), line.priority, line.message.as_str()),
            ("crust-api", 4, "slow")
        );
        let bytes = parse(
            r#"{"__REALTIME_TIMESTAMP":"1","_SYSTEMD_UNIT":"x.service","MESSAGE":[104,105]}"#,
        )
        .unwrap();
        assert_eq!(
            (bytes.unit.as_str(), bytes.message.as_str()),
            ("x.service", "hi")
        );
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn filters_change_the_journal_query() {
        let mut logs = Logs::default();
        assert!(logs.args().contains(&"--priority=6".to_owned()));
        assert!(logs.key(Key::Char('u')));
        assert!(logs.args().contains(&"--unit=crust-api".to_owned()));
        logs.key(Key::Char('/'));
        assert!(!logs.key(Key::Char('u')), "typing a search");
        assert_eq!(logs.search, "u");
    }
}

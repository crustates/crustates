//! The crab mark, drawn from `assets/mark-light.svg` at any size: as
//! pixels for the boot splash, as half blocks for the console.
//!
//! The mark is pixel art made of rectangles and paths of straight
//! horizontal and vertical lines, so a few lines rasterise it; outlines
//! (strokes) are left out, they are the background colour.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::palette::Role;

const SVG: &str = include_str!("../assets/mark-light.svg");

/// One filled shape: a polygon and its colour.
struct Shape {
    points: Vec<(f32, f32)>,
    color: Role,
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let len = tag[start..].find('"')?;
    Some(&tag[start..start + len])
}

fn number(tag: &str, name: &str) -> f32 {
    attr(tag, name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

fn color(fill: &str) -> Role {
    match fill.to_ascii_uppercase().as_str() {
        "#E8552D" => Role::Shadow,
        "#FFF6E5" => Role::Text,
        _ => Role::Bar,
    }
}

/// `M x y H x V y … Z`, absolute commands only, as the mark uses.
fn path(d: &str) -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    let (mut x, mut y) = (0.0, 0.0);
    let mut tokens = d.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        let (command, rest) = token.split_at(1);
        let mut value = || -> f32 {
            if rest.is_empty() {
                tokens.next().and_then(|t| t.parse().ok()).unwrap_or(0.0)
            } else {
                rest.parse().unwrap_or(0.0)
            }
        };
        match command {
            "M" => {
                x = value();
                y = tokens.next().and_then(|t| t.parse().ok()).unwrap_or(0.0);
            }
            "H" => x = value(),
            "V" => y = value(),
            _ => continue,
        }
        points.push((x, y));
    }
    points
}

fn shapes() -> Vec<Shape> {
    let mut shapes = Vec::new();
    let mut offset = (0.0, 0.0);
    for tag in SVG.split('<').map(|t| t.trim_end_matches(['>', '/'])) {
        if tag.starts_with("g ") {
            let translate = attr(tag, "transform")
                .and_then(|t| t.strip_prefix("translate("))
                .and_then(|t| t.strip_suffix(')'))
                .unwrap_or("0 0");
            let mut xy = translate
                .split_whitespace()
                .map(|n| n.parse().unwrap_or(0.0));
            offset = (xy.next().unwrap_or(0.0), xy.next().unwrap_or(0.0));
        } else if tag.starts_with("/g") {
            offset = (0.0, 0.0);
        }
        let points = if tag.starts_with("rect ") {
            let (x, y) = (number(tag, "x"), number(tag, "y"));
            let (w, h) = (number(tag, "width"), number(tag, "height"));
            vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
        } else if tag.starts_with("path ") {
            path(attr(tag, "d").unwrap_or_default())
        } else {
            continue;
        };
        shapes.push(Shape {
            points: points
                .iter()
                .map(|(x, y)| (x + offset.0, y + offset.1))
                .collect(),
            color: color(attr(tag, "fill").unwrap_or_default()),
        });
    }
    shapes
}

/// Even-odd rule: the polygons are closed and axis-aligned.
fn inside(points: &[(f32, f32)], (x, y): (f32, f32)) -> bool {
    let mut inside = false;
    for (i, &(x1, y1)) in points.iter().enumerate() {
        let (x2, y2) = points[(i + 1) % points.len()];
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
            inside = !inside;
        }
    }
    inside
}

/// The mark as `width` columns of square pixels, top row first; `None` is
/// transparent. The height follows from the mark's shape and is even.
#[must_use]
pub fn pixels(width: u16) -> Vec<Vec<Option<Role>>> {
    let shapes = shapes();
    let all = shapes.iter().flat_map(|s| s.points.iter());
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in all {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let size = (x1 - x0) / f32::from(width.max(1));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a few dozen rows"
    )]
    let rows = ((y1 - y0) / size).ceil() as u16;
    // 4×4 samples per pixel; the most frequent colour wins.
    let samples = [0.125, 0.375, 0.625, 0.875];
    (0..rows.next_multiple_of(2))
        .map(|row| {
            (0..width)
                .map(|col| {
                    let mut counts: Vec<(Option<Role>, u32)> = Vec::new();
                    for sy in samples {
                        for sx in samples {
                            let point = (
                                x0 + (f32::from(col) + sx) * size,
                                y0 + (f32::from(row) + sy) * size,
                            );
                            let hit = shapes
                                .iter()
                                .rev()
                                .find(|s| inside(&s.points, point))
                                .map(|s| s.color);
                            match counts.iter_mut().find(|(c, _)| *c == hit) {
                                Some((_, n)) => *n += 1,
                                None => counts.push((hit, 1)),
                            }
                        }
                    }
                    counts.iter().max_by_key(|(_, n)| *n).and_then(|(c, _)| *c)
                })
                .collect()
        })
        .collect()
}

/// The mark in half blocks, `width` cells wide: two pixels per cell.
#[must_use]
pub fn lines(width: u16) -> Vec<Line<'static>> {
    let pixels = pixels(width);
    pixels
        .chunks(2)
        .map(|pair| {
            let spans: Vec<Span<'static>> = pair[0]
                .iter()
                .zip(&pair[1])
                .map(|(top, bottom)| match (top, bottom) {
                    (None, None) => Span::styled(" ", Role::Bg.fg()),
                    (Some(t), None) => Span::styled("▀", Role::fg(*t)),
                    (None, Some(b)) => Span::styled("▄", Role::fg(*b)),
                    (Some(t), Some(b)) if t == b => Span::styled("█", Role::fg(*t)),
                    (Some(t), Some(b)) => {
                        Span::styled("▀", Style::new().fg(t.color()).bg(b.color()))
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_has_its_shapes_and_draws_at_console_size() {
        let shapes = shapes();
        assert_eq!(shapes.len(), 17, "7 shadows, 7 shapes, 2 pupils, the mouth");
        assert!(shapes.iter().any(|s| s.color == Role::Text), "the eyes");
        let lines = lines(18);
        assert!((4..=8).contains(&lines.len()), "{} rows", lines.len());
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(text.contains('█'), "{text}");
    }

    #[test]
    fn paths_follow_horizontal_and_vertical_moves() {
        assert_eq!(
            path("M0 12 H5 V21 H11 Z"),
            [(0.0, 12.0), (5.0, 12.0), (5.0, 21.0), (11.0, 21.0)]
        );
        let square = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        assert!(inside(&square, (1.0, 1.0)));
        assert!(!inside(&square, (3.0, 1.0)));
    }
}

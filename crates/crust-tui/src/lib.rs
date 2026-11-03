//! What the Crustates text-mode screens share (the local console now, the
//! installer later): the palette, the console font and its glyphs, and the
//! widgets of the design kit (bar, panes, status lines, menus, QR codes,
//! the crab).
//!
//! Screens draw with [`palette::Role`] colours, which are indices into the
//! Linux console's 16-colour palette; [`palette::to_rgb`] turns them into
//! true colour for other terminals.

pub mod glyphs;
pub mod mark;
pub mod palette;
pub mod psf;
pub mod widgets;

//! RenderState snapshot → `FrameDelta`. Touches only viewport rows (spec §6.3).
use libghostty_vt::error::Result;
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RowIterator, Snapshot};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::{RgbColor, Underline};
use wraith_proto::{Color, Colors, CursorShape, CursorState, FrameDelta, Row, Scrollbar, StyleFlags, StyleRun};

fn rgb(c: RgbColor) -> (u8, u8, u8) {
    (c.r, c.g, c.b)
}

/// Build a frame from `snap`, consuming its dirty state. `force_full` sends every row.
/// `scrollbar` and `title` are left for the caller (they live on the Terminal, not the snapshot).
pub fn build_frame(snap: &Snapshot<'_, '_>, seq: u64, force_full: bool) -> Result<FrameDelta> {
    let full = force_full || matches!(snap.dirty()?, Dirty::Full);
    let mut rows_it = RowIterator::new()?;
    let mut cells_it = CellIterator::new()?;
    let mut dirty_rows = Vec::new();
    let mut text = String::new();

    let mut it = rows_it.update(snap)?;
    let mut y: u16 = 0;
    while let Some(row) = it.next() {
        if full || row.dirty()? {
            let mut runs: Vec<StyleRun> = Vec::new();
            let mut cells = cells_it.update(row)?;
            while let Some(cell) = cells.next() {
                if matches!(cell.raw_cell()?.wide()?, CellWide::SpacerTail | CellWide::SpacerHead) {
                    continue;
                }
                let fg = cell.fg_color()?.map_or(Color::Default, |c| Color::Rgb(c.r, c.g, c.b));
                let bg = cell.bg_color()?.map_or(Color::Default, |c| Color::Rgb(c.r, c.g, c.b));
                let mut flags = if cell.has_styling()? { flags(&cell.style()?) } else { StyleFlags(0) };
                if cell.is_selected()? {
                    flags.0 |= StyleFlags::SELECTED;
                }
                if cell.graphemes_len()? == 0 {
                    text.clear();
                    text.push(' ');
                } else {
                    cell.graphemes_utf8(&mut text)?;
                }
                match runs.last_mut() {
                    Some(r) if r.fg == fg && r.bg == bg && r.flags == flags => r.text.push_str(&text),
                    _ => runs.push(StyleRun { fg, bg, flags, hyperlink_id: 0, text: text.clone() }),
                }
            }
            // shortcut: soft-wrap flag and hyperlink ids not reported yet; needed for selection-unwrap and OSC 8 (M3).
            dirty_rows.push(Row { y, runs, wrapped: false });
        }
        row.set_dirty(false)?;
        y += 1;
    }
    snap.set_dirty(Dirty::Clean)?;

    let colors = snap.colors()?;
    let cursor = snap.cursor_viewport()?;
    Ok(FrameDelta {
        seq,
        full,
        cols: snap.cols()?,
        rows: snap.rows()?,
        dirty_rows,
        cursor: CursorState {
            x: cursor.map_or(0, |c| c.x),
            y: cursor.map_or(0, |c| c.y),
            visible: cursor.is_some() && snap.cursor_visible()?,
            style: match snap.cursor_visual_style()? {
                CursorVisualStyle::Bar => CursorShape::Bar,
                CursorVisualStyle::Underline => CursorShape::Underline,
                CursorVisualStyle::BlockHollow => CursorShape::BlockHollow,
                _ => CursorShape::Block,
            },
        },
        colors: Colors {
            fg: rgb(colors.foreground),
            bg: rgb(colors.background),
            cursor: colors.cursor.map(rgb),
            palette: if full { colors.palette.iter().copied().map(rgb).collect() } else { Vec::new() },
        },
        scrollbar: Scrollbar { total: 0, offset: 0, len: 0 },
        title: None,
        images: Vec::new(),
    })
}

fn flags(s: &libghostty_vt::style::Style) -> StyleFlags {
    let mut f = 0;
    for (on, bit) in [
        (s.bold, StyleFlags::BOLD),
        (s.italic, StyleFlags::ITALIC),
        (s.faint, StyleFlags::FAINT),
        (s.inverse, StyleFlags::INVERSE),
        (s.invisible, StyleFlags::INVISIBLE),
        (s.strikethrough, StyleFlags::STRIKE),
        (s.blink, StyleFlags::BLINK),
        (matches!(s.underline, Underline::Single | Underline::Dotted | Underline::Dashed), StyleFlags::UNDERLINE),
        (matches!(s.underline, Underline::Double), StyleFlags::DOUBLE_UNDERLINE),
        (matches!(s.underline, Underline::Curly), StyleFlags::CURLY_UNDERLINE),
    ] {
        if on {
            f |= bit;
        }
    }
    StyleFlags(f)
}

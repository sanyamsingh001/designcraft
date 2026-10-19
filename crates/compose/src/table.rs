//! Table layout: a table anchored in a story is laid out at its anchor paragraph.
//!
//! Column widths are scaled down to fit the text column; each cell's story is composed into its
//! cell (insets applied) with the regular composer; a row is as tall as its tallest single-row
//! cell (or its fixed height); rows that don't fit continue in the next column / frame, repeating
//! header (and footer) rows. Rows tied together by row spans never split.

use std::sync::Arc;

use designcraft_doc::{Align, CellStroke, Document, ItemId, ParaProps, RowHeightMode, Story, Table, TextFrameOptions, VerticalJustification};
use designcraft_geom::{Point, Rect};

use crate::{ComposeOptions, ComposedStory, Cursor, FrameSpec, Line};

/// A cell placed in a frame (frame inner space).
#[derive(Clone, Debug)]
pub struct PlacedCell {
    pub row: usize,
    pub col: usize,
    pub rect: Rect,
    /// Fill swatch and tint (cell fill or alternating fill).
    pub fill: Option<(String, f32)>,
    /// Where the cell's composed text (its frame 0, in content space) sits in the frame.
    pub origin: Point,
    pub text: Arc<ComposedStory>,
    /// The cell's story text (exporters map glyphs back to text).
    pub source: Arc<str>,
    /// Content height available in the cell (for clipping).
    pub clip: Rect,
    pub overset: bool,
    /// A header/footer row repeated in a continuation fragment.
    pub repeated: bool,
    /// Graphic cell content (drawn at `clip`'s top left, clipped to it).
    pub graphic: Option<designcraft_doc::Graphic>,
}

/// One cell edge (or the table border) to stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeSeg {
    pub a: Point,
    pub b: Point,
    pub stroke: CellStroke,
}

/// The part of a table shown in one text column.
#[derive(Clone, Debug)]
pub struct TableFrag {
    pub table: u64,
    /// Story byte of the anchor character.
    pub anchor: usize,
    pub column: u32,
    pub rect: Rect,
    pub cells: Vec<PlacedCell>,
    pub strokes: Vec<StrokeSeg>,
    /// Index of the fragment's (glyph-less) line in the frame's lines.
    pub line: usize,
    pub first: bool,
    pub last: bool,
}

impl TableFrag {
    pub(crate) fn shift(&mut self, dy: f64) {
        if dy == 0.0 {
            return;
        }
        let v = designcraft_geom::Vec2::new(0.0, dy);
        self.rect = self.rect + v;
        for c in &mut self.cells {
            c.rect = c.rect + v;
            c.clip = c.clip + v;
            c.origin += v;
        }
        for s in &mut self.strokes {
            s.a += v;
            s.b += v;
        }
    }

    /// The placed cell owning grid position (row, col), if it is in this fragment.
    pub fn cell(&self, row: usize, col: usize) -> Option<&PlacedCell> {
        self.cells.iter().find(|c| c.row == row && c.col == col)
    }
}

struct CellLayout {
    text: Arc<ComposedStory>,
    content_h: f64,
}

/// Compose a cell story at width `w` (content space, top-left at the origin).
fn compose_cell(doc: &Document, story: &Story, w: f64, opts: &ComposeOptions, left_page: bool) -> CellLayout {
    let spec = FrameSpec {
        id: ItemId(0),
        area: Rect::new(0.0, 0.0, w.max(1.0), 1.0e6),
        opts: TextFrameOptions::default(),
        vertical: false,
        exclusions: vec![],
        page_name: opts.page_name.clone(),
        page: opts.page,
        grid: None,
        left_page,
        page_rect: None,
    };
    let cs = crate::compose(doc, story, std::slice::from_ref(&spec), opts);
    let content_h = cs.frames.first().map(|f| if f.lines.is_empty() { 0.0 } else { f.content_height }).unwrap_or(0.0);
    CellLayout { text: Arc::new(cs), content_h }
}

/// Lay out `table` (anchored at byte `anchor` of paragraph `pi`) from the cursor. Returns false
/// when it doesn't fit the remaining frames (overset).
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_table(
    doc: &Document,
    table: &Table,
    anchor: usize,
    pi: usize,
    pp: &ParaProps,
    frames: &[FrameSpec],
    cols: &[Vec<Rect>],
    cur: &mut Cursor,
    out: &mut ComposedStory,
    opts: &ComposeOptions,
) -> bool {
    if cur.fi >= frames.len() || table.nrows() == 0 || table.ncols() == 0 {
        return false;
    }
    let col_rect = |cur: &Cursor| cur.clip(cols[cur.fi][cur.col.min(cols[cur.fi].len() - 1)]);
    let mut col = col_rect(cur);
    // Column widths: scaled to fit the measure.
    let avail = (col.width() - pp.left_indent - pp.right_indent).max(1.0);
    let natural = table.width();
    let k = if natural > avail && natural > 0.0 { avail / natural } else { 1.0 };
    let widths: Vec<f64> = table.columns.iter().map(|c| c.width * k).collect();
    let total_w: f64 = widths.iter().sum();
    let x_off = pp.left_indent
        + match pp.align {
            Align::Center | Align::CenterJustified => (avail - total_w) / 2.0,
            Align::Right | Align::RightJustified => avail - total_w,
            _ => 0.0,
        }
        .max(0.0);
    let mut colx = vec![0.0; widths.len() + 1];
    for (i, w) in widths.iter().enumerate() {
        colx[i + 1] = colx[i] + w;
    }
    let (nr, nc) = (table.nrows(), table.ncols());
    let owners = table.owners();
    let is_owner = |r: usize, c: usize| owners[r * nc + c] == (r, c);
    let left_page = frames[cur.fi].left_page;
    // Compose every owner cell.
    let mut layouts: Vec<Option<CellLayout>> = Vec::with_capacity(nr * nc);
    for r in 0..nr {
        for c in 0..nc {
            if !is_owner(r, c) {
                layouts.push(None);
                continue;
            }
            let cell = &table.cells[r * nc + c];
            let cs = (cell.col_span.max(1) as usize).min(nc - c);
            let w = colx[c + cs] - colx[c] - cell.insets[1] - cell.insets[3];
            layouts.push(Some(compose_cell(doc, &cell.text, w, opts, left_page)));
        }
    }
    // Row heights.
    let need = |r: usize, c: usize| -> f64 {
        let cell = &table.cells[r * nc + c];
        layouts[r * nc + c].as_ref().map_or(0.0, |l| l.content_h) + cell.insets[0] + cell.insets[2]
    };
    let mut heights: Vec<f64> = table
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| match row.mode {
            RowHeightMode::Exactly => row.height.max(0.0),
            RowHeightMode::AtLeast => {
                let content = (0..nc).filter(|&c| is_owner(r, c) && table.cells[r * nc + c].row_span <= 1).map(|c| need(r, c)).fold(0.0, f64::max);
                content.max(row.height)
            }
        })
        .collect();
    // Row-spanning cells grow the last row they span (unless it is fixed).
    for r in 0..nr {
        for c in 0..nc {
            let cell = &table.cells[r * nc + c];
            if !is_owner(r, c) || cell.row_span <= 1 {
                continue;
            }
            let end = (r + cell.row_span as usize).min(nr);
            let have: f64 = heights[r..end].iter().sum();
            let n = need(r, c);
            if n > have
                && let Some(i) = (r..end).rev().find(|&i| table.rows[i].mode == RowHeightMode::AtLeast)
            {
                heights[i] += n - have;
            }
        }
    }
    // Row groups that must stay together: end (exclusive) of the group starting at each row.
    let mut group_end: Vec<usize> = (0..nr).map(|r| r + 1).collect();
    {
        let mut r = 0;
        while r < nr {
            let mut e = r + 1;
            let mut i = r;
            while i < e {
                for c in 0..nc {
                    if is_owner(i, c) {
                        e = e.max((i + table.cells[i * nc + c].row_span.max(1) as usize).min(nr));
                    }
                }
                i += 1;
            }
            for g in group_end.iter_mut().take(e).skip(r) {
                *g = e;
            }
            r = e;
        }
    }
    let header = table.header_rows();
    let footer = table.footer_rows();
    let body_end = nr - footer;
    let sum = |a: usize, b: usize| heights[a..b].iter().sum::<f64>();
    let footer_h = sum(body_end, nr);
    let o = &table.options;
    let mut y = match cur.last_baseline {
        Some(b) => b + cur.last_descent + cur.pending + o.space_before,
        None => col.y0,
    };
    let mut frag_rows: Vec<(usize, f64, bool)> = Vec::new(); // (row, y, repeated)
    let mut first_frag = true;
    let mut body_placed = 0usize;
    let mut top = y;
    let place_rows = |rows: std::ops::Range<usize>, y: &mut f64, repeated: bool, frag_rows: &mut Vec<(usize, f64, bool)>| {
        for r in rows {
            frag_rows.push((r, *y, repeated));
            *y += heights[r];
        }
    };
    place_rows(0..header, &mut y, false, &mut frag_rows);
    let mut r = header;
    loop {
        if r >= body_end {
            break;
        }
        let ge = group_end[r].min(body_end).max(r + 1);
        let gh = sum(r, ge);
        let at_col_top = cur.last_baseline.is_none() && (top - col.y0).abs() < 1e-6;
        if y + gh + footer_h <= col.y1 + 0.01 || (body_placed == 0 && at_col_top) {
            if y + gh + footer_h > col.y1 + 0.01 && body_placed == 0 && at_col_top && y + gh > col.y1 + 0.01 {
                // A row group taller than an empty column can never fit.
                return false;
            }
            place_rows(r..ge, &mut y, false, &mut frag_rows);
            body_placed += 1;
            r = ge;
            continue;
        }
        // Continue in the next column / frame.
        if body_placed > 0 {
            if o.repeat_footer {
                place_rows(body_end..nr, &mut y, true, &mut frag_rows);
            }
            emit(table, anchor, pi, &frag_rows, &heights, &colx, col.x0 + x_off, top, y, first_frag, false, &layouts, &owners, cur, col, out);
            first_frag = false;
        }
        let was_first = first_frag;
        cur.next_column(cols);
        if cur.fi >= frames.len() {
            return false;
        }
        col = col_rect(cur);
        y = col.y0;
        top = y;
        frag_rows.clear();
        body_placed = 0;
        // Headers: the first fragment's header moves along; later ones repeat it.
        if was_first {
            place_rows(0..header, &mut y, false, &mut frag_rows);
        } else if o.repeat_header {
            place_rows(0..header, &mut y, true, &mut frag_rows);
        }
    }
    place_rows(body_end..nr, &mut y, false, &mut frag_rows);
    emit(table, anchor, pi, &frag_rows, &heights, &colx, col.x0 + x_off, top, y, first_frag, true, &layouts, &owners, cur, col, out);
    cur.last_baseline = Some(y);
    cur.last_descent = 0.0;
    cur.pending = o.space_after;
    true
}

/// Build one fragment from its rows and push it (with its glyph-less line) to the current frame.
#[allow(clippy::too_many_arguments)]
fn emit(
    table: &Table,
    anchor: usize,
    pi: usize,
    rows: &[(usize, f64, bool)],
    heights: &[f64],
    colx: &[f64],
    x0: f64,
    top: f64,
    bottom: f64,
    first: bool,
    last: bool,
    layouts: &[Option<CellLayout>],
    owners: &[(usize, usize)],
    cur: &Cursor,
    col: Rect,
    out: &mut ComposedStory,
) {
    let nc = table.ncols();
    let mut row_y = vec![None; table.nrows()];
    for &(r, y, _) in rows {
        if let Some(slot) = row_y.get_mut(r)
            && slot.is_none()
        {
            *slot = Some(y);
        }
    }
    let ry = |r: usize| row_y.get(r).copied().flatten();
    let header = table.header_rows();
    let body = table.body_rows();
    let nbody = body.len();
    let mut cells = Vec::new();
    let mut strokes = Vec::new();
    let o = &table.options;
    let right = x0 + colx[nc];
    let rtl = table.options.direction == designcraft_doc::TextDirection::RightToLeft;
    for (row_index, &(r, y, repeated)) in rows.iter().enumerate() {
        let above_row = row_index.checked_sub(1).and_then(|i| rows.get(i)).map(|&(row, _, _)| row);
        for c in 0..nc {
            if owners[r * nc + c] != (r, c) {
                continue;
            }
            let cell = &table.cells[r * nc + c];
            let cs = (cell.col_span.max(1) as usize).min(nc - c);
            let rs = (cell.row_span.max(1) as usize).min(table.nrows() - r);
            let last_row = r + rs - 1;
            let y1 = ry(last_row).map_or(y + heights[r], |ly| ly + heights[last_row]);
            let rect = if table.options.direction == designcraft_doc::TextDirection::RightToLeft {
                Rect::new(right - colx[c + cs], y, right - colx[c], y1)
            } else {
                Rect::new(x0 + colx[c], y, x0 + colx[c + cs], y1)
            };
            let fill = if cell.has_fill() {
                Some((cell.fill.clone(), cell.fill_tint))
            } else if body.contains(&r) {
                let i = r - header;
                o.alt_rows
                    .as_ref()
                    .and_then(|a| a.fill_for(i, nbody))
                    .or_else(|| o.alt_cols.as_ref().and_then(|a| a.fill_for(c, nc)))
                    .map(|(s, t)| (s.to_string(), t))
            } else {
                None
            };
            let Some(lay) = layouts.get(r * nc + c).and_then(Option::as_ref) else { continue };
            let clip = Rect::new(rect.x0 + cell.insets[1], rect.y0 + cell.insets[0], rect.x1 - cell.insets[3], rect.y1 - cell.insets[2]);
            let space = clip.height() - lay.content_h;
            let dy = match cell.vj {
                VerticalJustification::Center if space > 0.0 => space / 2.0,
                VerticalJustification::Bottom if space > 0.0 => space,
                _ => 0.0,
            };
            let overset = space < -0.01;
            let mut text = lay.text.clone();
            if overset && let Some(f) = text.frames.first() {
                // Drop the lines that don't fit the cell.
                let keep: Vec<Line> = f.lines.iter().filter(|l| l.baseline + l.descent <= clip.height() + 0.01).cloned().collect();
                let mut t = (*text).clone();
                t.frames[0].lines = keep;
                text = Arc::new(t);
            }
            cells.push(PlacedCell {
                row: r,
                col: c,
                rect,
                fill,
                origin: Point::new(clip.x0, clip.y0 + dy),
                text,
                source: Arc::from(cell.text.text.as_str()),
                clip,
                overset,
                repeated,
                graphic: cell.graphic.clone(),
            });
            // Perimeter edges retain the table border unless the cell explicitly overrides it.
            // Interior seams use both adjacent owners, once from the lower/right cell.
            let at_top = (y - top).abs() < 1e-6;
            let at_bottom = (y1 - bottom).abs() < 1e-6;
            let at_left = (rect.x0 - x0).abs() < 1e-6;
            let edge = |i: usize| {
                if !cell.border_overrides[i] { o.border.clone() } else { cell.strokes[i].clone() }
            };
            if at_top {
                push_stroke(&mut strokes, Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), edge(0));
            } else {
                push_shared_strokes(
                    &mut strokes,
                    (0..cs).map(|i| {
                        let column = if rtl { c + cs - 1 - i } else { c + i };
                        let (left, right) =
                            if rtl { (right - colx[column + 1], right - colx[column]) } else { (x0 + colx[column], x0 + colx[column + 1]) };
                        let above = above_row.and_then(|row| owners.get(row * nc + column)).and_then(|&(row, col)| table.cell(row, col));
                        StrokeSeg {
                            a: Point::new(left, rect.y0),
                            b: Point::new(right, rect.y0),
                            stroke: shared_stroke(
                                table.border_resolution,
                                &cell.strokes[0],
                                cell.border_overrides[0],
                                above.map(|cell| (&cell.strokes[2], cell.border_overrides[2])),
                            ),
                        }
                    }),
                );
            }
            if at_left {
                push_stroke(&mut strokes, Point::new(rect.x0, rect.y0), Point::new(rect.x0, rect.y1), edge(1));
            } else {
                let adjacent_col = if rtl { (c + cs < nc).then_some(c + cs) } else { c.checked_sub(1) };
                push_shared_strokes(
                    &mut strokes,
                    (r..=last_row).filter_map(|row| {
                        let y = ry(row)?;
                        let adjacent = adjacent_col.and_then(|col| owners.get(row * nc + col)).and_then(|&(row, col)| table.cell(row, col));
                        Some(StrokeSeg {
                            a: Point::new(rect.x0, y),
                            b: Point::new(rect.x0, (y + heights[row]).min(rect.y1)),
                            stroke: shared_stroke(
                                table.border_resolution,
                                &cell.strokes[1],
                                cell.border_overrides[1],
                                adjacent.map(|cell| (&cell.strokes[3], cell.border_overrides[3])),
                            ),
                        })
                    }),
                );
            }
            if at_bottom {
                push_stroke(&mut strokes, Point::new(rect.x0, rect.y1), Point::new(rect.x1, rect.y1), edge(2));
            }
            if (rect.x1 - right).abs() < 1e-6 {
                push_stroke(&mut strokes, Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), edge(3));
            }
        }
    }
    let ft = &mut out.frames[cur.fi];
    let line = ft.lines.len();
    let h = bottom - top;
    ft.lines.push(Line {
        column: cur.col as u32,
        baseline: bottom,
        x0: col.x0,
        x1: col.x1,
        ascent: h,
        descent: 0.0,
        leading: h,
        range: anchor..anchor + designcraft_doc::TABLE_ANCHOR.len_utf8(),
        para: pi,
        glyphs: vec![],
        hyphenated: false,
        first_in_para: first,
        last_in_para: last,
        end_x: right,
        spacing: 1.0,
        hj: 0,
        keep_violation: false,
    });
    ft.tables.push(TableFrag {
        table: table.id,
        anchor,
        column: cur.col as u32,
        rect: Rect::new(x0, top, right, bottom),
        cells,
        strokes,
        line,
        first,
        last,
    });
}

/// Explicit top/left edges keep conflict priority, including removals. An explicit
/// bottom/right edge beats an implicit default only on its overlap with this owner.
/// Legacy tables retain the preferred edge when neither side has explicit priority.
fn shared_stroke(
    resolution: designcraft_doc::TableBorderResolution,
    preferred: &CellStroke,
    explicit: bool,
    adjacent: Option<(&CellStroke, bool)>,
) -> CellStroke {
    if explicit {
        return preferred.clone();
    }
    if let Some((stroke, true)) = adjacent {
        return stroke.clone();
    }
    if resolution == designcraft_doc::TableBorderResolution::Legacy || preferred.is_visible() {
        preferred.clone()
    } else {
        adjacent.map(|(stroke, _)| stroke).filter(|stroke| stroke.is_visible()).unwrap_or(preferred).clone()
    }
}

fn push_stroke(strokes: &mut Vec<StrokeSeg>, a: Point, b: Point, stroke: CellStroke) {
    if stroke.is_visible() {
        strokes.push(StrokeSeg { a, b, stroke });
    }
}

/// Split a merged edge only where adjacent owners change its visible stroke.
fn push_shared_strokes(strokes: &mut Vec<StrokeSeg>, segments: impl Iterator<Item = StrokeSeg>) {
    let mut run: Option<StrokeSeg> = None;
    for segment in segments {
        if let Some(previous) = run.as_mut()
            && previous.b == segment.a
            && previous.stroke == segment.stroke
        {
            previous.b = segment.b;
            continue;
        }
        if let Some(previous) = run.take() {
            push_stroke(strokes, previous.a, previous.b, previous.stroke);
        }
        run = Some(segment);
    }
    if let Some(last) = run {
        push_stroke(strokes, last.a, last.b, last.stroke);
    }
}

// ---------- hit testing and carets in cells ----------

/// The cell under `p` (frame inner space) in frame `fi`: (table, row, col, story byte in the cell).
pub fn hit_cell(cs: &ComposedStory, fi: usize, p: Point) -> Option<(u64, usize, usize, usize)> {
    let ft = cs.frames.get(fi)?;
    for t in &ft.tables {
        if !t.rect.contains(p) {
            continue;
        }
        for c in &t.cells {
            if c.rect.contains(p) {
                let local = Point::new(p.x - c.origin.x, p.y - c.origin.y);
                let b = crate::hit(&c.text, 0, local).unwrap_or(0);
                return Some((t.table, c.row, c.col, b));
            }
        }
    }
    None
}

/// Frame index and placed cell for a cell address (the first, non-repeated occurrence).
pub fn find_cell(cs: &ComposedStory, table: u64, row: usize, col: usize) -> Option<(usize, &TableFrag, &PlacedCell)> {
    let mut found = None;
    for (fi, ft) in cs.frames.iter().enumerate() {
        for t in ft.tables.iter().filter(|t| t.table == table) {
            if let Some(c) = t.cell(row, col) {
                if !c.repeated {
                    return Some((fi, t, c));
                }
                found = found.or(Some((fi, t, c)));
            }
        }
    }
    found
}

/// Caret geometry for byte `pos` of a cell's story, in frame inner space:
/// (frame index, x, baseline, ascent, descent).
pub fn cell_caret(cs: &ComposedStory, table: u64, row: usize, col: usize, pos: usize) -> Option<(usize, f64, f64, f64, f64)> {
    let (fi, _, c) = find_cell(cs, table, row, col)?;
    match crate::caret(&c.text, pos) {
        Some((_, x, bl, a, d)) => Some((fi, x + c.origin.x, bl + c.origin.y, a, d)),
        None => Some((fi, c.origin.x, c.origin.y + 10.0, 9.0, 3.0)),
    }
}

#[cfg(test)]
mod border_tests {
    use super::*;
    use designcraft_doc::{CellRange, ParaFormat, SpreadRef, build::NewDocument};

    fn compose_table(table: Table) -> ComposedStory {
        compose_table_frames(table, 200.0, 1)
    }

    fn compose_table_frames(table: Table, height: f64, count: usize) -> ComposedStory {
        let mut doc = Document::new(&NewDocument::default());
        let layer = doc.default_layer();
        let (mut frame, sid) = doc.add_text_frame(SpreadRef::Doc(0), Rect::new(0.0, 0.0, 120.0, height), layer, "", ParaFormat::default()).unwrap();
        for i in 1..count {
            let y = i as f64 * (height + 20.0);
            let (next, _) = doc.add_text_frame(SpreadRef::Doc(0), Rect::new(0.0, y, 120.0, y + height), layer, "", ParaFormat::default()).unwrap();
            doc.thread(frame, next).unwrap();
            frame = next;
        }
        doc.story_mut(sid).unwrap().insert_table(0, table);
        crate::compose_story(&doc, sid, &ComposeOptions::default())
    }

    fn set_rules(table: &mut Table, edge: usize) {
        table.options.border = CellStroke::none();
        for row in &mut table.rows {
            row.mode = RowHeightMode::Exactly;
            row.height = 20.0;
        }
        for cell in &mut table.cells {
            cell.insets = [0.0; 4];
            cell.strokes = std::array::from_fn(|_| CellStroke::none());
            cell.strokes[edge] = CellStroke { weight: 0.5, ..Default::default() };
            cell.border_overrides[edge] = true;
        }
    }

    fn rule_table(edge: usize) -> Table {
        let mut table = Table::new(77, 2, 2, 0, 0, 100.0);
        set_rules(&mut table, edge);
        table
    }

    fn assert_segments(fragment: &TableFrag, expected: &[(f64, f64, f64, f64, f64)]) {
        assert_eq!(fragment.strokes.len(), expected.len(), "{:?}", fragment.strokes);
        for &(x0, y0, x1, y1, weight) in expected {
            assert_eq!(
                fragment.strokes.iter().filter(|s| s.a == Point::new(x0, y0) && s.b == Point::new(x1, y1) && s.stroke.weight == weight).count(),
                1,
                "expected exactly one segment ({x0}, {y0})–({x1}, {y1}) at {weight} pt: {:?}",
                fragment.strokes,
            );
        }
    }

    #[test]
    fn explicit_cell_edges_override_table_border_without_changing_native_defaults() {
        let mut table = Table::new(77, 1, 1, 0, 0, 100.0);
        table.options.border.weight = 2.0;
        table.cell_mut(0, 0).unwrap().strokes = std::array::from_fn(|_| CellStroke::none());
        let native = compose_table(table.clone());
        let strokes = &native.frames[0].tables[0].strokes;
        assert_eq!(strokes.len(), 4, "default cells still use all four table borders");
        assert!(strokes.iter().all(|s| s.stroke.weight == 2.0));

        let cell = table.cell_mut(0, 0).unwrap();
        cell.border_overrides = [true, true, true, false];
        cell.strokes[2] = CellStroke { weight: 3.0, ..Default::default() };
        let composed = compose_table(table);
        let fragment = &composed.frames[0].tables[0];
        assert_eq!(fragment.strokes.len(), 2, "top and left None overrides suppress borders");
        let bottom = fragment.strokes.iter().find(|s| s.a.y == fragment.rect.y1 && s.b.y == fragment.rect.y1).unwrap();
        assert_eq!(bottom.stroke.weight, 3.0);
        let right = fragment.strokes.iter().find(|s| s.a.x == fragment.rect.x1 && s.b.x == fragment.rect.x1).unwrap();
        assert_eq!(right.stroke.weight, 2.0, "unoverridden right edge keeps the table border");
    }

    #[test]
    fn merged_cell_keeps_explicit_border_suppression() {
        let mut table = Table::new(77, 2, 2, 0, 0, 100.0);
        table.merge(CellRange::new(0, 0, 1, 1)).unwrap();
        let cell = table.cell_mut(0, 0).unwrap();
        cell.border_overrides = [true; 4];
        cell.strokes = std::array::from_fn(|_| CellStroke::none());
        let composed = compose_table(table);
        let fragment = &composed.frames[0].tables[0];
        assert_eq!(fragment.cells.len(), 1);
        assert!(fragment.strokes.is_empty());
    }

    #[test]
    fn bottom_only_row_rules_draw_four_segments_once() {
        for direction in [designcraft_doc::TextDirection::LeftToRight, designcraft_doc::TextDirection::RightToLeft] {
            let mut table = rule_table(2);
            table.options.direction = direction;
            let composed = compose_table(table);
            assert_segments(
                &composed.frames[0].tables[0],
                &[(0.0, 20.0, 50.0, 20.0, 0.5), (50.0, 20.0, 100.0, 20.0, 0.5), (0.0, 40.0, 50.0, 40.0, 0.5), (50.0, 40.0, 100.0, 40.0, 0.5)],
            );
        }
    }

    #[test]
    fn right_only_column_rules_draw_four_segments_once() {
        for direction in [designcraft_doc::TextDirection::LeftToRight, designcraft_doc::TextDirection::RightToLeft] {
            let mut table = rule_table(3);
            table.options.direction = direction;
            let composed = compose_table(table);
            assert_segments(
                &composed.frames[0].tables[0],
                &[(50.0, 0.0, 50.0, 20.0, 0.5), (100.0, 0.0, 100.0, 20.0, 0.5), (50.0, 20.0, 50.0, 40.0, 0.5), (100.0, 20.0, 100.0, 40.0, 0.5)],
            );
        }
    }

    #[test]
    fn merged_edges_resolve_each_adjacent_owner_and_ignore_covered_cells() {
        for direction in [designcraft_doc::TextDirection::LeftToRight, designcraft_doc::TextDirection::RightToLeft] {
            let rtl = direction == designcraft_doc::TextDirection::RightToLeft;
            let mut horizontal = rule_table(2);
            horizontal.options.direction = direction;
            horizontal.cell_mut(0, 1).unwrap().strokes[2].weight = 1.0;
            horizontal.merge(CellRange::new(1, 0, 1, 1)).unwrap();
            horizontal.cell_mut(1, 1).unwrap().strokes[2].weight = 99.0;
            let composed = compose_table(horizontal.clone());
            assert_segments(
                &composed.frames[0].tables[0],
                &[
                    (0.0, 20.0, 50.0, 20.0, if rtl { 1.0 } else { 0.5 }),
                    (50.0, 20.0, 100.0, 20.0, if rtl { 0.5 } else { 1.0 }),
                    (0.0, 40.0, 100.0, 40.0, 0.5),
                ],
            );

            horizontal.cell_mut(0, 1).unwrap().strokes[2].weight = 0.5;
            let uniform = compose_table(horizontal);
            assert_segments(&uniform.frames[0].tables[0], &[(0.0, 20.0, 100.0, 20.0, 0.5), (0.0, 40.0, 100.0, 40.0, 0.5)]);

            let mut vertical = rule_table(3);
            vertical.options.direction = direction;
            let (merged, adjacent) = if rtl { (0, 1) } else { (1, 0) };
            vertical.cell_mut(1, adjacent).unwrap().strokes[3].weight = 1.0;
            vertical.merge(CellRange::new(0, merged, 1, merged)).unwrap();
            vertical.cell_mut(1, merged).unwrap().strokes[3].weight = 99.0;
            let composed = compose_table(vertical.clone());
            assert_segments(
                &composed.frames[0].tables[0],
                &[(50.0, 0.0, 50.0, 20.0, 0.5), (50.0, 20.0, 50.0, 40.0, 1.0), (100.0, 0.0, 100.0, 40.0, 0.5)],
            );
            vertical.cell_mut(1, adjacent).unwrap().strokes[3].weight = 0.5;
            let uniform = compose_table(vertical);
            assert_segments(&uniform.frames[0].tables[0], &[(50.0, 0.0, 50.0, 40.0, 0.5), (100.0, 0.0, 100.0, 40.0, 0.5)]);
        }
    }

    #[test]
    fn shared_seams_keep_visible_top_left_choices_and_table_perimeter() {
        let mut table = rule_table(2);
        table.options.border.weight = 7.0;
        for cell in &mut table.cells {
            cell.border_overrides = [false; 4];
            cell.strokes = [2.0, 4.0, 3.0, 5.0].map(|weight| CellStroke { weight, ..Default::default() });
        }
        let composed = compose_table(table);
        assert_segments(
            &composed.frames[0].tables[0],
            &[
                (0.0, 0.0, 50.0, 0.0, 7.0),
                (50.0, 0.0, 100.0, 0.0, 7.0),
                (0.0, 0.0, 0.0, 20.0, 7.0),
                (0.0, 20.0, 0.0, 40.0, 7.0),
                (100.0, 0.0, 100.0, 20.0, 7.0),
                (100.0, 20.0, 100.0, 40.0, 7.0),
                (0.0, 40.0, 50.0, 40.0, 7.0),
                (50.0, 40.0, 100.0, 40.0, 7.0),
                (0.0, 20.0, 50.0, 20.0, 2.0),
                (50.0, 20.0, 100.0, 20.0, 2.0),
                (50.0, 0.0, 50.0, 20.0, 4.0),
                (50.0, 20.0, 50.0, 40.0, 4.0),
            ],
        );
    }

    #[test]
    fn explicit_empty_preferred_edges_suppress_only_their_shared_span() {
        let mut horizontal = rule_table(2);
        horizontal.cell_mut(1, 1).unwrap().border_overrides[0] = true;
        let composed = compose_table(horizontal);
        assert_segments(&composed.frames[0].tables[0], &[(0.0, 20.0, 50.0, 20.0, 0.5), (0.0, 40.0, 50.0, 40.0, 0.5), (50.0, 40.0, 100.0, 40.0, 0.5)]);
        for direction in [designcraft_doc::TextDirection::LeftToRight, designcraft_doc::TextDirection::RightToLeft] {
            let mut vertical = rule_table(3);
            vertical.options.direction = direction;
            let preferred = if direction == designcraft_doc::TextDirection::RightToLeft { 0 } else { 1 };
            vertical.cell_mut(1, preferred).unwrap().border_overrides[1] = true;
            let composed = compose_table(vertical);
            assert_segments(
                &composed.frames[0].tables[0],
                &[(50.0, 0.0, 50.0, 20.0, 0.5), (100.0, 0.0, 100.0, 20.0, 0.5), (100.0, 20.0, 100.0, 40.0, 0.5)],
            );
        }
    }

    #[test]
    fn bottom_only_rules_follow_repeated_headers_and_footers() {
        for footer in [0, 1] {
            let mut table = Table::new(77, 3, 2, 1, footer, 100.0);
            set_rules(&mut table, 2);
            table.options.repeat_footer = footer > 0;
            for col in 0..2 {
                table.cell_mut(0, col).unwrap().strokes[2].weight = 0.75;
            }
            let composed = compose_table_frames(table, 59.0 + 20.0 * footer as f64, 3);
            assert!(!composed.is_overset());
            assert_eq!(composed.frames.len(), 3);
            for (i, frame) in composed.frames.iter().enumerate() {
                assert_eq!(frame.tables.len(), 1);
                let fragment = &frame.tables[0];
                assert_eq!(fragment.cell(0, 0).unwrap().repeated, i > 0);
                assert!(fragment.cell(i + 1, 0).is_some());
                let y = fragment.rect.y0;
                let mut expected = vec![
                    (0.0, y + 20.0, 50.0, y + 20.0, 0.75),
                    (50.0, y + 20.0, 100.0, y + 20.0, 0.75),
                    (0.0, y + 40.0, 50.0, y + 40.0, 0.5),
                    (50.0, y + 40.0, 100.0, y + 40.0, 0.5),
                ];
                if footer > 0 {
                    assert_eq!(fragment.cell(4, 0).unwrap().repeated, i < 2);
                    expected.extend([(0.0, y + 60.0, 50.0, y + 60.0, 0.5), (50.0, y + 60.0, 100.0, y + 60.0, 0.5)]);
                }
                assert_segments(fragment, &expected);
            }
        }
    }

    #[test]
    fn r5_explicit_adjacent_strokes_resolve_only_their_merged_overlap() {
        for horizontal in [false, true] {
            for rtl in [false, true] {
                for explicit_preferred in [false, true] {
                    for stroke in [
                        CellStroke::none(),
                        CellStroke { weight: 4.0, color: designcraft_color::swatch::NONE.into(), ..Default::default() },
                        CellStroke { weight: 4.0, tint: 0.6, kind: designcraft_doc::StrokeType::Dotted, ..Default::default() },
                    ] {
                        let mut table = Table::new(77, 2, 2, 0, 0, 100.0);
                        table.options.direction =
                            if rtl { designcraft_doc::TextDirection::RightToLeft } else { designcraft_doc::TextDirection::LeftToRight };
                        for row in &mut table.rows {
                            row.mode = RowHeightMode::Exactly;
                            row.height = 20.0;
                        }
                        let (source, preferred, covered, edge, opposite) = if horizontal {
                            table.merge(CellRange::new(1, 0, 1, 1)).unwrap();
                            ((0, 1), (1, 0), (1, 1), 2, 0)
                        } else {
                            let source_col = usize::from(rtl);
                            let preferred_col = 1 - source_col;
                            table.merge(CellRange::new(0, preferred_col, 1, preferred_col)).unwrap();
                            ((0, source_col), (0, preferred_col), (1, preferred_col), 3, 1)
                        };
                        let cell = table.cell_mut(source.0, source.1).unwrap();
                        cell.strokes[edge] = stroke.clone();
                        cell.border_overrides[edge] = true;
                        let preferred_stroke = CellStroke { weight: if explicit_preferred { 7.0 } else { 1.0 }, ..Default::default() };
                        let cell = table.cell_mut(preferred.0, preferred.1).unwrap();
                        cell.strokes[opposite] = preferred_stroke.clone();
                        cell.border_overrides[opposite] = explicit_preferred;
                        let placeholder = table.cell_mut(covered.0, covered.1).unwrap();
                        placeholder.strokes[opposite].weight = 99.0;
                        placeholder.border_overrides[opposite] = true;
                        let composed = compose_table(table);
                        let fragment = &composed.frames[0].tables[0];
                        let mut seam: Vec<_> = fragment
                            .strokes
                            .iter()
                            .filter_map(|segment| {
                                if horizontal {
                                    let y = fragment.rect.y0 + 20.0;
                                    (segment.a.y == y && segment.b.y == y).then_some((
                                        segment.a.x - fragment.rect.x0,
                                        segment.b.x - fragment.rect.x0,
                                        segment.stroke.clone(),
                                    ))
                                } else {
                                    let x = fragment.rect.x0 + 50.0;
                                    (segment.a.x == x && segment.b.x == x).then_some((
                                        segment.a.y - fragment.rect.y0,
                                        segment.b.y - fragment.rect.y0,
                                        segment.stroke.clone(),
                                    ))
                                }
                            })
                            .collect();
                        seam.sort_by(|a, b| a.0.total_cmp(&b.0));
                        let half: f64 = if horizontal { 50.0 } else { 20.0 };
                        let mut expected = if explicit_preferred {
                            vec![(0.0, 2.0 * half, preferred_stroke)]
                        } else {
                            let selected = if horizontal && !rtl { half } else { 0.0 };
                            let rest = half - selected;
                            let mut segments = vec![(rest, rest + half, CellStroke::default())];
                            if stroke.is_visible() {
                                segments.push((selected, selected + half, stroke));
                            }
                            segments
                        };
                        expected.sort_by(|a, b| a.0.total_cmp(&b.0));
                        assert_eq!(seam, expected, "explicit preferred={explicit_preferred}, horizontal={horizontal}, RTL={rtl}");
                    }
                }
            }
        }
    }

    #[test]
    fn r5_explicit_bottom_rules_beat_implicit_tops_at_repeated_row_boundaries() {
        for rtl in [false, true] {
            for footer in [0, 1] {
                let mut table = Table::new(77, 3, 2, 1, footer, 100.0);
                set_rules(&mut table, 2);
                table.options.direction = if rtl { designcraft_doc::TextDirection::RightToLeft } else { designcraft_doc::TextDirection::LeftToRight };
                table.options.repeat_footer = footer > 0;
                for cell in &mut table.cells {
                    cell.strokes[0] = CellStroke::default();
                }
                for col in 0..2 {
                    table.cell_mut(0, col).unwrap().strokes[2].weight = 0.75;
                }
                let composed = compose_table_frames(table, 59.0 + 20.0 * footer as f64, 3);
                assert!(!composed.is_overset());
                assert_eq!(composed.frames.len(), 3);
                for (index, frame) in composed.frames.iter().enumerate() {
                    let fragment = &frame.tables[0];
                    assert_eq!(fragment.cell(0, 0).unwrap().repeated, index > 0);
                    assert!(fragment.cell(index + 1, 0).is_some());
                    let y = fragment.rect.y0;
                    let mut expected = vec![
                        (0.0, y + 20.0, 50.0, y + 20.0, 0.75),
                        (50.0, y + 20.0, 100.0, y + 20.0, 0.75),
                        (0.0, y + 40.0, 50.0, y + 40.0, 0.5),
                        (50.0, y + 40.0, 100.0, y + 40.0, 0.5),
                    ];
                    if footer > 0 {
                        assert_eq!(fragment.cell(4, 0).unwrap().repeated, index < 2);
                        expected.extend([(0.0, y + 60.0, 50.0, y + 60.0, 0.5), (50.0, y + 60.0, 100.0, y + 60.0, 0.5)]);
                    }
                    assert_segments(fragment, &expected);
                }
            }
        }
    }
}

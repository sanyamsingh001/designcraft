//! Table menu: create, convert, insert/delete rows and columns, merge, cell/row/column options,
//! table options and cell selection.
//!
//! Commands target the table at the text selection (a caret in a cell or next to a table anchor),
//! the selected cells, or an explicit `{story, table}`; `rows: [a, b]` / `cols: [a, b]` (or `row` /
//! `col`) narrow the cell range (default: the selected cells, else the caret's cell, else the
//! whole table).

use designcraft_compose as compose;
use designcraft_doc::{
    AltFills, CellAddr, CellRange, CellStroke, Document, RowHeightMode, Selection, StoryId, StrokeType, Table, TableSel, TextSel,
    VerticalJustification,
};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, f64_or, has_doc, has_text, ok, str_param};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    let mut v = style_specs();
    v.extend(table_specs());
    v
}

fn style_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "style.cell.create",
            "New Cell Style…",
            [],
            None,
            "{name, fromSelection?: bool (the target cell's look), fill?, tint?, insets?: n|[t,l,b,r], vj?: top|center|bottom|justify, stroke?: {weight, color, tint?}, paragraphStyle?} → {name}",
            has_doc,
            |s, p| cell_style_create(s, p)
        ),
        cmd!(
            "style.cell.apply",
            "Apply Cell Style",
            [],
            None,
            "{name, story?, table?} — the target cells (the whole table when the cursor is in it)",
            in_table,
            cell_style_apply
        ),
        cmd!(
            "style.cell.edit",
            "Cell Style Options…",
            [],
            None,
            "{name, …same as style.cell.create} — cells using the style follow",
            has_doc,
            cell_style_edit
        ),
        cmd!(
            "style.table.create",
            "New Table Style…",
            [],
            None,
            "{name, header?, body?, footer?, leftColumn?, rightColumn?: cell style names, border?: {weight, color}, altRows?: {first, firstColor, next, nextColor}, spaceBefore?, spaceAfter?} → {name}",
            has_doc,
            table_style_create
        ),
        cmd!("style.table.apply", "Apply Table Style", [], None, "{name, story?, table?}", in_table, table_style_apply),
        cmd!(
            "style.table.edit",
            "Table Style Options…",
            [],
            None,
            "{name, …same as style.table.create} — tables using the style follow",
            has_doc,
            table_style_edit
        ),
    ]
}

fn table_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "table.insert",
            "Create Table",
            [],
            Some("Cmd+Alt+Shift+T"),
            "{rows?: body rows (4), cols? (4), headerRows? (0), footerRows? (0), width?} — at the text insertion point",
            has_text,
            insert
        ),
        cmd!(
            "table.convertFromText",
            "Convert Text to Table",
            ["Table"],
            None,
            "{columnSeparator?: tab|comma} — selected paragraphs become rows",
            has_text,
            convert_from_text
        ),
        cmd!(
            "table.convertToText",
            "Convert Table to Text",
            ["Table"],
            None,
            "{} — cells separated by tabs, rows by paragraphs",
            in_table,
            convert_to_text
        ),
        cmd!("table.insertRow", "Insert Row", [], None, "{where?: above|below, count? (1)}", in_table, |s, p| insert_row(s, p, None)),
        cmd!("table.insertRowAbove", "Insert Row Above", ["Table", "Insert"], None, "{count?}", in_table, |s, p| insert_row(s, p, Some(true))),
        cmd!("table.insertRowBelow", "Insert Row Below", ["Table", "Insert"], None, "{count?}", in_table, |s, p| insert_row(s, p, Some(false))),
        cmd!("table.insertColumn", "Insert Column", [], None, "{where?: left|right, count? (1), width?}", in_table, |s, p| insert_col(s, p, None)),
        cmd!("table.insertColumnLeft", "Insert Column Left", ["Table", "Insert"], None, "{count?}", in_table, |s, p| insert_col(s, p, Some(true))),
        cmd!("table.insertColumnRight", "Insert Column Right", ["Table", "Insert"], None, "{count?}", in_table, |s, p| insert_col(s, p, Some(false))),
        cmd!("table.deleteRow", "Delete Row", ["Table", "Delete"], None, "{} — the rows of the target cells", in_table, delete_rows),
        cmd!("table.deleteColumn", "Delete Column", ["Table", "Delete"], None, "{} — the columns of the target cells", in_table, delete_cols),
        cmd!("table.delete", "Delete Table", ["Table", "Delete"], None, "{}", in_table, delete_table),
        cmd!("table.merge", "Merge Cells", ["Table"], None, "{} — merge the target cell range", in_table, merge),
        cmd!(
            "table.dropCells",
            "Drag Rows or Columns",
            [],
            None,
            "{frame, from: [x, y], to: [x, y]} — selected whole rows (or columns) dragged from `from` move to the row (column) at `to`; otherwise a text drag",
            has_doc,
            drop_cells
        ),
        cmd!(
            "table.placeGraphic",
            "Convert Cell to Graphic Cell",
            ["Table", "Convert Cell Type"],
            None,
            "{path | base64+name, fit?: proportional|fill (default proportional)} — an image in the target cell (its text is kept but hidden)",
            in_table,
            place_graphic
        ),
        cmd!(
            "table.textCell",
            "Convert Cell to Text Cell",
            ["Table", "Convert Cell Type"],
            None,
            "{} — drop the target cells' graphics",
            in_table,
            |s, p| {
                let g = target(s, p, "table.textCell")?;
                edit_table(s, &g, "table.textCell", |t| {
                    for r in g.range.r0..=g.range.r1 {
                        for c in g.range.c0..=g.range.c1 {
                            if let Some(cell) = t.cell_mut(r, c) {
                                cell.graphic = None;
                            }
                        }
                    }
                    ok()
                })
            }
        ),
        cmd!(
            "table.moveRow",
            "Move Row",
            [],
            None,
            "{from?, to} (0-based; from defaults to the target row) — like dragging a row",
            in_table,
            |s, p| move_rc(s, p, true)
        ),
        cmd!("table.moveColumn", "Move Column", [], None, "{from?, to} (0-based; from defaults to the target column)", in_table, |s, p| move_rc(
            s, p, false
        )),
        cmd!(
            "table.sortRows",
            "Sort",
            ["Table"],
            None,
            "{column?: 0-based (default: the target cell's), descending?: bool} — body rows by that column's text (numbers numerically)",
            in_table,
            sort_rows
        ),
        cmd!("table.unmerge", "Unmerge Cells", ["Table"], None, "{}", in_table, unmerge),
        cmd!(
            "table.splitHorizontally",
            "Split Cell Horizontally",
            ["Table"],
            None,
            "{} — the target cell becomes two, one above the other",
            in_table,
            |s, p| split(s, p, true)
        ),
        cmd!(
            "table.splitVertically",
            "Split Cell Vertically",
            ["Table"],
            None,
            "{} — the target cell becomes two, side by side",
            in_table,
            |s, p| split(s, p, false)
        ),
        cmd!(
            "table.setCell",
            "Cell Options",
            [],
            None,
            "{fill?: swatch, tint?, insets?: n | [t,l,b,r], vj?: top|center|bottom|justify, text?, stroke?: {weight?, color?, tint?, type?: solid|dashed|dotted, edges?: all|outer|inner|top|left|bottom|right}}",
            in_table,
            set_cell
        ),
        cmd!("table.setRowHeight", "Row Height", [], None, "{height, mode?: atLeast|exactly}", in_table, set_row_height),
        cmd!("table.setColumnWidth", "Column Width", [], None, "{width}", in_table, set_col_width),
        cmd!("table.distributeColumns", "Distribute Columns Evenly", ["Table"], None, "{}", in_table, distribute_cols),
        cmd!(
            "table.options",
            "Table Options",
            [],
            None,
            "{direction?: leftToRight|rightToLeft, border?: {weight?, color?, tint?, type?}, spaceBefore?, spaceAfter?, headerRows?, footerRows?, repeatHeader?, repeatFooter?, altRows?: {first, firstColor, firstTint, next, nextColor, nextTint, skipFirst, skipLast} | null, altCols?: … | null}",
            in_table,
            options
        ),
        cmd!(noundo "table.select", "Select Cells", [], None, "{story?, table?, rows?: [a,b], cols?: [a,b], what?: cell|row|column|table}", in_table_or_ids, select),
        cmd!(noundo "table.selectTable", "Select Table", ["Table", "Select"], None, "{}", in_table, |s, p| {
            select(s, &super::with_param(p, "what", json!("table")))
        }),
        cmd!(noundo "table.selectRow", "Select Row", ["Table", "Select"], None, "{}", in_table, |s, p| {
            select(s, &super::with_param(p, "what", json!("row")))
        }),
        cmd!(noundo "table.selectColumn", "Select Column", ["Table", "Select"], None, "{}", in_table, |s, p| {
            select(s, &super::with_param(p, "what", json!("column")))
        }),
        cmd!(noundo "table.nextCell", "Next Cell", [], None, "{}", in_cell, |s, _| step_cell(s, true)),
        cmd!(noundo "table.prevCell", "Previous Cell", [], None, "{}", in_cell, |s, _| step_cell(s, false)),
        cmd!(query "table.get", "Get Table", [], None, "{story?, table?} → rows, columns, cells (text, spans, fill), options", in_table_or_ids, get),
    ]
}

// ---------- enablement and targeting ----------

fn in_table(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    let st = s.active().ok_or("no document open")?;
    if st.selection.cells.is_some() || st.selection.text.is_some_and(|t| t.cell.is_some()) {
        return Ok(());
    }
    if let Some(t) = st.selection.text
        && st.doc.story(t.story).is_some_and(|x| x.table_at(t.focus).is_some())
    {
        return Ok(());
    }
    Err("place the insertion point in a table".into())
}

fn in_table_or_ids(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)
}

fn in_cell(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| d.selection.text.is_some_and(|t| t.cell.is_some())) { Ok(()) } else { Err("not in a table cell".into()) }
}

/// The table a command acts on and its target cell range.
struct Tgt {
    story: StoryId,
    table: u64,
    range: CellRange,
}

fn find_table(doc: &Document, id: u64) -> Option<StoryId> {
    doc.stories.iter().find(|(_, s)| s.tables.contains_key(&id)).map(|(k, _)| *k)
}

fn pair(p: &Value, many: &str, one: &str) -> Option<(usize, usize)> {
    if let Some(a) = p.get(many).and_then(Value::as_array) {
        let a0 = a.first()?.as_u64()? as usize;
        let a1 = a.get(1).and_then(Value::as_u64).map_or(a0, |v| v as usize);
        return Some((a0.min(a1), a0.max(a1)));
    }
    p.get(one).and_then(Value::as_u64).map(|v| (v as usize, v as usize))
}

fn target(s: &Session, p: &Value, cmd: &str) -> Result<Tgt> {
    let st = s.doc()?;
    let doc = &st.doc;
    let explicit_table = p.get("table").and_then(Value::as_u64);
    let explicit_story = p.get("story").and_then(Value::as_u64).map(StoryId);
    let (story, table, range) = if let Some(tid) = explicit_table {
        let sid = explicit_story.or_else(|| find_table(doc, tid)).ok_or_else(|| bad(cmd, format!("no table {tid}")))?;
        (sid, tid, None)
    } else if let Some(c) = st.selection.cells {
        (c.story, c.table, Some(c.range))
    } else if let Some(t) = st.selection.text {
        match t.cell {
            Some(c) => (t.story, c.table, Some(CellRange::cell(c.row, c.col))),
            None => {
                let tid = doc.story(t.story).and_then(|x| x.table_at(t.focus)).ok_or_else(|| bad(cmd, "the insertion point is not in a table"))?;
                (t.story, tid, None)
            }
        }
    } else {
        return Err(bad(cmd, "place the insertion point in a table"));
    };
    let t = doc.story(story).and_then(|x| x.tables.get(&table)).ok_or_else(|| bad(cmd, format!("no table {table}")))?;
    let (nr, nc) = (t.nrows(), t.ncols());
    let mut range = range.unwrap_or(CellRange { r0: 0, c0: 0, r1: nr - 1, c1: nc - 1 });
    if let Some((a, b)) = pair(p, "rows", "row") {
        range.r0 = a;
        range.r1 = b;
    }
    if let Some((a, b)) = pair(p, "cols", "col") {
        range.c0 = a;
        range.c1 = b;
    }
    if range.r1 >= nr || range.c1 >= nc {
        return Err(bad(cmd, format!("cell range out of bounds ({nr}×{nc} table)")));
    }
    Ok(Tgt { story, table, range })
}

/// Edit the target table; afterwards selections pointing into it are clamped to its new shape.
fn edit_table<T>(s: &mut Session, g: &Tgt, cmd: &str, f: impl FnOnce(&mut Table) -> Result<T>) -> Result<T> {
    let cmd = cmd.to_string();
    s.edit(|d, sel| {
        let st = d.story_mut(g.story).ok_or_else(|| bad(&cmd, "no story"))?;
        let t = st.table_mut(g.table).ok_or_else(|| bad(&cmd, "no table"))?;
        let r = f(t)?;
        clamp_selection(d, sel);
        Ok(r)
    })
}

/// Keep text/cell selections valid after the table changed shape (or disappeared).
fn clamp_selection(d: &Document, sel: &mut Selection) {
    if let Some(ts) = sel.cells {
        match d.story(ts.story).and_then(|x| x.tables.get(&ts.table)) {
            Some(t) => {
                let (mr, mc) = (t.nrows() - 1, t.ncols() - 1);
                let r = ts.range;
                sel.cells = Some(TableSel { range: CellRange::new(r.r0.min(mr), r.c0.min(mc), r.r1.min(mr), r.c1.min(mc)), ..ts });
            }
            None => sel.cells = None,
        }
    }
    if let Some(t) = sel.text
        && let Some(c) = t.cell
    {
        let story = d.story(t.story);
        match story.and_then(|x| x.tables.get(&c.table)) {
            Some(tb) => {
                let (r, col) = tb.owner(c.row.min(tb.nrows() - 1), c.col.min(tb.ncols() - 1));
                let len = tb.cell(r, col).map_or(0, |x| x.text.len());
                sel.text =
                    Some(TextSel { anchor: t.anchor.min(len), focus: t.focus.min(len), cell: Some(CellAddr { table: c.table, row: r, col }), ..t });
            }
            None => {
                let len = story.map_or(0, |x| x.len());
                sel.text = Some(TextSel { anchor: t.anchor.min(len), focus: t.anchor.min(len), cell: None, ..t });
            }
        }
    }
}

fn count(p: &Value) -> usize {
    p.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 1000) as usize
}

fn color_param(p: &Value, key: &str) -> Option<String> {
    str_param(p, key).map(str::to_string)
}

// ---------- create / convert ----------

/// Width of the text column at the caret (the new table spans it).
fn column_width(s: &Session, t: &TextSel) -> f64 {
    let Some(st) = s.active() else { return 300.0 };
    let specs = compose::frame_specs(&st.doc, t.story);
    let spec = t.frame.and_then(|f| specs.iter().find(|x| x.id == f)).or(specs.first());
    spec.and_then(|f| f.columns().first().map(|c| c.width())).unwrap_or(300.0)
}

// Capture only an existing blank EOF, before deleting a selection can expose a new one.
fn insert_table_at_range(st: &mut designcraft_doc::Story, range: std::ops::Range<usize>, table: Table) -> usize {
    let eof = if range.start == st.len() && st.text.ends_with('\n') { st.paras.last().cloned() } else { None };
    let fmt = super::text::prepare_eof_marker(st, range.start, designcraft_doc::TABLE_ANCHOR);
    st.delete(range.clone());
    let anchor = st.insert_table(range.start, table);
    if let Some(fmt) = fmt {
        // The populated anchor paragraph keeps its baseline; its inserted bytes own the stage.
        let end = st.len();
        st.format_chars(anchor..end, |f| *f = fmt.clone());
    }
    if let (Some(eof), Some(pf)) = (eof, st.paras.last_mut()) {
        // The generated final LF leaves a true blank EOF with its exact original paragraph format.
        *pf = eof;
    }
    anchor
}

/// The direction of the paragraph at `offset` in `story`, as composition resolves it (its style
/// chain, then local overrides). A table created there takes it.
fn para_direction(d: &Document, story: StoryId, offset: usize) -> designcraft_doc::TextDirection {
    d.story(story)
        .and_then(|st| st.paras.get(st.para_at(offset)))
        .map_or(designcraft_doc::TextDirection::LeftToRight, |pf| d.styles.resolve_para(pf).0.direction)
}

fn insert(s: &mut Session, p: &Value) -> Result<Value> {
    let t = s.doc()?.selection.text.ok_or_else(|| bad("table.insert", "no insertion point"))?;
    if t.cell.is_some() {
        return Err(bad("table.insert", "tables can't be nested in cells"));
    }
    let rows = p.get("rows").and_then(Value::as_u64).unwrap_or(4).clamp(1, 500) as usize;
    let cols = p.get("cols").or_else(|| p.get("columns")).and_then(Value::as_u64).unwrap_or(4).clamp(1, 200) as usize;
    let header = p.get("headerRows").and_then(Value::as_u64).unwrap_or(0).min(100) as usize;
    let footer = p.get("footerRows").and_then(Value::as_u64).unwrap_or(0).min(100) as usize;
    let width = p.get("width").and_then(Value::as_f64).unwrap_or_else(|| column_width(s, &t));
    s.edit(|d, sel| {
        let id = d.alloc();
        let mut table = Table::new(id, rows, cols, header, footer, width);
        table.options.direction = para_direction(d, t.story, t.range().start);
        let st = d.story_mut(t.story).ok_or_else(|| bad("table.insert", "no story"))?;
        // Cells start with the paragraph format at the caret.
        let mut pf = st.paras[st.para_at(t.range().start)].clone();
        if t.range().start == st.len() && st.text.ends_with('\n') {
            // A blank outer EOF carries typing attrs, not new cell defaults.
            pf.chars = pf.eof_inherited_chars.take().unwrap_or_default();
        }
        let cf = st.char_format_at(t.range().start).clone();
        for c in &mut table.cells {
            c.text.paras[0] = designcraft_doc::ParaFormat { table: None, ..pf.clone() };
            c.text.chars[0].format = cf.clone();
        }
        insert_table_at_range(st, t.range(), table);
        *sel = Selection::text(TextSel { story: t.story, anchor: 0, focus: 0, frame: t.frame, cell: Some(CellAddr { table: id, row: 0, col: 0 }) });
        Ok(json!({"story": t.story.0, "table": id}))
    })
}

fn convert_from_text(s: &mut Session, p: &Value) -> Result<Value> {
    let t = s.doc()?.selection.text.ok_or_else(|| bad("table.convertFromText", "no text selected"))?;
    if t.cell.is_some() {
        return Err(bad("table.convertFromText", "select text outside tables"));
    }
    let sep = if str_param(p, "columnSeparator") == Some("comma") { ',' } else { '\t' };
    let width = p.get("width").and_then(Value::as_f64).unwrap_or_else(|| column_width(s, &t));
    s.edit(|d, sel| {
        let id = d.alloc();
        let direction = para_direction(d, t.story, t.range().start);
        let st = d.story_mut(t.story).ok_or_else(|| bad("table.convertFromText", "no story"))?;
        let ranges = st.para_ranges();
        let (pa, pb) = (st.para_at(t.range().start), st.para_at(t.range().end));
        if (pa..=pb).any(|i| st.paras[i].table.is_some()) {
            return Err(bad("table.convertFromText", "the selection contains a table"));
        }
        let (a, b) = (ranges[pa].start, ranges[pb].end);
        let data: Vec<Vec<String>> = st.text[a..b].split('\n').map(|line| line.split(sep).map(|c| c.trim().to_string()).collect()).collect();
        let mut pf = st.paras[pa].clone();
        if a == st.len() && st.text.ends_with('\n') {
            // Converting a blank outer EOF must use the same baseline as Create Table.
            pf.chars = pf.eof_inherited_chars.take().unwrap_or_default();
        }
        let cf = st.format_after(a).clone();
        let mut table = Table::from_strings(id, &data, width, &pf, &cf);
        table.options.direction = direction;
        let anchor = insert_table_at_range(st, a..b, table);
        *sel = Selection::text(TextSel::caret(t.story, anchor));
        Ok(json!({"table": id, "rows": data.len()}))
    })
}

fn convert_to_text(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.convertToText")?;
    s.edit(|d, sel| {
        let st = d.story_mut(g.story).ok_or_else(|| bad("table.convertToText", "no story"))?;
        let text = st.tables.get(&g.table).map(|t| t.plain_text()).unwrap_or_default();
        let a = st.table_anchor(g.table).ok_or_else(|| bad("table.convertToText", "no anchor"))?;
        super::text::replace_preserving_eof(st, a..a + designcraft_doc::TABLE_ANCHOR.len_utf8(), &text, true, false);
        *sel = Selection::text(TextSel::caret(g.story, a + text.len()));
        ok()
    })
}

fn delete_table(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.delete")?;
    s.edit(|d, sel| {
        let st = d.story_mut(g.story).ok_or_else(|| bad("table.delete", "no story"))?;
        let a = st.table_anchor(g.table).ok_or_else(|| bad("table.delete", "no anchor"))?;
        // Remove the anchor paragraph (with one separator) so no empty paragraph is left behind.
        let end = a + designcraft_doc::TABLE_ANCHOR.len_utf8();
        let r = if &st.text[end..] == "\n" && a > 0 && st.text[..a].ends_with('\n') {
            // Inserted at the end of the story: remove the empty paragraph after it too.
            a - 1..end + 1
        } else if st.text[end..].starts_with('\n') {
            a..end + 1
        } else if a > 0 && st.text[..a].ends_with('\n') {
            a - 1..end
        } else {
            a..end
        };
        super::text::delete_preserving_eof(st, r.clone(), true);
        *sel = Selection::text(TextSel::caret(g.story, r.start.min(st.len())));
        ok()
    })
}

// ---------- rows and columns ----------

fn insert_row(s: &mut Session, p: &Value, above: Option<bool>) -> Result<Value> {
    let g = target(s, p, "table.insertRow")?;
    let above = above.unwrap_or(str_param(p, "where") == Some("above"));
    let n = count(p);
    let at = if above { g.range.r0 } else { g.range.r1 + 1 };
    let r = edit_table(s, &g, "table.insertRow", |t| {
        t.insert_rows(at, n);
        Ok(json!({"rows": t.nrows()}))
    })?;
    shift_selection(s, g.table, at, n, true);
    Ok(r)
}

/// After inserting rows/columns at `at`, selections at or after it follow their cells.
fn shift_selection(s: &mut Session, table: u64, at: usize, n: usize, rows: bool) {
    let Ok(st) = s.doc_mut() else { return };
    if let Some(t) = st.selection.text.as_mut()
        && let Some(c) = t.cell.as_mut()
        && c.table == table
    {
        let v = if rows { &mut c.row } else { &mut c.col };
        if *v >= at {
            *v += n;
        }
    }
    if let Some(ts) = st.selection.cells.as_mut()
        && ts.table == table
    {
        let (a, b) = if rows { (&mut ts.range.r0, &mut ts.range.r1) } else { (&mut ts.range.c0, &mut ts.range.c1) };
        if *a >= at {
            *a += n;
        }
        if *b >= at {
            *b += n;
        }
    }
}

fn insert_col(s: &mut Session, p: &Value, left: Option<bool>) -> Result<Value> {
    let g = target(s, p, "table.insertColumn")?;
    let left = left.unwrap_or(str_param(p, "where") == Some("left"));
    let rtl = s
        .doc()?
        .doc
        .story(g.story)
        .and_then(|st| st.tables.get(&g.table))
        .is_some_and(|t| t.options.direction == designcraft_doc::TextDirection::RightToLeft);
    let n = count(p);
    let at = if left != rtl { g.range.c0 } else { g.range.c1 + 1 };
    let width = p.get("width").and_then(Value::as_f64);
    let r = edit_table(s, &g, "table.insertColumn", |t| {
        let w = width.unwrap_or_else(|| t.columns[g.range.c0.min(t.ncols() - 1)].width);
        t.insert_cols(at, n, w);
        Ok(json!({"columns": t.ncols()}))
    })?;
    shift_selection(s, g.table, at, n, false);
    Ok(r)
}

fn delete_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.deleteRow")?;
    let all = s.doc()?.doc.story(g.story).and_then(|x| x.tables.get(&g.table)).is_some_and(|t| g.range.r0 == 0 && g.range.r1 + 1 >= t.nrows());
    if all {
        return delete_table(s, p);
    }
    edit_table(s, &g, "table.deleteRow", |t| {
        t.delete_rows(g.range.r0, g.range.r1 - g.range.r0 + 1);
        Ok(json!({"rows": t.nrows()}))
    })
}

fn delete_cols(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.deleteColumn")?;
    let all = s.doc()?.doc.story(g.story).and_then(|x| x.tables.get(&g.table)).is_some_and(|t| g.range.c0 == 0 && g.range.c1 + 1 >= t.ncols());
    if all {
        return delete_table(s, p);
    }
    edit_table(s, &g, "table.deleteColumn", |t| {
        t.delete_cols(g.range.c0, g.range.c1 - g.range.c0 + 1);
        Ok(json!({"columns": t.ncols()}))
    })
}

fn merge(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.merge")?;
    let r = edit_table(s, &g, "table.merge", |t| {
        t.merge(g.range).map_err(|e| bad("table.merge", e))?;
        Ok(t.expand_range(g.range))
    })?;
    // The caret goes to the merged cell.
    let st = s.doc_mut()?;
    st.selection = Selection::text(TextSel {
        story: g.story,
        anchor: 0,
        focus: 0,
        frame: st.selection.text.and_then(|t| t.frame),
        cell: Some(CellAddr { table: g.table, row: r.r0, col: r.c0 }),
    });
    ok()
}

fn split(s: &mut Session, p: &Value, horizontal: bool) -> Result<Value> {
    let g = target(s, p, "table.split")?;
    edit_table(s, &g, "table.split", |t| {
        t.split_cell(g.range.r0, g.range.c0, horizontal);
        Ok(json!({"rows": t.nrows(), "cols": t.ncols()}))
    })
}

fn unmerge(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.unmerge")?;
    edit_table(s, &g, "table.unmerge", |t| {
        for r in g.range.r0..=g.range.r1 {
            for c in g.range.c0..=g.range.c1 {
                t.unmerge(r, c);
            }
        }
        ok()
    })
}

fn parse_stroke(v: &Value, mut base: CellStroke) -> CellStroke {
    if let Some(w) = v.get("weight").and_then(Value::as_f64) {
        base.weight = w.max(0.0);
    }
    if let Some(c) = color_param(v, "color") {
        base.color = c;
    }
    if let Some(t) = v.get("tint").and_then(Value::as_f64) {
        base.tint = t.clamp(0.0, 1.0) as f32;
    }
    match str_param(v, "type") {
        Some("dashed") => base.kind = StrokeType::Dashed { pattern: vec![base.weight.max(1.0) * 3.0, base.weight.max(1.0) * 2.0] },
        Some("dotted") => base.kind = StrokeType::Dotted,
        Some("solid") => base.kind = StrokeType::Solid,
        _ => {}
    }
    base
}

fn set_cell(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.setCell")?;
    let fill = color_param(p, "fill");
    let tint = p.get("tint").and_then(Value::as_f64).map(|t| t.clamp(0.0, 1.0) as f32);
    let insets: Option<[f64; 4]> = match p.get("insets") {
        Some(Value::Number(n)) => n.as_f64().map(|v| [v.max(0.0); 4]),
        Some(Value::Array(a)) if a.len() == 4 => {
            let v: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap_or(0.0).max(0.0)).collect();
            Some([v[0], v[1], v[2], v[3]])
        }
        _ => None,
    };
    let vj = match str_param(p, "vj").or_else(|| str_param(p, "verticalJustification")) {
        Some("top") => Some(VerticalJustification::Top),
        Some("center") => Some(VerticalJustification::Center),
        Some("bottom") => Some(VerticalJustification::Bottom),
        Some("justify") => Some(VerticalJustification::Justify),
        Some(o) => return Err(bad("table.setCell", format!("unknown vj `{o}`"))),
        None => None,
    };
    let text = str_param(p, "text").map(str::to_string);
    let stroke = p.get("stroke").cloned();
    edit_table(s, &g, "table.setCell", |t| {
        let owners = t.owners();
        let nc = t.ncols();
        let rg = g.range;
        let rtl = t.options.direction == designcraft_doc::TextDirection::RightToLeft;
        let mut edited_boundaries = Vec::new();
        for r in rg.r0..=rg.r1 {
            for c in rg.c0..=rg.c1 {
                if owners[r * nc + c] != (r, c) {
                    continue;
                }
                let Some(cell) = t.cell_mut(r, c) else { continue };
                if let Some(f) = &fill {
                    cell.fill = f.clone();
                }
                if let Some(ti) = tint {
                    cell.fill_tint = ti;
                }
                if let Some(i) = insets {
                    cell.insets = i;
                }
                if let Some(v) = vj {
                    cell.vj = v;
                }
                if let Some(tx) = &text {
                    let len = cell.text.len();
                    cell.text.replace(0..len, tx);
                }
                if let Some(sv) = &stroke {
                    let edges = str_param(sv, "edges").unwrap_or("all");
                    let last_col = c.saturating_add(cell.col_span.max(1) as usize).saturating_sub(1);
                    for (i, on) in [
                        ("top", r == rg.r0),
                        ("left", if rtl { last_col == rg.c1 } else { c == rg.c0 }),
                        ("bottom", r + cell.row_span as usize - 1 == rg.r1),
                        ("right", if rtl { c == rg.c0 } else { last_col == rg.c1 }),
                    ]
                    .iter()
                    .enumerate()
                    .map(|(i, (name, outer))| (i, edges == "all" || edges == *name || (edges == "outer" && *outer) || (edges == "inner" && !*outer)))
                    {
                        if on {
                            cell.strokes[i] = parse_stroke(sv, cell.strokes[i].clone());
                            cell.border_overrides[i] = true;
                            if i == 2 || i == 3 {
                                edited_boundaries.push((r, c, i));
                            }
                        }
                    }
                }
            }
        }
        // Visit every edited owner's real bottom/right boundary. A partial overlap
        // stays on its source owner; composition resolves it without widening the edit.
        if let Some(sv) = &stroke {
            let edges = str_param(sv, "edges").unwrap_or("all");
            let mirror = |t: &mut Table, from: (usize, usize), to: (usize, usize), edge: usize| {
                let owner = |(r, c): (usize, usize)| owners.get(r.checked_mul(nc)?.checked_add(c)?).copied();
                let (Some(from), Some(to)) = (owner(from), owner(to)) else { return };
                if from == to || !rg.contains(from.0, from.1) {
                    return;
                }
                let (Some(source), Some(neighbour)) = (t.cell(from.0, from.1), t.cell(to.0, to.1)) else { return };
                let row_end = |r: usize, span: u32| r.saturating_add(span.max(1) as usize).min(t.nrows());
                let col_end = |c: usize, span: u32| c.saturating_add(span.max(1) as usize).min(nc);
                let (opposite, shared, covered) = match edge {
                    2 => (
                        0,
                        row_end(from.0, source.row_span) == to.0,
                        from.1 <= to.1 && col_end(to.1, neighbour.col_span) <= col_end(from.1, source.col_span),
                    ),
                    3 => (
                        1,
                        if rtl { col_end(to.1, neighbour.col_span) == from.1 } else { col_end(from.1, source.col_span) == to.1 },
                        from.0 <= to.0 && row_end(to.0, neighbour.row_span) <= row_end(from.0, source.row_span),
                    ),
                    _ => return,
                };
                if !shared || !covered {
                    return;
                }
                // A footer owner is reused after other body rows in earlier fragments.
                // Keep a body-only edit on its source so each actual adjacency resolves it.
                if edge == 2
                    && t.options.repeat_footer
                    && t.rows.get(from.0).is_some_and(|row| row.kind == designcraft_doc::RowKind::Body)
                    && t.rows.get(to.0).is_some_and(|row| row.kind == designcraft_doc::RowKind::Footer)
                {
                    return;
                }
                // Preserve a top/left edge independently edited by this same command,
                // including its colour/tint/type when the command changes only weight.
                let (name, outer) = if opposite == 0 {
                    ("top", to.0 == rg.r0)
                } else {
                    ("left", if rtl { col_end(to.1, neighbour.col_span).saturating_sub(1) == rg.c1 } else { to.1 == rg.c0 })
                };
                if rg.contains(to.0, to.1) && (edges == "all" || edges == name || (edges == "outer" && outer) || (edges == "inner" && !outer)) {
                    return;
                }
                let Some(value) = source.strokes.get(edge).cloned() else { return };
                if let Some(cell) = t.cell_mut(to.0, to.1)
                    && let Some(stroke) = cell.strokes.get_mut(opposite)
                    && let Some(explicit) = cell.border_overrides.get_mut(opposite)
                {
                    *stroke = value;
                    *explicit = true;
                }
            };
            for (r, c, edge) in edited_boundaries {
                let Some(cell) = t.cell(r, c) else { continue };
                let row_end = r.saturating_add(cell.row_span.max(1) as usize).min(t.nrows());
                let col_end = c.saturating_add(cell.col_span.max(1) as usize).min(nc);
                match edge {
                    2 if row_end < t.nrows() => {
                        for column in c..col_end {
                            mirror(t, (r, c), (row_end, column), edge);
                        }
                    }
                    3 => {
                        // Logical columns run leftwards in RTL.
                        let neighbour = if rtl { c.checked_sub(1) } else { (col_end < nc).then_some(col_end) };
                        if let Some(column) = neighbour {
                            for row in r..row_end {
                                mirror(t, (r, c), (row, column), edge);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        ok()
    })
}

fn set_row_height(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.setRowHeight")?;
    let h = p.get("height").and_then(Value::as_f64).map(|h| h.clamp(0.0, 10000.0));
    let mode = match str_param(p, "mode") {
        Some("exactly") => Some(RowHeightMode::Exactly),
        Some("atLeast") => Some(RowHeightMode::AtLeast),
        Some(o) => return Err(bad("table.setRowHeight", format!("unknown mode `{o}`"))),
        None => None,
    };
    if h.is_none() && mode.is_none() {
        return Err(bad("table.setRowHeight", "missing height"));
    }
    edit_table(s, &g, "table.setRowHeight", |t| {
        for row in &mut t.rows[g.range.r0..=g.range.r1] {
            if let Some(h) = h {
                row.height = h;
            }
            if let Some(m) = mode {
                row.mode = m;
            }
        }
        ok()
    })
}

fn set_col_width(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.setColumnWidth")?;
    let w = p.get("width").and_then(Value::as_f64).ok_or_else(|| bad("table.setColumnWidth", "missing width"))?.clamp(3.0, 10000.0);
    edit_table(s, &g, "table.setColumnWidth", |t| {
        for c in &mut t.columns[g.range.c0..=g.range.c1] {
            c.width = w;
        }
        ok()
    })
}

fn distribute_cols(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.distributeColumns")?;
    edit_table(s, &g, "table.distributeColumns", |t| {
        let cols = &mut t.columns[g.range.c0..=g.range.c1];
        let avg = cols.iter().map(|c| c.width).sum::<f64>() / cols.len() as f64;
        cols.iter_mut().for_each(|c| c.width = avg);
        ok()
    })
}

fn parse_alt(v: &Value) -> Option<AltFills> {
    if v.is_null() {
        return None;
    }
    let d = AltFills::default();
    Some(AltFills {
        first: v.get("first").and_then(Value::as_u64).map_or(d.first, |x| x as u32),
        first_color: color_param(v, "firstColor").unwrap_or(d.first_color),
        first_tint: v.get("firstTint").and_then(Value::as_f64).map_or(d.first_tint, |x| x as f32),
        next: v.get("next").and_then(Value::as_u64).map_or(d.next, |x| x as u32),
        next_color: color_param(v, "nextColor").unwrap_or(d.next_color),
        next_tint: v.get("nextTint").and_then(Value::as_f64).map_or(d.next_tint, |x| x as f32),
        skip_first: v.get("skipFirst").and_then(Value::as_u64).map_or(0, |x| x as u32),
        skip_last: v.get("skipLast").and_then(Value::as_u64).map_or(0, |x| x as u32),
    })
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.options")?;
    let p = p.clone();
    edit_table(s, &g, "table.options", |t| {
        let o = &mut t.options;
        if let Some(v) = p.get("direction") {
            o.direction = serde_json::from_value(v.clone()).map_err(|e| bad("table.options", e.to_string()))?;
        }
        if let Some(b) = p.get("border") {
            o.border = parse_stroke(b, o.border.clone());
        }
        o.space_before = f64_or(&p, "spaceBefore", o.space_before);
        o.space_after = f64_or(&p, "spaceAfter", o.space_after);
        if let Some(b) = p.get("repeatHeader").and_then(Value::as_bool) {
            o.repeat_header = b;
        }
        if let Some(b) = p.get("repeatFooter").and_then(Value::as_bool) {
            o.repeat_footer = b;
        }
        if let Some(v) = p.get("altRows") {
            o.alt_rows = parse_alt(v);
        }
        if let Some(v) = p.get("altCols") {
            o.alt_cols = parse_alt(v);
        }
        let h = p.get("headerRows").and_then(Value::as_u64).map(|v| v as usize);
        let f = p.get("footerRows").and_then(Value::as_u64).map(|v| v as usize);
        if h.is_some() || f.is_some() {
            let (h, f) = (h.unwrap_or(t.header_rows()), f.unwrap_or(t.footer_rows()));
            // Add rows when there aren't enough for the requested header/footer plus one body row.
            let need = h + f + 1;
            if t.nrows() < need {
                let at = t.nrows();
                t.insert_rows(at, need - at);
            }
            t.set_header_footer(h, f);
        }
        Ok(json!({"options": t.options}))
    })
}

// ---------- selection ----------

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.select")?;
    let (nr, nc) = {
        let st = s.doc()?;
        let t = st.doc.story(g.story).and_then(|x| x.tables.get(&g.table)).ok_or_else(|| bad("table.select", "no table"))?;
        (t.nrows(), t.ncols())
    };
    let mut r = g.range;
    match str_param(p, "what") {
        Some("table") => r = CellRange { r0: 0, c0: 0, r1: nr - 1, c1: nc - 1 },
        Some("row") => {
            r.c0 = 0;
            r.c1 = nc - 1;
        }
        Some("column") => {
            r.r0 = 0;
            r.r1 = nr - 1;
        }
        _ => {}
    }
    let st = s.doc_mut()?;
    let frame = st.selection.text.and_then(|t| t.frame);
    st.selection = Selection {
        cells: Some(TableSel { story: g.story, table: g.table, range: r }),
        text: Some(TextSel { story: g.story, anchor: 0, focus: 0, frame, cell: Some(CellAddr { table: g.table, row: r.r0, col: r.c0 }) }),
        ..Default::default()
    };
    st.revision += 1;
    Ok(json!({"story": g.story.0, "table": g.table, "range": r}))
}

/// Move the caret to the next/previous cell (Tab / Shift-Tab), selecting its text. Tab in the last
/// cell adds a row.
pub(crate) fn step_cell(s: &mut Session, forward: bool) -> Result<Value> {
    let t = s.doc()?.selection.text.ok_or_else(|| bad("table.nextCell", "no caret"))?;
    let c = t.cell.ok_or_else(|| bad("table.nextCell", "not in a cell"))?;
    let (owners, nr, nc) = {
        let st = s.doc()?;
        let tb = st.doc.story(t.story).and_then(|x| x.tables.get(&c.table)).ok_or_else(|| bad("table.nextCell", "no table"))?;
        (tb.owners(), tb.nrows(), tb.ncols())
    };
    let cur = c.row * nc + c.col;
    let next =
        if forward { (cur + 1..nr * nc).find(|&i| owners[i] == (i / nc, i % nc)) } else { (0..cur).rev().find(|&i| owners[i] == (i / nc, i % nc)) };
    let (row, col) = match next {
        Some(i) => (i / nc, i % nc),
        None if forward => {
            let g = Tgt { story: t.story, table: c.table, range: CellRange::cell(nr - 1, 0) };
            edit_table(s, &g, "table.nextCell", |tb| {
                tb.insert_rows(nr, 1);
                ok()
            })?;
            (nr, 0)
        }
        None => return ok(),
    };
    let st = s.doc_mut()?;
    let len = st.doc.text_story(t.story, Some(CellAddr { table: c.table, row, col })).map_or(0, |x| x.len());
    st.selection = Selection::text(TextSel { anchor: 0, focus: len, cell: Some(CellAddr { table: c.table, row, col }), ..t });
    st.revision += 1;
    Ok(json!({"row": row, "col": col}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.get")?;
    let st = s.doc()?;
    let t = st.doc.story(g.story).and_then(|x| x.tables.get(&g.table)).ok_or_else(|| bad("table.get", "no table"))?;
    let owners = t.owners();
    let cells: Vec<Value> = (0..t.nrows())
        .flat_map(|r| (0..t.ncols()).map(move |c| (r, c)))
        .filter(|&(r, c)| owners[r * t.ncols() + c] == (r, c))
        .filter_map(|(r, c)| {
            let cell = t.cell(r, c)?;
            Some(json!({"row": r, "col": c, "rowSpan": cell.row_span, "colSpan": cell.col_span, "text": cell.text.text, "fill": cell.fill,
                "insets": cell.insets, "vj": cell.vj, "style": cell.style}))
        })
        .collect();
    Ok(json!({
        "story": g.story.0, "table": g.table, "anchor": st.doc.story(g.story).and_then(|x| x.table_anchor(g.table)),
        "rows": t.rows, "columns": t.columns, "headerRows": t.header_rows(), "footerRows": t.footer_rows(),
        "cells": cells, "options": t.options, "range": g.range, "style": t.style,
    }))
}

// ---------- cell and table styles ----------

fn stroke_param(v: Option<&Value>) -> Option<designcraft_doc::CellStroke> {
    let o = v?.as_object()?;
    let mut st = designcraft_doc::CellStroke::default();
    if let Some(w) = o.get("weight").and_then(Value::as_f64) {
        st.weight = w.max(0.0);
    }
    if let Some(c) = o.get("color").and_then(Value::as_str) {
        st.color = c.to_string();
    }
    if let Some(t) = o.get("tint").and_then(Value::as_f64) {
        st.tint = t.clamp(0.0, 1.0) as f32;
    }
    Some(st)
}

/// Fill in the cell style fields given in `p`.
fn cell_style_fields(cs: &mut designcraft_doc::CellStyle, p: &Value, cmd: &str) -> Result<()> {
    if let Some(f) = color_param(p, "fill") {
        cs.fill = Some(f);
    }
    if let Some(t) = p.get("tint").and_then(Value::as_f64) {
        cs.fill_tint = Some(t.clamp(0.0, 1.0) as f32);
    }
    match p.get("insets") {
        Some(Value::Number(n)) => cs.insets = n.as_f64().map(|v| [v.max(0.0); 4]),
        Some(Value::Array(a)) if a.len() == 4 => {
            let v: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap_or(0.0).max(0.0)).collect();
            cs.insets = Some([v[0], v[1], v[2], v[3]]);
        }
        _ => {}
    }
    if let Some(v) = p.get("vj") {
        cs.vj = Some(serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("vj: {e}")))?);
    }
    if let Some(st) = stroke_param(p.get("stroke")) {
        cs.stroke = Some(st);
    }
    if let Some(ps) = str_param(p, "paragraphStyle") {
        cs.paragraph_style = Some(ps.to_string());
    }
    Ok(())
}

fn cell_style_create(s: &mut Session, p: &Value) -> Result<Value> {
    let base = str_param(p, "name").unwrap_or("Cell Style 1").to_string();
    let mut cs = designcraft_doc::CellStyle::default();
    if p.get("fromSelection").and_then(Value::as_bool).unwrap_or(false) {
        let g = target(s, p, "style.cell.create")?;
        let d = &s.doc()?.doc;
        let cell = d
            .story(g.story)
            .and_then(|st| st.tables.get(&g.table))
            .and_then(|t| t.cell(g.range.r0, g.range.c0))
            .ok_or_else(|| bad("style.cell.create", "no cell"))?;
        cs.fill = Some(cell.fill.clone());
        cs.fill_tint = Some(cell.fill_tint);
        cs.insets = Some(cell.insets);
        cs.vj = Some(cell.vj);
        cs.strokes = Some(cell.strokes.clone().map(Some));
        cs.paragraph_style = cell.text.paras.first().map(|p| p.style.clone());
    }
    cell_style_fields(&mut cs, p, "style.cell.create")?;
    s.edit(|d, _| {
        let name = designcraft_doc::Styles::unique_name(|n| d.styles.cell.iter().any(|c| c.name == n), &base);
        cs.name = name.clone();
        d.styles_mut().cell.push(cs);
        Ok(json!({"name": name}))
    })
}

fn cell_style_apply(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("style.cell.apply", "missing name"))?.to_string();
    let cs = s
        .doc()?
        .doc
        .styles
        .cell
        .iter()
        .find(|c| c.name == name)
        .cloned()
        .ok_or_else(|| bad("style.cell.apply", format!("no cell style `{name}`")))?;
    let g = target(s, p, "style.cell.apply")?;
    edit_table(s, &g, "style.cell.apply", |t| {
        let owners = t.owners();
        let nc = t.ncols();
        let mut n = 0;
        for r in g.range.r0..=g.range.r1 {
            for c in g.range.c0..=g.range.c1 {
                if owners[r * nc + c] == (r, c)
                    && let Some(cell) = t.cell_mut(r, c)
                {
                    cs.apply_to(cell);
                    n += 1;
                }
            }
        }
        Ok(json!({"cells": n}))
    })
}

/// Re-apply cell styles named in `names` to every cell using them (after a style edit).
fn reapply_cell_styles(d: &mut Document, names: &[String]) {
    let styles = d.styles.cell.clone();
    for sid in d.stories.keys().copied().collect::<Vec<_>>() {
        let Some(st) = d.story_mut(sid) else { continue };
        for t in st.tables.values_mut() {
            if !t.cells.iter().any(|c| names.contains(&c.style)) {
                continue;
            }
            for cell in &mut std::sync::Arc::make_mut(t).cells {
                if names.contains(&cell.style)
                    && let Some(cs) = styles.iter().find(|x| x.name == cell.style)
                {
                    cs.apply_to(cell);
                }
            }
        }
        st.rev = st.rev.wrapping_add(1);
    }
}

fn cell_style_edit(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("style.cell.edit", "missing name"))?.to_string();
    let p = p.clone();
    s.edit(|d, _| {
        let cs = d.styles_mut().cell.iter_mut().find(|c| c.name == name).ok_or_else(|| bad("style.cell.edit", format!("no cell style `{name}`")))?;
        cell_style_fields(cs, &p, "style.cell.edit")?;
        reapply_cell_styles(d, std::slice::from_ref(&name));
        ok()
    })
}

fn table_style_fields(ts: &mut designcraft_doc::TableStyle, p: &Value) {
    for (k, f) in [
        ("header", &mut ts.header),
        ("body", &mut ts.body),
        ("footer", &mut ts.footer),
        ("leftColumn", &mut ts.left_column),
        ("rightColumn", &mut ts.right_column),
    ] {
        if let Some(v) = p.get(k) {
            *f = v.as_str().map(str::to_string);
        }
    }
    if let Some(b) = stroke_param(p.get("border")) {
        ts.border = Some(b);
    }
    if let Some(a) = p.get("altRows").and_then(Value::as_object) {
        let n = |k: &str, d: u32| a.get(k).and_then(Value::as_u64).map_or(d, |v| v as u32);
        let c = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or(designcraft_color::swatch::NONE).to_string();
        let t = |k: &str| a.get(k).and_then(Value::as_f64).map_or(1.0, |v| v.clamp(0.0, 1.0) as f32);
        ts.alt_rows = Some(designcraft_doc::AltFills {
            first: n("first", 1),
            first_color: c("firstColor"),
            first_tint: t("firstTint"),
            next: n("next", 1),
            next_color: c("nextColor"),
            next_tint: t("nextTint"),
            skip_first: n("skipFirst", 0),
            skip_last: n("skipLast", 0),
        });
    }
    if let Some(v) = p.get("spaceBefore").and_then(Value::as_f64) {
        ts.space_before = Some(v);
    }
    if let Some(v) = p.get("spaceAfter").and_then(Value::as_f64) {
        ts.space_after = Some(v);
    }
}

fn table_style_create(s: &mut Session, p: &Value) -> Result<Value> {
    let base = str_param(p, "name").unwrap_or("Table Style 1").to_string();
    let mut ts = designcraft_doc::TableStyle::default();
    table_style_fields(&mut ts, p);
    s.edit(|d, _| {
        let name = designcraft_doc::Styles::unique_name(|n| d.styles.table.iter().any(|c| c.name == n), &base);
        ts.name = name.clone();
        d.styles_mut().table.push(ts);
        Ok(json!({"name": name}))
    })
}

fn table_style_apply(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("style.table.apply", "missing name"))?.to_string();
    let doc = &s.doc()?.doc;
    let ts = doc.styles.table.iter().find(|c| c.name == name).cloned().ok_or_else(|| bad("style.table.apply", format!("no table style `{name}`")))?;
    let cells = doc.styles.cell.clone();
    let g = target(s, p, "style.table.apply")?;
    edit_table(s, &g, "style.table.apply", |t| {
        ts.apply_to(t, &cells);
        ok()
    })
}

fn table_style_edit(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("style.table.edit", "missing name"))?.to_string();
    let p = p.clone();
    s.edit(|d, _| {
        let ts =
            d.styles_mut().table.iter_mut().find(|c| c.name == name).ok_or_else(|| bad("style.table.edit", format!("no table style `{name}`")))?;
        table_style_fields(ts, &p);
        let (ts, cells) = (ts.clone(), d.styles.cell.clone());
        for sid in d.stories.keys().copied().collect::<Vec<_>>() {
            let Some(st) = d.story_mut(sid) else { continue };
            for t in st.tables.values_mut() {
                if t.style == name {
                    ts.apply_to(std::sync::Arc::make_mut(t), &cells);
                }
            }
            st.rev = st.rev.wrapping_add(1);
        }
        ok()
    })
}

#[cfg(test)]
mod style_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn clearing_native_top_and_left_seams_survives_undo_redo() {
        use designcraft_doc::StoryId;
        for (rows, cols, row, col, edge) in [(2, 1, 1, 0, "top"), (1, 2, 0, 1, "left")] {
            let mut s = Session::new();
            s.execute("file.new", &json!({})).unwrap();
            let frame = s.execute("frame.create", &json!({"rect": [72, 72, 400, 400], "content": "text", "text": ""})).unwrap();
            let sid = StoryId(frame["story"].as_u64().unwrap());
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
            s.execute("table.insert", &json!({"rows": rows, "cols": cols})).unwrap();
            let segments = |s: &Session| {
                let d = &s.doc().unwrap().doc;
                let composed = designcraft_compose::compose_story(d, sid, &Default::default());
                composed.frames[0].tables[0].strokes.len()
            };
            let before = segments(&s);
            s.execute("table.setCell", &json!({"rows": [row, row], "cols": [col, col], "stroke": {"edges": edge, "weight": 0}})).unwrap();
            assert_eq!(segments(&s), before - 1, "explicit removal must suppress the visible adjacent stroke");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(segments(&s), before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(segments(&s), before - 1);
        }
    }

    #[test]
    fn cell_and_table_styles_apply_and_follow_edits() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 500, 400], "content": "text", "text": ""})).unwrap();
        let sid = r["story"].as_u64().unwrap();
        s.execute("text.select", &json!({"story": sid, "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 3, "cols": 2, "headerRows": 1})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Cell Head"})).unwrap();
        s.execute("style.cell.create", &json!({"name": "Head", "fill": "Black", "paragraphStyle": "Cell Head", "insets": 6})).unwrap();
        s.execute("style.cell.create", &json!({"name": "Body", "fill": "[Paper]"})).unwrap();
        s.execute("style.table.create", &json!({"name": "Data", "header": "Head", "body": "Body", "border": {"weight": 2, "color": "Black"}}))
            .unwrap();
        s.execute("style.table.apply", &json!({"name": "Data"})).unwrap();
        let t = |s: &Session| -> designcraft_doc::Table {
            let d = &s.doc().unwrap().doc;
            (**d.story(designcraft_doc::StoryId(sid)).unwrap().tables.values().next().unwrap()).clone()
        };
        let tb = t(&s);
        assert_eq!(tb.style, "Data");
        assert_eq!(tb.options.border.weight, 2.0);
        let head = tb.cell(0, 0).unwrap();
        assert_eq!((head.fill.as_str(), head.style.as_str(), head.insets), ("Black", "Head", [6.0; 4]));
        assert_eq!(head.text.paras[0].style, "Cell Head");
        assert_eq!(tb.cell(1, 1).unwrap().style, "Body");
        // Editing the cell style updates its cells.
        s.execute("style.cell.edit", &json!({"name": "Head", "fill": "C=100 M=0 Y=0 K=0"})).unwrap();
        assert_eq!(t(&s).cell(0, 1).unwrap().fill, "C=100 M=0 Y=0 K=0");
        // Editing the table style re-applies it.
        s.execute("style.table.edit", &json!({"name": "Data", "border": {"weight": 4}})).unwrap();
        assert_eq!(t(&s).options.border.weight, 4.0);
        assert!(s.execute("style.cell.apply", &json!({"name": "Nope"})).is_err());
    }
}

/// The table cell under spread point `pt` in text frame `frame`: (story, table, row, col).
fn cell_at(s: &Session, frame: designcraft_doc::ItemId, pt: designcraft_geom::Point) -> Option<(StoryId, u64, usize, usize)> {
    let st = s.active()?;
    let loc = st.doc.find(frame)?;
    let it = st.doc.item_at(&loc)?;
    let sid = it.text_frame()?.story;
    let inner = (st.doc.parent_xf(&loc) * st.doc.text_xf(it)).inverse() * pt;
    let cs = s.cache.get(&st.doc, sid, None);
    let fi = cs.frames.iter().position(|f| f.frame == frame)?;
    let (table, row, col, _) = designcraft_compose::hit_cell(&cs, fi, inner)?;
    Some((sid, table, row, col))
}

fn drop_cells(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "table.dropCells";
    let frame = super::id_param(p, "frame").ok_or_else(|| bad(ID, "`frame` required"))?;
    let pt = |k: &str| -> Result<designcraft_geom::Point> {
        let a = p.get(k).and_then(Value::as_array).ok_or_else(|| bad(ID, format!("`{k}` required")))?;
        Ok(designcraft_geom::Point::new(a.first().and_then(Value::as_f64).unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)))
    };
    let (from, to) = (pt("from")?, pt("to")?);
    let sel = s.doc()?.selection.cells;
    let hit_from = cell_at(s, frame, from);
    let hit_to = cell_at(s, frame, to);
    if let (Some(ts), Some((sid, tid, fr, fc)), Some((_, tid2, tr, tc))) = (sel, hit_from, hit_to)
        && ts.story == sid
        && ts.table == tid
        && tid2 == tid
        && ts.range.contains(fr, fc)
    {
        let (nr, nc) = {
            let t = s.doc()?.doc.story(sid).and_then(|st| st.tables.get(&tid)).ok_or_else(|| bad(ID, "no table"))?;
            (t.nrows(), t.ncols())
        };
        let r = ts.range;
        let rows = r.c0 == 0 && r.c1 + 1 == nc;
        let cols = r.r0 == 0 && r.r1 + 1 == nr;
        if rows || cols {
            let (lo, hi, target) = if rows { (r.r0, r.r1, tr) } else { (r.c0, r.c1, tc) };
            if target >= lo && target <= hi {
                return Ok(json!({"moved": 0}));
            }
            let n = hi - lo + 1;
            let r2 = s.edit(|d, selm| {
                let t = d.story_mut(sid).and_then(|st| st.table_mut(tid)).ok_or_else(|| bad(ID, "no table"))?;
                let mv = |t: &mut designcraft_doc::Table, a: usize, b: usize| if rows { t.move_row(a, b) } else { t.move_col(a, b) };
                let new_lo = if target > hi {
                    for _ in 0..n {
                        mv(t, lo, target).map_err(|e| bad(ID, e))?;
                    }
                    target + 1 - n
                } else {
                    for k in 0..n {
                        mv(t, lo + k, target + k).map_err(|e| bad(ID, e))?;
                    }
                    target
                };
                let range =
                    if rows { CellRange::new(new_lo, r.c0, new_lo + n - 1, r.c1) } else { CellRange::new(r.r0, new_lo, r.r1, new_lo + n - 1) };
                selm.cells = Some(TableSel { range, ..ts });
                Ok(json!({"moved": n, "to": new_lo}))
            })?;
            return Ok(r2);
        }
    }
    // Not a row/column drag: select text from the press to the release.
    s.execute("text.placeCaret", &json!({"frame": frame.0, "point": [from.x, from.y]}))?;
    s.execute("text.extendTo", &json!({"frame": frame.0, "point": [to.x, to.y]}))
}

fn place_graphic(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "table.placeGraphic";
    let g = target(s, p, ID)?;
    let (bytes, name, link) = super::file::read_source(p)?;
    let (pw, ph) = designcraft_render::image_size(&bytes).ok_or_else(|| bad(ID, "unsupported or corrupt image"))?;
    let fill = p.get("fit").and_then(Value::as_str) == Some("fill");
    s.edit(|d, sel| {
        let aid = designcraft_doc::AssetId(d.alloc());
        let mime = designcraft_render::image_mime(&bytes).to_string();
        d.assets.insert(
            aid,
            std::sync::Arc::new(designcraft_doc::Asset {
                page: 0,
                id: aid,
                name,
                mime,
                link,
                data: std::sync::Arc::new(bytes),
                pixels: Some((pw, ph)),
            }),
        );
        let st = d.story_mut(g.story).ok_or_else(|| bad(ID, "no story"))?;
        let t = st.table_mut(g.table).ok_or_else(|| bad(ID, "no table"))?;
        let (r, c) = t.owner(g.range.r0, g.range.c0);
        let cell = t.cell(r, c).ok_or_else(|| bad(ID, "no cell"))?;
        let (rs, cs) = (cell.row_span.max(1) as usize, cell.col_span.max(1) as usize);
        let w: f64 = t.columns[c..(c + cs).min(t.ncols())].iter().map(|x| x.width).sum();
        let h: f64 = t.rows[r..(r + rs).min(t.nrows())].iter().map(|x| x.height).sum();
        let ins = cell.insets;
        let (bw, bh) = ((w - ins[1] - ins[3]).max(1.0), (h - ins[0] - ins[2]).max(1.0));
        let (nw, nh) = (pw as f64, ph as f64);
        let k = if fill { (bw / nw).max(bh / nh) } else { (bw / nw).min(bh / nh) };
        let xf = designcraft_geom::Affine::translate(((bw - nw * k) / 2.0, (bh - nh * k) / 2.0)) * designcraft_geom::Affine::scale(k);
        let cell = t.cell_mut(r, c).ok_or_else(|| bad(ID, format!("no cell {r},{c}")))?;
        cell.graphic = Some(designcraft_doc::Graphic {
            asset: aid,
            size: (nw, nh),
            xf,
            auto_fit: if fill { designcraft_doc::Fitting::FillProportionally } else { designcraft_doc::Fitting::FitProportionally },
            fit_align: 4,
            crop: [0.0; 4],
        });
        clamp_selection(d, sel);
        Ok(json!({"asset": aid.0, "row": r, "col": c}))
    })
}

fn move_rc(s: &mut Session, p: &Value, rows: bool) -> Result<Value> {
    const ID: &str = "table.move";
    let g = target(s, p, ID)?;
    let from = p.get("from").and_then(Value::as_u64).map_or(if rows { g.range.r0 } else { g.range.c0 }, |v| v as usize);
    let to = p.get("to").and_then(Value::as_u64).ok_or_else(|| bad(ID, "`to` required"))? as usize;
    edit_table(s, &g, ID, |t| {
        if rows { t.move_row(from, to) } else { t.move_col(from, to) }.map_err(|e| bad(ID, e))?;
        Ok(json!({"from": from, "to": to}))
    })
}

fn sort_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let g = target(s, p, "table.sortRows")?;
    let col = p.get("column").and_then(Value::as_u64).map_or(g.range.c0, |c| c as usize);
    let desc = p.get("descending").and_then(Value::as_bool).unwrap_or(false);
    edit_table(s, &g, "table.sortRows", |t| {
        let (nr, nc) = (t.nrows(), t.ncols());
        if col >= nc {
            return Err(bad("table.sortRows", format!("no column {col}")));
        }
        if t.cells.iter().any(|c| c.row_span > 1) {
            return Err(bad("table.sortRows", "unmerge cells that span rows first"));
        }
        let (h, f) = (t.header_rows(), t.footer_rows());
        let body: Vec<usize> = (h..nr - f).collect();
        let key = |r: usize| t.cell(r, col).map(|c| c.text.text.trim().to_string()).unwrap_or_default();
        let mut order = body.clone();
        order.sort_by(|a, b| {
            let (ka, kb) = (key(*a), key(*b));
            let o = match (ka.replace(',', "").parse::<f64>(), kb.replace(',', "").parse::<f64>()) {
                (Ok(x), Ok(y)) => x.total_cmp(&y),
                _ => ka.to_lowercase().cmp(&kb.to_lowercase()),
            };
            if desc { o.reverse() } else { o }
        });
        let rows = t.rows.clone();
        let cells = t.cells.clone();
        for (k, src) in order.iter().enumerate() {
            let dst = body[k];
            t.rows[dst] = rows[*src].clone();
            for c in 0..nc {
                t.cells[dst * nc + c] = cells[src * nc + c].clone();
            }
        }
        Ok(json!({"sorted": body.len()}))
    })
}

#[cfg(test)]
mod sort_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn sort_body_rows_by_a_column() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 500, 400], "content": "text", "text": ""})).unwrap();
        s.execute("text.select", &json!({"story": r["story"], "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 3, "cols": 2, "headerRows": 1})).unwrap();
        let cells = [("Name", "Qty"), ("pear", "10"), ("Apple", "9"), ("fig", "100")];
        for (r, (a, b)) in cells.iter().enumerate() {
            s.execute("table.setCell", &json!({"rows": [r, r], "cols": [0, 0], "text": a})).unwrap();
            s.execute("table.setCell", &json!({"rows": [r, r], "cols": [1, 1], "text": b})).unwrap();
        }
        let col = |s: &Session, c: usize| -> Vec<String> {
            let d = &s.doc().unwrap().doc;
            let t = d.stories.values().flat_map(|st| st.tables.values()).next().unwrap();
            (0..t.nrows()).map(|r| t.cell(r, c).unwrap().text.text.clone()).collect()
        };
        s.execute("table.sortRows", &json!({"column": 0})).unwrap();
        assert_eq!(col(&s, 0), ["Name", "Apple", "fig", "pear"], "header stays; case-insensitive");
        s.execute("table.sortRows", &json!({"column": 1, "descending": true})).unwrap();
        assert_eq!(col(&s, 1), ["Qty", "100", "10", "9"], "numbers by value");
        s.execute("table.splitVertically", &json!({"rows": [1, 1], "cols": [0, 0]})).unwrap();
        let r = s.execute("table.splitHorizontally", &json!({"rows": [0, 0], "cols": [1, 1]})).unwrap();
        assert_eq!((r["rows"].as_u64(), r["cols"].as_u64()), (Some(5), Some(3)));
    }

    #[test]
    fn graphic_cells_draw_their_image() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 500, 400], "content": "text", "text": ""})).unwrap();
        s.execute("text.select", &json!({"story": r["story"], "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 2, "cols": 2})).unwrap();
        s.execute("table.rowHeight", &json!({"height": 100})).ok();
        // A red image into the first cell.
        let px: Vec<u8> = (0..40 * 40).flat_map(|_| [255, 0, 0, 255]).collect();
        let png = designcraft_render::Rendered { width: 40, height: 40, pixels: px }.to_png();
        s.execute("table.setCell", &json!({"rows": [0, 0], "cols": [0, 0], "text": ""})).ok();
        let g = s.execute("table.placeGraphic", &json!({"base64": super::super::file::base64_encode(&png), "name": "r.png", "fit": "fill"})).unwrap();
        assert_eq!((g["row"].as_u64(), g["col"].as_u64()), (Some(0), Some(0)));
        let d = s.doc().unwrap().doc.clone();
        let st = d.stories.values().find(|st| !st.tables.is_empty()).unwrap();
        let t = st.tables.values().next().unwrap();
        let cs = s.cache.get(&d, st.id, None);
        let cell = cs.frames[0].tables[0].cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap().clone();
        let fx = d.item(cs.frames[0].frame).unwrap().xf;
        let centre = fx * cell.clip.center();
        let mut rr = designcraft_render::Renderer::new();
        rr.threads = 0;
        let img = rr.render_page(&d, &s.cache, 0, 1.0, false, &Default::default()).unwrap();
        let p = img.pixel(centre.x as u32, centre.y as u32);
        assert!(p[0] > 200 && p[1] < 60, "red in the graphic cell: {p:?}");
        assert!(t.cell(0, 0).unwrap().graphic.is_some());
        s.execute("table.textCell", &json!({})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        assert!(d.stories.values().flat_map(|st| st.tables.values()).all(|t| t.cell(0, 0).unwrap().graphic.is_none()));
    }

    #[test]
    fn move_rows_and_columns() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 500, 400], "content": "text", "text": ""})).unwrap();
        s.execute("text.select", &json!({"story": r["story"], "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 3, "cols": 2})).unwrap();
        for (r, c, t) in [(0, 0, "a"), (1, 0, "b"), (2, 0, "c"), (0, 1, "x")] {
            s.execute("table.setCell", &json!({"rows": [r, r], "cols": [c, c], "text": t})).unwrap();
        }
        let cell = |s: &Session, r: usize, c: usize| {
            let d = &s.doc().unwrap().doc;
            let t = d.stories.values().flat_map(|st| st.tables.values()).next().unwrap();
            t.cell(r, c).unwrap().text.text.clone()
        };
        // Dragging a selected row onto the last row moves it there.
        let d = s.doc().unwrap().doc.clone();
        let (sid, tid) = d.stories.values().find_map(|st| st.tables.keys().next().map(|k| (st.id, *k))).unwrap();
        let fid = d.stories[&sid].frames[0];
        let cs = s.cache.get(&d, sid, None);
        let cells = &cs.frames[0].tables[0].cells;
        let centre = |r: usize| {
            let c = cells.iter().find(|c| c.row == r && c.col == 0).unwrap().rect.center();
            d.text_xf(d.item(fid).unwrap()) * c
        };
        let (from, to) = (centre(0), centre(2));
        s.execute("table.select", &json!({"story": sid.0, "table": tid, "rows": [0, 0], "what": "row"})).unwrap();
        let r2 = s.execute("table.dropCells", &json!({"frame": fid.0, "from": [from.x, from.y], "to": [to.x, to.y]})).unwrap();
        assert_eq!(r2["moved"], 1);
        assert_eq!([cell(&s, 0, 0), cell(&s, 1, 0), cell(&s, 2, 0)], ["b", "c", "a"]);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!([cell(&s, 0, 0), cell(&s, 1, 0), cell(&s, 2, 0)], ["a", "b", "c"]);
        s.execute("table.moveRow", &json!({"from": 0, "to": 2})).unwrap();
        assert_eq!([cell(&s, 0, 0), cell(&s, 1, 0), cell(&s, 2, 0)], ["b", "c", "a"]);
        s.execute("table.moveColumn", &json!({"from": 1, "to": 0})).unwrap();
        assert_eq!((cell(&s, 2, 0), cell(&s, 2, 1)), ("x".to_string(), "a".to_string()));
    }
}

#[cfg(test)]
mod arabic_tests {
    use super::*;
    #[test]
    fn arabic_table_direction_and_physical_left_insertion_support_undo() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [0, 0, 400, 300], "content": "text", "text": ""})).unwrap();
        s.execute("text.select", &json!({"story": r["story"], "anchor": 0, "focus": 0})).unwrap();
        s.execute("table.insert", &json!({"rows": 1, "cols": 2})).unwrap();
        s.execute("table.setCell", &json!({"rows": [0, 0], "cols": [0, 0], "text": "Right"})).unwrap();
        s.execute("table.options", &json!({"direction": "rightToLeft"})).unwrap();
        s.execute("table.select", &json!({"rows": [0, 0], "cols": [0, 0]})).unwrap();
        s.execute("table.insertColumnLeft", &json!({})).unwrap();
        let t = s.doc().unwrap().doc.stories.values().flat_map(|st| st.tables.values()).next().unwrap();
        assert_eq!(t.ncols(), 3);
        assert_eq!(t.cell(0, 0).unwrap().text.text, "Right");
        assert!(t.cell(0, 1).unwrap().text.text.is_empty());
        s.execute("edit.undo", &json!({})).unwrap();
        let t = s.doc().unwrap().doc.stories.values().flat_map(|st| st.tables.values()).next().unwrap();
        assert_eq!(t.ncols(), 2);
        assert_eq!(t.options.direction, designcraft_doc::TextDirection::RightToLeft);
    }

    /// A text frame whose paragraphs have a paragraph style with `direction`, with the story's
    /// text selected. Returns the session and the story id.
    fn styled_frame(direction: &str, text: &str) -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"preset": "A4", "pages": 1})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Dir", "para": {"direction": direction}})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [40, 330, 555, 420], "content": "text", "text": text, "caret": true})).unwrap();
        s.execute("edit.selectAll", &json!({})).unwrap();
        s.execute("style.paragraph.apply", &json!({"name": "Dir"})).unwrap();
        s.execute("edit.selectAll", &json!({})).unwrap();
        (s, r["story"].as_u64().unwrap())
    }

    fn only_table(s: &Session) -> std::sync::Arc<designcraft_doc::Table> {
        s.doc().unwrap().doc.stories.values().flat_map(|st| st.tables.values()).next().unwrap().clone()
    }

    /// Text converted to a table in a right-to-left paragraph laid its columns out left to right
    /// (#100): the first column belongs on the right.
    #[test]
    fn tables_take_the_direction_of_their_paragraph() {
        use designcraft_doc::TextDirection::{LeftToRight, RightToLeft};
        let (mut s, sid) = styled_frame("rightToLeft", "١\t٢\t٣\nأ\tب\tج");
        s.execute("table.convertFromText", &json!({"columnSeparator": "tab"})).unwrap();
        assert_eq!(only_table(&s).options.direction, RightToLeft);
        let st = s.doc().unwrap();
        let cs = s.cache.get(&st.doc, designcraft_doc::StoryId(sid), None);
        let placed = &cs.frames[0].tables[0];
        let (first, last) = (placed.cell(0, 0).unwrap().rect, placed.cell(0, 2).unwrap().rect);
        assert!(first.x0 > last.x0, "cell ١ is the rightmost: {first:?} {last:?}");

        let (mut s, _) = styled_frame("leftToRight", "1\t2\t3\na\tb\tc");
        s.execute("table.convertFromText", &json!({"columnSeparator": "tab"})).unwrap();
        assert_eq!(only_table(&s).options.direction, LeftToRight);

        for (direction, expected) in [("rightToLeft", RightToLeft), ("leftToRight", LeftToRight)] {
            let (mut s, _) = styled_frame(direction, "x");
            s.execute("table.insert", &json!({"rows": 1, "cols": 2})).unwrap();
            assert_eq!(only_table(&s).options.direction, expected, "table.insert in a {direction} paragraph");
        }
    }
}

#[cfg(test)]
mod asymmetric_style_snapshot_tests {
    use super::*;

    mod fixture {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../idml/src/tests/table_styles_fixture.rs"));
    }

    fn imported_selection() -> (Session, StoryId, u64) {
        let doc = designcraft_idml::import_idml(&fixture::fixture(true, true)).unwrap();
        let (sid, tid) = doc.stories.iter().find_map(|(sid, story)| story.tables.keys().next().map(|tid| (*sid, *tid))).unwrap();
        let mut session = Session::new();
        session.execute("file.new", &json!({})).unwrap();
        session.doc_mut().unwrap().doc = std::sync::Arc::new(doc);
        session.execute("table.select", &json!({"story": sid.0, "table": tid, "rows": [1, 1], "cols": [1, 2]})).unwrap();
        (session, sid, tid)
    }

    fn native_roundtrip(doc: &Document) {
        assert_eq!(designcraft_format::load(&designcraft_format::save(doc).unwrap()).unwrap(), *doc);
    }

    #[test]
    fn from_selection_preserves_asymmetric_edges_range_and_roundtrips_with_undo() {
        let (mut session, sid, tid) = imported_selection();
        let before = session.doc().unwrap().doc.clone();
        let selection = session.doc().unwrap().selection.clone();
        let table = &before.story(sid).unwrap().tables[&tid];
        let source = table.cell(1, 1).unwrap();
        assert_eq!(source.strokes.clone().map(|stroke| stroke.weight), [0.0, 0.0, 2.0, 0.0]);
        assert_eq!(source.strokes[2].kind, StrokeType::Dotted);
        assert_eq!(source.strokes[2].tint, 0.6);
        let result = session.execute("style.cell.create", &json!({"name": "Copied body", "fromSelection": true})).unwrap();
        let name = result["name"].as_str().unwrap();
        let created = session.doc().unwrap().doc.clone();
        let style = created.styles.cell.iter().find(|s| s.name == name).unwrap();
        assert!(style.stroke.is_none());
        assert_eq!(style.strokes, Some(source.strokes.clone().map(Some)));
        assert_eq!(created.story(sid).unwrap().tables[&tid], *table, "creation alone does not reformat cells");
        assert_eq!(session.doc().unwrap().selection, selection);

        session.execute("style.cell.apply", &json!({"name": name})).unwrap();
        let applied = session.doc().unwrap().doc.clone();
        let after = &applied.story(sid).unwrap().tables[&tid];
        for row in 0..3 {
            for col in 0..3 {
                let cell = after.cell(row, col).unwrap();
                if row == 1 && col >= 1 {
                    assert_eq!(cell.strokes, source.strokes, "all four edge weights/colors/tints/types are copied");
                    assert_eq!(cell.fill, source.fill);
                    assert_eq!(cell.fill_tint, source.fill_tint);
                    assert_eq!(cell.insets, source.insets);
                    assert_eq!(cell.vj, source.vj);
                    assert_eq!(cell.style, name);
                    assert_eq!(cell.text.paras[0].style, "Local");
                    assert_eq!(cell.text.text, table.cell(row, col).unwrap().text.text);
                    assert_eq!(cell.border_overrides, table.cell(row, col).unwrap().border_overrides);
                } else {
                    assert_eq!(cell, table.cell(row, col).unwrap(), "unselected cells stay unchanged");
                }
            }
        }
        assert_eq!(session.doc().unwrap().selection, selection);
        native_roundtrip(&applied);
        let back = designcraft_idml::import_idml(&designcraft_idml::export_idml(&applied)).unwrap();
        let back_table = back.stories.values().flat_map(|s| s.tables.values()).next().unwrap();
        let back_style = back.styles.cell.iter().find(|s| s.name == name).unwrap();
        assert_eq!(back_style.strokes, style.strokes);
        for col in 1..3 {
            let cell = back_table.cell(1, col).unwrap();
            assert_eq!(cell.strokes, source.strokes);
            assert_eq!(cell.fill, source.fill);
            assert_eq!(cell.fill_tint, source.fill_tint);
            assert_eq!(cell.insets, source.insets);
            assert_eq!(cell.vj, source.vj);
            assert_eq!(cell.text.paras[0].style, "Local");
            assert_eq!(cell.border_overrides, after.cell(1, col).unwrap().border_overrides);
        }
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *created);
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *before);
        session.execute("edit.redo", &json!({})).unwrap();
        session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *applied);
        assert_eq!(session.doc().unwrap().selection, selection);
    }

    #[test]
    fn explicit_uniform_stroke_overrides_the_from_selection_edge_snapshot() {
        let (mut session, sid, tid) = imported_selection();
        let selection = session.doc().unwrap().selection.clone();
        session
            .execute(
                "style.cell.create",
                &json!({
                    "name": "Uniform copy", "fromSelection": true,
                    "stroke": {"weight": 3, "color": "[Black]", "tint": 0.7}
                }),
            )
            .unwrap();
        let created = session.doc().unwrap().doc.clone();
        let style = created.styles.cell.iter().find(|s| s.name == "Uniform copy").unwrap();
        assert!(style.strokes.is_some(), "the original imported snapshot is retained");
        let uniform = style.stroke.as_ref().unwrap();
        assert_eq!(uniform.weight, 3.0);
        session.execute("style.cell.apply", &json!({"name": "Uniform copy"})).unwrap();
        let applied = session.doc().unwrap().doc.clone();
        for col in 1..3 {
            assert!(applied.story(sid).unwrap().tables[&tid].cell(1, col).unwrap().strokes.iter().all(|stroke| stroke == uniform));
        }
        assert_eq!(session.doc().unwrap().selection, selection);
        native_roundtrip(&applied);
        let back = designcraft_idml::import_idml(&designcraft_idml::export_idml(&applied)).unwrap();
        let table = back.stories.values().flat_map(|s| s.tables.values()).next().unwrap();
        for col in 1..3 {
            assert!(table.cell(1, col).unwrap().strokes.iter().all(|stroke| stroke == uniform));
        }
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *created);
        session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *applied);
    }

    #[test]
    fn effective_same_as_body_regions_and_overrides_survive_native_zip() {
        for same in [
            [false; 4],
            [true, false, false, false],
            [false, true, false, false],
            [false, false, true, false],
            [false, false, false, true],
            [true; 4],
        ] {
            for (explicit, local) in [(false, false), (true, false), (false, true), (true, true)] {
                let doc = designcraft_idml::import_idml(&fixture::fixture_with_regions(explicit, local, same)).unwrap();
                let style = doc.styles.table.iter().find(|s| s.name == "Catalogue").unwrap();
                for (i, (binding, separate)) in
                    [(&style.header, "Header"), (&style.footer, "Footer"), (&style.left_column, "Pastille"), (&style.right_column, "Right")]
                        .into_iter()
                        .enumerate()
                {
                    assert_eq!(binding.as_deref(), Some(if same[i] { "Body row" } else { separate }));
                }
                native_roundtrip(&doc);
            }
        }
    }
}

#[cfg(test)]
mod boundary_mirror_tests {
    use super::*;
    use designcraft_compose::TableFrag;
    use designcraft_geom::Point;

    fn native_table(rows: usize, cols: usize, rtl: bool) -> Session {
        let mut session = Session::new();
        session.execute("file.new", &json!({})).unwrap();
        let frame = session.execute("frame.create", &json!({"rect": [72, 72, 400, 400], "content": "text", "text": ""})).unwrap();
        session.execute("text.select", &json!({"story": frame["story"], "anchor": 0, "focus": 0})).unwrap();
        session.execute("table.insert", &json!({"rows": rows, "cols": cols, "width": 100})).unwrap();
        session.execute("table.setRowHeight", &json!({"rows": [0, rows - 1], "height": 20, "mode": "exactly"})).unwrap();
        session.execute("table.options", &json!({"direction": if rtl { "rightToLeft" } else { "leftToRight" }})).unwrap();
        session
    }

    fn table(doc: &Document) -> &Table {
        doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
    }

    fn fragment(doc: &Document) -> TableFrag {
        let sid = doc.stories.iter().find_map(|(sid, story)| (!story.tables.is_empty()).then_some(*sid)).unwrap();
        let composed = compose::compose_story(doc, sid, &Default::default());
        assert!(!composed.is_overset());
        composed.frames.iter().flat_map(|frame| &frame.tables).next().unwrap().clone()
    }

    fn assert_vertical_edge(doc: &Document, x: f64, weight: f64) {
        let fragment = fragment(doc);
        let a = Point::new(fragment.rect.x0 + x, fragment.rect.y0);
        let b = Point::new(a.x, fragment.rect.y1);
        let segments: Vec<_> = fragment.strokes.iter().filter(|segment| segment.a == a && segment.b == b).collect();
        if weight == 0.0 {
            assert!(segments.is_empty(), "edge at x={x} must be absent: {segments:?}");
        } else {
            assert_eq!(segments.len(), 1, "edge at x={x} must appear exactly once");
            assert_eq!(segments[0].stroke.weight, weight);
        }
    }

    fn check_undo_and_roundtrips(session: &mut Session, before: &Document, check: impl Fn(&Document)) {
        let after = session.doc().unwrap().doc.clone();
        check(&after);
        let native = designcraft_format::load(&designcraft_format::save(&after).unwrap()).unwrap();
        assert_eq!(native, *after);
        check(&native);
        let idml = designcraft_idml::import_idml(&designcraft_idml::export_idml(&after)).unwrap();
        idml.check().unwrap();
        check(&idml);
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *before);
        assert_eq!(fragment(session.doc().unwrap().doc.as_ref()).strokes, fragment(before).strokes);
        session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *after);
        check(session.doc().unwrap().doc.as_ref());
    }

    #[test]
    fn right_edge_edits_keep_the_unselected_perimeter_and_roundtrip() {
        for (rtl, col, x) in [(true, 0, 100.0), (true, 1, 50.0), (false, 0, 50.0), (false, 1, 100.0)] {
            for weight in [0.0, 4.0] {
                let mut session = native_table(1, 2, rtl);
                let before = session.doc().unwrap().doc.clone();
                session
                    .execute("table.setCell", &json!({"rows": [0, 0], "cols": [col, col], "stroke": {"edges": "right", "weight": weight}}))
                    .unwrap();
                check_undo_and_roundtrips(&mut session, &before, |doc| {
                    for edge_x in [0.0, 50.0, 100.0] {
                        assert_vertical_edge(doc, edge_x, if edge_x == x { weight } else { 1.0 });
                    }
                    let cell = table(doc).cell(0, col).unwrap();
                    assert_eq!(cell.strokes[3].weight, weight);
                    assert!(cell.border_overrides[3]);
                    if x == 100.0 {
                        assert_eq!(table(doc).cell(0, 1 - col).unwrap().strokes, table(&before).cell(0, 1 - col).unwrap().strokes);
                        assert_eq!(table(doc).cell(0, 1 - col).unwrap().border_overrides, [false; 4]);
                    }
                });
            }
        }
    }

    #[test]
    fn rtl_outer_edges_follow_the_physical_range_perimeter() {
        for weight in [0.0, 4.0] {
            let mut session = native_table(1, 3, true);
            session.execute("table.setColumnWidth", &json!({"cols": [0, 2], "width": 50})).unwrap();
            let before = session.doc().unwrap().doc.clone();
            session.execute("table.setCell", &json!({"rows": [0, 0], "cols": [0, 1], "stroke": {"edges": "outer", "weight": weight}})).unwrap();
            check_undo_and_roundtrips(&mut session, &before, |doc| {
                for (x, expected) in [(0.0, 1.0), (50.0, weight), (100.0, 1.0), (150.0, weight)] {
                    assert_vertical_edge(doc, x, expected);
                }
                assert_eq!(table(doc).cell(0, 2).unwrap().strokes, table(&before).cell(0, 2).unwrap().strokes);
                assert_eq!(table(doc).cell(0, 2).unwrap().border_overrides, [false; 4]);
            });
        }
    }

    #[test]
    fn merged_bottom_and_right_edits_use_owners_for_the_whole_shared_edge() {
        for rtl in [false, true] {
            for merged_neighbour in [false, true] {
                for edge in ["bottom", "right"] {
                    for weight in [0.0, 4.0] {
                        let mut session = native_table(2, 2, rtl);
                        let source_col = if rtl { 1 } else { 0 };
                        let neighbour_col = 1 - source_col;
                        let source_range = if edge == "bottom" {
                            json!({"rows": [0, 0], "cols": [0, 1]})
                        } else {
                            json!({"rows": [0, 1], "cols": [source_col, source_col]})
                        };
                        let neighbour_range = if edge == "bottom" {
                            json!({"rows": [1, 1], "cols": [0, 1]})
                        } else {
                            json!({"rows": [0, 1], "cols": [neighbour_col, neighbour_col]})
                        };
                        if merged_neighbour {
                            session.execute("table.merge", &neighbour_range).unwrap();
                        }
                        session.execute("table.merge", &source_range).unwrap();
                        let before = session.doc().unwrap().doc.clone();
                        let mut params = source_range;
                        params["stroke"] = json!({"edges": edge, "weight": weight});
                        session.execute("table.setCell", &params).unwrap();
                        let after_table = table(session.doc().unwrap().doc.as_ref());
                        let covered_source = if edge == "bottom" { (0, 1) } else { (1, source_col) };
                        assert_eq!(
                            after_table.cell(covered_source.0, covered_source.1),
                            table(&before).cell(covered_source.0, covered_source.1),
                            "a covered source is not a stroke owner"
                        );
                        if merged_neighbour {
                            let covered_neighbour = if edge == "bottom" { (1, 1) } else { (1, neighbour_col) };
                            assert_eq!(
                                after_table.cell(covered_neighbour.0, covered_neighbour.1),
                                table(&before).cell(covered_neighbour.0, covered_neighbour.1),
                                "a covered destination must not be formatted"
                            );
                        }
                        check_undo_and_roundtrips(&mut session, &before, |doc| {
                            let fragment = fragment(doc);
                            let mut segments: Vec<_> = fragment
                                .strokes
                                .iter()
                                .filter_map(|segment| {
                                    if edge == "bottom" {
                                        let y = fragment.rect.y0 + 20.0;
                                        (segment.a.y == y && segment.b.y == y).then_some((
                                            segment.a.x - fragment.rect.x0,
                                            segment.b.x - fragment.rect.x0,
                                            segment.stroke.weight,
                                        ))
                                    } else {
                                        let x = fragment.rect.x0 + 50.0;
                                        (segment.a.x == x && segment.b.x == x).then_some((
                                            segment.a.y - fragment.rect.y0,
                                            segment.b.y - fragment.rect.y0,
                                            segment.stroke.weight,
                                        ))
                                    }
                                })
                                .collect();
                            segments.sort_by(|a, b| a.0.total_cmp(&b.0));
                            let half = if edge == "bottom" { 50.0 } else { 20.0 };
                            let expected = if weight == 0.0 {
                                Vec::new()
                            } else if merged_neighbour {
                                vec![(0.0, 2.0 * half, weight)]
                            } else {
                                vec![(0.0, half, weight), (half, 2.0 * half, weight)]
                            };
                            assert_eq!(segments, expected, "the requested stroke must cover the whole merged {edge}, in either direction");
                            for index in 0..2 {
                                let owner = if edge == "bottom" { table(doc).owner(1, index) } else { table(doc).owner(index, neighbour_col) };
                                let cell = table(doc).cell(owner.0, owner.1).unwrap();
                                let opposite = if edge == "bottom" { 0 } else { 1 };
                                assert_eq!(cell.strokes[opposite].weight, weight);
                                assert!(cell.border_overrides[opposite]);
                            }
                        });
                    }
                }
            }
        }
    }

    #[test]
    fn a_range_boundary_inside_a_merged_owner_is_not_a_shared_edge() {
        for rtl in [false, true] {
            for edge in ["bottom", "right"] {
                let mut session = native_table(2, 2, rtl);
                session.execute("table.merge", &json!({"rows": [0, 1], "cols": [0, 1]})).unwrap();
                let before = session.doc().unwrap().doc.clone();
                session.execute("table.setCell", &json!({"rows": [0, 0], "cols": [0, 0], "stroke": {"edges": edge, "weight": 0}})).unwrap();
                let after_table = table(session.doc().unwrap().doc.as_ref());
                for (row, col) in [(0, 1), (1, 0), (1, 1)] {
                    assert_eq!(after_table.cell(row, col), table(&before).cell(row, col));
                }
                check_undo_and_roundtrips(&mut session, &before, |doc| {
                    assert_vertical_edge(doc, 0.0, 1.0);
                    assert_vertical_edge(doc, 100.0, if edge == "right" { 0.0 } else { 1.0 });
                    let fragment = fragment(doc);
                    assert_eq!(
                        fragment.strokes.iter().filter(|segment| segment.a.y == fragment.rect.y0 && segment.b.y == fragment.rect.y0).count(),
                        1
                    );
                    assert_eq!(
                        fragment.strokes.iter().filter(|segment| segment.a.y == fragment.rect.y1 && segment.b.y == fragment.rect.y1).count(),
                        if edge == "bottom" { 0 } else { 1 }
                    );
                });
            }
        }
    }

    fn r5_seam(doc: &Document, horizontal: bool, position: f64) -> Vec<(f64, f64, CellStroke)> {
        let fragment = fragment(doc);
        let mut segments: Vec<_> = fragment
            .strokes
            .iter()
            .filter_map(|segment| {
                if horizontal {
                    let y = fragment.rect.y0 + position;
                    (segment.a.y == y && segment.b.y == y).then_some((
                        segment.a.x - fragment.rect.x0,
                        segment.b.x - fragment.rect.x0,
                        segment.stroke.clone(),
                    ))
                } else {
                    let x = fragment.rect.x0 + position;
                    (segment.a.x == x && segment.b.x == x).then_some((
                        segment.a.y - fragment.rect.y0,
                        segment.b.y - fragment.rect.y0,
                        segment.stroke.clone(),
                    ))
                }
            })
            .collect();
        segments.sort_by(|a, b| a.0.total_cmp(&b.0));
        segments
    }

    fn r5_strokes() -> [(Value, CellStroke); 3] {
        [
            (json!({"weight": 0}), CellStroke::none()),
            (json!({"weight": 4, "color": "[None]"}), CellStroke { weight: 4.0, color: "[None]".into(), ..Default::default() }),
            (
                json!({"weight": 4, "color": "[Black]", "tint": 0.6, "type": "dotted"}),
                CellStroke { weight: 4.0, tint: 0.6, kind: StrokeType::Dotted, ..Default::default() },
            ),
        ]
    }

    fn r5_partial_neighbour(horizontal: bool, rtl: bool) -> (Session, Value, (usize, usize), (usize, usize)) {
        let mut session = native_table(2, 2, rtl);
        let (params, source, neighbour) = if horizontal {
            session.execute("table.merge", &json!({"rows": [1, 1], "cols": [0, 1]})).unwrap();
            (json!({"rows": [0, 0], "cols": [1, 1]}), (0, 1), (1, 0))
        } else {
            let source_col = usize::from(rtl);
            let neighbour_col = 1 - source_col;
            session.execute("table.merge", &json!({"rows": [0, 1], "cols": [neighbour_col, neighbour_col]})).unwrap();
            (json!({"rows": [0, 0], "cols": [source_col, source_col]}), (0, source_col), (0, neighbour_col))
        };
        (session, params, source, neighbour)
    }

    #[test]
    fn r5_partial_bottom_and_right_edits_preserve_the_unselected_merged_remainder() {
        for horizontal in [false, true] {
            for rtl in [false, true] {
                for (mut stroke, expected) in r5_strokes() {
                    let (mut session, mut params, source, neighbour) = r5_partial_neighbour(horizontal, rtl);
                    let before = session.doc().unwrap().doc.clone();
                    stroke["edges"] = json!(if horizontal { "bottom" } else { "right" });
                    params["stroke"] = stroke;
                    session.execute("table.setCell", &params).unwrap();
                    check_undo_and_roundtrips(&mut session, &before, |doc| {
                        let half: f64 = if horizontal { 50.0 } else { 20.0 };
                        let selected_start = if horizontal && !rtl { half } else { 0.0 };
                        let remainder_start = half - selected_start;
                        let mut wanted = vec![(remainder_start, remainder_start + half, CellStroke::default())];
                        if expected.is_visible() {
                            wanted.push((selected_start, selected_start + half, expected.clone()));
                        }
                        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
                        assert_eq!(r5_seam(doc, horizontal, if horizontal { 20.0 } else { 50.0 }), wanted);
                        let source = table(doc).cell(source.0, source.1).unwrap();
                        let edge = if horizontal { 2 } else { 3 };
                        assert_eq!(source.strokes[edge], expected);
                        assert!(source.border_overrides[edge]);
                        let original_neighbour = table(&before).cell(neighbour.0, neighbour.1).unwrap();
                        let neighbour = table(doc).cell(neighbour.0, neighbour.1).unwrap();
                        assert_eq!(neighbour.strokes, original_neighbour.strokes);
                        assert_eq!(neighbour.border_overrides, original_neighbour.border_overrides);
                    });
                    let after = table(session.doc().unwrap().doc.as_ref());
                    for r in 0..2 {
                        for c in 0..2 {
                            if (r, c) != source {
                                assert_eq!(after.cell(r, c), table(&before).cell(r, c), "neighbours and covered placeholders are unchanged");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn r5_bulk_bottom_and_right_edits_match_single_cell_edits_and_roundtrip() {
        for horizontal in [false, true] {
            for rtl in [false, true] {
                for (mut stroke, expected) in r5_strokes() {
                    let (rows, cols) = if horizontal { (3, 1) } else { (1, 3) };
                    let mut bulk = native_table(rows, cols, rtl);
                    let mut single = native_table(rows, cols, rtl);
                    if !horizontal {
                        for session in [&mut bulk, &mut single] {
                            session.execute("table.setColumnWidth", &json!({"cols": [0, 2], "width": 50})).unwrap();
                        }
                    }
                    let before = bulk.doc().unwrap().doc.clone();
                    stroke["edges"] = json!(if horizontal { "bottom" } else { "right" });
                    bulk.execute("table.setCell", &json!({"rows": [0, rows - 1], "cols": [0, cols - 1], "stroke": stroke})).unwrap();
                    for index in 0..3 {
                        let (r, c) = if horizontal { (index, 0) } else { (0, index) };
                        single.execute("table.setCell", &json!({"rows": [r, r], "cols": [c, c], "stroke": stroke})).unwrap();
                    }
                    let single = single.doc().unwrap().doc.clone();
                    check_undo_and_roundtrips(&mut bulk, &before, |doc| {
                        assert_eq!(fragment(doc).strokes, fragment(&single).strokes, "bulk and individual edits have identical composed output");
                        let step = if horizontal { 20.0 } else { 50.0 };
                        let length = if horizontal { 100.0 } else { 20.0 };
                        for boundary in 1..=3 {
                            let wanted = if expected.is_visible() { vec![(0.0, length, expected.clone())] } else { Vec::new() };
                            assert_eq!(r5_seam(doc, horizontal, step * f64::from(boundary)), wanted);
                        }
                        assert_eq!(r5_seam(doc, horizontal, 0.0), vec![(0.0, length, CellStroke::default())], "opposite perimeter is untouched");
                    });
                    assert_eq!(
                        table(bulk.doc().unwrap().doc.as_ref()),
                        table(&single),
                        "bulk and individual edits retain the same cell strokes/markers"
                    );
                }
            }
        }
    }

    #[test]
    fn r5_all_inner_and_outer_keep_independently_edited_top_left_attributes() {
        for rtl in [false, true] {
            for mode in ["all", "inner", "outer"] {
                let mut session = native_table(2, 2, rtl);
                session
                    .execute(
                        "table.setCell",
                        &json!({"rows": [1, 1], "cols": [0, 1], "stroke": {"edges": "top", "color": "[Paper]", "tint": 0.5, "type": "dotted"}}),
                    )
                    .unwrap();
                let right_col = usize::from(!rtl);
                session.execute("table.setCell", &json!({"rows": [0, 1], "cols": [right_col, right_col], "stroke": {"edges": "left", "color": "[Paper]", "tint": 0.5, "type": "dotted"}})).unwrap();
                let before = session.doc().unwrap().doc.clone();
                session.execute("table.setCell", &json!({"rows": [0, 1], "cols": [0, 1], "stroke": {"edges": mode, "weight": 4}})).unwrap();
                check_undo_and_roundtrips(&mut session, &before, |doc| {
                    let interior_weight = if mode == "outer" { 1.0 } else { 4.0 };
                    let interior = CellStroke { weight: interior_weight, color: "[Paper]".into(), tint: 0.5, kind: StrokeType::Dotted };
                    assert_eq!(r5_seam(doc, true, 20.0), vec![(0.0, 50.0, interior.clone()), (50.0, 100.0, interior.clone())]);
                    assert_eq!(r5_seam(doc, false, 50.0), vec![(0.0, 20.0, interior.clone()), (20.0, 40.0, interior)]);
                    let perimeter = CellStroke { weight: if mode == "inner" { 1.0 } else { 4.0 }, ..Default::default() };
                    for y in [0.0, 40.0] {
                        assert_eq!(r5_seam(doc, true, y), vec![(0.0, 50.0, perimeter.clone()), (50.0, 100.0, perimeter.clone())]);
                    }
                    for x in [0.0, 100.0] {
                        assert_eq!(r5_seam(doc, false, x), vec![(0.0, 20.0, perimeter.clone()), (20.0, 40.0, perimeter.clone())]);
                    }
                });
            }
        }
    }

    #[test]
    fn r5_unequal_explicit_top_left_conflicts_keep_the_preferred_edge() {
        for horizontal in [false, true] {
            for rtl in [false, true] {
                for (mut preferred, expected) in r5_strokes() {
                    let (mut session, mut params, source, neighbour) = r5_partial_neighbour(horizontal, rtl);
                    preferred["edges"] = json!(if horizontal { "top" } else { "left" });
                    session.execute("table.setCell", &json!({"rows": [neighbour.0, if horizontal { neighbour.0 } else { 1 }], "cols": [neighbour.1, if horizontal { 1 } else { neighbour.1 }], "stroke": preferred})).unwrap();
                    let before = session.doc().unwrap().doc.clone();
                    params["stroke"] = json!({"edges": if horizontal { "bottom" } else { "right" }, "weight": 3});
                    session.execute("table.setCell", &params).unwrap();
                    check_undo_and_roundtrips(&mut session, &before, |doc| {
                        let length = if horizontal { 100.0 } else { 40.0 };
                        let wanted = if expected.is_visible() { vec![(0.0, length, expected.clone())] } else { Vec::new() };
                        assert_eq!(
                            r5_seam(doc, horizontal, if horizontal { 20.0 } else { 50.0 }),
                            wanted,
                            "a partial adjacent edit cannot replace a higher-priority explicit merged edge"
                        );
                        let neighbour = table(doc).cell(neighbour.0, neighbour.1).unwrap();
                        let opposite = if horizontal { 0 } else { 1 };
                        assert_eq!(neighbour.strokes[opposite], expected);
                        assert!(neighbour.border_overrides[opposite]);
                        assert_eq!(table(doc).cell(source.0, source.1).unwrap().strokes[if horizontal { 2 } else { 3 }].weight, 3.0);
                    });
                }
            }
        }
    }

    #[test]
    fn r5_narrow_range_edits_visit_the_merged_owners_real_boundary() {
        for horizontal in [false, true] {
            for rtl in [false, true] {
                for (mut stroke, expected) in r5_strokes() {
                    let (rows, cols) = if horizontal { (3, 1) } else { (1, 3) };
                    let mut session = native_table(rows, cols, rtl);
                    let source_col = if horizontal || !rtl { 0 } else { 1 };
                    if horizontal {
                        session.execute("table.merge", &json!({"rows": [0, 1], "cols": [0, 0]})).unwrap();
                    } else {
                        session.execute("table.setColumnWidth", &json!({"cols": [0, 2], "width": 50})).unwrap();
                        session.execute("table.merge", &json!({"rows": [0, 0], "cols": [source_col, source_col + 1]})).unwrap();
                    }
                    let before = session.doc().unwrap().doc.clone();
                    stroke["edges"] = json!(if horizontal { "bottom" } else { "right" });
                    session.execute("table.setCell", &json!({"rows": [0, 0], "cols": [source_col, source_col], "stroke": stroke})).unwrap();
                    check_undo_and_roundtrips(&mut session, &before, |doc| {
                        let position = if horizontal { 40.0 } else { 100.0 };
                        let length = if horizontal { 100.0 } else { 20.0 };
                        let wanted = if expected.is_visible() { vec![(0.0, length, expected.clone())] } else { Vec::new() };
                        assert_eq!(r5_seam(doc, horizontal, position), wanted);
                        assert!(r5_seam(doc, horizontal, position / 2.0).is_empty(), "a merged owner's interior is not a shared edge");
                        assert_eq!(r5_seam(doc, horizontal, 0.0), vec![(0.0, length, CellStroke::default())]);
                        let neighbour = if horizontal { (2, 0) } else { (0, if rtl { 0 } else { 2 }) };
                        let neighbour = table(doc).cell(neighbour.0, neighbour.1).unwrap();
                        let opposite = if horizontal { 0 } else { 1 };
                        assert_eq!(neighbour.strokes[opposite], expected);
                        assert!(neighbour.border_overrides[opposite]);
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod r6_idml_roundtrip_tests {
    use super::*;
    use designcraft_compose::TableFrag;
    use designcraft_doc::ItemId;

    fn table(doc: &Document) -> &Table {
        doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
    }

    fn fragments(doc: &Document) -> Vec<TableFrag> {
        let sid = doc.stories.iter().find_map(|(sid, story)| (!story.tables.is_empty()).then_some(*sid)).unwrap();
        let composed = compose::compose_story(doc, sid, &Default::default());
        assert!(!composed.is_overset());
        composed.frames.iter().flat_map(|frame| frame.tables.iter().cloned()).collect()
    }

    fn horizontal(fragment: &TableFrag, offset: f64) -> Vec<(f64, f64, CellStroke)> {
        let y = fragment.rect.y0 + offset;
        let mut segments: Vec<_> = fragment
            .strokes
            .iter()
            .filter_map(|segment| {
                (segment.a.y == y && segment.b.y == y).then_some((
                    segment.a.x - fragment.rect.x0,
                    segment.b.x - fragment.rect.x0,
                    segment.stroke.clone(),
                ))
            })
            .collect();
        segments.sort_by(|a, b| a.0.total_cmp(&b.0));
        segments
    }

    fn assert_boundary(fragment: &TableFrag, offset: f64, spans: &[(f64, f64)], stroke: &CellStroke) {
        let expected: Vec<_> = spans.iter().filter(|_| stroke.is_visible()).map(|&(a, b)| (a, b, stroke.clone())).collect();
        assert_eq!(horizontal(fragment, offset), expected);
    }

    fn check_roundtrips_and_undo(session: &mut Session, before: &Document, check: impl Fn(&Document)) {
        let after = session.doc().unwrap().doc.clone();
        let selection = session.doc().unwrap().selection.clone();
        check(&after);
        let native = designcraft_format::load(&designcraft_format::save(&after).unwrap()).unwrap();
        assert_eq!(native, *after);
        check(&native);
        let exported = session.execute("file.exportIdml", &json!({})).unwrap();
        let mut imported = Session::new();
        imported.execute("file.openIdml", &json!({"base64": exported["base64"], "name": "r6.idml"})).unwrap();
        let idml = &imported.doc().unwrap().doc;
        idml.check().unwrap();
        check(idml);
        assert_eq!(session.doc().unwrap().selection, selection);
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *before);
        session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *after);
        assert_eq!(session.doc().unwrap().selection, selection);
        check(session.doc().unwrap().doc.as_ref());
    }

    fn strokes() -> [(Value, CellStroke); 3] {
        [
            (json!({"edges": "bottom", "weight": 0}), CellStroke::none()),
            (json!({"edges": "bottom", "weight": 4, "color": "[None]"}), CellStroke { weight: 4.0, color: "[None]".into(), ..Default::default() }),
            (
                json!({"edges": "bottom", "weight": 4, "color": "[Black]", "tint": 0.6, "type": "dotted"}),
                CellStroke { weight: 4.0, tint: 0.6, kind: StrokeType::Dotted, ..Default::default() },
            ),
        ]
    }

    fn native_footer(repeat: bool, rtl: bool) -> Session {
        let mut session = Session::new();
        session.execute("file.new", &json!({})).unwrap();
        let mut ids = Vec::new();
        let mut sid = StoryId(0);
        for i in 0..3 {
            let y = f64::from(i) * 99.0;
            let frame = session.execute("frame.create", &json!({"rect": [0.0, y, 120.0, y + 79.0], "content": "text", "text": ""})).unwrap();
            ids.push(ItemId(frame["id"].as_u64().unwrap()));
            if i == 0 {
                sid = StoryId(frame["story"].as_u64().unwrap());
            }
        }
        session
            .edit(|doc, _| {
                doc.thread(ids[0], ids[1])?;
                doc.thread(ids[1], ids[2])?;
                Ok(())
            })
            .unwrap();
        session.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        session.execute("table.insert", &json!({"rows": 3, "cols": 2, "headerRows": 1, "footerRows": 1, "width": 100})).unwrap();
        session.execute("table.setRowHeight", &json!({"rows": [0, 4], "height": 20, "mode": "exactly"})).unwrap();
        session.execute("table.setCell", &json!({"rows": [0, 4], "cols": [0, 1], "insets": 0})).unwrap();
        session
            .execute(
                "table.options",
                &json!({"repeatHeader": true, "repeatFooter": repeat, "direction": if rtl { "rightToLeft" } else { "leftToRight" }}),
            )
            .unwrap();
        session.execute("table.select", &json!({"rows": [3, 3], "cols": [0, 1]})).unwrap();
        session
    }

    #[test]
    fn r6_last_body_bottom_edit_keeps_other_footer_boundaries_and_roundtrips() {
        for repeat in [true, false] {
            for rtl in [false, true] {
                for (stroke, expected) in strokes() {
                    let mut session = native_footer(repeat, rtl);
                    let before = session.doc().unwrap().doc.clone();
                    let initial = fragments(&before);
                    assert_eq!(initial.len(), 3);
                    for fragment in &initial {
                        assert_boundary(fragment, 40.0, &[(0.0, 50.0), (50.0, 100.0)], &CellStroke::default());
                    }
                    session.execute("table.setCell", &json!({"stroke": stroke})).unwrap();
                    check_roundtrips_and_undo(&mut session, &before, |doc| {
                        let composed = fragments(doc);
                        assert_eq!(composed.len(), 3);
                        let default = CellStroke::default();
                        for (index, fragment) in composed.iter().enumerate() {
                            assert!(fragment.cell(index + 1, 0).is_some());
                            assert_eq!(fragment.cell(0, 0).unwrap().repeated, index > 0);
                            if repeat || index == 2 {
                                assert_eq!(fragment.cell(4, 0).unwrap().repeated, index < 2);
                            } else {
                                assert!(fragment.cell(4, 0).is_none());
                            }
                            let edge = if index == 2 { &expected } else { &default };
                            assert_boundary(fragment, 40.0, &[(0.0, 50.0), (50.0, 100.0)], edge);
                            assert_boundary(fragment, 20.0, &[(0.0, 50.0), (50.0, 100.0)], &CellStroke::default());
                        }
                        let after = table(doc);
                        for col in 0..2 {
                            let source = after.cell(3, col).unwrap();
                            assert_eq!(source.strokes[2], expected);
                            assert!(source.border_overrides[2]);
                            let footer = after.cell(4, col).unwrap();
                            if repeat {
                                assert_eq!(footer, table(&before).cell(4, col).unwrap(), "an unselected repeatable owner stays unchanged");
                            } else {
                                assert_eq!(footer.strokes[0], expected, "ordinary non-repeated mirroring is retained");
                                assert!(footer.border_overrides[0]);
                            }
                        }
                        for row in 0..3 {
                            for col in 0..2 {
                                assert_eq!(after.cell(row, col), table(&before).cell(row, col), "earlier rows are not edited");
                            }
                        }
                    });
                }
            }
        }
    }

    #[test]
    fn r6_repeated_footer_preserves_explicit_top_priority_and_partial_merged_limit() {
        for rtl in [false, true] {
            for merged in [false, true] {
                for (mut preferred, expected) in strokes() {
                    let mut session = native_footer(true, rtl);
                    if merged {
                        session.execute("table.merge", &json!({"rows": [4, 4], "cols": [0, 1]})).unwrap();
                    }
                    preferred["edges"] = json!("top");
                    session.execute("table.setCell", &json!({"rows": [4, 4], "cols": [0, 1], "stroke": preferred})).unwrap();
                    session.execute("table.select", &json!({"rows": [3, 3], "cols": if merged { [1, 1] } else { [0, 1] }})).unwrap();
                    let before = session.doc().unwrap().doc.clone();
                    session.execute("table.setCell", &json!({"stroke": {"edges": "bottom", "weight": 3}})).unwrap();
                    for col in 0..2 {
                        assert_eq!(
                            table(&session.doc().unwrap().doc).cell(4, col),
                            table(&before).cell(4, col),
                            "native footer slots stay unchanged"
                        );
                    }
                    check_roundtrips_and_undo(&mut session, &before, |doc| {
                        let composed = fragments(doc);
                        assert_eq!(composed.len(), 3);
                        let spans = if merged { vec![(0.0, 100.0)] } else { vec![(0.0, 50.0), (50.0, 100.0)] };
                        for fragment in &composed {
                            assert_boundary(fragment, 40.0, &spans, &expected);
                        }
                        // IDML stores the merged owner; covered slots are rebuilt with defaults.
                        // Their native preservation is checked above, while all exported owners stay exact.
                        for col in 0..if merged { 1 } else { 2 } {
                            assert_eq!(table(doc).cell(4, col), table(&before).cell(4, col), "explicit footer owners are preserved");
                        }
                        let source = table(doc).cell(3, 1).unwrap();
                        assert_eq!(source.strokes[2].weight, 3.0);
                        assert!(source.border_overrides[2]);
                    });
                }
            }
        }
    }

    fn assert_stripe(doc: &Document, enabled: bool) {
        assert_eq!(table(doc).style, "Stripe");
        let style = doc.styles.table.iter().find(|style| style.name == "Stripe").unwrap();
        let pattern = style.alt_rows.as_ref().unwrap();
        assert_eq!(pattern.first_color, "[Black]");
        assert_eq!(pattern.first_tint, 0.1);
        assert_eq!(table(doc).options.alt_rows.as_ref(), enabled.then_some(pattern));
        let composed = fragments(doc);
        assert_eq!(composed.len(), 1);
        assert_eq!(composed[0].cells.len(), 3);
        for cell in &composed[0].cells {
            let fill = (enabled && cell.row % 2 == 0).then(|| ("[Black]".to_string(), 0.1));
            assert_eq!(cell.fill, fill);
        }
    }

    #[test]
    fn r6_disabling_style_row_fills_survives_native_commands_composition_and_idml() {
        for rtl in [false, true] {
            let mut session = Session::new();
            session.execute("file.new", &json!({})).unwrap();
            let frame = session.execute("frame.create", &json!({"rect": [72, 72, 400, 400], "content": "text", "text": ""})).unwrap();
            session.execute("text.select", &json!({"story": frame["story"], "anchor": 0, "focus": 0})).unwrap();
            session.execute("table.insert", &json!({"rows": 3, "cols": 1, "width": 100})).unwrap();
            session.execute("table.setRowHeight", &json!({"rows": [0, 2], "height": 20, "mode": "exactly"})).unwrap();
            session.execute("table.options", &json!({"direction": if rtl { "rightToLeft" } else { "leftToRight" }})).unwrap();
            session
                .execute(
                    "style.table.create",
                    &json!({"name": "Stripe", "altRows": {"first": 1, "firstColor": "[Black]", "firstTint": 0.1, "next": 1, "nextColor": "[None]"}}),
                )
                .unwrap();
            session.execute("style.table.apply", &json!({"name": "Stripe"})).unwrap();
            let before = session.doc().unwrap().doc.clone();
            assert_stripe(&before, true);
            session.execute("table.options", &json!({"altRows": null})).unwrap();
            check_roundtrips_and_undo(&mut session, &before, |doc| {
                assert_stripe(doc, false);
                assert_eq!(doc.styles.table, before.styles.table, "disabling the local option keeps the named style definition");
            });
            session.execute("edit.undo", &json!({})).unwrap();
            assert_stripe(session.doc().unwrap().doc.as_ref(), true);
            session.execute("edit.redo", &json!({})).unwrap();
            assert_stripe(session.doc().unwrap().doc.as_ref(), false);
        }
    }
}

#[cfg(test)]
mod row_fill_skip_tests {
    use super::*;

    const LABELS: [&str; 3] = ["row zero", "row one", "row two"];

    fn pattern((skip_first, skip_last): (u32, u32)) -> AltFills {
        AltFills {
            first_color: "[Black]".into(),
            first_tint: 0.1,
            next_color: "[Black]".into(),
            next_tint: 0.1,
            skip_first,
            skip_last,
            ..Default::default()
        }
    }

    fn table(doc: &Document) -> &Table {
        doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
    }

    fn style(doc: &Document) -> &designcraft_doc::TableStyle {
        doc.styles.table.iter().find(|style| style.name == "Stripe").unwrap()
    }

    fn target(doc: &Document) -> Value {
        let (story, table) = doc.stories.iter().find_map(|(sid, story)| story.tables.keys().next().map(|tid| (*sid, *tid))).unwrap();
        json!({"story": story.0, "table": table})
    }

    fn fills(doc: &Document) -> Vec<Option<(String, f32)>> {
        let sid = doc.stories.iter().find_map(|(sid, story)| (!story.tables.is_empty()).then_some(*sid)).unwrap();
        let composed = compose::compose_story(doc, sid, &Default::default());
        assert!(!composed.is_overset());
        let fragments: Vec<_> = composed.frames.iter().flat_map(|frame| &frame.tables).collect();
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].cells.iter().map(|cell| (cell.row, cell.col)).collect::<Vec<_>>(), [(0, 0), (1, 0), (2, 0)]);
        fragments[0].cells.iter().map(|cell| cell.fill.clone()).collect()
    }

    fn expected_fills(skips: (u32, u32)) -> Vec<Option<(String, f32)>> {
        // Both phases use the same tint so skipped rows are the only cause of an unfilled cell.
        // These literal row oracles do not recompute AltFills::fill_for.
        let black = Some(("[Black]".to_string(), 0.1));
        match skips {
            (0, 0) => vec![black.clone(), black.clone(), black],
            (1, 0) => vec![None, black.clone(), black],
            (0, 1) => vec![black.clone(), black, None],
            (1, 1) => vec![None, black, None],
            _ => panic!("unexpected test skip counts"),
        }
    }

    fn assert_effective(doc: &Document, skips: (u32, u32)) {
        doc.check().unwrap();
        assert_eq!(table(doc).style, "Stripe");
        assert_eq!(table(doc).options.alt_rows.as_ref(), Some(&pattern(skips)));
        assert_eq!(table(doc).cells.iter().map(|cell| cell.text.text.as_str()).collect::<Vec<_>>(), LABELS);
        assert_eq!(fills(doc), expected_fills(skips));
    }

    fn native_table(skips: (u32, u32), rtl: bool) -> Session {
        let mut session = Session::new();
        session.execute("file.new", &json!({"pages": 1, "facingPages": false})).unwrap();
        let frame = session.execute("frame.create", &json!({"rect": [0, 0, 120, 200], "content": "text", "text": ""})).unwrap();
        session.execute("text.select", &json!({"story": frame["story"], "anchor": 0, "focus": 0})).unwrap();
        let inserted = session.execute("table.insert", &json!({"rows": 3, "cols": 1, "width": 100})).unwrap();
        let tid = inserted["table"].as_u64().unwrap();
        session.execute("table.setRowHeight", &json!({"table": tid, "rows": [0, 2], "height": 20, "mode": "exactly"})).unwrap();
        for (row, text) in LABELS.iter().enumerate() {
            session.execute("table.setCell", &json!({"table": tid, "rows": [row, row], "cols": [0, 0], "insets": 0, "text": text})).unwrap();
        }
        session.execute("table.options", &json!({"table": tid, "direction": if rtl { "rightToLeft" } else { "leftToRight" }})).unwrap();
        session.execute("style.table.create", &json!({"name": "Stripe", "altRows": pattern(skips)})).unwrap();
        session.execute("style.table.apply", &json!({"table": tid, "name": "Stripe"})).unwrap();
        assert_effective(&session.doc().unwrap().doc, skips);
        assert_eq!(style(&session.doc().unwrap().doc).alt_rows.as_ref(), Some(&pattern(skips)));
        session
    }

    fn reopen_idml(session: &mut Session) -> Session {
        let exported = session.execute("file.exportIdml", &json!({})).unwrap();
        let mut reopened = Session::new();
        reopened.execute("file.openIdml", &json!({"base64": exported["base64"], "name": "row-skips.idml"})).unwrap();
        reopened
    }

    #[test]
    fn r7a_row_fill_skips_named_style_properties_survive_native_and_idml() {
        for skips in [(1, 0), (0, 1), (1, 1)] {
            for rtl in [false, true] {
                let mut session = native_table(skips, rtl);
                let before = session.doc().unwrap().doc.clone();
                let native = designcraft_format::load(&designcraft_format::save(&before).unwrap()).unwrap();
                assert_eq!(native, *before, "native ZIP retains the complete source and style properties");
                assert_effective(&native, skips);
                let reopened = reopen_idml(&mut session);
                let doc = &reopened.doc().unwrap().doc;
                assert_effective(doc, skips); // Local table attributes already retain these skips.
                assert_eq!(doc.styles.table, before.styles.table, "IDML must also retain the complete named style definition");
                assert_eq!(table(doc).rows, table(&before).rows);
                assert_eq!(table(doc).columns, table(&before).columns);
                assert_eq!(table(doc).options, table(&before).options);
            }
        }
    }

    fn check_public_reapply(skips: (u32, u32), rtl: bool) {
        let mut session = native_table(skips, rtl);
        let original = session.doc().unwrap().doc.clone();
        let native = designcraft_format::load(&designcraft_format::save(&original).unwrap()).unwrap();
        assert_eq!(native, *original);
        assert_effective(&native, skips);
        let mut reopened = reopen_idml(&mut session);
        assert_effective(&reopened.doc().unwrap().doc, skips);
        let params = target(&reopened.doc().unwrap().doc);
        reopened.execute("table.select", &params).unwrap();
        let selection = reopened.doc().unwrap().selection.clone();
        let cells = table(&reopened.doc().unwrap().doc).cells.clone();
        let mut disable = params.clone();
        disable["altRows"] = Value::Null;
        reopened.execute("table.options", &disable).unwrap();
        let disabled = reopened.doc().unwrap().doc.clone();
        assert!(table(&disabled).options.alt_rows.is_none());
        assert_eq!(fills(&disabled), vec![None; 3]);
        let mut apply = params;
        apply["name"] = json!("Stripe");
        reopened.execute("style.table.apply", &apply).unwrap();
        let applied = reopened.doc().unwrap().doc.clone();
        // Reapplication is the public behavior oracle; checking only retained local options misses it.
        assert_eq!(fills(&applied), expected_fills(skips), "public reapply must preserve skipped first/last body rows");
        assert_effective(&applied, skips);
        assert_eq!(applied.styles.table, original.styles.table);
        assert_eq!(table(&applied).cells, cells, "reapply leaves source text and all cell properties intact");
        assert_eq!(reopened.doc().unwrap().selection, selection);
        let native = designcraft_format::load(&designcraft_format::save(&applied).unwrap()).unwrap();
        assert_eq!(native, *applied);
        assert_effective(&native, skips);
        reopened.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*reopened.doc().unwrap().doc, *disabled, "Undo restores the explicitly disabled local pattern");
        assert_eq!(fills(&reopened.doc().unwrap().doc), vec![None; 3]);
        reopened.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*reopened.doc().unwrap().doc, *applied);
        assert_effective(&reopened.doc().unwrap().doc, skips);
        assert_eq!(reopened.doc().unwrap().selection, selection);
    }

    #[test]
    fn r7a_row_fill_skips_public_reapply_preserves_fills_and_undo() {
        for skips in [(1, 1), (1, 0), (0, 1)] {
            for rtl in [false, true] {
                check_public_reapply(skips, rtl);
            }
        }
    }

    #[test]
    fn r7a_row_fill_skips_zero_skip_public_roundtrip_and_reapply_control() {
        for rtl in [false, true] {
            check_public_reapply((0, 0), rtl);
        }
    }
}

#[cfg(test)]
mod interchange_r8_tests {
    use super::*;

    fn table(doc: &Document) -> &Table {
        doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
    }

    fn target(doc: &Document) -> Value {
        let (story, table) = doc.stories.iter().find_map(|(sid, story)| story.tables.keys().next().map(|tid| (*sid, *tid))).unwrap();
        json!({"story": story.0, "table": table})
    }

    fn native_table(rows: usize, cols: usize, rtl: bool) -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1, "facingPages": false})).unwrap();
        let frame = s.execute("frame.create", &json!({"rect": [0, 0, 120, 200], "content": "text", "text": ""})).unwrap();
        s.execute("text.select", &json!({"story": frame["story"], "anchor": 0, "focus": 0})).unwrap();
        let inserted = s.execute("table.insert", &json!({"rows": rows, "cols": cols, "width": 100})).unwrap();
        let tid = inserted["table"].as_u64().unwrap();
        s.execute("table.setRowHeight", &json!({"table": tid, "rows": [0, rows - 1], "height": 20, "mode": "exactly"})).unwrap();
        for r in 0..rows {
            for c in 0..cols {
                s.execute("table.setCell", &json!({"table": tid, "rows": [r, r], "cols": [c, c], "insets": 0, "text": format!("r{r}c{c}")})).unwrap();
            }
        }
        s.execute("table.options", &json!({"table": tid, "direction": if rtl { "rightToLeft" } else { "leftToRight" }})).unwrap();
        s
    }

    fn reopen_idml(s: &mut Session) -> Session {
        let exported = s.execute("file.exportIdml", &json!({})).unwrap();
        let mut reopened = Session::new();
        reopened.execute("file.openIdml", &json!({"base64": exported["base64"], "name": "interchange-r8.idml"})).unwrap();
        // Restore public cell/text selection: explicit ids do not satisfy the enabled gate.
        let mut cell = target(&reopened.doc().unwrap().doc);
        cell["rows"] = json!([0, 0]);
        cell["cols"] = json!([0, 0]);
        reopened.execute("table.select", &cell).unwrap();
        reopened
    }

    fn native_full_doc(doc: &Document) {
        doc.check().unwrap();
        let zip = designcraft_format::load(&designcraft_format::save(doc).unwrap()).unwrap();
        let legacy = designcraft_format::load(&serde_json::to_vec(doc).unwrap()).unwrap();
        for back in [zip, legacy] {
            assert_eq!(back, *doc, "native persistence retains every document field");
            assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(doc).unwrap(), "the complete document JSON remains identical");
        }
    }

    fn fragment(doc: &Document) -> compose::TableFrag {
        let sid = doc.stories.iter().find_map(|(sid, story)| (!story.tables.is_empty()).then_some(*sid)).unwrap();
        let composed = compose::compose_story(doc, sid, &Default::default());
        assert!(!composed.is_overset());
        let fragments: Vec<_> = composed.frames.iter().flat_map(|frame| &frame.tables).collect();
        assert_eq!(fragments.len(), 1);
        fragments[0].clone()
    }

    fn fills(doc: &Document) -> Vec<Option<(String, f32)>> {
        let f = fragment(doc);
        assert_eq!(f.cells.iter().map(|cell| (cell.row, cell.col)).collect::<Vec<_>>(), [(0, 0), (1, 0), (2, 0)]);
        f.cells.iter().map(|cell| cell.fill.clone()).collect()
    }

    fn segments(doc: &Document) -> Vec<(f64, f64, f64, f64, CellStroke)> {
        let f = fragment(doc);
        f.strokes.iter().map(|s| (s.a.x - f.rect.x0, s.a.y - f.rect.y0, s.b.x - f.rect.x0, s.b.y - f.rect.y0, s.stroke.clone())).collect()
    }

    fn assert_table_source(original: &Document, reopened: &Document) {
        let (a, b) = (table(original), table(reopened));
        assert_eq!(b.rows, a.rows);
        assert_eq!(b.columns, a.columns);
        assert_eq!(b.options, a.options);
        assert_eq!(b.style, a.style);
        for (a, b) in a.cells.iter().zip(&b.cells) {
            assert_eq!(b.text.text, a.text.text);
            assert_eq!(b.strokes, a.strokes);
            assert_eq!(b.border_overrides, a.border_overrides);
        }
    }

    fn zero_first((skip_first, skip_last): (u32, u32)) -> AltFills {
        AltFills {
            first: 0,
            first_color: "[None]".into(),
            first_tint: 1.0,
            next: 1,
            next_color: "[Black]".into(),
            next_tint: 0.4,
            skip_first,
            skip_last,
        }
    }

    fn zero_fills(skips: (u32, u32)) -> Vec<Option<(String, f32)>> {
        let black = Some(("[Black]".to_string(), 0.4));
        // Literal composed-row oracles; these do not reuse the pattern implementation.
        match skips {
            (0, 0) => vec![black.clone(), black.clone(), black],
            (1, 0) => vec![None, black.clone(), black],
            (0, 1) => vec![black.clone(), black, None],
            (1, 1) => vec![None, black, None],
            _ => panic!("unexpected test skips"),
        }
    }

    #[test]
    fn r8_public_zero_first_local_beats_different_named_pattern_and_disabled_control() {
        for rtl in [false, true] {
            for skips in [(0, 0), (1, 1)] {
                let mut s = native_table(3, 1, rtl);
                s.execute(
                    "style.table.create",
                    &json!({"name": "Stripe", "altRows": {
                        "first": 1, "firstColor": "[Black]", "firstTint": 0.1,
                        "next": 1, "nextColor": "[None]"
                    }}),
                )
                .unwrap();
                let mut p = target(&s.doc().unwrap().doc);
                p["name"] = json!("Stripe");
                s.execute("style.table.apply", &p).unwrap();
                let named = s.doc().unwrap().doc.styles.table.clone();
                p["altRows"] = json!(zero_first(skips));
                s.execute("table.options", &p).unwrap();
                let original = s.doc().unwrap().doc.clone();
                assert_eq!(fills(&original), zero_fills(skips));
                native_full_doc(&original);

                let mut back = reopen_idml(&mut s);
                assert_table_source(&original, &back.doc().unwrap().doc);
                assert_eq!(back.doc().unwrap().doc.styles.table, named);
                assert_eq!(
                    fills(&back.doc().unwrap().doc),
                    zero_fills(skips),
                    "local zero/one remains effective above the named positive-first pattern"
                );
                let selection = back.doc().unwrap().selection.clone();
                let enabled = back.doc().unwrap().doc.clone();
                let mut p = target(&enabled);
                p["altRows"] = Value::Null;
                back.execute("table.options", &p).unwrap();
                let disabled = back.doc().unwrap().doc.clone();
                assert!(table(&disabled).options.alt_rows.is_none());
                assert_eq!(fills(&disabled), vec![None; 3]);
                native_full_doc(&disabled);
                let disabled_back = reopen_idml(&mut back);
                assert!(
                    table(&disabled_back.doc().unwrap().doc).options.alt_rows.is_none(),
                    "explicit local zero/zero still disables the named pattern"
                );
                assert_eq!(fills(&disabled_back.doc().unwrap().doc), vec![None; 3]);
                back.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(*back.doc().unwrap().doc, *enabled);
                assert_eq!(fills(&back.doc().unwrap().doc), zero_fills(skips));
                back.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(*back.doc().unwrap().doc, *disabled);
                assert_eq!(back.doc().unwrap().selection, selection);
                p["name"] = json!("Stripe");
                back.execute("style.table.apply", &p).unwrap();
                assert_eq!(
                    fills(&back.doc().unwrap().doc),
                    vec![Some(("[Black]".into(), 0.1)), None, Some(("[Black]".into(), 0.1))],
                    "reapply uses the distinct retained named definition"
                );
                assert_eq!(back.doc().unwrap().doc.styles.table, named);
            }
        }
    }

    #[test]
    fn r8_public_zero_first_named_reapply_keeps_resource_skips_history_and_native_full_doc() {
        for rtl in [false, true] {
            for skips in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let mut s = native_table(3, 1, rtl);
                let pattern = zero_first(skips);
                s.execute("style.table.create", &json!({"name": "Zero", "altRows": pattern})).unwrap();
                let mut p = target(&s.doc().unwrap().doc);
                p["name"] = json!("Zero");
                s.execute("style.table.apply", &p).unwrap();
                let original = s.doc().unwrap().doc.clone();
                assert_eq!(fills(&original), zero_fills(skips));
                native_full_doc(&original);
                let mut back = reopen_idml(&mut s);
                assert_table_source(&original, &back.doc().unwrap().doc);
                assert_eq!(back.doc().unwrap().doc.styles.table, original.styles.table);
                assert_eq!(fills(&back.doc().unwrap().doc), zero_fills(skips));
                let selection = back.doc().unwrap().selection.clone();
                let cells = table(&back.doc().unwrap().doc).cells.clone();
                let mut p = target(&back.doc().unwrap().doc);
                p["altRows"] = Value::Null;
                back.execute("table.options", &p).unwrap();
                let disabled = back.doc().unwrap().doc.clone();
                assert_eq!(fills(&disabled), vec![None; 3]);
                p["name"] = json!("Zero");
                back.execute("style.table.apply", &p).unwrap();
                let applied = back.doc().unwrap().doc.clone();
                assert_eq!(table(&applied).options.alt_rows.as_ref(), Some(&pattern));
                assert_eq!(fills(&applied), zero_fills(skips), "retained local values alone cannot prove named-resource reapply");
                assert_eq!(applied.styles.table, original.styles.table);
                assert_eq!(table(&applied).cells, cells);
                native_full_doc(&applied);
                let again = reopen_idml(&mut back);
                assert_table_source(&applied, &again.doc().unwrap().doc);
                assert_eq!(again.doc().unwrap().doc.styles.table, applied.styles.table);
                assert_eq!(fills(&again.doc().unwrap().doc), zero_fills(skips));
                back.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(*back.doc().unwrap().doc, *disabled);
                assert_eq!(fills(&back.doc().unwrap().doc), vec![None; 3]);
                back.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(*back.doc().unwrap().doc, *applied);
                assert_eq!(back.doc().unwrap().selection, selection);
                assert_eq!(fills(&back.doc().unwrap().doc), zero_fills(skips));
            }
        }
    }

    #[test]
    fn r8_public_positive_first_zero_next_and_local_disabled_controls_survive_idml() {
        for rtl in [false, true] {
            let mut s = native_table(3, 1, rtl);
            let pattern = AltFills { first: 1, first_color: "[Black]".into(), first_tint: 0.2, next: 0, ..Default::default() };
            s.execute("style.table.create", &json!({"name": "Positive", "altRows": pattern})).unwrap();
            let mut p = target(&s.doc().unwrap().doc);
            p["name"] = json!("Positive");
            s.execute("style.table.apply", &p).unwrap();
            let before = s.doc().unwrap().doc.clone();
            let mut back = reopen_idml(&mut s);
            assert_table_source(&before, &back.doc().unwrap().doc);
            assert_eq!(back.doc().unwrap().doc.styles.table, before.styles.table);
            assert_eq!(fills(&back.doc().unwrap().doc), vec![Some(("[Black]".into(), 0.2)); 3]);
            let mut p = target(&back.doc().unwrap().doc);
            p["altRows"] = Value::Null;
            back.execute("table.options", &p).unwrap();
            let disabled = reopen_idml(&mut back);
            assert!(table(&disabled.doc().unwrap().doc).options.alt_rows.is_none());
            assert_eq!(fills(&disabled.doc().unwrap().doc), vec![None; 3]);
        }
    }

    #[test]
    fn r8_public_weight_dependent_default_dashes_preserve_concrete_composed_seams() {
        for rtl in [false, true] {
            for (weight, pattern) in [(0.0, vec![3.0, 2.0]), (0.5, vec![3.0, 2.0]), (2.0, vec![6.0, 4.0]), (4.0, vec![12.0, 8.0])] {
                let mut s = native_table(2, 2, rtl);
                let mut p = target(&s.doc().unwrap().doc);
                p["border"] = json!({"type": "dashed", "weight": weight, "color": "[Black]", "tint": 0.6});
                s.execute("table.options", &p).unwrap();
                p["rows"] = json!([0, 0]);
                p["cols"] = json!([0, 1]);
                p["stroke"] = json!({"edges": "bottom", "type": "dashed", "weight": weight, "color": "[Black]", "tint": 0.6});
                s.execute("table.setCell", &p).unwrap();
                let before = s.doc().unwrap().doc.clone();
                let col = usize::from(rtl);
                p["rows"] = json!([0, 1]);
                p["cols"] = json!([col, col]);
                p["stroke"]["edges"] = json!("right");
                s.execute("table.setCell", &p).unwrap();
                let after = s.doc().unwrap().doc.clone();
                let expected = StrokeType::Dashed { pattern };
                assert_eq!(table(&after).options.border.kind, expected);
                assert_eq!(table(&after).cell(0, 0).unwrap().strokes[2].kind, expected);
                assert_eq!(table(&after).cell(0, col).unwrap().strokes[3].kind, expected);
                let seams = segments(&after);
                if weight == 0.0 {
                    assert!(seams.is_empty());
                } else {
                    assert!(!seams.is_empty());
                    assert!(seams.iter().all(|s| s.4.kind == expected && s.4.weight == weight));
                }
                native_full_doc(&after);
                let back = reopen_idml(&mut s);
                assert_table_source(&after, &back.doc().unwrap().doc);
                assert_eq!(
                    segments(&back.doc().unwrap().doc),
                    seams,
                    "renderer/PDF input retains physical RTL and horizontal/vertical seam patterns"
                );
                let selection = s.doc().unwrap().selection.clone();
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(*s.doc().unwrap().doc, *before);
                assert_eq!(segments(&s.doc().unwrap().doc), segments(&before));
                s.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(*s.doc().unwrap().doc, *after);
                assert_eq!(segments(&s.doc().unwrap().doc), seams);
                assert_eq!(s.doc().unwrap().selection, selection);
            }
        }
    }

    #[test]
    fn r8_public_weight_only_edit_preserves_stored_dashes_separately_from_new_defaults() {
        for rtl in [false, true] {
            let mut s = native_table(2, 2, rtl);
            let mut p = target(&s.doc().unwrap().doc);
            p["border"] = json!({"type": "dashed", "weight": 2});
            s.execute("table.options", &p).unwrap();
            p["border"] = json!({"weight": 4});
            s.execute("table.options", &p).unwrap();
            p["rows"] = json!([0, 0]);
            p["cols"] = json!([0, 1]);
            p["stroke"] = json!({"edges": "bottom", "type": "dashed", "weight": 2});
            s.execute("table.setCell", &p).unwrap();
            let col = usize::from(rtl);
            p["rows"] = json!([0, 1]);
            p["cols"] = json!([col, col]);
            p["stroke"] = json!({"edges": "right", "type": "dashed", "weight": 4});
            s.execute("table.setCell", &p).unwrap();
            let before = s.doc().unwrap().doc.clone();
            p["rows"] = json!([0, 0]);
            p["cols"] = json!([0, 1]);
            p["stroke"] = json!({"edges": "bottom", "weight": 4});
            s.execute("table.setCell", &p).unwrap();
            let after = s.doc().unwrap().doc.clone();
            let stored = StrokeType::Dashed { pattern: vec![6.0, 4.0] };
            let fresh = StrokeType::Dashed { pattern: vec![12.0, 8.0] };
            assert_eq!(table(&after).options.border.weight, 4.0);
            assert_eq!(table(&after).options.border.kind, stored);
            assert_eq!(table(&after).cell(0, 0).unwrap().strokes[2].weight, 4.0);
            assert_eq!(table(&after).cell(0, 0).unwrap().strokes[2].kind, stored);
            assert_eq!(table(&after).cell(0, col).unwrap().strokes[3].kind, fresh);
            native_full_doc(&after);
            let back = reopen_idml(&mut s);
            assert_table_source(&after, &back.doc().unwrap().doc);
            assert_eq!(segments(&back.doc().unwrap().doc), segments(&after), "weight alone cannot reconstruct a nondefault stored pattern");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *after);
        }
    }

    #[test]
    fn r8_public_solid_dotted_zero_weight_and_explicit_clear_controls_survive_idml() {
        for rtl in [false, true] {
            for (kind, expected, weight, color) in [
                ("solid", StrokeType::Solid, 2.0, "[Black]"),
                ("dotted", StrokeType::Dotted, 2.0, "[Black]"),
                ("dashed", StrokeType::Dashed { pattern: vec![3.0, 2.0] }, 0.0, "[Black]"),
                ("dashed", StrokeType::Dashed { pattern: vec![6.0, 4.0] }, 2.0, "[None]"),
            ] {
                let mut s = native_table(1, 1, rtl);
                let mut p = target(&s.doc().unwrap().doc);
                p["border"] = json!({"type": kind, "weight": weight, "color": color, "tint": 0.6});
                s.execute("table.options", &p).unwrap();
                p["stroke"] = json!({"edges": "all", "type": kind, "weight": weight, "color": color, "tint": 0.6});
                s.execute("table.setCell", &p).unwrap();
                let before = s.doc().unwrap().doc.clone();
                assert_eq!(table(&before).options.border.kind, expected);
                assert!(table(&before).cells[0].strokes.iter().all(|s| s.kind == expected));
                native_full_doc(&before);
                let back = reopen_idml(&mut s);
                assert_table_source(&before, &back.doc().unwrap().doc);
                assert_eq!(segments(&back.doc().unwrap().doc), segments(&before));
                if weight == 0.0 || color == "[None]" {
                    assert!(segments(&back.doc().unwrap().doc).is_empty());
                }
            }
        }
    }
}

#[cfg(test)]
mod review11_legacy_border_tests {
    use std::io::{Cursor, Read, Write};
    use std::sync::Arc;

    use designcraft_compose::{self as compose, TableFrag};
    use designcraft_doc::{CellStroke, Document, StoryId, StrokeType, Table, TableBorderResolution};
    use designcraft_geom::Point;
    use serde_json::json;

    use crate::Session;

    mod fixture {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../idml/src/tests/table_styles_fixture.rs"));
    }

    fn table(doc: &Document) -> &Table {
        doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
    }

    fn assert_resolution_roundtrip(t: &Table, expected: TableBorderResolution) {
        assert_eq!(t.border_resolution, expected);
        let value = serde_json::to_value(t).unwrap();
        match expected {
            TableBorderResolution::Legacy => assert!(value.get("borderResolution").is_none()),
            TableBorderResolution::Shared => assert_eq!(value["borderResolution"], json!("shared")),
        }
        assert_eq!(serde_json::from_value::<Table>(value).unwrap(), *t);
    }

    fn fragment(doc: &Document) -> TableFrag {
        doc.check().unwrap();
        let sid = doc.stories.iter().find_map(|(sid, story)| (!story.tables.is_empty()).then_some(*sid)).unwrap();
        let composed = compose::compose_story(doc, sid, &Default::default());
        assert!(!composed.is_overset());
        let fragments: Vec<_> = composed.frames.iter().flat_map(|frame| &frame.tables).collect();
        assert_eq!(fragments.len(), 1);
        fragments[0].clone()
    }

    fn assert_seam(f: &TableFrag, horizontal: bool, preferred: (usize, usize), expected: Option<&CellStroke>, label: &str) {
        let rect = f.cell(preferred.0, preferred.1).unwrap().rect;
        let a = Point::new(rect.x0, rect.y0);
        let b = if horizontal { Point::new(rect.x1, rect.y0) } else { Point::new(rect.x0, rect.y1) };
        let seam: Vec<_> = f
            .strokes
            .iter()
            .filter(|seg| {
                if horizontal {
                    seg.a.y == a.y && seg.b.y == b.y && seg.a.x.max(a.x) < seg.b.x.min(b.x)
                } else {
                    seg.a.x == a.x && seg.b.x == b.x && seg.a.y.max(a.y) < seg.b.y.min(b.y)
                }
            })
            .collect();
        if let Some(expected) = expected {
            assert_eq!(seam.len(), 1, "{label}: expected one complete shared rule: {seam:?}");
            assert_eq!((seam[0].a, seam[0].b), (a, b), "{label}");
            assert_eq!(&seam[0].stroke, expected, "{label}");
        } else {
            assert!(seam.is_empty(), "{label}: a cleared shared edge must stay absent: {seam:?}");
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum NativeFormat {
        Zip,
        Json,
    }

    fn native_reopen(input: &mut Session, kind: NativeFormat) -> Session {
        // Each byte stream originates from the un-opened fixture. ZIP must not be
        // produced from a legacy-reader result that already migrated its metadata.
        let bytes = match kind {
            NativeFormat::Zip => {
                let saved = input.execute("file.serialize", &json!({})).unwrap();
                let bytes = crate::cmd::file::base64_decode(saved["base64"].as_str().unwrap());
                assert!(bytes.starts_with(b"PK"), "exercise the real native ZIP writer");
                bytes
            }
            NativeFormat::Json => {
                let bytes = serde_json::to_vec(&*input.doc().unwrap().doc).unwrap();
                assert!(bytes.starts_with(b"{"));
                bytes
            }
        };
        let mut opened = Session::new();
        opened.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
        opened
    }

    fn native_fixture(horizontal: bool, preferred: CellStroke, adjacent: CellStroke, priorities: [bool; 2]) -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1, "facingPages": false})).unwrap();
        let frame = s.execute("frame.create", &json!({"rect": [0, 0, 120, 200], "content": "text", "text": ""})).unwrap();
        let sid = StoryId(frame["story"].as_u64().unwrap());
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0})).unwrap();
        let (rows, cols) = if horizontal { (2, 1) } else { (1, 2) };
        let inserted = s.execute("table.insert", &json!({"rows": rows, "cols": cols, "width": 100})).unwrap();
        let tid = inserted["table"].as_u64().unwrap();
        s.execute("table.setRowHeight", &json!({"table": tid, "rows": [0, rows - 1], "height": 20, "mode": "exactly"})).unwrap();
        s.execute("table.setCell", &json!({"table": tid, "rows": [0, rows - 1], "cols": [0, cols - 1], "insets": 0})).unwrap();

        // Model the fields emitted at base 14e677b: setCell changed strokes but left
        // borderOverrides false. Calling the candidate clear command would miss this case.
        let doc = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let t = doc.story_mut(sid).unwrap().table_mut(tid).unwrap();
        let (row, col, edge, opposite) = if horizontal { (1, 0, 0, 2) } else { (0, 1, 1, 3) };
        let cell = t.cell_mut(row, col).unwrap();
        cell.strokes[edge] = preferred;
        cell.border_overrides[edge] = priorities[0];
        let neighbour = t.cell_mut(0, 0).unwrap();
        neighbour.strokes[opposite] = adjacent;
        neighbour.border_overrides[opposite] = priorities[1];
        assert_resolution_roundtrip(t, TableBorderResolution::Shared);
        s
    }

    fn check_native_forms(input: &mut Session, horizontal: bool, expected: Option<&CellStroke>, label: &str, formats: &[NativeFormat]) {
        let before = input.doc().unwrap().doc.clone();
        let preferred = if horizontal { (1, 0) } else { (0, 1) };
        for &kind in formats {
            let opened = native_reopen(input, kind);
            let doc = &opened.doc().unwrap().doc;
            assert_eq!(doc.as_ref(), before.as_ref(), "{label}: {kind:?}: native load must preserve the full document");
            let f = fragment(doc);
            assert_seam(&f, horizontal, preferred, expected, &format!("{label}: {kind:?}"));
            // A compatibility correction must not remove visible table perimeters.
            let perimeter: Vec<_> = f
                .strokes
                .iter()
                .filter(|seg| {
                    (seg.a.y == f.rect.y0 && seg.b.y == f.rect.y0)
                        || (seg.a.y == f.rect.y1 && seg.b.y == f.rect.y1)
                        || (seg.a.x == f.rect.x0 && seg.b.x == f.rect.x0)
                        || (seg.a.x == f.rect.x1 && seg.b.x == f.rect.x1)
                })
                .collect();
            assert_eq!(perimeter.len(), 6, "{label}: {kind:?}");
            assert!(perimeter.iter().all(|seg| seg.stroke == CellStroke::default()));
        }
    }

    fn legacy_clear(horizontal: bool, clear: CellStroke, kind: NativeFormat) {
        // The non-default adjacent rule prevents a default-1pt-only compatibility fix.
        for adjacent in [CellStroke::default(), CellStroke { weight: 3.25, tint: 0.6, kind: StrokeType::Dotted, ..Default::default() }] {
            let mut input = native_fixture(horizontal, clear.clone(), adjacent, [false; 2]);
            // Base-era tables had no resolution metadata. Keep this fixture valid after
            // new table constructors begin selecting the new policy explicitly.
            {
                let doc = Arc::make_mut(&mut input.doc_mut().unwrap().doc);
                let (sid, tid) = doc.stories.iter().find_map(|(sid, story)| story.tables.keys().next().map(|tid| (*sid, *tid))).unwrap();
                let t = doc.story_mut(sid).unwrap().table_mut(tid).unwrap();
                let mut value = serde_json::to_value(&*t).unwrap();
                value.as_object_mut().unwrap().remove("borderResolution");
                *t = serde_json::from_value(value).unwrap();
                assert_resolution_roundtrip(t, TableBorderResolution::Legacy);
            }
            assert!(table(&input.doc().unwrap().doc).cells.iter().all(|cell| cell.border_overrides == [false; 4]));
            check_native_forms(&mut input, horizontal, None, if horizontal { "legacy top clear" } else { "legacy left clear" }, &[kind]);
        }
    }

    #[test]
    fn r11_legacy_top_zero_clear_native_zip() {
        legacy_clear(true, CellStroke::none(), NativeFormat::Zip);
    }

    #[test]
    fn r11_legacy_top_zero_clear_legacy_json() {
        legacy_clear(true, CellStroke::none(), NativeFormat::Json);
    }

    #[test]
    fn r11_legacy_left_zero_clear_native_zip() {
        legacy_clear(false, CellStroke::none(), NativeFormat::Zip);
    }

    #[test]
    fn r11_legacy_left_zero_clear_legacy_json() {
        legacy_clear(false, CellStroke::none(), NativeFormat::Json);
    }

    #[test]
    fn r11_legacy_top_none_clear_native_zip() {
        legacy_clear(true, CellStroke { weight: 2.5, color: "[None]".into(), ..Default::default() }, NativeFormat::Zip);
    }

    #[test]
    fn r11_legacy_top_none_clear_legacy_json() {
        legacy_clear(true, CellStroke { weight: 2.5, color: "[None]".into(), ..Default::default() }, NativeFormat::Json);
    }

    #[test]
    fn r11_legacy_left_none_clear_native_zip() {
        legacy_clear(false, CellStroke { weight: 2.5, color: "[None]".into(), ..Default::default() }, NativeFormat::Zip);
    }

    #[test]
    fn r11_legacy_left_none_clear_legacy_json() {
        legacy_clear(false, CellStroke { weight: 2.5, color: "[None]".into(), ..Default::default() }, NativeFormat::Json);
    }

    #[test]
    fn r11_native_visible_and_explicit_priority_controls_survive_native_load() {
        let formats = [NativeFormat::Zip, NativeFormat::Json];
        for horizontal in [true, false] {
            let preferred = CellStroke { weight: 2.5, ..Default::default() };
            let adjacent = CellStroke { weight: 3.25, tint: 0.6, kind: StrokeType::Dotted, ..Default::default() };
            let mut input = native_fixture(horizontal, preferred.clone(), adjacent.clone(), [false; 2]);
            check_native_forms(&mut input, horizontal, Some(&preferred), "visible unflagged top/left keeps precedence", &formats);
            for clear in [CellStroke::none(), CellStroke { color: "[None]".into(), ..Default::default() }] {
                let mut input = native_fixture(horizontal, clear.clone(), adjacent.clone(), [false, true]);
                check_native_forms(&mut input, horizontal, Some(&adjacent), "explicit bottom/right rule beats unflagged clear", &formats);
                let mut input = native_fixture(horizontal, preferred.clone(), clear.clone(), [false, true]);
                check_native_forms(&mut input, horizontal, None, "explicit bottom/right clear beats unflagged visible rule", &formats);
                let mut input = native_fixture(horizontal, clear, adjacent.clone(), [true; 2]);
                check_native_forms(&mut input, horizontal, None, "explicit top/left clear keeps precedence", &formats);
            }
        }
    }

    fn named_rule_fixture(horizontal: bool) -> Session {
        let mut input = zip::ZipArchive::new(Cursor::new(fixture::fixture(false, false))).unwrap();
        let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for index in 0..input.len() {
            let mut file = input.by_index(index).unwrap();
            let path = file.name().to_string();
            let mut text = String::new();
            file.read_to_string(&mut text).unwrap();
            if path == "Stories/Story_s1.xml" {
                assert_eq!(text.matches(" BottomEdgeStrokePriority=\"1\"").count(), 9);
                text = text.replace(" BottomEdgeStrokePriority=\"1\"", "");
            }
            if !horizontal && path == "Resources/Styles.xml" {
                text = text.replace("BottomEdge", "R11TemporaryEdge").replace("RightEdge", "BottomEdge").replace("R11TemporaryEdge", "RightEdge");
            }
            output.start_file(path, options).unwrap();
            output.write_all(text.as_bytes()).unwrap();
        }
        let bytes = output.finish().unwrap().into_inner();
        let mut s = Session::new();
        s.execute("file.openIdml", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
        s
    }

    fn assert_named_rule(doc: &Document, horizontal: bool, bound: bool) {
        let t = table(doc);
        assert_resolution_roundtrip(t, TableBorderResolution::Shared);
        let (adjacent, edge, preferred_edge) = if horizontal { ((0, 1), 2, 0) } else { ((1, 0), 3, 1) };
        let source = t.cell(adjacent.0, adjacent.1).unwrap();
        let preferred = t.cell(1, 1).unwrap();
        let expected = CellStroke { weight: 0.5, ..Default::default() };
        assert_eq!(source.strokes[edge], expected);
        assert!(!preferred.strokes[preferred_edge].is_visible());
        assert!(t.cells.iter().all(|cell| cell.border_overrides == [false; 4]));
        if bound {
            let style = doc.styles.cell.iter().find(|style| style.name == source.style).unwrap();
            assert!(style.stroke.is_none());
            assert_eq!(style.strokes.as_ref().unwrap()[edge].as_ref(), Some(&expected));
        } else {
            assert!(t.cells.iter().all(|cell| cell.style.is_empty()));
        }
        let f = fragment(doc);
        assert_seam(&f, horizontal, (1, 1), Some(&expected), "named asymmetric rule without local priority");
        // Style-derived interior fallback does not authorize replacing the table perimeter.
        assert!(f.strokes.iter().all(|seg| {
            !(seg.a.y == f.rect.y0 && seg.b.y == f.rect.y0
                || seg.a.y == f.rect.y1 && seg.b.y == f.rect.y1
                || seg.a.x == f.rect.x0 && seg.b.x == f.rect.x0
                || seg.a.x == f.rect.x1 && seg.b.x == f.rect.x1)
        }));
    }

    #[test]
    fn r11_imported_named_bottom_right_rules_keep_false_priorities_through_native_loads() {
        for horizontal in [true, false] {
            let mut input = named_rule_fixture(horizontal);
            assert_named_rule(&input.doc().unwrap().doc, horizontal, true);
            for kind in [NativeFormat::Zip, NativeFormat::Json] {
                let opened = native_reopen(&mut input, kind);
                assert_named_rule(&opened.doc().unwrap().doc, horizontal, true);
            }
            // Applying [None] drops the binding but leaves stroke values in place.
            // Rule provenance cannot depend only on a currently bound style name.
            let before = input.doc().unwrap().doc.clone();
            let tid = table(&before).id;
            let (sid, anchor) = before.stories.iter().find_map(|(sid, story)| story.table_anchor(tid).map(|anchor| (*sid, anchor))).unwrap();
            input.execute("text.select", &json!({"story": sid.0, "anchor": anchor})).unwrap();
            input.execute("style.cell.apply", &json!({"name": "[None]", "table": tid})).unwrap();
            for (old, new) in table(&before).cells.iter().zip(&table(&input.doc().unwrap().doc).cells) {
                assert_eq!(old.strokes, new.strokes);
            }
            assert_named_rule(&input.doc().unwrap().doc, horizontal, false);
            for kind in [NativeFormat::Zip, NativeFormat::Json] {
                let opened = native_reopen(&mut input, kind);
                assert_named_rule(&opened.doc().unwrap().doc, horizontal, false);
            }
        }
    }

    // Main qualification control: policy changes must preserve an existing native-to-IDML seam.
    fn r11_main_legacy_idml_seam(horizontal: bool) {
        let mut input = native_fixture(horizontal, CellStroke::none(), CellStroke::default(), [false; 2]);
        let doc = Arc::make_mut(&mut input.doc_mut().unwrap().doc);
        let (sid, tid) = doc.stories.iter().find_map(|(sid, story)| story.tables.keys().next().map(|tid| (*sid, *tid))).unwrap();
        doc.story_mut(sid).unwrap().table_mut(tid).unwrap().border_resolution = TableBorderResolution::Legacy;
        let preferred = if horizontal { (1, 0) } else { (0, 1) };
        assert_seam(&fragment(&input.doc().unwrap().doc), horizontal, preferred, None, "legacy before IDML");
        let before = input.doc().unwrap().doc.clone();
        let exported = input.execute("file.exportIdml", &json!({})).unwrap();
        assert_eq!(*input.doc().unwrap().doc, *before);
        let mut opened = Session::new();
        opened.execute("file.openIdml", &json!({"base64": exported["base64"]})).unwrap();
        assert_resolution_roundtrip(table(&opened.doc().unwrap().doc), TableBorderResolution::Legacy);
        assert_seam(&fragment(&opened.doc().unwrap().doc), horizontal, preferred, None, "legacy after IDML");
    }

    #[test]
    fn r11_main_legacy_top_clear_survives_idml() {
        r11_main_legacy_idml_seam(true);
    }

    #[test]
    fn r11_main_legacy_left_clear_survives_idml() {
        r11_main_legacy_idml_seam(false);
    }

    #[test]
    fn r11_idml_border_label_is_scoped_and_optional() {
        const KEY: &str = "ai.storyteller.designcraft.tableBorderResolution";
        let mut input = native_fixture(true, CellStroke::none(), CellStroke::default(), [false; 2]);
        let before = input.doc().unwrap().doc.clone();
        let exported = input.execute("file.exportIdml", &json!({})).unwrap();
        assert_eq!(*input.doc().unwrap().doc, *before);
        let original = crate::cmd::file::base64_decode(exported["base64"].as_str().unwrap());
        for (key, value, legacy) in
            [("", "", false), ("another-app", "legacy", false), (KEY, "shared", false), (KEY, "unknown", false), (KEY, "legacy", true)]
        {
            let mut archive = zip::ZipArchive::new(Cursor::new(&original)).unwrap();
            let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            let mut tables = 0;
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).unwrap();
                let path = entry.name().to_owned();
                let mut text = String::new();
                entry.read_to_string(&mut text).unwrap();
                assert!(!text.contains(KEY), "new Shared tables need no compatibility label");
                if let Some(start) = text.find("<Table ") {
                    assert!(path.starts_with("Stories/"));
                    tables += 1;
                    if !key.is_empty() {
                        let end = start + text[start..].find('>').unwrap() + 1;
                        text.insert_str(end, &format!("<Properties><Label><KeyValuePair Key=\"{key}\" Value=\"{value}\"/></Label></Properties>"));
                    }
                }
                output.start_file(path, options).unwrap();
                output.write_all(text.as_bytes()).unwrap();
            }
            assert_eq!(tables, 1);
            let bytes = output.finish().unwrap().into_inner();
            let mut reopened = Session::new();
            reopened.execute("file.openIdml", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
            let doc = &reopened.doc().unwrap().doc;
            assert_resolution_roundtrip(table(doc), if legacy { TableBorderResolution::Legacy } else { TableBorderResolution::Shared });
            let visible = CellStroke::default();
            assert_seam(&fragment(doc), true, (1, 0), if legacy { None } else { Some(&visible) }, "scoped IDML label");
            assert!(table(doc).cells.iter().all(|cell| cell.border_overrides == [false; 4]));
            assert_eq!(
                table(doc).cells.iter().map(|cell| &cell.strokes).collect::<Vec<_>>(),
                table(&before).cells.iter().map(|cell| &cell.strokes).collect::<Vec<_>>()
            );
        }
    }
}

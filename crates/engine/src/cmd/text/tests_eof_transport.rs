//! R12-EDITING-1: source EOF typing stages transported by formatted paste and selected drag.
//! Fixtures and edits use Session commands. Oracles cover effective attrs, actual LF runs,
//! genuine paragraph defaults, complete Document history, and both native readers.

use designcraft_doc::{ItemId, StoryId};
use designcraft_geom::Point;
use serde_json::{Value, json};

use crate::Session;

fn full_doc(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, range: std::ops::Range<usize>) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": range.start, "focus": range.end})).unwrap();
}

fn frame(s: &mut Session, text: &str, y: f64) -> (ItemId, StoryId) {
    let f = s.execute("frame.create", &json!({"rect": [40.0, y, 440.0, y + 180.0], "content": "text", "text": text})).unwrap();
    (ItemId(f["id"].as_u64().unwrap()), StoryId(f["story"].as_u64().unwrap()))
}

fn edit(s: &mut Session, command: &str, params: &Value) -> Value {
    let before = full_doc(s);
    let depth = s.doc().unwrap().history.undo.len();
    let result = s.execute(command, params).unwrap();
    let after = full_doc(s);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "{command} owns one history entry");
    s.doc().unwrap().doc.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(full_doc(s), before, "{command}: Undo restores the complete document");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(full_doc(s), after, "{command}: Redo restores the complete document");
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn native_copies(s: &mut Session) -> [Session; 2] {
    let full = full_doc(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let zip = crate::cmd::file::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"), "exercise the real native ZIP serializer");
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    let mut copies = [Session::new(), Session::new()];
    for (back, bytes) in copies.iter_mut().zip([zip, legacy]) {
        // No name override: the complete serialized title is part of the oracle.
        back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
        assert_eq!(full_doc(back), full, "native reopen preserves the complete document");
        back.doc().unwrap().doc.check().unwrap();
    }
    copies
}

fn query(s: &mut Session, sid: StoryId, range: std::ops::Range<usize>, size: f64, leading: Value) -> Value {
    select(s, sid, range);
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["chars"]["size"], json!(size), "effective size of the actual selected bytes");
    assert_eq!(q["chars"]["leading"], leading, "effective leading of the actual selected bytes");
    q
}

fn local_defaults(s: &mut Session, sid: StoryId, range: std::ops::Range<usize>) {
    s.execute("style.paragraph.create", &json!({"name": "R12 local base", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
        .unwrap();
    select(s, sid, range);
    s.execute("style.paragraph.apply", &json!({"name": "R12 local base"})).unwrap();
    s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
}

fn staged_source(inherited: bool, leading: bool) -> (Session, ItemId, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (fid, sid) = frame(&mut s, "A", 40.0);
    if inherited {
        local_defaults(&mut s, sid, 0..1);
    }
    select(&mut s, sid, 0..1);
    let attrs = if leading { json!({"size": 20, "leading": {"kind": "points", "value": 36}}) } else { json!({"size": 20}) };
    s.execute("type.char", &json!({"attrs": attrs})).unwrap();
    select(&mut s, sid, 1..1);
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "A\n");
    (s, fid, sid)
}

fn copy(s: &mut Session, sid: StoryId, range: std::ops::Range<usize>) {
    select(s, sid, range);
    s.execute("edit.copy", &json!({})).unwrap();
}

fn glyph_point(s: &Session, fid: ItemId, sid: StoryId, byte: usize) -> Point {
    let d = &s.doc().unwrap().doc;
    let composed = s.cache.get(d, sid, None);
    let ft = composed.frame(fid).unwrap();
    let (line, glyph) = ft.lines.iter().find_map(|line| line.glyphs.iter().find(|g| g.byte == byte && g.len > 0).map(|g| (line, g))).unwrap();
    let inner = Point::new(glyph.x + glyph.adv * 0.25, line.baseline - line.ascent * 0.5);
    d.parent_xf(&d.find(fid).unwrap()) * d.text_xf(d.item(fid).unwrap()) * inner
}

#[test]
fn r12_eof_transport_public_copy_before_existing_text() {
    // Exact review sequence: A at 20 pt, Enter, copy A+LF, paste before default B, clear B.
    let (mut s, _, src) = staged_source(false, false);
    let source = s.doc().unwrap().doc.story(src).unwrap().clone();
    copy(&mut s, src, 0..2);
    let (_, dst) = frame(&mut s, "B", 300.0);
    let original = s.doc().unwrap().doc.story(dst).unwrap().clone();
    select(&mut s, dst, 0..0);
    edit(&mut s, "edit.paste", &json!({"formatted": true}));
    let st = s.doc().unwrap().doc.story(dst).unwrap();
    assert_eq!(st.text, "A\nB");
    assert_eq!(st.format_after(0), source.format_after(0), "copied A retains its direct run");
    assert_eq!(st.format_after(1), source.format_after(1), "the actual copied LF retains its direct run");
    assert_eq!(st.format_after(2), original.format_after(0), "B's original run is unchanged");
    assert_eq!(st.paras[0], original.paras[0], "the first pasted paragraph retains destination defaults");
    assert_eq!(s.doc().unwrap().doc.story(src).unwrap(), &source, "copy/paste does not edit the source");
    select(&mut s, dst, 2..3);
    let before_clear = s.execute("type.selectionAttrs", &json!({})).unwrap();
    edit(&mut s, "type.char", &json!({"attrs": {"size": null}}));
    let q = query(&mut s, dst, 2..3, 12.0, json!({"kind": "auto"}));
    assert_eq!(q["charOverrides"], 0, "clear restores Basic Paragraph without inventing an override");
    assert_eq!(before_clear["chars"]["size"], json!(12.0), "paste itself must preserve B's effective baseline");
    assert_eq!(before_clear["charOverrides"], 0);
    for mut back in native_copies(&mut s) {
        let q = query(&mut back, dst, 2..3, 12.0, json!({"kind": "auto"}));
        assert_eq!(q["charOverrides"], 0);
    }
}

#[test]
fn r12_eof_transport_size_leading_and_separator_restore_baselines() {
    for inherited in [false, true] {
        for suffix in ["B", "\nB"] {
            let (mut s, _, src) = staged_source(inherited, true);
            let source = s.doc().unwrap().doc.story(src).unwrap().clone();
            copy(&mut s, src, 0..2);
            let (_, dst) = frame(&mut s, suffix, 300.0);
            select(&mut s, dst, 0..1);
            s.execute("type.char", &json!({"attrs": {"tracking": 17}})).unwrap();
            let original = s.doc().unwrap().doc.story(dst).unwrap().clone();
            select(&mut s, dst, 0..0);
            edit(&mut s, "edit.paste", &json!({"formatted": true}));
            let st = s.doc().unwrap().doc.story(dst).unwrap();
            assert_eq!(st.text, format!("A\n{suffix}"));
            assert_eq!(st.format_after(0), source.format_after(0));
            assert_eq!(st.format_after(1), source.format_after(1), "pasted LF is an actual source byte");
            assert_eq!(st.format_after(2), original.format_after(0), "destination run is preserved");
            assert_eq!(st.paras[0], original.paras[0]);
            assert_eq!(s.doc().unwrap().doc.story(src).unwrap(), &source);
            let (size, leading) = if inherited { (24.0, json!({"kind": "points", "value": 26.0})) } else { (12.0, json!({"kind": "auto"})) };
            let q = query(&mut s, dst, 2..3, size, leading.clone());
            assert_eq!(q["charOverrides"], 1, "only destination tracking remains direct");
            assert_eq!(q["chars"]["tracking"], json!(17.0));
            let st = s.doc().unwrap().doc.story(dst).unwrap();
            assert_eq!(st.paras[1].chars, source.paras[0].chars, "restore the real source paragraph baseline");
            assert!(st.paras[1].eof_inherited_chars.is_none(), "the destination byte consumes the former EOF");
            if suffix.starts_with('\n') {
                let composed = s.cache.get(&s.doc().unwrap().doc, dst, None);
                let line = composed.frames[0].lines.iter().find(|l| l.para == 1).unwrap();
                assert!(line.glyphs.is_empty(), "the existing LF owns a glyphless paragraph");
                let expected = if inherited { 26.0 } else { 14.4 };
                assert!((line.leading - expected).abs() < 1e-6, "the existing LF uses its baseline leading");
                query(&mut s, dst, 3..4, 12.0, json!({"kind": "auto"}));
            }
            select(&mut s, dst, 2..3);
            edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
            let q = query(&mut s, dst, 2..3, size, leading.clone());
            assert_eq!(q["charOverrides"], 1);
            for mut back in native_copies(&mut s) {
                let q = query(&mut back, dst, 2..3, size, leading.clone());
                assert_eq!(q["charOverrides"], 1);
            }
        }
    }
}

#[test]
fn r12_eof_transport_drag_copy_and_move_restore_existing_prefix() {
    for copy in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (fid, sid) = frame(&mut s, "B A", 40.0);
        select(&mut s, sid, 2..3);
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        select(&mut s, sid, 3..3);
        s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let press = glyph_point(&s, fid, sid, 2);
        let drop = glyph_point(&s, fid, sid, 0);
        select(&mut s, sid, 2..4);
        let armed = s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [press.x, press.y]})).unwrap();
        assert_eq!(armed["dragging"], true, "a public press inside selected A must arm the drag");
        let result = edit(&mut s, "text.release", &json!({"frame": fid.0, "point": [drop.x, drop.y], "moved": true, "copy": copy}));
        assert_eq!(result["at"], 0);
        let st = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.text, if copy { "A\nB A\n" } else { "A\nB " });
        assert_eq!(st.format_after(0), original.format_after(2));
        assert_eq!(st.format_after(1), original.format_after(3), "drag retains the source LF's real run");
        assert_eq!(st.format_after(2), original.format_after(0), "the original B run is unchanged");
        if copy {
            assert_eq!(st.paras.last(), original.paras.last(), "the remaining true EOF keeps its typing stage");
        }
        let q = query(&mut s, sid, 2..3, 12.0, json!({"kind": "auto"}));
        assert_eq!(q["charOverrides"], 0);
        edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        query(&mut s, sid, 2..3, 12.0, json!({"kind": "auto"}));
        for mut back in native_copies(&mut s) {
            query(&mut back, sid, 2..3, 12.0, json!({"kind": "auto"}));
        }
    }
}

#[test]
fn r12_eof_transport_true_blank_eof_keeps_stage_control() {
    let (mut s, _, src) = staged_source(true, true);
    let source = s.doc().unwrap().doc.story(src).unwrap().clone();
    copy(&mut s, src, 0..2);
    let (_, dst) = frame(&mut s, "D\n", 300.0);
    select(&mut s, dst, 2..2);
    s.execute("type.char", &json!({"attrs": {"size": 18, "leading": {"kind": "points", "value": 28}}})).unwrap();
    let destination = s.doc().unwrap().doc.story(dst).unwrap().clone();
    edit(&mut s, "edit.paste", &json!({"formatted": true}));
    let st = s.doc().unwrap().doc.story(dst).unwrap();
    assert_eq!(st.text, "D\nA\n");
    assert_eq!(st.paras.last(), source.paras.last(), "a source EOF that stays true EOF retains its whole typing stage");
    assert_eq!(st.paras[0], destination.paras[0]);
    assert!(st.paras[1].chars.is_empty(), "existing destination EOF stage is consumed by the source A");
    assert_eq!(st.format_after(2), source.format_after(0));
    assert_eq!(st.format_after(3), source.format_after(1));
    assert_eq!(s.doc().unwrap().doc.story(src).unwrap(), &source);
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        query(&mut back, dst, 4..4, 20.0, json!({"kind": "points", "value": 36.0}));
        edit(&mut back, "text.insert", &json!({"text": "X", "raw": true}));
        query(&mut back, dst, 4..5, 20.0, json!({"kind": "points", "value": 36.0}));
        edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        let q = query(&mut back, dst, 4..5, 24.0, json!({"kind": "points", "value": 26.0}));
        assert_eq!(q["charOverrides"], 0, "typing consumes the stage but keeps its inherited paragraph defaults");
    }
}

#[test]
fn r12_eof_transport_clipped_and_nonblank_defaults_survive_control() {
    for end in [2, 3] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (_, src) = frame(&mut s, "A\nQ", 40.0);
        local_defaults(&mut s, src, 2..3);
        select(&mut s, src, 2..3);
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        let source = s.doc().unwrap().doc.story(src).unwrap().clone();
        // At 2 the slice ends after LF, but the real source continues: these are defaults, not a stage.
        copy(&mut s, src, 0..end);
        let (_, dst) = frame(&mut s, "B", 300.0);
        let destination = s.doc().unwrap().doc.story(dst).unwrap().clone();
        select(&mut s, dst, 0..0);
        edit(&mut s, "edit.paste", &json!({"formatted": true}));
        let st = s.doc().unwrap().doc.story(dst).unwrap();
        assert_eq!(st.text, if end == 2 { "A\nB" } else { "A\nQB" });
        assert_eq!(st.paras[0], destination.paras[0]);
        assert_eq!(st.paras[1], source.paras[1], "ordinary source paragraph defaults remain whole");
        assert_eq!(st.format_after(1), source.format_after(1));
        assert_eq!(st.format_after(end), destination.format_after(0));
        assert_eq!(s.doc().unwrap().doc.story(src).unwrap(), &source);
        let q = query(&mut s, dst, end..end + 1, 24.0, json!({"kind": "points", "value": 26.0}));
        assert_eq!(q["charOverrides"], 0);
        if end == 3 {
            query(&mut s, dst, 2..3, 20.0, json!({"kind": "points", "value": 36.0}));
            edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
            query(&mut s, dst, 2..3, 24.0, json!({"kind": "points", "value": 26.0}));
        }
        for mut back in native_copies(&mut s) {
            query(&mut back, dst, end..end + 1, 24.0, json!({"kind": "points", "value": 26.0}));
        }
    }
}

// R13: an outer-story EOF typing stage cannot become a child paragraph's defaults.
// All edits and selections use public Session commands; source/document reads are oracles.
fn r13_child(s: &mut Session, note: bool, text: &str) -> (ItemId, StoryId, designcraft_doc::CellAddr) {
    let (fid, sid) = frame(s, "", 300.0);
    select(s, sid, 0..0);
    if note {
        edit(s, "footnote.insert", &json!({"text": text}));
    } else {
        edit(s, "table.insert", &json!({"rows": 1, "cols": 1, "width": 300}));
        if !text.is_empty() {
            edit(s, "text.insert", &json!({"text": text, "raw": true}));
        }
    }
    let t = s.doc().unwrap().selection.text.unwrap();
    assert_eq!(t.story, sid);
    (fid, sid, t.cell.unwrap())
}

fn r13_select_child(s: &mut Session, fid: ItemId, sid: StoryId, cell: designcraft_doc::CellAddr, range: std::ops::Range<usize>) {
    let (point, before, count) = {
        let d = &s.doc().unwrap().doc;
        let text = &d.text_story(sid, Some(cell)).unwrap().text;
        let before = text[..range.start].chars().count();
        let count = text[range.clone()].chars().count();
        let cs = s.cache.get(d, sid, None);
        let (_, x, baseline, ascent, _) = if let Some(id) = cell.footnote_id() {
            designcraft_compose::note_caret(&cs, id, 0).unwrap()
        } else {
            designcraft_compose::cell_caret(&cs, cell.table, cell.row, cell.col, 0).unwrap()
        };
        let xf = d.parent_xf(&d.find(fid).unwrap()) * d.text_xf(d.item(fid).unwrap());
        (xf * Point::new(x, baseline - ascent * 0.5), before, count)
    };
    s.execute("edit.deselectAll", &json!({})).unwrap();
    s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [point.x, point.y]})).unwrap();
    assert_eq!(s.doc().unwrap().selection.text.unwrap().cell, Some(cell), "hit the actual child story");
    s.execute("text.move", &json!({"dir": "storyStart"})).unwrap();
    for _ in 0..before {
        s.execute("text.move", &json!({"dir": "right"})).unwrap();
    }
    for _ in 0..count {
        s.execute("text.move", &json!({"dir": "right", "extend": true})).unwrap();
    }
    let t = s.doc().unwrap().selection.text.unwrap();
    assert_eq!((t.story, t.cell, t.range()), (sid, Some(cell), range));
    assert!(s.doc().unwrap().selection.cells.is_none(), "format only the requested text range");
}

fn r13_metrics(s: &mut Session, size: f64, leading: Value, overrides: usize) {
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["chars"]["size"], json!(size));
    assert_eq!(q["chars"]["leading"], leading);
    assert_eq!(q["charOverrides"], json!(overrides));
}

fn r13_baseline(inherited: bool) -> (f64, Value) {
    if inherited { (24.0, json!({"kind": "points", "value": 26.0})) } else { (12.0, json!({"kind": "auto"})) }
}

fn r13_type_clear(s: &mut Session, fid: ItemId, sid: StoryId, cell: designcraft_doc::CellAddr, at: usize, inherited: bool) {
    if at == 0 && cell.footnote_id().is_none() {
        // Conversion leaves the outer caret: explicitly enter the newly created cell.
        s.execute("table.select", &json!({"story": sid.0, "table": cell.table, "rows": [0, 0], "cols": [0, 0]})).unwrap();
        assert_eq!(s.doc().unwrap().selection.text.unwrap().cell, Some(cell));
    } else {
        r13_select_child(s, fid, sid, cell, at..at);
    }
    edit(s, "text.insert", &json!({"text": "é", "raw": true}));
    r13_select_child(s, fid, sid, cell, at..at + 2);
    r13_metrics(s, 20.0, json!({"kind": "points", "value": 36.0}), 2);
    let (size, leading) = r13_baseline(inherited);
    edit(s, "type.char", &json!({"attrs": {"size": null}}));
    r13_metrics(s, size, json!({"kind": "points", "value": 36.0}), 1);
    edit(s, "type.char", &json!({"attrs": {"leading": null}}));
    r13_metrics(s, size, leading.clone(), 0);
    let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
    assert!(child.format_after(at).over.is_empty());
    assert!(!child.slice_has_eof_stage);
    assert!(child.paras.iter().all(|pf| pf.eof_inherited_chars.is_none()));
    for mut back in native_copies(s) {
        r13_select_child(&mut back, fid, sid, cell, at..at + 2);
        r13_metrics(&mut back, size, leading.clone(), 0);
    }
}

fn r13_paste_stage(note: bool) {
    for inherited in [false, true] {
        for suffix in [false, true] {
            let (mut s, _, src) = staged_source(inherited, true);
            let source = s.doc().unwrap().doc.story(src).unwrap().clone();
            copy(&mut s, src, 0..2);
            let (fid, sid, cell) = r13_child(&mut s, note, if suffix { "DB" } else { "D" });
            let original = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap().clone();
            r13_select_child(&mut s, fid, sid, cell, 1..1);
            edit(&mut s, "edit.paste", &json!({"formatted": true}));
            let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
            assert_eq!(child.text, if suffix { "DA\nB" } else { "DA\n" });
            assert_eq!(child.format_after(0), original.format_after(0));
            assert_eq!(child.format_after(1), source.format_after(0), "preserve the copied A run");
            assert_eq!(child.format_after(2), source.format_after(1), "preserve the actual copied LF run");
            assert_eq!(child.paras[0], original.paras[0]);
            assert_eq!(child.paras[1].chars, source.paras[0].chars, "restore genuine defaults, including a nonempty inherited baseline");
            assert!(child.paras[1].eof_inherited_chars.is_none());
            assert!(!child.slice_has_eof_stage);
            if suffix {
                assert_eq!(child.format_after(3), original.format_after(1));
            }
            assert_eq!(s.doc().unwrap().doc.story(src).unwrap(), &source, "copy and transport do not edit the source");
            let [zip, legacy] = native_copies(&mut s);
            for mut back in [s, zip, legacy] {
                r13_type_clear(&mut back, fid, sid, cell, 3, inherited);
                assert_eq!(back.doc().unwrap().doc.story(src).unwrap(), &source);
            }
        }
    }
}

#[test]
fn r13_eof_child_formatted_paste_cell_restores_baseline() {
    r13_paste_stage(false);
}

#[test]
fn r13_eof_child_formatted_paste_note_restores_baseline() {
    r13_paste_stage(true);
}

#[test]
fn r13_eof_child_table_creation_restores_baseline() {
    for inherited in [false, true] {
        for convert in [false, true] {
            let (mut s, fid, sid) = staged_source(inherited, true);
            let source = s.doc().unwrap().doc.story(sid).unwrap().clone();
            let command = if convert { "table.convertFromText" } else { "table.insert" };
            let result = edit(&mut s, command, &json!({"rows": 2, "cols": 2, "width": 300}));
            let tid = result["table"].as_u64().unwrap();
            let parent = s.doc().unwrap().doc.story(sid).unwrap();
            assert!(parent.text.starts_with("A\n"));
            assert_eq!(parent.format_after(0), source.format_after(0));
            assert_eq!(parent.format_after(1), source.format_after(1));
            assert_eq!(parent.paras.last(), source.paras.last(), "the outer story's remaining true EOF keeps its stage");
            let table = &parent.tables[&tid];
            let cells = table.cells.clone();
            for cell in &cells {
                assert!(cell.text.is_empty());
                assert_eq!(cell.text.chars[0].format, *source.char_format_at(source.len()), "keep the actual source run as the cell's initial run");
                assert!(!cell.text.slice_has_eof_stage);
            }
            let cell = designcraft_doc::CellAddr { table: tid, row: 0, col: 0 };
            let [zip, legacy] = native_copies(&mut s);
            for mut back in [s, zip, legacy] {
                r13_type_clear(&mut back, fid, sid, cell, 0, inherited);
                let table = &back.doc().unwrap().doc.story(sid).unwrap().tables[&tid];
                for cell in &table.cells {
                    assert_eq!(cell.text.paras[0].chars, source.paras[0].chars);
                    assert!(cell.text.paras[0].eof_inherited_chars.is_none());
                }
                assert_eq!(table.cells[1..], cells[1..], "other newly created cells remain exact");
            }
        }
    }
}

#[test]
fn r13_eof_child_ordinary_table_defaults_survive_control() {
    for convert in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (fid, sid) = frame(&mut s, "A", 40.0);
        local_defaults(&mut s, sid, 0..1);
        select(&mut s, sid, 0..1);
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        let source = s.doc().unwrap().doc.story(sid).unwrap().clone();
        select(&mut s, sid, 1..1);
        let command = if convert { "table.convertFromText" } else { "table.insert" };
        let result = edit(&mut s, command, &json!({"rows": 1, "cols": 1, "width": 300}));
        let cell = designcraft_doc::CellAddr { table: result["table"].as_u64().unwrap(), row: 0, col: 0 };
        let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
        assert_eq!(child.paras[0].chars, source.paras[0].chars);
        assert!(child.paras[0].eof_inherited_chars.is_none());
        let at = child.len();
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            r13_type_clear(&mut back, fid, sid, cell, at, true);
        }
    }
}

#[test]
fn r13_eof_child_clipped_and_nonblank_source_defaults_survive_control() {
    for note in [false, true] {
        for end in [2, 3] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let (_, src) = frame(&mut s, "A\nQ", 40.0);
            local_defaults(&mut s, src, 2..3);
            select(&mut s, src, 0..3);
            s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
            let source = s.doc().unwrap().doc.story(src).unwrap().clone();
            copy(&mut s, src, 0..end);
            let (fid, sid, cell) = r13_child(&mut s, note, "D");
            edit(&mut s, "edit.paste", &json!({"formatted": true}));
            let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
            assert_eq!(child.paras[1], source.paras[1], "clipped LF and nonblank sources contain genuine paragraph defaults");
            for byte in 0..end {
                assert_eq!(child.format_after(1 + byte), source.format_after(byte));
            }
            let [zip, legacy] = native_copies(&mut s);
            for mut back in [s, zip, legacy] {
                r13_type_clear(&mut back, fid, sid, cell, 1 + end, true);
                assert_eq!(back.doc().unwrap().doc.story(src).unwrap(), &source);
            }
        }
    }
}

#[test]
fn r13_eof_child_child_source_blank_defaults_survive_control() {
    for note in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (sf, src, sc) = r13_child(&mut s, note, "A\n");
        s.execute("style.paragraph.create", &json!({"name": "R13 child base", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
            .unwrap();
        r13_select_child(&mut s, sf, src, sc, 0..2);
        s.execute("style.paragraph.apply", &json!({"name": "R13 child base"})).unwrap();
        s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        let source = s.doc().unwrap().doc.text_story(src, Some(sc)).unwrap().clone();
        assert_eq!(source.paras[1].chars.size, Some(24.0));
        assert!(source.paras[1].eof_inherited_chars.is_none());
        s.execute("edit.copy", &json!({})).unwrap();
        let (fid, sid, cell) = r13_child(&mut s, !note, "D");
        edit(&mut s, "edit.paste", &json!({"formatted": true}));
        let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
        assert_eq!(child.paras[1], source.paras[1], "a child EOF never carries an outer-story stage");
        assert_eq!(child.format_after(1), source.format_after(0));
        assert_eq!(child.format_after(2), source.format_after(1));
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            r13_type_clear(&mut back, fid, sid, cell, 3, true);
            assert_eq!(back.doc().unwrap().doc.text_story(src, Some(sc)).unwrap(), &source);
        }
    }
}

#[test]
fn r13_eof_child_note_creation_uses_note_defaults_control() {
    for inherited in [false, true] {
        let (mut s, fid, sid) = staged_source(inherited, true);
        let result = edit(&mut s, "footnote.insert", &json!({}));
        let cell = designcraft_doc::CellAddr::footnote(result["id"].as_u64().unwrap());
        let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
        assert!(child.paras[0].chars.is_empty() && child.paras[0].eof_inherited_chars.is_none());
        assert!(child.chars[0].format.over.is_empty());
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            r13_select_child(&mut back, fid, sid, cell, 0..0);
            edit(&mut back, "text.insert", &json!({"text": "X", "raw": true}));
            r13_select_child(&mut back, fid, sid, cell, 0..1);
            r13_metrics(&mut back, 12.0, json!({"kind": "auto"}), 0);
            edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
            r13_metrics(&mut back, 12.0, json!({"kind": "auto"}), 0);
            native_copies(&mut back);
        }
        let (mut s, _, sid) = staged_source(inherited, true);
        let result = edit(&mut s, "endnote.insert", &json!({"text": "X\n"}));
        let id = result["id"].as_u64().unwrap();
        let [zip, legacy] = native_copies(&mut s);
        for back in [s, zip, legacy] {
            let doc = &back.doc().unwrap().doc;
            let note = doc.story(sid).unwrap().endnotes.iter().find(|note| note.id == id).unwrap();
            assert_eq!(note.text.text, "X\n");
            assert!(note.text.paras.iter().all(|pf| pf.chars.is_empty() && pf.eof_inherited_chars.is_none()));
            assert!(!note.text.slice_has_eof_stage);
            let base = doc.styles.resolve_para(&note.text.paras[0]).1;
            let chars = doc.styles.resolve_char(&base, note.text.format_after(0));
            assert_eq!(chars.size, 12.0);
            assert_eq!(chars.leading, designcraft_doc::Leading::Auto);
        }
    }
}

// R14: genuine defaults in a non-stage slice must remain inherited when it
// becomes a blank outer EOF. All document mutations use public Session commands.
#[derive(Clone, Copy)]
enum R14SourceKind {
    Clipped,
    Cell,
    Note,
}

fn r14_copy_source(kind: R14SourceKind) -> (Session, ItemId, StoryId, Option<designcraft_doc::CellAddr>) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (fid, sid, cell) = match kind {
        R14SourceKind::Clipped => {
            let (fid, sid) = frame(&mut s, "A\nQ", 40.0);
            (fid, sid, None)
        }
        R14SourceKind::Cell | R14SourceKind::Note => {
            let (fid, sid, cell) = r13_child(&mut s, matches!(kind, R14SourceKind::Note), "A\n");
            (fid, sid, Some(cell))
        }
    };
    s.execute("style.paragraph.create", &json!({"name": "R14 baseline", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}})).unwrap();
    if let Some(cell) = cell {
        r13_select_child(&mut s, fid, sid, cell, 0..2);
    } else {
        select(&mut s, sid, 2..3);
    }
    s.execute("style.paragraph.apply", &json!({"name": "R14 baseline"})).unwrap();
    s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    if cell.is_none() {
        select(&mut s, sid, 0..3);
    }
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
    let source = s.doc().unwrap().doc.text_story(sid, cell).unwrap();
    assert_eq!(source.paras[1].chars.size, Some(24.0));
    assert!(source.paras[1].eof_inherited_chars.is_none());
    if cell.is_none() {
        select(&mut s, sid, 0..2);
    }
    s.execute("edit.copy", &json!({})).unwrap();
    (s, fid, sid, cell)
}

struct R14Transfer {
    frame: ItemId,
    story: StoryId,
    source_id: StoryId,
    source_cell: Option<designcraft_doc::CellAddr>,
    source: designcraft_doc::Story,
}

fn r14_pasted(kind: R14SourceKind, interior: bool) -> (Session, R14Transfer) {
    let (mut s, _, source_id, source_cell) = r14_copy_source(kind);
    let source = s.doc().unwrap().doc.text_story(source_id, source_cell).unwrap().clone();
    let (fid, sid) = frame(&mut s, if interior { "DB" } else { "D" }, 500.0);
    let destination = s.doc().unwrap().doc.story(sid).unwrap().clone();
    select(&mut s, sid, 1..1);
    edit(&mut s, "edit.paste", &json!({"formatted": true}));
    let pasted = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(pasted.text, if interior { "DA\nB" } else { "DA\n" });
    assert_eq!(pasted.paras[0], destination.paras[0]);
    assert_eq!(pasted.format_after(0), destination.format_after(0));
    assert_eq!(pasted.format_after(1), source.format_after(0));
    assert_eq!(pasted.format_after(2), source.format_after(1), "keep the actual copied LF run");
    if interior {
        assert_eq!(pasted.paras[1], source.paras[1], "an interior transfer keeps genuine paragraph defaults");
        assert_eq!(pasted.format_after(3), destination.format_after(1));
    }
    let transfer = R14Transfer { frame: fid, story: sid, source_id, source_cell, source };
    r14_prefix_and_source(&s, &transfer);
    (s, transfer)
}

fn r14_prefix_and_source(s: &Session, transfer: &R14Transfer) {
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.text_story(transfer.source_id, transfer.source_cell).unwrap(), &transfer.source);
    let pasted = d.story(transfer.story).unwrap();
    assert!(pasted.text.starts_with("DA\n"));
    assert_eq!(pasted.format_after(1), transfer.source.format_after(0));
    assert_eq!(pasted.format_after(2), transfer.source.format_after(1));
}

fn r14_type_clear(s: &mut Session, transfer: &R14Transfer) {
    let sid = transfer.story;
    let q = query(s, sid, 3..3, 20.0, json!({"kind": "points", "value": 36.0}));
    assert_eq!(q["charOverrides"], 2, "the actual LF's overrides remain the typing format");
    edit(s, "text.insert", &json!({"text": "é", "raw": true}));
    query(s, sid, 3..5, 20.0, json!({"kind": "points", "value": 36.0}));
    edit(s, "type.char", &json!({"attrs": {"size": null}}));
    query(s, sid, 3..5, 24.0, json!({"kind": "points", "value": 36.0}));
    edit(s, "type.char", &json!({"attrs": {"leading": null}}));
    let q = query(s, sid, 3..5, 24.0, json!({"kind": "points", "value": 26.0}));
    assert_eq!(q["charOverrides"], 0);
    let pasted = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(pasted.paras[1], transfer.source.paras[1], "typing restores the complete genuine paragraph baseline");
    assert!(pasted.format_after(3).over.is_empty());
    r14_prefix_and_source(s, transfer);
    for mut back in native_copies(s) {
        query(&mut back, sid, 3..5, 24.0, json!({"kind": "points", "value": 26.0}));
        r14_prefix_and_source(&back, transfer);
    }
}

fn r14_source_typing_case(kind: R14SourceKind) {
    let (mut s, transfer) = r14_pasted(kind, false);
    // Reopen before any typing: the baseline/stage distinction must be persisted.
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        r14_type_clear(&mut back, &transfer);
    }
}

#[test]
fn r14_eof_nonstage_clipped_source_typing_clear_and_native_history() {
    r14_source_typing_case(R14SourceKind::Clipped);
}

#[test]
fn r14_eof_nonstage_cell_source_typing_clear_and_native_history() {
    r14_source_typing_case(R14SourceKind::Cell);
}

#[test]
fn r14_eof_nonstage_note_source_typing_clear_and_native_history() {
    r14_source_typing_case(R14SourceKind::Note);
}

#[test]
fn r14_eof_nonstage_formatted_insert_and_table_consumers_preserve_baseline() {
    for kind in [R14SourceKind::Clipped, R14SourceKind::Cell, R14SourceKind::Note] {
        for consumer in ["paste", "place", "table.insert", "table.convertFromText"] {
            let (mut s, transfer) = r14_pasted(kind, false);
            let [zip, legacy] = native_copies(&mut s);
            for mut back in [s, zip, legacy] {
                if consumer == "paste" || consumer == "place" {
                    if consumer == "paste" {
                        let (_, donor) = frame(&mut back, "Z", 40.0);
                        select(&mut back, donor, 0..1);
                        back.execute("type.char", &json!({"attrs": {"size": 31, "leading": {"kind": "points", "value": 41}}})).unwrap();
                        copy(&mut back, donor, 0..1);
                    }
                    select(&mut back, transfer.story, 3..3);
                    if consumer == "paste" {
                        edit(&mut back, "edit.paste", &json!({"formatted": true}));
                        query(&mut back, transfer.story, 3..4, 31.0, json!({"kind": "points", "value": 41.0}));
                    } else {
                        let bytes = crate::cmd::file::base64_encode(b"Z");
                        edit(&mut back, "file.place", &json!({"name": "r14.txt", "base64": bytes}));
                        query(&mut back, transfer.story, 3..4, 24.0, json!({"kind": "points", "value": 26.0}));
                    }
                    edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
                    let q = query(&mut back, transfer.story, 3..4, 24.0, json!({"kind": "points", "value": 26.0}));
                    assert_eq!(q["charOverrides"], 0);
                    assert_eq!(back.doc().unwrap().doc.story(transfer.story).unwrap().paras[1], transfer.source.paras[1]);
                } else {
                    select(&mut back, transfer.story, 3..3);
                    let eof = back.doc().unwrap().doc.story(transfer.story).unwrap().paras.last().unwrap().clone();
                    let result = edit(&mut back, consumer, &json!({"rows": 1, "cols": 1, "width": 300}));
                    let tid = result["table"].as_u64().unwrap();
                    let outer = back.doc().unwrap().doc.story(transfer.story).unwrap();
                    assert_eq!(outer.paras.last(), Some(&eof), "the remaining outer EOF retains its typing stage");
                    let cell = outer.tables[&tid].cell(0, 0).unwrap();
                    assert_eq!(cell.text.paras[0], transfer.source.paras[1], "new cells keep genuine defaults, not the typing stage");
                    assert_eq!(cell.text.chars[0].format, *transfer.source.format_after(1));
                    let address = designcraft_doc::CellAddr { table: tid, row: 0, col: 0 };
                    r13_type_clear(&mut back, transfer.frame, transfer.story, address, 0, true);
                }
                r14_prefix_and_source(&back, &transfer);
                for reopened in native_copies(&mut back) {
                    r14_prefix_and_source(&reopened, &transfer);
                }
            }
        }
    }
}

#[test]
fn r14_eof_nonstage_interior_transfer_remains_inherited_control() {
    for kind in [R14SourceKind::Clipped, R14SourceKind::Cell, R14SourceKind::Note] {
        let (mut s, transfer) = r14_pasted(kind, true);
        query(&mut s, transfer.story, 3..4, 24.0, json!({"kind": "points", "value": 26.0}));
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            r14_type_clear(&mut back, &transfer);
        }
    }
}

#[test]
fn r14_eof_true_source_stage_stays_distinct_from_actual_lf_control() {
    let (mut s, _, src) = staged_source(true, true);
    edit(&mut s, "type.char", &json!({"attrs": {"size": 28, "leading": {"kind": "points", "value": 38}}}));
    let source = s.doc().unwrap().doc.story(src).unwrap().clone();
    assert_eq!(source.format_after(1).over.size, Some(20.0));
    copy(&mut s, src, 0..2);
    let (_, dst) = frame(&mut s, "D", 500.0);
    select(&mut s, dst, 1..1);
    edit(&mut s, "edit.paste", &json!({"formatted": true}));
    let pasted = s.doc().unwrap().doc.story(dst).unwrap();
    assert_eq!(pasted.paras.last(), source.paras.last(), "a true source stage must not be replaced by its LF's format");
    assert_eq!(pasted.format_after(2), source.format_after(1));
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        query(&mut back, dst, 3..3, 28.0, json!({"kind": "points", "value": 38.0}));
        edit(&mut back, "text.insert", &json!({"text": "é", "raw": true}));
        query(&mut back, dst, 3..5, 28.0, json!({"kind": "points", "value": 38.0}));
        edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        query(&mut back, dst, 3..5, 24.0, json!({"kind": "points", "value": 26.0}));
        assert_eq!(back.doc().unwrap().doc.story(src).unwrap(), &source);
        assert_eq!(back.doc().unwrap().doc.story(dst).unwrap().format_after(2), source.format_after(1));
    }
}

#[test]
fn r14_eof_nonstage_empty_paragraph_defaults_keep_existing_policy_control() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let (_, src) = frame(&mut s, "A\nQ", 40.0);
    select(&mut s, src, 0..3);
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
    let source = s.doc().unwrap().doc.story(src).unwrap().clone();
    copy(&mut s, src, 0..2);
    let (_, dst) = frame(&mut s, "D", 500.0);
    select(&mut s, dst, 1..1);
    edit(&mut s, "edit.paste", &json!({"formatted": true}));
    assert_eq!(s.doc().unwrap().doc.story(dst).unwrap().paras[1], source.paras[1]);
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        select(&mut back, dst, 3..3);
        edit(&mut back, "text.insert", &json!({"text": "é", "raw": true}));
        query(&mut back, dst, 3..5, 20.0, json!({"kind": "points", "value": 36.0}));
        edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        query(&mut back, dst, 3..5, 12.0, json!({"kind": "auto"}));
    }
}

#[test]
fn r14_eof_nonstage_selected_drag_move_and_copy_preserve_baseline() {
    for copy in [false, true] {
        let (mut s, fid, sid, _) = r14_copy_source(R14SourceKind::Clipped);
        let source = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let press = glyph_point(&s, fid, sid, 0);
        let drop = {
            let d = &s.doc().unwrap().doc;
            let cs = s.cache.get(d, sid, None);
            let line = cs.frame(fid).unwrap().lines.last().unwrap();
            let point = Point::new(line.end_x + 12.0, line.baseline - line.ascent * 0.5);
            d.parent_xf(&d.find(fid).unwrap()) * d.text_xf(d.item(fid).unwrap()) * point
        };
        let armed = s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [press.x, press.y]})).unwrap();
        assert_eq!(armed["dragging"], true);
        edit(&mut s, "text.release", &json!({"frame": fid.0, "point": [drop.x, drop.y], "moved": true, "copy": copy}));
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, if copy { "A\nQA\n" } else { "QA\n" });
        let at = story.len();
        assert_eq!(story.format_after(at - 1), source.format_after(1), "drag keeps the real copied LF run");
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            select(&mut back, sid, at..at);
            edit(&mut back, "text.insert", &json!({"text": "é", "raw": true}));
            query(&mut back, sid, at..at + 2, 20.0, json!({"kind": "points", "value": 36.0}));
            edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
            query(&mut back, sid, at..at + 2, 24.0, json!({"kind": "points", "value": 26.0}));
            let story = back.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(story.paras.last(), source.paras.last());
            assert_eq!(story.format_after(at - 1), source.format_after(1));
        }
    }
}

// R15: newly exposed EOFs and note references must preserve the distinction
// between direct typing attrs and genuine paragraph defaults.
fn r15_tail_source() -> (Session, StoryId, designcraft_doc::ParaFormat) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (_, sid) = frame(&mut s, "A\nB", 40.0);
    local_defaults(&mut s, sid, 2..3);
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[1].clone();
    assert_eq!(baseline.chars.size, Some(24.0));
    assert_eq!(baseline.chars.leading, Some(designcraft_doc::Leading::Points(26.0)));
    assert!(baseline.eof_inherited_chars.is_none());
    select(&mut s, sid, 0..3);
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
    query(&mut s, sid, 2..3, 20.0, json!({"kind": "points", "value": 36.0}));
    (s, sid, baseline)
}

fn r15_type_clear_eof(s: &mut Session, sid: StoryId, baseline: &designcraft_doc::ParaFormat, size: f64, leading: f64) {
    let at = s.doc().unwrap().doc.story(sid).unwrap().len();
    query(s, sid, at..at, size, json!({"kind": "points", "value": leading}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.paras.last().unwrap().eof_inherited_chars.as_ref(), Some(&baseline.chars));
    let lf = st.format_after(at - 1).clone();
    let [mut zip, mut legacy] = native_copies(s);
    for back in [&mut zip, &mut legacy] {
        query(back, sid, at..at, size, json!({"kind": "points", "value": leading}));
    }
    select(s, sid, at..at);
    edit(s, "text.insert", &json!({"text": "é", "raw": true}));
    let q = query(s, sid, at..at + 2, size, json!({"kind": "points", "value": leading}));
    assert_eq!(q["charOverrides"], 2);
    let expected = s.doc().unwrap().doc.styles.resolve_para(baseline).1;
    edit(s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    let q = query(s, sid, at..at + 2, expected.size, serde_json::to_value(expected.leading).unwrap());
    assert_eq!(q["charOverrides"], 0, "clearing exposes the genuine baseline");
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.paras.last(), Some(baseline), "the complete paragraph format survives consumption");
    assert_eq!(st.format_after(at - 1), &lf, "typing and clearing do not reformat the actual LF");
    native_copies(s);
}

fn r15_delete_tail(command: &str, params: Value, range: std::ops::Range<usize>) {
    let (mut s, sid, baseline) = r15_tail_source();
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    select(&mut s, sid, range);
    let result = edit(&mut s, command, &params);
    if command == "edit.cut" {
        assert_eq!(result["text"], "B");
    }
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\n");
    assert_eq!(st.format_after(0), before.format_after(0));
    assert_eq!(st.format_after(1), before.format_after(1));
    r15_type_clear_eof(&mut s, sid, &baseline, 20.0, 36.0);
}

#[test]
fn r15_eof_transition_clear_exposes_tail() {
    r15_delete_tail("edit.clear", json!({}), 2..3);
}

#[test]
fn r15_eof_transition_cut_exposes_tail() {
    r15_delete_tail("edit.cut", json!({}), 2..3);
}

#[test]
fn r15_eof_transition_selected_delete_exposes_tail() {
    r15_delete_tail("text.delete", json!({}), 2..3);
}

#[test]
fn r15_eof_transition_backspace_exposes_tail() {
    r15_delete_tail("text.delete", json!({}), 3..3);
}

#[test]
fn r15_eof_transition_forward_delete_exposes_tail() {
    r15_delete_tail("text.delete", json!({"forward": true}), 2..2);
}

#[test]
fn r15_eof_transition_selected_replacement_ending_lf() {
    for crosses_paragraph in [false, true] {
        let (mut s, sid, mut baseline) = r15_tail_source();
        if crosses_paragraph {
            s.execute(
                "style.paragraph.create",
                &json!({"name": "R15 first base", "chars": {"size": 32, "leading": {"kind": "points", "value": 34}}}),
            )
            .unwrap();
            select(&mut s, sid, 0..1);
            s.execute("style.paragraph.apply", &json!({"name": "R15 first base"})).unwrap();
            s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
            baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
            assert_eq!(baseline.chars.size, Some(32.0));
        }
        query(&mut s, sid, 2..3, 20.0, json!({"kind": "points", "value": 36.0}));
        select(&mut s, sid, if crosses_paragraph { 1..3 } else { 2..3 });
        edit(&mut s, "text.insert", &json!({"text": if crosses_paragraph { "\n" } else { "Q\n" }, "raw": true}));
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, if crosses_paragraph { "A\n" } else { "A\nQ\n" });
        r15_type_clear_eof(&mut s, sid, &baseline, 20.0, 36.0);
    }
}

#[test]
fn r15_eof_transition_existing_stage_survives_prefix_delete_control() {
    let (mut s, _, sid) = staged_source(true, true);
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
    edit(&mut s, "type.char", &json!({"attrs": {"size": 28, "leading": {"kind": "points", "value": 38}}}));
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    select(&mut s, sid, 0..1);
    edit(&mut s, "edit.clear", &json!({}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "\n");
    assert_eq!(st.paras.last(), before.paras.last(), "the old final LF and its explicit stage survive");
    assert_eq!(st.format_after(0), before.format_after(1));
    r15_type_clear_eof(&mut s, sid, &baseline, 28.0, 38.0);
}

fn r15_note_body(s: &Session, sid: StoryId, id: u64, endnote: bool) -> designcraft_doc::Story {
    let d = &s.doc().unwrap().doc;
    let st = d.story(sid).unwrap();
    let notes = if endnote { &st.endnotes } else { &st.notes };
    let body = &notes.iter().find(|note| note.id == id).unwrap().text;
    assert_eq!(body.text, "note");
    assert!(body.paras[0].chars.is_empty() && body.paras[0].eof_inherited_chars.is_none());
    let base = d.styles.resolve_para(&body.paras[0]).1;
    let chars = d.styles.resolve_char(&base, body.format_after(0));
    assert_eq!(chars.size, 12.0, "the child body uses its own defaults");
    assert_eq!(chars.leading, designcraft_doc::Leading::Auto);
    body.clone()
}

fn r15_note_consumes_stage(endnote: bool) {
    for inherited in [false, true] {
        let (mut s, _, sid) = staged_source(inherited, true);
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let baseline = original.paras[0].clone();
        let (size, leading) = if inherited { (28.0, 38.0) } else { (20.0, 36.0) };
        if inherited {
            edit(&mut s, "type.char", &json!({"attrs": {"size": size, "leading": {"kind": "points", "value": leading}}}));
        }
        query(&mut s, sid, 2..2, size, json!({"kind": "points", "value": leading}));
        let result = edit(&mut s, if endnote { "endnote.insert" } else { "footnote.insert" }, &json!({"text": "note"}));
        let id = result["id"].as_u64().unwrap();
        let body = r15_note_body(&s, sid, id, endnote);
        if !endnote {
            s.execute("footnote.goToReference", &json!({})).unwrap();
        }
        let at = s.doc().unwrap().selection.text.unwrap().focus;
        let marker = if endnote { designcraft_doc::ENDNOTE_REF } else { designcraft_doc::FOOTNOTE_REF };
        assert_eq!(at, 2 + marker.len_utf8());
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, format!("A\n{marker}"));
        query(&mut s, sid, 2..at, size, json!({"kind": "points", "value": leading}));
        let st = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.paras.last(), Some(&baseline), "the marker consumes the outer stage");
        assert_eq!(st.format_after(1), original.format_after(1));
        let [mut zip, mut legacy] = native_copies(&mut s);
        for back in [&mut zip, &mut legacy] {
            query(back, sid, 2..at, size, json!({"kind": "points", "value": leading}));
            assert_eq!(r15_note_body(back, sid, id, endnote), body);
        }
        select(&mut s, sid, at..at);
        edit(&mut s, "text.insert", &json!({"text": "é", "raw": true}));
        let q = query(&mut s, sid, at..at + 2, size, json!({"kind": "points", "value": leading}));
        assert_eq!(q["charOverrides"], 2);
        edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        let (base_size, base_leading) = r13_baseline(inherited);
        let q = query(&mut s, sid, at..at + 2, base_size, base_leading);
        assert_eq!(q["charOverrides"], 0);
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras.last(), Some(&baseline));
        assert_eq!(r15_note_body(&s, sid, id, endnote), body, "outer editing does not change the child body");
        native_copies(&mut s);
    }
}

#[test]
fn r15_eof_transition_footnote_consumes_stage() {
    r15_note_consumes_stage(false);
}

#[test]
fn r15_eof_transition_endnote_consumes_stage() {
    r15_note_consumes_stage(true);
}

#[test]
fn r15_eof_transition_ordinary_note_insertion_control() {
    for endnote in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (_, sid) = frame(&mut s, "AB", 40.0);
        local_defaults(&mut s, sid, 0..2);
        select(&mut s, sid, 0..2);
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        select(&mut s, sid, 1..1);
        let result = edit(&mut s, if endnote { "endnote.insert" } else { "footnote.insert" }, &json!({"text": "note"}));
        let marker = if endnote { designcraft_doc::ENDNOTE_REF } else { designcraft_doc::FOOTNOTE_REF };
        let end = 1 + marker.len_utf8();
        query(&mut s, sid, 1..end, 20.0, json!({"kind": "points", "value": 36.0}));
        let st = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.text, format!("A{marker}B"));
        assert_eq!(st.paras, original.paras);
        assert_eq!(st.format_after(end), original.format_after(1));
        r15_note_body(&s, sid, result["id"].as_u64().unwrap(), endnote);
        edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        let q = query(&mut s, sid, 1..end, 24.0, json!({"kind": "points", "value": 26.0}));
        assert_eq!(q["charOverrides"], 0);
        native_copies(&mut s);
    }
}

#[test]
fn r15_eof_transition_child_tail_delete_keeps_body_policy_control() {
    for note in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (fid, sid, cell) = r13_child(&mut s, note, "A\nB");
        s.execute("style.paragraph.create", &json!({"name": "R15 child base", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
            .unwrap();
        r13_select_child(&mut s, fid, sid, cell, 2..3);
        s.execute("style.paragraph.apply", &json!({"name": "R15 child base"})).unwrap();
        s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
        let baseline = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap().paras[1].clone();
        r13_select_child(&mut s, fid, sid, cell, 0..3);
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        r13_select_child(&mut s, fid, sid, cell, 2..3);
        r13_metrics(&mut s, 20.0, json!({"kind": "points", "value": 36.0}), 2);
        edit(&mut s, "edit.clear", &json!({}));
        let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
        assert_eq!(child.text, "A\n");
        assert_eq!(child.paras.last(), Some(&baseline));
        assert!(child.paras.last().unwrap().eof_inherited_chars.is_none());
        native_copies(&mut s);
        r13_type_clear(&mut s, fid, sid, cell, 2, true);
        assert_eq!(s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap().paras.last(), Some(&baseline));
    }
}

// R16: three public-command regressions at existing C5 EOF consumers.
#[test]
fn r16_eof_object_paste_consumes_typing_stage() {
    let (mut s, _, sid) = staged_source(true, true);
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
    edit(&mut s, "type.char", &json!({"attrs": {"size": 28, "leading": {"kind": "points", "value": 38}}}));
    let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let made = s.execute("frame.create", &json!({"rect": [300,300,340,340], "content": "unassigned", "shape": "ellipse"})).unwrap();
    s.execute("selection.set", &json!({"ids": [made["id"]]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    select(&mut s, sid, 2..2);
    let pasted = edit(&mut s, "edit.paste", &json!({}));
    assert_eq!(pasted["anchored"], 1);
    let at = pasted["pos"].as_u64().unwrap() as usize;
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, format!("A\n{}", designcraft_doc::OBJECT_MARK));
    assert_eq!(story.paras.last(), Some(&baseline), "object insertion consumes only the typing stage");
    assert_eq!(story.format_after(1), original.format_after(1), "the actual LF remains unchanged");
    assert_eq!(story.objects.len(), 1);
    query(&mut s, sid, 2..at, 28.0, json!({"kind": "points", "value": 38.0}));
    native_copies(&mut s);
    select(&mut s, sid, at..at);
    edit(&mut s, "text.insert", &json!({"text": "Z", "raw": true}));
    query(&mut s, sid, at..at + 1, 28.0, json!({"kind": "points", "value": 38.0}));
    edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    let q = query(&mut s, sid, at..at + 1, 24.0, json!({"kind": "points", "value": 26.0}));
    assert_eq!(q["charOverrides"], 0);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras.last(), Some(&baseline));
    native_copies(&mut s);

    // Negative control: replacement exposes a blank EOF only after deleting B.
    // Its genuine defaults must not be mistaken for a pre-existing typing stage.
    let (_, replaced) = frame(&mut s, "A\nB", 500.0);
    select(&mut s, replaced, 2..3);
    s.execute("style.paragraph.apply", &json!({"name": "R12 local base"})).unwrap();
    s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    let baseline = s.doc().unwrap().doc.story(replaced).unwrap().paras[1].clone();
    select(&mut s, replaced, 0..3);
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
    select(&mut s, replaced, 2..3);
    let pasted = edit(&mut s, "edit.paste", &json!({}));
    let end = pasted["pos"].as_u64().unwrap() as usize;
    assert_eq!(s.doc().unwrap().doc.story(replaced).unwrap().text, format!("A\n{}", designcraft_doc::OBJECT_MARK));
    query(&mut s, replaced, 2..end, 20.0, json!({"kind": "points", "value": 36.0}));
    edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    query(&mut s, replaced, 2..end, 24.0, json!({"kind": "points", "value": 26.0}));
    assert_eq!(s.doc().unwrap().doc.story(replaced).unwrap().paras.last(), Some(&baseline));
    native_copies(&mut s);
}

#[test]
fn r16_eof_tail_drag_preserves_genuine_defaults() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (fid, sid) = frame(&mut s, "A\nB", 40.0);
    local_defaults(&mut s, sid, 2..3);
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[1].clone();
    select(&mut s, sid, 0..3);
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
    let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let press = glyph_point(&s, fid, sid, 2);
    let drop = glyph_point(&s, fid, sid, 0);
    select(&mut s, sid, 2..3);
    let armed = s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [press.x, press.y]})).unwrap();
    assert_eq!(armed["dragging"], true);
    edit(&mut s, "text.release", &json!({"frame": fid.0, "point": [drop.x, drop.y], "moved": true, "copy": false}));
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, "BA\n");
    assert_eq!(story.paras[0], original.paras[0]);
    assert_eq!(story.paras[1].eof_inherited_chars.as_ref(), Some(&baseline.chars));
    assert_eq!(story.format_after(0), original.format_after(2));
    assert_eq!(story.format_after(2), original.format_after(1));
    query(&mut s, sid, 3..3, 20.0, json!({"kind": "points", "value": 36.0}));
    native_copies(&mut s);
    select(&mut s, sid, 3..3);
    edit(&mut s, "text.insert", &json!({"text": "Z", "raw": true}));
    query(&mut s, sid, 3..4, 20.0, json!({"kind": "points", "value": 36.0}));
    edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    let q = query(&mut s, sid, 3..4, 24.0, json!({"kind": "points", "value": 26.0}));
    assert_eq!(q["charOverrides"], 0);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras.last(), Some(&baseline));
    native_copies(&mut s);
}

#[test]
fn r16_eof_font_replace_updates_typing_and_inherited_fonts() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (_, sid) = frame(&mut s, "A", 40.0);
    s.execute(
        "style.paragraph.create",
        &json!({"name": "R16 Font Base", "chars": {"fontFamily": "R16 Old Font", "size": 24, "leading": {"kind": "points", "value": 26}}}),
    )
    .unwrap();
    select(&mut s, sid, 0..1);
    s.execute("style.paragraph.apply", &json!({"name": "R16 Font Base"})).unwrap();
    s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
    s.execute(
        "type.char",
        &json!({"attrs": {"fontFamily": "R16 Old Font", "fontStyle": "Bold", "size": 20, "leading": {"kind": "points", "value": 36}}}),
    )
    .unwrap();
    select(&mut s, sid, 1..1);
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    edit(&mut s, "font.replace", &json!({"family": "R16 Old Font", "toFamily": "Source Sans 3", "redefineStyles": false}));
    let q = query(&mut s, sid, 2..2, 20.0, json!({"kind": "points", "value": 36.0}));
    assert_eq!(q["chars"]["fontFamily"], "Source Sans 3");
    assert_eq!(q["chars"]["fontStyle"], "Bold");
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.paras[0], baseline, "ordinary paragraph-default replacement policy is unchanged");
    let eof = story.paras.last().unwrap();
    assert_eq!(eof.chars.font_family.as_deref(), Some("Source Sans 3"));
    assert_eq!(eof.eof_inherited_chars.as_ref().unwrap().font_family.as_deref(), Some("Source Sans 3"));
    assert_eq!(story.format_after(1).over.font_family.as_deref(), Some("Source Sans 3"));
    native_copies(&mut s);
    select(&mut s, sid, 2..2);
    edit(&mut s, "text.insert", &json!({"text": "Z", "raw": true}));
    let q = query(&mut s, sid, 2..3, 20.0, json!({"kind": "points", "value": 36.0}));
    assert_eq!(q["chars"]["fontFamily"], "Source Sans 3");
    assert_eq!(q["chars"]["fontStyle"], "Bold");
    edit(&mut s, "type.char", &json!({"attrs": {"fontFamily": null, "fontStyle": null, "size": null, "leading": null}}));
    let q = query(&mut s, sid, 2..3, 24.0, json!({"kind": "points", "value": 26.0}));
    assert_eq!(q["chars"]["fontFamily"], "Source Sans 3", "clearing typing attrs exposes the replaced genuine baseline");
    assert_eq!(q["charOverrides"], 0);
    let mut expected = baseline;
    expected.chars.font_family = Some("Source Sans 3".into());
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras.last(), Some(&expected));
    native_copies(&mut s);
}

// R18 C5: existing outer markers must consume the same EOF stage as typed bytes.
#[derive(Clone, Copy)]
enum R18Marker {
    Editorial,
    Anchor,
    Xref,
}

impl R18Marker {
    fn command(self) -> &'static str {
        match self {
            Self::Editorial => "note.new",
            Self::Anchor => "anchor.create",
            Self::Xref => "xref.insert",
        }
    }

    fn text(self) -> String {
        use designcraft_doc::{ANCHOR_MARK, NOTE_MARK, XREF_MARK};
        match self {
            Self::Editorial => NOTE_MARK.to_string(),
            Self::Anchor => ANCHOR_MARK.to_string(),
            Self::Xref => XREF_MARK.to_string(),
        }
    }

    fn params(self, s: &mut Session) -> Value {
        match self {
            Self::Editorial => json!({"text": "Keep this comment", "author": "Editor"}),
            Self::Anchor => json!({"name": "EOF destination"}),
            Self::Xref => {
                let (_, target) = frame(s, "Target", 260.0);
                select(s, target, 0..0);
                let anchor = s.execute("anchor.create", &json!({"name": "Existing destination"})).unwrap();
                json!({"anchor": anchor["id"], "format": "Page Number"})
            }
        }
    }
}

fn r18_marker_type_clear(s: &mut Session, sid: StoryId, at: usize, baseline: &designcraft_doc::ParaFormat, inherited: bool) {
    select(s, sid, at..at);
    edit(s, "text.insert", &json!({"text": "é", "raw": true}));
    let q = query(s, sid, at..at + 2, 30.0, json!({"kind": "points", "value": 42.0}));
    assert_eq!(q["charOverrides"], 2);
    edit(s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    let (size, leading) = r13_baseline(inherited);
    let q = query(s, sid, at..at + 2, size, leading.clone());
    assert_eq!(q["charOverrides"], 0);
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.paras.last(), Some(baseline), "only genuine paragraph defaults remain");
    assert!(story.paras.last().unwrap().eof_inherited_chars.is_none());
    assert!(story.format_after(at).over.is_empty());
    for mut back in native_copies(s) {
        let q = query(&mut back, sid, at..at + 2, size, leading.clone());
        assert_eq!(q["charOverrides"], 0);
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().paras.last(), Some(baseline));
    }
}

fn r18_eof_marker(kind: R18Marker, selected: bool) {
    for inherited in [false, true] {
        let (mut s, _, sid) = staged_source(inherited, true);
        let params = kind.params(&mut s);
        select(&mut s, sid, 2..2);
        edit(&mut s, "type.char", &json!({"attrs": {"size": 30, "leading": {"kind": "points", "value": 42}}}));
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let baseline = original.paras[0].clone();
        let format =
            designcraft_doc::CharFormat { style: original.char_format_at(2).style.clone(), over: original.paras.last().unwrap().chars.clone() };
        assert_ne!(format, *original.format_after(1), "the staged attrs must conflict with the actual LF run");
        // New Note inserts at the selection's end; it does not replace the range.
        select(&mut s, sid, if selected { 0..2 } else { 2..2 });
        let result = edit(&mut s, kind.command(), &params);
        let marker_text = kind.text();
        let at = 2 + marker_text.len();
        let caret = s.doc().unwrap().selection.text.unwrap();
        assert_eq!((caret.story, caret.anchor, caret.focus, caret.cell), (sid, at, at, None));
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, format!("A\n{marker_text}"));
        assert_eq!(story.paras.last(), Some(&baseline), "the marker consumes the outer typing stage");
        assert_eq!(story.format_after(1), original.format_after(1), "the actual LF keeps its original format");
        for (offset, _) in marker_text.char_indices() {
            assert_eq!(story.format_after(2 + offset), &format, "each inserted marker retains the staged format");
        }
        match kind {
            R18Marker::Editorial => {
                let note = story.editorial.last().unwrap();
                assert_eq!(json!(note.id), result["id"]);
                assert_eq!((note.text.as_str(), note.author.as_str()), ("Keep this comment", "Editor"));
            }
            R18Marker::Anchor => assert_eq!(story.anchors.last().unwrap().name, "EOF destination"),
            R18Marker::Xref => {
                assert_eq!(json!(story.xrefs.last().unwrap().target), result["target"]);
                assert!(s.doc().unwrap().doc.find_anchor(result["target"].as_u64().unwrap()).is_some());
            }
        }
        let q = query(&mut s, sid, 2..at, 30.0, json!({"kind": "points", "value": 42.0}));
        assert_eq!(q["charOverrides"], 2);
        // Both public native readers must retain marker attrs and the baseline before typing.
        for mut back in native_copies(&mut s) {
            query(&mut back, sid, 2..at, 30.0, json!({"kind": "points", "value": 42.0}));
            r18_marker_type_clear(&mut back, sid, at, &baseline, inherited);
        }
        r18_marker_type_clear(&mut s, sid, at, &baseline, inherited);
    }
}

#[test]
fn r18_eof_editorial_note_consumes_stage_and_clears_to_baseline() {
    r18_eof_marker(R18Marker::Editorial, false);
}

#[test]
fn r18_eof_editorial_note_at_selected_end_consumes_stage_without_replacing_text() {
    r18_eof_marker(R18Marker::Editorial, true);
}

#[test]
fn r18_eof_text_anchor_consumes_stage_and_clears_to_baseline() {
    r18_eof_marker(R18Marker::Anchor, false);
}

#[test]
fn r18_eof_cross_reference_consumes_stage_and_clears_to_baseline() {
    r18_eof_marker(R18Marker::Xref, false);
}

#[test]
fn r18_marker_ordinary_formatting_and_xref_selected_tail_keep_genuine_defaults() {
    for kind in [R18Marker::Editorial, R18Marker::Anchor, R18Marker::Xref] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (_, sid) = frame(&mut s, "A\nB", 40.0);
        let params = kind.params(&mut s);
        local_defaults(&mut s, sid, 2..3);
        select(&mut s, sid, 0..3);
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        // Xref replaces B: deletion exposes an EOF whose defaults are not a typing stage.
        let replaces = matches!(kind, R18Marker::Xref);
        select(&mut s, sid, if replaces { 2..3 } else { 2..2 });
        edit(&mut s, kind.command(), &params);
        let text = kind.text();
        let at = 2 + text.len();
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, format!("A\n{text}{}", if replaces { "" } else { "B" }));
        assert_eq!(story.paras, original.paras, "ordinary paragraph defaults remain unchanged");
        assert_eq!(story.format_after(2), original.format_after(2));
        query(&mut s, sid, 2..at, 20.0, json!({"kind": "points", "value": 36.0}));
        edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        let q = query(&mut s, sid, 2..at, 24.0, json!({"kind": "points", "value": 26.0}));
        assert_eq!(q["charOverrides"], 0);
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras, original.paras);
        native_copies(&mut s);
    }
}

// R20: existing Story Editor and quote consumers must use the outer EOF typing stage.
fn r20_replace(s: &mut Session, params: &Value) -> Value {
    let journal = s.journal.len();
    let result = edit(s, "story.replaceRange", params);
    let edits: Vec<_> = s.journal[journal..].iter().filter(|(id, _)| !matches!(id.as_str(), "edit.undo" | "edit.redo")).collect();
    assert_eq!(edits.len(), 1, "range replacement must not dispatch a nested text.insert");
    assert_eq!(edits[0], &("story.replaceRange".to_string(), params.clone()));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Edit Story");
    result
}

#[test]
fn r20_eof_range_insert_unicode_consumes_stage_and_clears_to_genuine_baseline() {
    for inherited in [false, true] {
        for text in ["é界", "é\n界\n"] {
            let (mut s, _, sid) = staged_source(inherited, true);
            select(&mut s, sid, 2..2);
            s.execute("type.char", &json!({"attrs": {"size": 30, "leading": {"kind": "points", "value": 42}}})).unwrap();
            let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
            let baseline = original.paras[0].clone();
            assert_eq!(original.format_after(1).over.size, Some(20.0));
            assert_eq!(original.paras[1].chars.size, Some(30.0), "the EOF must conflict with its actual LF");
            // Story Editor's explicit range is independent of the canvas selection.
            select(&mut s, sid, 0..1);
            let selection = s.doc().unwrap().selection.clone();
            let end = 2 + text.len();
            let result = r20_replace(&mut s, &json!({"story": sid.0, "start": 2, "end": 2, "text": text}));
            assert_eq!(result, json!({"length": end}));
            assert_eq!(s.doc().unwrap().selection, selection, "do not move the canvas caret to the inserted text");
            let st = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(st.text, format!("A\n{text}"));
            assert_eq!(st.format_after(0), original.format_after(0));
            assert_eq!(st.format_after(1), original.format_after(1));
            for (offset, _) in text.char_indices() {
                let cf = st.format_after(2 + offset);
                assert_eq!(cf.style, original.char_format_at(2).style);
                assert_eq!(cf.over.size, Some(30.0));
                assert_eq!(cf.over.leading, Some(designcraft_doc::Leading::Points(42.0)));
            }
            for (pi, pf) in st.paras.iter().enumerate().skip(1) {
                if pi + 1 == st.paras.len() && text.ends_with('\n') {
                    assert_eq!(pf, &original.paras[1], "a new blank EOF retains the continuation stage");
                } else {
                    assert_eq!(pf, &baseline, "populated paragraphs regain their genuine inherited defaults");
                }
            }
            let [zip, legacy] = native_copies(&mut s);
            for mut back in [s, zip, legacy] {
                let q = query(&mut back, sid, 2..end, 30.0, json!({"kind": "points", "value": 42.0}));
                assert_eq!(q["charOverrides"], 2);
                edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
                let (size, leading) = r13_baseline(inherited);
                let q = query(&mut back, sid, 2..end, size, leading);
                assert_eq!(q["charOverrides"], 0, "clear must not expose the former 30-point typing stage");
                let st = back.doc().unwrap().doc.story(sid).unwrap();
                assert_eq!(st.paras[1], baseline);
                assert_eq!(st.format_after(1), original.format_after(1));
                if text.ends_with('\n') {
                    assert_eq!(st.paras.last(), original.paras.last(), "clearing bytes leaves the independent EOF stage");
                }
                native_copies(&mut back);
            }
        }
    }
}

#[test]
fn r20_eof_range_tail_delete_and_replacement_keep_baseline_and_explicit_selection() {
    for text in ["", "é\n"] {
        let (mut s, sid, baseline) = r15_tail_source();
        let [mut reference, _] = native_copies(&mut s);
        select(&mut reference, sid, 2..3);
        reference.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
        select(&mut s, sid, 0..1);
        let selection = s.doc().unwrap().selection.clone();
        r20_replace(&mut s, &json!({"story": sid.0, "start": 2, "end": 3, "text": text}));
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(full_doc(&s), full_doc(&reference), "both consumers use the existing outer EOF splice semantics");
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, format!("A\n{text}"));
        r15_type_clear_eof(&mut s, sid, &baseline, 20.0, 36.0);
    }
}

#[test]
fn r20_range_non_eof_keeps_literal_text_tracking_and_selection_clamping_policy() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (_, sid) = frame(&mut s, "Café fin", 40.0);
    local_defaults(&mut s, sid, 0..9);
    select(&mut s, sid, 0..9);
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
    s.execute("changes.track", &json!({"on": true})).unwrap();
    s.prefs.autocorrect = true;
    let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
    select(&mut s, sid, 0..1);
    let selection = s.doc().unwrap().selection.clone();
    let result = r20_replace(&mut s, &json!({"story": sid.0, "start": 3, "end": 5, "text": "\"Ω\""}));
    assert_eq!(result, json!({"length": 11}));
    assert_eq!(s.doc().unwrap().selection, selection);
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "Caf\"Ω\" fin", "range edits stay literal despite typographer's quotes and tracking preferences");
    assert_eq!(st.paras, original.paras);
    assert_eq!(st.format_after(3), original.format_after(3));
    assert!(st.chars.iter().all(|run| run.format.over.change.is_none()));
    native_copies(&mut s);
    // Existing selection coordinates are clamped to the new text, not replaced by a new caret policy.
    select(&mut s, sid, 11..11);
    let result = r20_replace(&mut s, &json!({"story": sid.0, "start": 0, "end": 11, "text": "é"}));
    assert_eq!(result, json!({"length": 2}));
    assert_eq!(s.doc().unwrap().selection.text.unwrap().range(), 2..2);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "é");
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras, original.paras);
    native_copies(&mut s);
}

#[test]
fn r20_eof_quotes_follow_staged_language_with_native_history_and_raw_control() {
    for (run_language, staged_language, quoted) in [("en", "fr", "«é»"), ("fr", "en", "“é”")] {
        for raw in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let (_, sid) = frame(&mut s, "é\n", 40.0);
            select(&mut s, sid, 0..3);
            s.execute("type.char", &json!({"attrs": {"language": run_language}})).unwrap();
            select(&mut s, sid, 3..3);
            s.execute("type.char", &json!({"attrs": {"language": staged_language}})).unwrap();
            let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
            assert_eq!(original.format_after(2).over.language.as_deref(), Some(run_language));
            assert_eq!(original.paras[1].chars.language.as_deref(), Some(staged_language));
            let [zip, legacy] = native_copies(&mut s);
            for mut back in [s, zip, legacy] {
                select(&mut back, sid, 3..3);
                let q = back.execute("type.selectionAttrs", &json!({})).unwrap();
                assert_eq!(q["chars"]["language"], staged_language);
                assert!(back.prefs.typographers_quotes);
                let expected = if raw { "\"é\"" } else { quoted };
                edit(&mut back, "text.insert", &json!({"text": "\"é\"", "raw": raw}));
                let st = back.doc().unwrap().doc.story(sid).unwrap();
                assert_eq!(st.text, format!("é\n{expected}"), "quote conversion and inserted language must agree");
                assert_eq!(st.paras[1], original.paras[0], "the consumed stage must not become paragraph language");
                assert_eq!(st.format_after(2), original.format_after(2));
                for (offset, _) in expected.char_indices() {
                    assert_eq!(st.format_after(3 + offset).over.language.as_deref(), Some(staged_language));
                }
                native_copies(&mut back);
            }
        }
    }
}

#[test]
fn r20_quotes_ordinary_positions_keep_run_language_over_paragraph_language() {
    for (run_language, paragraph_language, quoted) in [("en", "fr", "“é”"), ("fr", "en", "«é»")] {
        for text in ["é\nZ", "é "] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let (_, sid) = frame(&mut s, text, 40.0);
            s.execute("style.paragraph.create", &json!({"name": "R20 language", "chars": {"language": paragraph_language}})).unwrap();
            select(&mut s, sid, 0..text.len());
            s.execute("style.paragraph.apply", &json!({"name": "R20 language"})).unwrap();
            s.execute("type.char", &json!({"attrs": {"language": run_language}})).unwrap();
            let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
            select(&mut s, sid, 3..3);
            edit(&mut s, "text.insert", &json!({"text": "\"é\""}));
            let st = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(st.text, format!("{}{quoted}{}", &text[..3], &text[3..]));
            assert_eq!(st.paras, original.paras);
            assert_eq!(st.format_after(3), original.char_format_at(3));
            native_copies(&mut s);
        }
    }
}

#[test]
fn r20_quotes_cell_and_note_eof_keep_existing_run_and_baseline_policy() {
    for note in [false, true] {
        for (run_language, paragraph_language, quoted) in [("en", "fr", "“é”"), ("fr", "en", "«é»")] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"pages": 1})).unwrap();
            let (_, sid, cell) = r13_child(&mut s, note, "é\n");
            s.execute("text.move", &json!({"dir": "storyStart"})).unwrap();
            s.execute("text.move", &json!({"dir": "storyEnd", "extend": true})).unwrap();
            s.execute("style.paragraph.create", &json!({"name": "R20 child language", "chars": {"language": paragraph_language}})).unwrap();
            s.execute("style.paragraph.apply", &json!({"name": "R20 child language"})).unwrap();
            s.execute("type.char", &json!({"attrs": {"language": run_language, "size": 20}})).unwrap();
            s.execute("text.move", &json!({"dir": "storyEnd"})).unwrap();
            // Collapsed child EOF formatting remains outside the outer-story stage policy.
            s.execute("type.char", &json!({"attrs": {"language": paragraph_language, "size": 30}})).unwrap();
            let original = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap().clone();
            assert!(original.paras.iter().all(|pf| pf.chars.is_empty() && pf.eof_inherited_chars.is_none()));
            let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
            assert_eq!(q["chars"]["language"], run_language);
            assert_eq!(q["chars"]["size"], 20.0);
            edit(&mut s, "text.insert", &json!({"text": "\"é\""}));
            let st = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
            assert_eq!(st.text, format!("é\n{quoted}"));
            assert_eq!(st.paras, original.paras);
            assert_eq!(st.format_after(3), original.char_format_at(3));
            assert_eq!(s.doc().unwrap().selection.text.unwrap().cell, Some(cell));
            native_copies(&mut s);
        }
    }
}

// Break Link must preserve a child paragraph's baseline, even when its LF has a
// live character style and the child's ID happens to match the outer story's ID.
fn r33_child_layout(s: &Session, sid: StoryId, cell: designcraft_doc::CellAddr) -> Value {
    let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
    assert!(!cs.is_overset());
    let (child, origin, rect) = if let Some(id) = cell.footnote_id() {
        let note = cs.frames.iter().flat_map(|f| &f.notes).find(|n| n.id == id).unwrap();
        (note.text.as_ref(), note.origin, note.rect)
    } else {
        let placed = cs
            .frames
            .iter()
            .flat_map(|f| &f.tables)
            .filter(|t| t.table == cell.table)
            .flat_map(|t| &t.cells)
            .find(|c| c.row == cell.row && c.col == cell.col)
            .unwrap();
        (placed.text.as_ref(), placed.origin, placed.rect)
    };
    assert!(!child.is_overset());
    let lines: Vec<_> = child
        .frames
        .iter()
        .flat_map(|f| &f.lines)
        .map(|l| {
            json!({"range": [l.range.start, l.range.end], "baseline": l.baseline,
                "ascent": l.ascent, "descent": l.descent, "leading": l.leading,
                "x0": l.x0, "x1": l.x1, "endX": l.end_x})
        })
        .collect();
    json!({"origin": [origin.x, origin.y], "rect": [rect.x0, rect.y0, rect.x1, rect.y1], "lines": lines})
}

fn r33_child_break_link_appearance(note: bool) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (fid, sid, cell) = r13_child(&mut s, note, "A\n");
    // Identity-only fixture: child IDs are not document lookup keys. Formatting
    // and Break Link still go through the actual selection and Session commands.
    s.edit(|d, _| {
        d.text_story_mut(sid, Some(cell)).unwrap().id = sid;
        Ok(Value::Null)
    })
    .unwrap();
    r13_select_child(&mut s, fid, sid, cell, 0..2);
    s.execute("style.paragraph.create", &json!({"name": "R33 tracking only", "chars": {"tracking": 50}})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "R33 tracking only"})).unwrap();
    s.execute("style.character.create", &json!({"name": "R33 large LF", "chars": {"size": 40, "leading": {"kind": "points", "value": 60}}})).unwrap();
    s.execute("style.character.apply", &json!({"name": "R33 large LF"})).unwrap();
    let before_story = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap().clone();
    assert_eq!(before_story.id, sid, "exercise overlapping outer/child IDs");
    assert!(before_story.paras[1].chars.is_empty());
    assert!(before_story.paras[1].eof_inherited_chars.is_none());
    let before = r33_child_layout(&s, sid, cell);
    assert_eq!(before["lines"].as_array().unwrap().len(), 2);
    assert_eq!(before["lines"][0]["leading"], 60.0, "the named style must affect the real text");
    assert!((before["lines"][1]["leading"].as_f64().unwrap() - 14.4).abs() < 1e-6, "blank child EOF uses its paragraph baseline");

    s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
    assert_eq!(child.text, before_story.text);
    assert_eq!(child.chars, before_story.chars, "Break Link must leave the named LF style bound");
    let pf = &child.paras[1];
    assert_eq!(pf.style, designcraft_doc::styles::NO_PARA_STYLE);
    assert_eq!(pf.chars.tracking, Some(50.0));
    assert!(pf.eof_inherited_chars.is_none(), "child defaults must not become an outer typing stage");
    assert_eq!(r33_child_layout(&s, sid, cell), before, "Break Link must preserve child lines, placement and required area");
    s.doc().unwrap().doc.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap(), before_story);
    assert_eq!(r33_child_layout(&s, sid, cell), before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(r33_child_layout(&s, sid, cell), before);
}

#[test]
fn r33_cell_blank_eof_break_link_preserves_appearance_with_overlapping_id() {
    r33_child_break_link_appearance(false);
}

#[test]
fn r33_note_blank_eof_break_link_preserves_appearance_with_overlapping_id() {
    r33_child_break_link_appearance(true);
}

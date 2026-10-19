//! Bounded issue #174 follow-up: persisted character overrides for a final blank paragraph.
//! Commands and real Type-tool placement drive all edited fixtures; no new typing state.

use std::sync::Arc;

use designcraft_compose::Line;
use designcraft_doc::{CellAddr, ChangeMark, Document, ItemId, Leading, SpreadRef, Story, StoryId};
use designcraft_geom::Point;
use designcraft_tools::{PointerEvent, PointerKind};
use serde_json::json;

use crate::{Session, ViewInfo};

struct Before {
    doc: Arc<Document>,
    story: Story,
    lines: Vec<Line>,
}

fn frame(text: &str) -> (Session, ItemId, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("frame.create", &json!({"rect": [40, 40, 440, 500], "content": "text", "text": text})).unwrap();
    s.set_tool("type");
    (s, ItemId(r["id"].as_u64().unwrap()), StoryId(r["story"].as_u64().unwrap()))
}

fn lines(s: &Session, sid: StoryId) -> Vec<Line> {
    s.cache.get(&s.doc().unwrap().doc, sid, None).frames[0].lines.clone()
}

fn spread_point(s: &Session, fid: ItemId, inner: Point) -> Point {
    let d = &s.doc().unwrap().doc;
    d.parent_xf(&d.find(fid).unwrap()) * d.text_xf(d.item(fid).unwrap()) * inner
}

fn click(s: &mut Session, fid: ItemId, inner: Point) {
    let canvas = s.layout().xf(SpreadRef::Doc(0)) * spread_point(s, fid, inner);
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, canvas.x, canvas.y), ViewInfo::at_zoom(1.0)).unwrap();
    }
}

fn eof(s: &mut Session, fid: ItemId, sid: StoryId) {
    s.set_tool("type");
    let l = lines(s, sid).last().unwrap().clone();
    assert!(l.range.is_empty() && l.glyphs.is_empty(), "fixture must compose an actual final blank paragraph");
    click(s, fid, Point::new(l.end_x, l.baseline - l.ascent * 0.5));
    let t = s.doc().unwrap().selection.text.unwrap();
    let len = s.doc().unwrap().doc.story(sid).unwrap().len();
    assert_eq!((t.anchor, t.focus, t.cell), (len, len, None));
    assert_eq!(t.frame, Some(fid));
}

fn fixture(text: &str, conflicting_prior_run: bool) -> (Session, ItemId, StoryId, Before) {
    assert!(text.ends_with('\n'));
    let (mut s, fid, sid) = frame(text);
    if conflicting_prior_run {
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
        s.execute("type.char", &json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}})).unwrap();
    }
    eof(&mut s, fid, sid);
    let before = Before { doc: s.doc().unwrap().doc.clone(), story: s.doc().unwrap().doc.story(sid).unwrap().clone(), lines: lines(&s, sid) };
    assert!(before.story.paras.last().unwrap().chars.is_empty());
    (s, fid, sid, before)
}

fn format(s: &mut Session) {
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
}

fn unchanged_neighbors(s: &Session, sid: StoryId, before: &Before) {
    let d = &s.doc().unwrap().doc;
    let st = d.story(sid).unwrap();
    assert_eq!(st.text, before.story.text);
    assert_eq!(st.chars, before.story.chars, "there are no EOF bytes to format; all existing runs must remain exact");
    assert!(st.chars.iter().all(|r| r.len > 0), "do not add a zero-length run to a nonempty story");
    assert_eq!(st.paras.len(), before.story.paras.len());
    assert_eq!(st.paras[..st.paras.len() - 1], before.story.paras[..before.story.paras.len() - 1], "all preceding paragraph data must remain exact");
    let now = lines(s, sid);
    assert_eq!(now.len(), before.lines.len());
    for (a, b) in now.iter().zip(&before.lines).take(now.len() - 1) {
        assert_eq!((a.baseline, a.leading, a.ascent, a.descent, a.end_x), (b.baseline, b.leading, b.ascent, b.descent, b.end_x));
        assert_eq!(
            a.glyphs.iter().map(|g| (g.gid, g.byte, g.x, g.adv)).collect::<Vec<_>>(),
            b.glyphs.iter().map(|g| (g.gid, g.byte, g.x, g.adv)).collect::<Vec<_>>()
        );
    }
    d.check().unwrap();
}

fn blank_metrics(s: &mut Session, sid: StoryId, before: &Before) {
    unchanged_neighbors(s, sid, before);
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    let pf = st.paras.last().unwrap();
    assert_eq!(pf.chars.size, Some(20.0), "collapsed EOF formatting must persist on the final paragraph");
    assert_eq!(pf.chars.leading, Some(Leading::Points(36.0)));
    let old = before.lines.last().unwrap();
    let now = lines(s, sid).last().unwrap().clone();
    assert!(now.range.is_empty() && now.glyphs.is_empty());
    assert!((now.leading - 36.0).abs() < 1e-6, "composition must consume the stored EOF leading, got {}", now.leading);
    assert!((now.baseline - old.baseline - (36.0 - old.leading)).abs() < 1e-6, "EOF blank must consume the new line advance");
    let base_size = before.doc.styles.resolve_para(before.story.paras.last().unwrap()).1.size;
    assert!((now.ascent - old.ascent * 20.0 / base_size).abs() < 1e-6, "EOF blank ascent must consume size 20");
    assert!((now.descent - old.descent * 20.0 / base_size).abs() < 1e-6);
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["size"], json!(20.0), "selection query must not return the conflicting preceding run's size");
    assert_eq!(attrs["chars"]["leading"], json!({"kind": "points", "value": 36.0}));
}

fn typed(s: &mut Session, sid: StoryId, at: usize) {
    let d = &s.doc().unwrap().doc;
    let st = d.story(sid).unwrap();
    let f = st.format_after(at);
    assert_eq!(f.over.size, Some(20.0), "EOF typing must keep chosen size over preceding run overrides");
    assert_eq!(f.over.leading, Some(Leading::Points(36.0)));
    assert!((lines(s, sid).last().unwrap().leading - 36.0).abs() < 1e-6);
    d.check().unwrap();
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["size"], json!(20.0));
    assert_eq!(attrs["chars"]["leading"], json!({"kind": "points", "value": 36.0}));
}

#[test]
fn r7a_blank_eof_collapsed_format_changes_metrics_and_history() {
    for text in ["First\n", "Café\n\n"] {
        for cold in [false, true] {
            let (mut s, _, sid, before) = fixture(text, true);
            let undo = s.doc().unwrap().history.undo.len();
            if cold {
                s.cache.clear();
            }
            format(&mut s);
            blank_metrics(&mut s, sid, &before);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().rev, before.story.rev.wrapping_add(1));
            let after = s.doc().unwrap().doc.clone();
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before.doc));
            assert_eq!(lines(&s, sid).last().unwrap().leading, before.lines.last().unwrap().leading);
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
            blank_metrics(&mut s, sid, &before);
        }
    }
}

#[test]
fn r7a_blank_eof_zip_legacy_and_native_reopen_preserve_format() {
    let (mut s, fid, sid, before) = fixture("First\n", true);
    format(&mut s);
    let after = s.doc().unwrap().doc.clone();
    let native = s.execute("file.serialize", &json!({})).unwrap();
    let zip = crate::cmd::file::base64_decode(native["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"), "public serialize must produce the actual native ZIP format");
    let legacy = serde_json::to_vec(&*after).unwrap();
    assert!(legacy.starts_with(b"{"));
    // Omitting name preserves the serialized title; openBytes otherwise intentionally replaces it.
    for bytes in [&zip, &legacy] {
        let mut back = Session::new();
        back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(bytes)})).unwrap();
        assert_eq!(serde_json::to_value(&*back.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
        eof(&mut back, fid, sid);
        blank_metrics(&mut back, sid, &before);
        back.execute("text.insert", &json!({"text": "Z", "raw": true})).unwrap();
        typed(&mut back, sid, before.story.len());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        struct Saved(std::path::PathBuf);
        impl Drop for Saved {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("zip.designcraft"));
                let _ = std::fs::remove_file(self.0.join("legacy.designcraft"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let dir = std::env::temp_dir().join(format!("designcraft-issue174-eof-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let saved = Saved(dir);
        let zip_path = saved.0.join("zip.designcraft");
        let legacy_path = saved.0.join("legacy.designcraft");
        s.execute("file.saveAs", &json!({"path": zip_path.to_string_lossy()})).unwrap();
        assert!(std::fs::read(&zip_path).unwrap().starts_with(b"PK"));
        std::fs::write(&legacy_path, &legacy).unwrap();
        for path in [zip_path, legacy_path] {
            let mut back = Session::new();
            back.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
            assert_eq!(serde_json::to_value(&*back.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
            eof(&mut back, fid, sid);
            blank_metrics(&mut back, sid, &before);
        }
    }
}

#[test]
fn r7a_blank_eof_typing_retains_overrides_over_conflicting_prior_run() {
    let (mut s, _, sid, before) = fixture("First\n", true);
    format(&mut s);
    let formatted = s.doc().unwrap().doc.clone();
    s.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    typed(&mut s, sid, before.story.len());
    s.execute("text.insert", &json!({"text": "à", "raw": true})).unwrap();
    typed(&mut s, sid, before.story.len());
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "First\néà");
    assert_eq!(st.chars[0], before.story.chars[0], "previous paragraph run must remain unchanged while typing at EOF");
    assert_eq!(st.paras[0], before.story.paras[0]);
    let typed_doc = s.doc().unwrap().doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &formatted));
    assert_eq!(lines(&s, sid).last().unwrap().leading, 36.0);
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &typed_doc));
    typed(&mut s, sid, before.story.len());
}

#[test]
fn r7a_blank_eof_tracked_typing_retains_format_and_change_mark() {
    let (mut s, _, sid, before) = fixture("First\n", true);
    format(&mut s);
    s.execute("changes.track", &json!({"on": true})).unwrap();
    let formatted = s.doc().unwrap().doc.clone();
    s.execute("text.insert", &json!({"text": "Tail", "raw": true})).unwrap();
    typed(&mut s, sid, before.story.len());
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().format_after(before.story.len()).over.change, Some(ChangeMark::Inserted));
    let after = s.doc().unwrap().doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &formatted));
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
}

#[test]
fn r7a_blank_eof_newline_typing_copies_chosen_paragraph_format() {
    let (mut s, _, sid, before) = fixture("First\n", true);
    format(&mut s);
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "First\n\n");
    assert_eq!(st.paras[0], before.story.paras[0]);
    assert_eq!(st.chars[0], before.story.chars[0]);
    assert_eq!(st.paras[1].chars, before.story.paras[1].chars, "the earlier blank paragraph restores its inherited baseline");
    assert!(st.paras[1].eof_inherited_chars.is_none());
    let separator = st.format_after(before.story.len());
    assert_eq!(separator.over.size, Some(20.0), "the inserted LF owns the former EOF's direct format");
    assert_eq!(separator.over.leading, Some(Leading::Points(36.0)));
    let final_eof = st.paras.last().unwrap();
    assert_eq!(final_eof.chars.size, Some(20.0), "the true final EOF retains the persisted typing format");
    assert_eq!(final_eof.chars.leading, Some(Leading::Points(36.0)));
    for l in lines(&s, sid).iter().skip(1) {
        assert_eq!(l.leading, 36.0);
    }
    s.doc().unwrap().doc.check().unwrap();
}

#[test]
fn r7a_blank_eof_null_clearing_restores_defaults_and_history() {
    let (mut s, _, sid, before) = fixture("First\n", true);
    format(&mut s);
    blank_metrics(&mut s, sid, &before);
    let formatted = s.doc().unwrap().doc.clone();
    s.execute("type.char", &json!({"attrs": {"leading": null}})).unwrap();
    let partial = s.doc().unwrap().doc.clone();
    let pf = &s.doc().unwrap().doc.story(sid).unwrap().paras[1];
    assert_eq!((pf.chars.size, pf.chars.leading), (Some(20.0), None));
    assert_eq!(lines(&s, sid).last().unwrap().leading, 24.0, "cleared leading uses size 20 with existing Auto leading");
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["size"], json!(20.0));
    assert_eq!(attrs["chars"]["leading"], json!({"kind": "auto"}), "clearing must not restore the preceding run's leading 18");
    s.execute("type.char", &json!({"attrs": {"size": null}})).unwrap();
    assert!(s.doc().unwrap().doc.story(sid).unwrap().paras[1].chars.is_empty());
    unchanged_neighbors(&s, sid, &before);
    assert_eq!(lines(&s, sid).last().unwrap().leading, before.lines.last().unwrap().leading);
    let cleared = s.doc().unwrap().doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &partial));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &formatted));
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &cleared));
}

#[test]
fn r7a_blank_eof_explicit_defaults_override_named_style_and_reopen() {
    let (mut s, fid, sid, before) = fixture("First\n", true);
    s.execute("style.paragraph.create", &json!({"name": "EOF metrics", "chars": {"size": 24, "leading": {"kind": "points", "value": 30}}})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "EOF metrics"})).unwrap();
    assert_eq!(lines(&s, sid).last().unwrap().leading, 30.0, "named style fixture must have conflicting default metrics");
    s.execute("type.char", &json!({"attrs": {"size": 12, "leading": {"kind": "auto"}}})).unwrap();
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.paras.last().unwrap().chars.size, Some(12.0), "an explicit default size must remain a stored override");
    assert_eq!(st.paras.last().unwrap().chars.leading, Some(Leading::Auto));
    unchanged_neighbors(&s, sid, &before);
    assert!((lines(&s, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
    let native = s.execute("file.serialize", &json!({})).unwrap();
    let zip = crate::cmd::file::base64_decode(native["base64"].as_str().unwrap());
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    for bytes in [zip, legacy] {
        let mut back = Session::new();
        back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
        let pf = back.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap();
        assert_eq!(pf.style, "EOF metrics");
        assert_eq!((pf.chars.size, pf.chars.leading), (Some(12.0), Some(Leading::Auto)));
        eof(&mut back, fid, sid);
        assert!((lines(&back, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
        let attrs = back.execute("type.selectionAttrs", &json!({})).unwrap();
        assert_eq!(attrs["chars"]["size"], json!(12.0));
        assert_eq!(attrs["chars"]["leading"], json!({"kind": "auto"}));
        back.execute("text.insert", &json!({"text": "Z", "raw": true})).unwrap();
        let f = back.doc().unwrap().doc.story(sid).unwrap().format_after(before.story.len());
        assert_eq!((f.over.size, f.over.leading), (Some(12.0), Some(Leading::Auto)));
        assert!((lines(&back, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
        back.doc().unwrap().doc.check().unwrap();
    }
}

#[test]
fn r7a_blank_eof_single_metric_preserves_other_typing_attributes() {
    for prior_formatted in [false, true] {
        for (attrs, size, leading) in [
            (json!({"leading": {"kind": "points", "value": 36}}), if prior_formatted { 15.0 } else { 12.0 }, 36.0),
            (json!({"size": 20}), 20.0, if prior_formatted { 18.0 } else { 24.0 }),
        ] {
            let (mut s, _, sid, before) = fixture("First\n", prior_formatted);
            s.execute("type.char", &json!({"attrs": attrs})).unwrap();
            unchanged_neighbors(&s, sid, &before);
            assert_eq!(lines(&s, sid).last().unwrap().leading, leading, "changing one metric preserves the other typing attribute");
            let selected = s.execute("type.selectionAttrs", &json!({})).unwrap();
            assert_eq!(selected["chars"]["size"], json!(size));
            assert_eq!(
                selected["chars"]["leading"],
                if prior_formatted || leading == 36.0 { json!({"kind": "points", "value": leading}) } else { json!({"kind": "auto"}) }
            );
            s.execute("text.insert", &json!({"text": "Tail", "raw": true})).unwrap();
            assert_eq!(lines(&s, sid).last().unwrap().leading, leading, "typing keeps both the changed and untouched metrics");
            let st = s.doc().unwrap().doc.story(sid).unwrap();
            let base = s.doc().unwrap().doc.styles.resolve_para(st.paras.last().unwrap()).1;
            let props = s.doc().unwrap().doc.styles.resolve_char(&base, st.format_after(before.story.len()));
            assert_eq!(props.size, size, "an untouched explicit size survives; an absent size uses the paragraph default");
            let typed = s.execute("type.selectionAttrs", &json!({})).unwrap();
            assert_eq!(typed["chars"], selected["chars"], "chosen blank formatting must match the inserted text's resolved properties");
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().chars[0], before.story.chars[0]);
            s.doc().unwrap().doc.check().unwrap();
        }
    }
}

#[test]
fn r7a_blank_eof_ordinary_nonempty_carets_remain_noop_control() {
    for text in ["First", "First\nLast", "First\u{2028}"] {
        let (mut s, _, sid) = frame(text);
        let mut positions = vec![0, 2, 5, text.len()];
        positions.sort_unstable();
        positions.dedup();
        for pos in positions {
            s.execute("text.select", &json!({"story": sid.0, "anchor": pos})).unwrap();
            let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
            let old = lines(&s, sid);
            format(&mut s);
            assert_eq!(
                *s.doc().unwrap().doc.story(sid).unwrap(),
                before,
                "nonempty ordinary/EOF/forced-break caret formatting keeps its existing no-op behavior"
            );
            assert_eq!(
                lines(&s, sid).iter().map(|l| (l.leading, l.baseline, l.ascent, l.descent)).collect::<Vec<_>>(),
                old.iter().map(|l| (l.leading, l.baseline, l.ascent, l.descent)).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn r7a_blank_eof_existing_paragraph_data_already_composes_control() {
    let (s, fid, sid, before) = fixture("First\n", false);
    let mut legacy = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    legacy["stories"][sid.0.to_string()]["paras"].as_array_mut().unwrap().last_mut().unwrap()["chars"] =
        json!({"size": 20.0, "leading": {"kind": "points", "value": 36.0}});
    let mut back = Session::new();
    back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&serde_json::to_vec(&legacy).unwrap())})).unwrap();
    eof(&mut back, fid, sid);
    blank_metrics(&mut back, sid, &before);
    // This proves the unmodified document, parser and composer already support the chosen field.
}

#[test]
fn r7a_blank_eof_cell_and_note_hits_and_carets_remain_noop_control() {
    for note in [false, true] {
        let (mut s, fid, sid) = frame("Body");
        s.execute("text.select", &json!({"story": sid.0, "anchor": 1})).unwrap();
        let (addr, inner) = if note {
            let r = s.execute("footnote.insert", &json!({"text": "Note\n"})).unwrap();
            let id = r["id"].as_u64().unwrap();
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            let (_, x, bl, asc, _) = designcraft_compose::note_caret(&cs, id, 5).unwrap();
            (CellAddr::footnote(id), Point::new(x, bl - asc * 0.5))
        } else {
            let r = s.execute("table.insert", &json!({"rows": 1, "cols": 1, "width": 200})).unwrap();
            let table = r["table"].as_u64().unwrap();
            s.execute("table.setCell", &json!({"story": sid.0, "table": table, "rows": [0, 0], "cols": [0, 0], "text": "Cell\n"})).unwrap();
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            let (_, x, bl, asc, _) = designcraft_compose::cell_caret(&cs, table, 0, 0, 5).unwrap();
            (CellAddr { table, row: 0, col: 0 }, Point::new(x, bl - asc * 0.5))
        };
        click(&mut s, fid, inner);
        let t = s.doc().unwrap().selection.text.unwrap();
        assert_eq!((t.cell, t.anchor, t.focus), (Some(addr), 5, 5), "public hit path must reach the actual child story's blank EOF");
        let before = s.doc().unwrap().doc.text_story(sid, Some(addr)).unwrap().clone();
        assert!(before.text.ends_with('\n') && !before.is_empty());
        format(&mut s);
        assert_eq!(*s.doc().unwrap().doc.text_story(sid, Some(addr)).unwrap(), before, "cell/note EOF caret formatting remains unchanged");
        let after = s.doc().unwrap().selection.text.unwrap();
        assert_eq!((after.cell, after.anchor, after.focus), (Some(addr), 5, 5));
        s.doc().unwrap().doc.check().unwrap();
    }
}

// Untouched EOFs inherit the LF; explicitly cleared EOFs keep an empty typing format.
fn unset_eof_query_matches_typing(s: &mut Session, fid: ItemId, sid: StoryId, expected: &Arc<Document>, cleared: bool, cold: bool) {
    let prior = expected.story(sid).unwrap().clone();
    let at = prior.len();
    assert!(prior.text.ends_with('\n') && prior.paras.last().unwrap().chars.is_empty());
    assert_eq!(prior.paras.last().unwrap().eof_inherited_chars, cleared.then(designcraft_doc::CharAttrs::default));
    let mut inherited = prior.char_format_at(at).clone();
    if cleared {
        inherited.over = Default::default();
    }
    // Populating the final paragraph consumes its explicit-empty marker; all other data stays exact.
    let mut populated_paras = prior.paras.clone();
    populated_paras.last_mut().unwrap().eof_inherited_chars = None;
    let full_before = serde_json::to_value(&**expected).unwrap();
    let t = s.doc().unwrap().selection.text.unwrap();
    let selection = (t.story, t.cell, t.frame, t.anchor, t.focus);
    assert_eq!((t.anchor, t.focus, t.cell), (at, at, None));
    let undo = s.doc().unwrap().history.undo.len();
    let redo = s.doc().unwrap().history.redo.len();
    let _ = lines(s, sid);
    if cold {
        s.cache.clear();
    }
    let selected = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, expected), "the query must not modify document data");
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), full_before);
    let t = s.doc().unwrap().selection.text.unwrap();
    assert_eq!((t.story, t.cell, t.frame, t.anchor, t.focus), selection);
    assert_eq!((s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len()), (undo, redo));

    s.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    let resolved = {
        let d = &s.doc().unwrap().doc;
        let st = d.story(sid).unwrap();
        assert_eq!(st.text, format!("{}é", prior.text));
        assert_eq!(st.paras, populated_paras, "typing consumes only the EOF stage marker, preserving neighboring paragraph data");
        r8_prior_unchanged(s, sid, &prior);
        assert_eq!(st.format_after(at), &inherited, "typing must respect whether the empty EOF format was explicitly cleared");
        let base = d.styles.resolve_para(st.paras.last().unwrap()).1;
        d.check().unwrap();
        d.styles.resolve_char(&base, st.format_after(at))
    };
    let typed = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(
        typed["chars"],
        serde_json::to_value(&resolved).unwrap(),
        "the post-insert query must describe the actual inserted character properties"
    );
    assert_eq!(typed["chars"]["size"], json!(if cleared { 12.0 } else { 15.0 }));
    let leading = if cleared { json!({"kind": "auto"}) } else { json!({"kind": "points", "value": 18.0}) };
    assert_eq!(typed["chars"]["leading"], leading);
    assert_eq!(selected["chars"], typed["chars"], "unset blank EOF must report the same resolved properties that actual typing inherits");
    assert_eq!(selected["characterStyle"], typed["characterStyle"]);
    assert_eq!(selected["charOverrides"], typed["charOverrides"]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let typed_doc = s.doc().unwrap().doc.clone();
    let full_typed = serde_json::to_value(&*typed_doc).unwrap();

    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, expected));
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), full_before);
    // Place the actual EOF caret again before querying the restored document.
    eof(s, fid, sid);
    if cold {
        s.cache.clear();
    }
    let restored = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(restored["chars"], selected["chars"], "undo must restore the same unset EOF query");
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &typed_doc));
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), full_typed);
    let redone = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(redone["chars"], typed["chars"]);
}

#[test]
fn r7a_eof_unset_format_untouched_query_matches_typing_and_history() {
    for cold in [false, true] {
        let (mut s, fid, sid, before) = fixture("First\n", true);
        unset_eof_query_matches_typing(&mut s, fid, sid, &before.doc, false, cold);
    }
}

#[test]
fn r7a_eof_unset_format_fully_cleared_query_matches_typing_and_history() {
    for cold in [false, true] {
        let (mut s, fid, sid, before) = fixture("First\n", true);
        let undo = s.doc().unwrap().history.undo.len();
        format(&mut s);
        blank_metrics(&mut s, sid, &before);
        let formatted = s.doc().unwrap().doc.clone();
        s.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
        let cleared = s.doc().unwrap().doc.clone();
        assert!(cleared.story(sid).unwrap().paras.last().unwrap().chars.is_empty());
        unchanged_neighbors(&s, sid, &before);
        assert_eq!(
            lines(&s, sid).last().unwrap().leading,
            before.lines.last().unwrap().leading,
            "clearing leaves the existing blank composition unchanged"
        );
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 2);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &formatted));
        blank_metrics(&mut s, sid, &before);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before.doc));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &formatted));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &cleared));
        unset_eof_query_matches_typing(&mut s, fid, sid, &cleared, true, cold);
    }
}

fn r8_full_doc(s: &Session) -> serde_json::Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn r8_zip_reopen(s: &mut Session) -> Session {
    let full = r8_full_doc(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let bytes = crate::cmd::file::base64_decode(saved["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"), "exercise the native ZIP path");
    let mut back = Session::new();
    back.execute("file.openBytes", &json!({"base64": saved["base64"]})).unwrap();
    assert_eq!(r8_full_doc(&back), full, "native reopen retains the entire document");
    back
}

fn r8_query_metrics(s: &mut Session, size: f64, leading: serde_json::Value) {
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["chars"]["size"], json!(size));
    assert_eq!(q["chars"]["leading"], leading);
}

fn r8_prior_unchanged(s: &Session, sid: StoryId, prior: &Story) {
    let d = &s.doc().unwrap().doc;
    let st = d.story(sid).unwrap();
    assert_eq!(&st.text[..prior.len()], prior.text);
    assert_eq!(st.paras[..prior.paras.len() - 1], prior.paras[..prior.paras.len() - 1]);
    for (byte, _) in prior.text.char_indices() {
        assert_eq!(st.format_after(byte), prior.format_after(byte), "prior UTF-8 run formatting at {byte}");
    }
    d.check().unwrap();
}

#[test]
fn r8_eof_typed_range_clears_return_to_inherited_defaults_with_partial_utf8_and_history() {
    for tracking in [false, true] {
        for named in [false, true] {
            for explicit_defaults in [false, true] {
                let (mut s, _, sid, _) = fixture("First\n", true);
                if named {
                    s.execute(
                        "style.paragraph.create",
                        &json!({"name": "EOF parent", "chars": {"size": 24, "leading": {"kind": "points", "value": 30}}}),
                    )
                    .unwrap();
                    s.execute("style.paragraph.create", &json!({"name": "EOF child", "basedOn": "EOF parent"})).unwrap();
                    s.execute("style.paragraph.apply", &json!({"name": "EOF child"})).unwrap();
                }
                if tracking {
                    s.execute("changes.track", &json!({"on": true})).unwrap();
                }
                let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
                let (chosen_size, chosen_leading) = if explicit_defaults {
                    s.execute("type.char", &json!({"attrs": {"size": 12, "leading": {"kind": "auto"}}})).unwrap();
                    (12.0, json!({"kind": "auto"}))
                } else {
                    format(&mut s);
                    (20.0, json!({"kind": "points", "value": 36.0}))
                };
                // Repeated commands exercise UTF-8 byte accounting and inheritance after the first insertion.
                for text in ["é", "à"] {
                    s.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
                    r8_query_metrics(&mut s, chosen_size, chosen_leading.clone());
                }
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "First\néà");
                r8_prior_unchanged(&s, sid, &prior);
                let typed = r8_full_doc(&s);
                let mut back = r8_zip_reopen(&mut s);
                let base_size = if named { 24.0 } else { 12.0 };
                let base_leading = if named { json!({"kind": "points", "value": 30.0}) } else { json!({"kind": "auto"}) };
                s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 8})).unwrap();
                s.execute("type.char", &json!({"attrs": {"size": null}})).unwrap();
                r8_query_metrics(&mut s, base_size, chosen_leading.clone());
                s.execute("type.char", &json!({"attrs": {"leading": null}})).unwrap();
                r8_query_metrics(&mut s, base_size, base_leading.clone());
                // Clearing only é must leave à at the chosen typing format.
                s.execute("text.select", &json!({"story": sid.0, "anchor": 8, "focus": 10})).unwrap();
                r8_query_metrics(&mut s, chosen_size, chosen_leading.clone());
                s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 10})).unwrap();
                let before_clear = r8_full_doc(&s);
                let undo = s.doc().unwrap().history.undo.len();
                s.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
                r8_query_metrics(&mut s, base_size, base_leading.clone());
                let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
                assert_eq!(q["charOverrides"], json!(if tracking { 1 } else { 0 }));
                let expected_advance = if named { 30.0 } else { 14.4 };
                assert!(
                    (lines(&s, sid).last().unwrap().leading - expected_advance).abs() < 1e-6,
                    "cleared glyphs must compose with the inherited leading, not the staged EOF value"
                );
                r8_prior_unchanged(&s, sid, &prior);
                if tracking {
                    let changes = s.execute("changes.list", &json!({})).unwrap();
                    assert_eq!(changes.as_array().unwrap().len(), 1);
                    assert_eq!(changes[0]["text"], "éà");
                    assert_eq!(changes[0]["kind"], "inserted");
                }
                let cleared = r8_full_doc(&s);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(r8_full_doc(&s), before_clear);
                s.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(r8_full_doc(&s), cleared);
                let mut cleared_back = r8_zip_reopen(&mut s);
                cleared_back.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 10})).unwrap();
                r8_query_metrics(&mut cleared_back, base_size, base_leading.clone());
                // Clearing after reopen must have the same visible result as clearing before save.
                assert_eq!(r8_full_doc(&back), typed);
                back.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 10})).unwrap();
                back.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
                r8_query_metrics(&mut back, base_size, base_leading);
                assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().chars, s.doc().unwrap().doc.story(sid).unwrap().chars);
            }
        }
    }
}

#[test]
fn r8_eof_new_blank_continuation_survives_transfer_reopen_and_range_clear() {
    for tracking in [false, true] {
        for first in ["Tail\n", "Tail", "\nTail\n", "\n"] {
            let (mut s, fid, sid, before) = fixture("First\n", true);
            if tracking {
                s.execute("changes.track", &json!({"on": true})).unwrap();
            }
            format(&mut s);
            let formatted = r8_full_doc(&s);
            let undo = s.doc().unwrap().history.undo.len();
            s.execute("text.insert", &json!({"text": first, "raw": true})).unwrap();
            let inserted = r8_full_doc(&s);
            assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(r8_full_doc(&s), formatted);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(r8_full_doc(&s), inserted);
            if !first.ends_with('\n') {
                s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
            }
            let st = s.doc().unwrap().doc.story(sid).unwrap();
            assert!(st.text.ends_with('\n'));
            assert_eq!(st.paras.last().unwrap().chars.change, None, "empty continuation is not itself an inserted change");
            eof(&mut s, fid, sid);
            r8_query_metrics(&mut s, 20.0, json!({"kind": "points", "value": 36.0}));
            assert!((lines(&s, sid).last().unwrap().leading - 36.0).abs() < 1e-6);
            let mut back = r8_zip_reopen(&mut s);
            eof(&mut back, fid, sid);
            let at = back.doc().unwrap().doc.story(sid).unwrap().len();
            for text in ["é", "à"] {
                back.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
                r8_query_metrics(&mut back, 20.0, json!({"kind": "points", "value": 36.0}));
            }
            back.execute("text.select", &json!({"story": sid.0, "anchor": at, "focus": at + 4})).unwrap();
            back.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
            r8_query_metrics(&mut back, 12.0, json!({"kind": "auto"}));
            assert!((lines(&back, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
            r8_prior_unchanged(&back, sid, &before.story);
            // Earlier populated continuation text retains its chosen formatting after this clear.
            if let Some(tail) = back.doc().unwrap().doc.story(sid).unwrap().text.find("Tail") {
                back.execute("text.select", &json!({"story": sid.0, "anchor": tail, "focus": tail + 4})).unwrap();
                r8_query_metrics(&mut back, 20.0, json!({"kind": "points", "value": 36.0}));
            }
        }
    }
}

#[test]
fn r8_eof_empty_insert_keeps_staged_format_and_ordinary_direct_base_remains_inherited() {
    let (mut s, _, sid, _) = fixture("First\n", true);
    format(&mut s);
    let staged = s.doc().unwrap().doc.story(sid).unwrap().clone();
    s.execute("text.insert", &json!({"text": "", "raw": true})).unwrap();
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras, staged.paras);
    r8_query_metrics(&mut s, 20.0, json!({"kind": "points", "value": 36.0}));
    // Imported direct paragraph defaults on existing text are still inherited by range clears.
    let (s, _, sid) = frame("Ordinary éà");
    let mut native = r8_full_doc(&s);
    native["stories"][sid.0.to_string()]["paras"][0]["chars"] = json!({"size": 24.0, "leading": {"kind": "points", "value": 30.0}});
    let mut back = Session::new();
    back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&serde_json::to_vec(&native).unwrap())})).unwrap();
    back.execute("text.select", &json!({"story": sid.0, "anchor": 9, "focus": 13})).unwrap();
    format(&mut back);
    back.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
    r8_query_metrics(&mut back, 24.0, json!({"kind": "points", "value": 30.0}));
    back.doc().unwrap().doc.check().unwrap();
}

#[test]
fn r8_eof_tail_clear_has_no_overrides_and_uses_default_metrics() {
    let (mut s, _, sid, before) = fixture("First\n", true);
    format(&mut s);
    s.execute("text.insert", &json!({"text": "Tail", "raw": true})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 10})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["charOverrides"], 0);
    // A cleared run must not inherit the former blank-EOF typing overrides from its paragraph.
    assert_eq!(q["chars"]["size"], json!(12.0));
    assert_eq!(q["chars"]["leading"], json!({"kind": "auto"}));
    assert!((lines(&s, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
    r8_prior_unchanged(&s, sid, &before.story);
}

#[test]
fn r8_eof_newline_retains_character_style_values_above_direct_paragraph_defaults() {
    let (s, fid, sid) = frame("Tail");
    let mut native = r8_full_doc(&s);
    native["stories"][sid.0.to_string()]["paras"][0]["chars"] = json!({"size": 24.0, "leading": {"kind": "points", "value": 30.0}});
    let mut back = Session::new();
    back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&serde_json::to_vec(&native).unwrap())})).unwrap();
    back.execute("style.character.create", &json!({"name": "Explicit default parent", "chars": {"size": 12, "leading": {"kind": "auto"}}})).unwrap();
    back.execute("style.character.create", &json!({"name": "Inherited default child", "basedOn": "Explicit default parent"})).unwrap();
    back.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 4})).unwrap();
    back.execute("style.character.apply", &json!({"name": "Inherited default child"})).unwrap();
    r8_query_metrics(&mut back, 12.0, json!({"kind": "auto"}));
    let prior = back.doc().unwrap().doc.story(sid).unwrap().clone();
    back.execute("text.move", &json!({"dir": "storyEnd"})).unwrap();
    back.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    eof(&mut back, fid, sid);
    r8_query_metrics(&mut back, 12.0, json!({"kind": "auto"}));
    assert!((lines(&back, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
    back.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    r8_query_metrics(&mut back, 12.0, json!({"kind": "auto"}));
    let st = back.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.paras[0], prior.paras[0], "direct defaults on the preceding populated paragraph remain exact");
    assert_eq!(st.format_after(0), prior.format_after(0));
    assert_eq!(st.text, "Tail\né");
    back.doc().unwrap().doc.check().unwrap();
    r8_zip_reopen(&mut back);
}

// R9 editing proposals: public commands, real composition, full-document history,
// and native ZIP/legacy reopen distinguish inherited defaults from EOF typing attrs.
fn r9_legacy_reopen(s: &Session) -> Session {
    let full = r8_full_doc(s);
    let bytes = serde_json::to_vec(&full).unwrap();
    assert!(bytes.starts_with(b"{"));
    let mut back = Session::new();
    back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
    assert_eq!(r8_full_doc(&back), full, "legacy reopen retains the entire document");
    back
}

fn r9_history(s: &mut Session, before: &serde_json::Value, after: &serde_json::Value) {
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(&r8_full_doc(s), before, "Undo restores the entire document");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(&r8_full_doc(s), after, "Redo restores the entire document");
}

fn r9_named_fixture(inherited: bool, based_on: bool, chars: serde_json::Value) -> (Session, ItemId, StoryId) {
    let (s, fid, sid) = frame("Tail");
    let mut s = if inherited {
        let mut legacy = r8_full_doc(&s);
        legacy["stories"][sid.0.to_string()]["paras"][0]["chars"] = json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}});
        let mut back = Session::new();
        back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&serde_json::to_vec(&legacy).unwrap())})).unwrap();
        assert_eq!(r8_full_doc(&back), legacy, "supported paragraph character defaults enter through the legacy reader");
        back
    } else {
        s
    };
    s.execute("style.character.create", &json!({"name": "R9 parent", "chars": chars})).unwrap();
    let name = if based_on {
        s.execute("style.character.create", &json!({"name": "R9 child", "basedOn": "R9 parent"})).unwrap();
        "R9 child"
    } else {
        "R9 parent"
    };
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 4})).unwrap();
    s.execute("style.character.apply", &json!({"name": name})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 4})).unwrap();
    (s, fid, sid)
}

fn r9_enter(s: &mut Session, fid: ItemId, sid: StoryId) {
    let before = r8_full_doc(s);
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    let after = r8_full_doc(s);
    r9_history(s, &before, &after);
    eof(s, fid, sid);
}

fn r9_style_edit(s: &mut Session) {
    let before = r8_full_doc(s);
    s.execute("style.character.edit", &json!({"name": "R9 parent", "chars": {"size": 30.0, "leading": {"kind": "points", "value": 32.0}}})).unwrap();
    let after = r8_full_doc(s);
    r9_history(s, &before, &after);
}

fn r9_select_query(s: &mut Session, sid: StoryId, start: usize, end: usize, size: f64, leading: f64) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": start, "focus": end})).unwrap();
    r8_query_metrics(s, size, json!({"kind": "points", "value": leading}));
}

fn r9_stage_metrics(attrs: serde_json::Value, size: f64, leading: f64) {
    for based_on in [false, true] {
        for tracking in [false, true] {
            for cold in [false, true] {
                let (mut s, fid, sid) = r9_named_fixture(false, based_on, json!({"size": 18.0, "leading": {"kind": "points", "value": 32.0}}));
                if tracking {
                    s.execute("changes.track", &json!({"on": true})).unwrap();
                }
                r9_enter(&mut s, fid, sid);
                let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
                let before = r8_full_doc(&s);
                s.execute("type.char", &json!({"attrs": attrs})).unwrap();
                let staged = r8_full_doc(&s);
                r9_history(&mut s, &before, &staged);
                r8_prior_unchanged(&s, sid, &prior);
                for reopen in 0..2 {
                    let mut back = match reopen {
                        0 => r8_zip_reopen(&mut s),
                        _ => r9_legacy_reopen(&s),
                    };
                    eof(&mut back, fid, sid);
                    let _ = lines(&back, sid);
                    if cold {
                        back.cache.clear();
                    }
                    let full = r8_full_doc(&back);
                    r8_query_metrics(&mut back, size, json!({"kind": "points", "value": leading}));
                    assert_eq!(r8_full_doc(&back), full, "query does not mutate any document field");
                    let blank = lines(&back, sid).last().unwrap().clone();
                    assert!(blank.range.is_empty() && blank.glyphs.is_empty());
                    assert!(
                        (blank.leading - leading).abs() < 1e-6,
                        "blank EOF must compose the same effective leading as typing; got {}",
                        blank.leading
                    );
                    back.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
                    let typed = r8_full_doc(&back);
                    r9_history(&mut back, &full, &typed);
                    r8_query_metrics(&mut back, size, json!({"kind": "points", "value": leading}));
                    let actual = lines(&back, sid).last().unwrap().clone();
                    assert!((actual.leading - blank.leading).abs() < 1e-6);
                    assert!(
                        (actual.ascent - blank.ascent).abs() < 1e-6,
                        "blank EOF ascent must use the same effective named-style size as actual glyphs"
                    );
                    assert!((actual.descent - blank.descent).abs() < 1e-6);
                    assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().text, "Tail\né");
                    r8_prior_unchanged(&back, sid, &prior);
                    let f = back.doc().unwrap().doc.story(sid).unwrap().format_after(5);
                    assert_eq!(f.style, if based_on { "R9 child" } else { "R9 parent" });
                    assert_eq!(f.over.size, attrs["size"].as_f64(), "only explicitly staged size is a direct override");
                    assert_eq!(f.over.leading, attrs["leading"]["value"].as_f64().map(Leading::Points));
                    if tracking {
                        assert_eq!(f.over.change, Some(ChangeMark::Inserted));
                    }
                    r8_zip_reopen(&mut back);
                    r9_legacy_reopen(&back);
                }
            }
        }
    }
}

#[test]
fn r9_eof_named_size_stage_blank_composition_matches_typed_metrics() {
    r9_stage_metrics(json!({"size": 30.0}), 30.0, 32.0);
}

#[test]
fn r9_eof_based_on_leading_stage_blank_composition_matches_typed_metrics() {
    r9_stage_metrics(json!({"leading": {"kind": "points", "value": 40.0}}), 18.0, 40.0);
}

#[test]
fn r9_eof_named_both_stage_metrics_preservation_control() {
    r9_stage_metrics(json!({"size": 30.0, "leading": {"kind": "points", "value": 40.0}}), 30.0, 40.0);
}

#[test]
fn r9_eof_inherited_paragraph_chars_remain_live_after_typing() {
    for based_on in [false, true] {
        for tracking in [false, true] {
            let (mut s, fid, sid) = r9_named_fixture(true, based_on, json!({"fontStyle": "Italic"}));
            let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
            if tracking {
                s.execute("changes.track", &json!({"on": true})).unwrap();
            }
            r9_enter(&mut s, fid, sid);
            let blank = r8_full_doc(&s);
            for text in ["é", "à"] {
                s.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
            }
            let typed = r8_full_doc(&s);
            // Two independent insertions must both have complete-document history.
            s.execute("edit.undo", &json!({})).unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(r8_full_doc(&s), blank);
            s.execute("edit.redo", &json!({})).unwrap();
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(r8_full_doc(&s), typed);
            for reopen in 0..2 {
                let mut back = if reopen == 0 { r8_zip_reopen(&mut s) } else { r9_legacy_reopen(&s) };
                r9_style_edit(&mut back);
                r9_select_query(&mut back, sid, 0, 4, 30.0, 32.0);
                r9_select_query(&mut back, sid, 5, 9, 30.0, 32.0);
                let st = back.doc().unwrap().doc.story(sid).unwrap();
                assert_eq!(st.text, "Tail\néà");
                assert_eq!(st.paras[0], original.paras[0]);
                assert_eq!(st.format_after(0), original.format_after(0));
                assert_eq!(st.paras[1].chars, original.paras[0].chars, "populated continuation retains inherited paragraph defaults");
                for byte in [5, 7] {
                    let f = st.format_after(byte);
                    assert_eq!(f.style, if based_on { "R9 child" } else { "R9 parent" });
                    assert_eq!(f.over.size, None, "inherited paragraph size must never become a direct run override");
                    assert_eq!(f.over.leading, None);
                    assert_eq!(f.over.change, tracking.then_some(ChangeMark::Inserted));
                }
                assert!((lines(&back, sid).last().unwrap().leading - 32.0).abs() < 1e-6);
                back.doc().unwrap().doc.check().unwrap();
                r8_zip_reopen(&mut back);
                r9_legacy_reopen(&back);
            }
        }
    }
}

#[test]
fn r9_eof_inherited_paragraph_chars_remain_live_after_blank_reopen() {
    for tracking in [false, true] {
        let (mut s, fid, sid) = r9_named_fixture(true, true, json!({"fontStyle": "Italic"}));
        if tracking {
            s.execute("changes.track", &json!({"on": true})).unwrap();
        }
        r9_enter(&mut s, fid, sid);
        for reopen in 0..2 {
            let mut back = if reopen == 0 { r8_zip_reopen(&mut s) } else { r9_legacy_reopen(&s) };
            eof(&mut back, fid, sid);
            let _ = lines(&back, sid);
            r9_style_edit(&mut back);
            back.cache.clear();
            eof(&mut back, fid, sid);
            r8_query_metrics(&mut back, 30.0, json!({"kind": "points", "value": 32.0}));
            assert!((lines(&back, sid).last().unwrap().leading - 32.0).abs() < 1e-6);
            let blank = r8_full_doc(&back);
            back.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
            let typed = r8_full_doc(&back);
            r9_history(&mut back, &blank, &typed);
            r8_query_metrics(&mut back, 30.0, json!({"kind": "points", "value": 32.0}));
            r9_select_query(&mut back, sid, 0, 4, 30.0, 32.0);
            assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().format_after(5).over.size, None);
            r8_zip_reopen(&mut back);
            r9_legacy_reopen(&back);
        }
    }
}

fn r9_equal_value_stage(inherited: bool) {
    let (mut s, fid, sid) = r9_named_fixture(inherited, true, json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}}));
    r9_enter(&mut s, fid, sid);
    // An explicit choice equal to the current inherited value is still a direct override.
    s.execute("type.char", &json!({"attrs": {"size": 24.0, "leading": {"kind": "points", "value": 26.0}}})).unwrap();
    let staged = r8_full_doc(&s);
    for reopen in 0..2 {
        let mut back = if reopen == 0 { r8_zip_reopen(&mut s) } else { r9_legacy_reopen(&s) };
        assert_eq!(r8_full_doc(&back), staged);
        eof(&mut back, fid, sid);
        r9_style_edit(&mut back);
        eof(&mut back, fid, sid);
        r8_query_metrics(&mut back, 24.0, json!({"kind": "points", "value": 26.0}));
        assert!((lines(&back, sid).last().unwrap().leading - 26.0).abs() < 1e-6);
        let blank = r8_full_doc(&back);
        back.execute("text.insert", &json!({"text": "éà", "raw": true})).unwrap();
        let typed = r8_full_doc(&back);
        r9_history(&mut back, &blank, &typed);
        r9_select_query(&mut back, sid, 0, 4, 30.0, 32.0);
        r9_select_query(&mut back, sid, 5, 9, 24.0, 26.0);
        r9_select_query(&mut back, sid, 5, 7, 24.0, 26.0);
        let before_clear = r8_full_doc(&back);
        back.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
        let cleared = r8_full_doc(&back);
        r9_history(&mut back, &before_clear, &cleared);
        r9_select_query(&mut back, sid, 5, 7, 30.0, 32.0);
        r9_select_query(&mut back, sid, 7, 9, 24.0, 26.0);
        let before_cut = r8_full_doc(&back);
        assert_eq!(back.execute("edit.cut", &json!({})).unwrap()["text"], "à");
        let cut = r8_full_doc(&back);
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().text, "Tail\né");
        r9_history(&mut back, &before_cut, &cut);
        back.execute("text.select", &json!({"story": sid.0, "anchor": 5, "focus": 7})).unwrap();
        back.execute("style.character.apply", &json!({"name": "[None]"})).unwrap();
        let q = back.execute("type.selectionAttrs", &json!({})).unwrap();
        assert_eq!(q["chars"]["size"], json!(if inherited { 24.0 } else { 12.0 }), "clearing the named style reveals the actual paragraph baseline");
        assert_eq!(q["chars"]["leading"], if inherited { json!({"kind": "points", "value": 26.0}) } else { json!({"kind": "auto"}) });
        back.doc().unwrap().doc.check().unwrap();
        r8_zip_reopen(&mut back);
        r9_legacy_reopen(&back);
    }
}

#[test]
fn r9_eof_equal_value_true_stage_survives_live_style_edit_control() {
    r9_equal_value_stage(false);
}

#[test]
fn r9_eof_equal_value_stage_over_inherited_paragraph_chars_is_explicit() {
    r9_equal_value_stage(true);
}

#[test]
fn r9_eof_inherited_paragraph_defaults_clear_with_paragraph_style_control() {
    let (mut s, fid, sid) = r9_named_fixture(true, true, json!({"fontStyle": "Italic"}));
    r9_enter(&mut s, fid, sid);
    let before = r8_full_doc(&s);
    s.execute("style.paragraph.apply", &json!({"name": "[Basic Paragraph]", "clearOverrides": true})).unwrap();
    let after = r8_full_doc(&s);
    r9_history(&mut s, &before, &after);
    eof(&mut s, fid, sid);
    s.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    r8_query_metrics(&mut s, 12.0, json!({"kind": "auto"}));
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras[0].chars.size, Some(24.0), "clear affects only the selected final paragraph");
    assert!(s.doc().unwrap().doc.story(sid).unwrap().paras[1].chars.is_empty());
    s.doc().unwrap().doc.check().unwrap();
}

#[test]
fn r32_eof_clear_overrides_does_not_restore_lf_metrics() {
    let (mut s, _, sid) = frame("A");
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 24, "leading": {"kind": "points", "value": 30}}})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 1, "focus": 1})).unwrap();
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    let lf = s.doc().unwrap().doc.story(sid).unwrap().format_after(1).clone();
    r8_query_metrics(&mut s, 24.0, json!({"kind": "points", "value": 30.0}));
    s.execute("style.paragraph.create", &json!({"name": "R32 cleared", "chars": {"size": 18, "leading": {"kind": "points", "value": 22}}})).unwrap();
    let before = r8_full_doc(&s);
    s.execute("style.paragraph.apply", &json!({"name": "R32 cleared", "clearOverrides": true})).unwrap();
    let cleared = r8_full_doc(&s);
    r9_history(&mut s, &before, &cleared);
    r8_query_metrics(&mut s, 18.0, json!({"kind": "points", "value": 22.0}));
    assert_eq!(lines(&s, sid).last().unwrap().leading, 22.0, "blank metrics agree with the cleared typing format");
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.format_after(1), &lf);
    assert!(st.paras[1].chars.is_empty());
    assert!(st.paras[1].eof_inherited_chars.as_ref().is_some_and(|attrs| attrs.is_empty()));

    s.execute("text.insert", &json!({"text": "Z", "raw": true})).unwrap();
    let typed = r8_full_doc(&s);
    r9_history(&mut s, &cleared, &typed);
    r8_query_metrics(&mut s, 18.0, json!({"kind": "points", "value": 22.0}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\nZ");
    assert_eq!(st.format_after(1), &lf, "clearing the EOF never clears the preceding paragraph's LF");
    assert!(st.format_after(2).over.is_empty());
    assert_eq!(st.format_after(2).style, lf.style);
    assert!(st.paras[1].chars.is_empty());
    assert!(st.paras[1].eof_inherited_chars.is_none());
}

#[test]
fn r9_eof_inherited_paragraph_break_link_preserves_live_character_binding() {
    let (mut s, fid, sid) = r9_named_fixture(true, true, json!({"size": 30.0, "leading": {"kind": "points", "value": 32.0}}));
    r9_enter(&mut s, fid, sid);
    r8_query_metrics(&mut s, 30.0, json!({"kind": "points", "value": 32.0}));
    let before = r8_full_doc(&s);
    s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    let after = r8_full_doc(&s);
    r9_history(&mut s, &before, &after);
    eof(&mut s, fid, sid);
    r8_query_metrics(&mut s, 30.0, json!({"kind": "points", "value": 32.0}));
    assert!((lines(&s, sid).last().unwrap().leading - 32.0).abs() < 1e-6);
    s.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    r8_query_metrics(&mut s, 30.0, json!({"kind": "points", "value": 32.0}));
    s.execute("style.character.edit", &json!({"name": "R9 parent", "chars": {"size": 36, "leading": {"kind": "points", "value": 44}}})).unwrap();
    r9_select_query(&mut s, sid, 5, 7, 36.0, 44.0);
    s.execute("style.character.apply", &json!({"name": "[None]"})).unwrap();
    r8_query_metrics(&mut s, 24.0, json!({"kind": "points", "value": 26.0}));
    s.doc().unwrap().doc.check().unwrap();
}

// R10: only the two EOF review regressions and their existing formatted-insertion seams.
// All edits use public Session commands; legacy input changes only explicit fixture attrs.
fn r10_edit(s: &mut Session, command: &str, params: &serde_json::Value) -> serde_json::Value {
    let before = r8_full_doc(s);
    let depth = s.doc().unwrap().history.undo.len();
    let result = s.execute(command, params).unwrap();
    let after = r8_full_doc(s);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "{command} is one history entry");
    r9_history(s, &before, &after);
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn r10_query(s: &mut Session, sid: StoryId, range: std::ops::Range<usize>, size: f64, leading: serde_json::Value) -> serde_json::Value {
    s.execute("text.select", &json!({"story": sid.0, "anchor": range.start, "focus": range.end})).unwrap();
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["chars"]["size"], json!(size));
    assert_eq!(q["chars"]["leading"], leading);
    q
}

fn r10_legacy_input(full: &serde_json::Value) -> Session {
    let mut s = Session::new();
    s.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&serde_json::to_vec(full).unwrap())})).unwrap();
    assert_eq!(r8_full_doc(&s), *full, "the exact legacy fixture must survive the public reader");
    s
}

fn r10_break_link_case(inherited: bool, staged: bool, whole: bool) {
    let (mut s, _, sid) = r9_named_fixture(inherited, true, json!({"size": 30.0, "leading": {"kind": "points", "value": 32.0}}));
    s.execute("style.paragraph.create", &json!({"name": "R10 paragraph", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
        .unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 4})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "R10 paragraph"})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 4})).unwrap();
    r10_edit(&mut s, "text.insert", &json!({"text": "\n", "raw": true}));
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras[1].eof_inherited_chars.is_some(), inherited);
    if staged {
        r10_edit(&mut s, "type.char", &json!({"attrs": {"tracking": 10}}));
    }
    let sparse = if staged { json!({"tracking": 10.0}) } else { json!({}) };
    let q = r10_query(&mut s, sid, 5..5, 30.0, json!({"kind": "points", "value": 32.0}));
    assert_eq!(q["characterStyle"], "R9 child");
    assert_eq!(q["charOverrides"], json!(usize::from(staged)));
    // The None case must stay absent through the real legacy reader, not a synthetic Some({}).
    let mut s = r9_legacy_reopen(&s);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras[1].eof_inherited_chars.is_some(), inherited);
    let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
    s.execute("text.select", &json!({"story": sid.0, "anchor": if whole { 0 } else { 5 }, "focus": 5})).unwrap();
    r10_edit(&mut s, "style.breakLink", &json!({"kind": "paragraph"}));
    let q = r10_query(&mut s, sid, 5..5, 30.0, json!({"kind": "points", "value": 32.0}));
    assert_eq!(q["characterStyle"], "R9 child");
    assert_eq!(q["charOverrides"], json!(usize::from(staged)));
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.chars, prior.chars, "paragraph break-link does not rewrite existing runs");
    if !whole {
        assert_eq!(story.paras[0], prior.paras[0], "preceding paragraph is outside the target");
    }
    let pf = &story.paras[1];
    assert_eq!(pf.style, designcraft_doc::styles::NO_PARA_STYLE);
    assert_eq!(serde_json::to_value(&pf.chars).unwrap(), sparse);
    assert_eq!(serde_json::to_value(&pf.eof_inherited_chars).unwrap(), json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}}));
    assert!((lines(&s, sid).last().unwrap().leading - 32.0).abs() < 1e-6);
    for mut back in [r8_zip_reopen(&mut s), r9_legacy_reopen(&s)] {
        back.execute("text.select", &json!({"story": sid.0, "anchor": 5})).unwrap();
        r10_edit(&mut back, "text.insert", &json!({"text": "é", "raw": true}));
        let story = back.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, "Tail\né");
        assert_eq!(serde_json::to_value(&story.format_after(5).over).unwrap(), sparse);
        assert!(story.paras[1].eof_inherited_chars.is_none());
        assert_eq!(serde_json::to_value(&story.paras[1].chars).unwrap(), json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}}));
        r10_edit(&mut back, "style.character.edit", &json!({"name": "R9 parent", "chars": {"size": 36, "leading": {"kind": "points", "value": 44}}}));
        r10_query(&mut back, sid, 5..7, 36.0, json!({"kind": "points", "value": 44.0}));
        r10_edit(&mut back, "style.character.apply", &json!({"name": "[None]"}));
        let q = r10_query(&mut back, sid, 5..7, 24.0, json!({"kind": "points", "value": 26.0}));
        assert_eq!(q["charOverrides"], json!(usize::from(staged)));
        r8_zip_reopen(&mut back);
    }
}

#[test]
fn r10_eof_break_link_none_keeps_live_character_style_and_sparse_overrides() {
    for staged in [false, true] {
        for whole in [false, true] {
            r10_break_link_case(false, staged, whole);
        }
    }
}

#[test]
fn r10_eof_break_link_existing_inherited_baseline_control() {
    for staged in [false, true] {
        r10_break_link_case(true, staged, false);
    }
}

#[test]
fn r10_eof_break_link_unstaged_legacy_lf_keeps_actual_typing_overrides() {
    let (mut s, _, sid) = frame("Tail\n");
    s.execute("style.paragraph.create", &json!({"name": "R10 paragraph", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
        .unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 5})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "R10 paragraph"})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 30, "leading": {"kind": "points", "value": 32}}})).unwrap();
    let mut s = r9_legacy_reopen(&s);
    let pf = &s.doc().unwrap().doc.story(sid).unwrap().paras[1];
    assert!(pf.chars.is_empty() && pf.eof_inherited_chars.is_none());
    r10_query(&mut s, sid, 5..5, 30.0, json!({"kind": "points", "value": 32.0}));
    r10_edit(&mut s, "style.breakLink", &json!({"kind": "paragraph"}));
    r10_query(&mut s, sid, 5..5, 30.0, json!({"kind": "points", "value": 32.0}));
    let pf = &s.doc().unwrap().doc.story(sid).unwrap().paras[1];
    assert_eq!(serde_json::to_value(&pf.chars).unwrap(), json!({"size": 30.0, "leading": {"kind": "points", "value": 32.0}}));
    assert_eq!(serde_json::to_value(&pf.eof_inherited_chars).unwrap(), json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}}));
    r10_edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    let q = r10_query(&mut s, sid, 5..5, 24.0, json!({"kind": "points", "value": 26.0}));
    assert_eq!(q["charOverrides"], 0);
    r10_edit(&mut s, "text.insert", &json!({"text": "é", "raw": true}));
    assert!(s.doc().unwrap().doc.story(sid).unwrap().format_after(5).over.is_empty());
    r8_zip_reopen(&mut s);
}

fn r10_paste_case(inherited: bool, staged: bool, multiline: bool) {
    let (mut s, _, dst) = frame("First");
    if inherited {
        let mut full = r8_full_doc(&s);
        full["stories"][dst.0.to_string()]["paras"][0]["chars"] = json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}});
        s = r10_legacy_input(&full);
    }
    s.execute("style.paragraph.create", &json!({"name": "R10 destination", "chars": {}})).unwrap();
    s.execute("text.select", &json!({"story": dst.0, "anchor": 5})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "R10 destination"})).unwrap();
    r10_edit(&mut s, "text.insert", &json!({"text": "\n", "raw": true}));
    if staged {
        r10_edit(&mut s, "type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}}));
    }
    let f = s
        .execute("frame.create", &json!({"rect": [40, 510, 440, 760], "content": "text", "text": if multiline { "éà\nZ\n" } else { "éà" }}))
        .unwrap();
    let src = StoryId(f["story"].as_u64().unwrap());
    s.execute("text.select", &json!({"story": src.0, "anchor": 4})).unwrap();
    s.execute("footnote.insert", &json!({"text": "Keep this citation — Café 李"})).unwrap();
    s.execute("footnote.goToReference", &json!({})).unwrap();
    let n = s.doc().unwrap().doc.story(src).unwrap().len();
    s.execute("text.select", &json!({"story": src.0, "anchor": 0, "focus": n})).unwrap();
    s.execute("style.character.create", &json!({"name": "R10 copied", "chars": {"tracking": 31}})).unwrap();
    s.execute("style.character.apply", &json!({"name": "R10 copied"})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 22, "leading": {"kind": "points", "value": 40}}})).unwrap();
    // Clipboard is populated after reopen: the native document persists the stage, not the clipboard.
    let mut s = r8_zip_reopen(&mut s);
    let source = s.doc().unwrap().doc.story(src).unwrap().clone();
    let prior = s.doc().unwrap().doc.story(dst).unwrap().clone();
    assert_eq!(prior.paras[1].eof_inherited_chars.is_some(), inherited);
    assert_eq!(
        serde_json::to_value(&prior.paras[1].chars).unwrap(),
        if staged { json!({"size": 20.0, "leading": {"kind": "points", "value": 36.0}}) } else { json!({}) }
    );
    s.execute("text.select", &json!({"story": src.0, "anchor": 0, "focus": n})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("text.select", &json!({"story": dst.0, "anchor": 6})).unwrap();
    let result = r10_edit(&mut s, "edit.paste", &json!({"formatted": true}));
    assert_eq!(result["formatted"], true);
    let dest = s.doc().unwrap().doc.story(dst).unwrap();
    assert_eq!(dest.text, format!("First\n{}", source.text));
    assert_eq!(*s.doc().unwrap().doc.story(src).unwrap(), source, "the full copy source remains exact");
    assert_eq!(dest.paras[0], prior.paras[0]);
    for (byte, _) in source.text.char_indices() {
        assert_eq!(dest.format_after(6 + byte), source.format_after(byte), "copied run at source UTF-8 byte {byte}");
    }
    assert_eq!(dest.notes.len(), 1);
    assert_eq!(dest.notes[0].text, source.notes[0].text, "the complete footnote story survives consumption");
    assert_eq!(dest.paras[2..], source.paras[1..], "later source paragraphs keep their own formats, including a final blank");
    let baseline = if inherited { json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}}) } else { json!({}) };
    assert_eq!(serde_json::to_value(&dest.paras[1].chars).unwrap(), baseline);
    assert!(dest.paras[1].eof_inherited_chars.is_none());
    r10_query(&mut s, dst, 6..8, 22.0, json!({"kind": "points", "value": 40.0}));
    r10_edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    let (size, leading) = if inherited { (24.0, json!({"kind": "points", "value": 26.0})) } else { (12.0, json!({"kind": "auto"})) };
    let q = r10_query(&mut s, dst, 6..8, size, leading);
    assert_eq!(q["charOverrides"], 0);
    assert_eq!(q["characterStyle"], "R10 copied");
    assert_eq!(q["chars"]["tracking"], json!(31.0));
    assert!(s.doc().unwrap().doc.story(dst).unwrap().format_after(6).over.is_empty());
    r10_query(&mut s, dst, 8..10, 22.0, json!({"kind": "points", "value": 40.0}));
    r10_edit(&mut s, "style.paragraph.edit", &json!({"name": "R10 destination", "chars": {"size": 14, "leading": {"kind": "points", "value": 22}}}));
    for mut back in [r8_zip_reopen(&mut s), r9_legacy_reopen(&s)] {
        let (size, leading) = if inherited { (24.0, 26.0) } else { (14.0, 22.0) };
        let q = r10_query(&mut back, dst, 6..8, size, json!({"kind": "points", "value": leading}));
        assert_eq!(q["charOverrides"], 0);
        assert_eq!(serde_json::to_value(&back.doc().unwrap().doc.story(dst).unwrap().paras[1].chars).unwrap(), baseline);
        assert_eq!(*back.doc().unwrap().doc.story(src).unwrap(), source);
        back.doc().unwrap().doc.check().unwrap();
    }
}

#[test]
fn r10_eof_formatted_paste_consumes_stage_and_preserves_runs_payload_and_history() {
    for inherited in [false, true] {
        for multiline in [false, true] {
            r10_paste_case(inherited, true, multiline);
        }
    }
}

#[test]
fn r10_eof_formatted_paste_unstaged_baseline_controls() {
    for inherited in [false, true] {
        r10_paste_case(inherited, false, false);
    }
}

#[test]
fn r10_eof_formatted_replace_preserves_ordinary_paragraph_defaults_control() {
    let (s, _, sid) = frame("First\nQ");
    let mut full = r8_full_doc(&s);
    full["stories"][sid.0.to_string()]["paras"][1]["chars"] = json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}});
    let mut s = r10_legacy_input(&full);
    let pf = s.doc().unwrap().doc.story(sid).unwrap().paras[1].clone();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // Deletion will temporarily make a blank EOF, but there was no typing stage to consume.
    s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 7})).unwrap();
    r10_edit(&mut s, "edit.paste", &json!({"formatted": true}));
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "First\nF");
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras[1], pf);
    let q = r10_query(&mut s, sid, 6..7, 24.0, json!({"kind": "points", "value": 26.0}));
    assert_eq!(q["charOverrides"], 0);
    r8_zip_reopen(&mut s);
}

#[test]
fn r10_eof_placed_text_consumes_stage_but_empty_placement_keeps_it() {
    let (mut s, _, sid) = frame("First\n");
    s.execute("text.select", &json!({"story": sid.0, "anchor": 6})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
    let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
    s.execute("file.place", &json!({"name": "empty.txt", "base64": ""})).unwrap();
    assert_eq!(*s.doc().unwrap().doc.story(sid).unwrap(), prior, "empty formatted insertion must not consume the stage");
    r10_edit(&mut s, "file.place", &json!({"name": "one.txt", "base64": crate::cmd::file::base64_encode("é".as_bytes())}));
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, "First\né");
    assert_eq!(story.paras[0], prior.paras[0]);
    assert!(story.paras[1].chars.is_empty() && story.paras[1].eof_inherited_chars.is_none());
    let q = r10_query(&mut s, sid, 6..8, 12.0, json!({"kind": "auto"}));
    assert_eq!(q["charOverrides"], 0);
    r8_zip_reopen(&mut s);
}

#[test]
fn r10_eof_drag_copy_and_move_consume_stage_keep_copied_format() {
    for copy in [false, true] {
        let (mut s, fid, sid) = frame("éX\n");
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 2})).unwrap();
        s.execute("type.char", &json!({"attrs": {"size": 22, "leading": {"kind": "points", "value": 40}}})).unwrap();
        let copied = s.doc().unwrap().doc.story(sid).unwrap().format_after(0).clone();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 4})).unwrap();
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}})).unwrap();
        let (press, drop) = {
            let ls = lines(&s, sid);
            let first = &ls[0];
            let glyph = first.glyphs.iter().find(|g| g.byte == 0 && g.len > 0).unwrap();
            let last = ls.last().unwrap();
            assert!(last.range.is_empty());
            (Point::new(glyph.x + glyph.adv * 0.25, first.baseline - first.ascent * 0.5), Point::new(last.end_x, last.baseline - last.ascent * 0.5))
        };
        let press = spread_point(&s, fid, press);
        let drop = spread_point(&s, fid, drop);
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 2})).unwrap();
        let armed = s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [press.x, press.y]})).unwrap();
        assert_eq!(armed["dragging"], true, "public press must arm a drag within the selected glyph");
        let result = r10_edit(&mut s, "text.release", &json!({"frame": fid.0, "point": [drop.x, drop.y], "moved": true, "copy": copy}));
        let at = if copy { 4 } else { 2 };
        assert_eq!(result["at"], json!(at));
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, if copy { "éX\né" } else { "X\né" });
        assert_eq!(story.format_after(at), &copied);
        assert!(story.paras[1].chars.is_empty() && story.paras[1].eof_inherited_chars.is_none());
        r10_query(&mut s, sid, at..at + 2, 22.0, json!({"kind": "points", "value": 40.0}));
        r10_edit(&mut s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
        let q = r10_query(&mut s, sid, at..at + 2, 12.0, json!({"kind": "auto"}));
        assert_eq!(q["charOverrides"], 0);
        r8_zip_reopen(&mut s);
    }
}

// R11: a former EOF owns its LF's format; only the true blank EOF retains a typing stage.
fn r11_separator_fixture(inherited: bool) -> (Session, StoryId) {
    let (s, _, sid) = frame("A");
    let mut s = if inherited {
        let mut full = r8_full_doc(&s);
        full["stories"][sid.0.to_string()]["paras"][0]["chars"] = json!({"size": 24.0, "leading": {"kind": "points", "value": 26.0}});
        r10_legacy_input(&full)
    } else {
        s
    };
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
    r10_edit(&mut s, "type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 36}}}));
    s.execute("text.select", &json!({"story": sid.0, "anchor": 1})).unwrap();
    r10_edit(&mut s, "text.insert", &json!({"text": "\n", "raw": true}));
    (s, sid)
}

fn r11_separator_baseline(inherited: bool) -> (f64, serde_json::Value) {
    if inherited { (24.0, json!({"kind": "points", "value": 26.0})) } else { (12.0, json!({"kind": "auto"})) }
}

fn r11_consumed_paragraph(s: &Session, sid: StoryId, pi: usize, inherited: bool) {
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    let pf = &story.paras[pi];
    assert_eq!(pf.chars, story.paras[0].chars, "a former EOF restores the actual paragraph baseline");
    assert!(pf.eof_inherited_chars.is_none(), "the paragraph's LF or text now owns the direct typing format");
    let (size, leading) = r11_separator_baseline(inherited);
    let base = s.doc().unwrap().doc.styles.resolve_para(pf).1;
    assert_eq!(base.size, size);
    assert_eq!(serde_json::to_value(base.leading).unwrap(), leading);
}

#[test]
fn r11_eof_public_repeated_enter_clear_matches_main_cli_reproduction() {
    let (mut s, _, sid) = frame("");
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0})).unwrap();
    s.execute("text.insert", &json!({"text": "A", "raw": true})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 28}}})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 1})).unwrap();
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 2})).unwrap();
    s.execute("text.insert", &json!({"text": "X", "raw": true})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 3})).unwrap();
    let before_clear = r8_full_doc(&s);
    s.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["chars"]["size"], json!(12.0));
    assert_eq!(q["chars"]["leading"], json!({"kind": "auto"}));
    assert_eq!(q["charOverrides"], 0);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "A\nX\n");
    let after_clear = r8_full_doc(&s);
    r9_history(&mut s, &before_clear, &after_clear);
    for mut back in [r8_zip_reopen(&mut s), r9_legacy_reopen(&s)] {
        let q = r10_query(&mut back, sid, 2..3, 12.0, json!({"kind": "auto"}));
        assert_eq!(q["charOverrides"], 0);
        r11_consumed_paragraph(&back, sid, 1, false);
        back.doc().unwrap().doc.check().unwrap();
    }
}

#[test]
fn r11_eof_repeated_enter_former_blank_text_clear_returns_to_paragraph_baseline() {
    for inherited in [false, true] {
        let (mut s, sid) = r11_separator_fixture(inherited);
        r10_edit(&mut s, "text.insert", &json!({"text": "\n", "raw": true}));
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "A\n\n");
        // Revisit the former EOF after both supported native readers, not just the final EOF.
        for mut back in [r8_zip_reopen(&mut s), r9_legacy_reopen(&s)] {
            back.execute("text.select", &json!({"story": sid.0, "anchor": 2})).unwrap();
            r10_edit(&mut back, "text.insert", &json!({"text": "X", "raw": true}));
            assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().text, "A\nX\n");
            r10_query(&mut back, sid, 2..3, 20.0, json!({"kind": "points", "value": 36.0}));
            r10_edit(&mut back, "type.char", &json!({"attrs": {"size": null}}));
            let (size, leading) = r11_separator_baseline(inherited);
            let q = r10_query(&mut back, sid, 2..3, size, json!({"kind": "points", "value": 36.0}));
            assert_eq!(q["charOverrides"], 1);
            r10_edit(&mut back, "type.char", &json!({"attrs": {"leading": null}}));
            let q = r10_query(&mut back, sid, 2..3, size, leading);
            assert_eq!(q["charOverrides"], 0);
            assert!(back.doc().unwrap().doc.story(sid).unwrap().format_after(2).over.is_empty());
            r11_consumed_paragraph(&back, sid, 1, inherited);
            r8_zip_reopen(&mut back);
            r9_legacy_reopen(&back);
        }
    }
}

#[test]
fn r11_eof_former_blank_separator_clear_changes_metrics_and_preserves_final_stage() {
    for inherited in [false, true] {
        let (mut s, sid) = r11_separator_fixture(inherited);
        r10_edit(&mut s, "text.insert", &json!({"text": "\n", "raw": true}));
        r10_query(&mut s, sid, 2..3, 20.0, json!({"kind": "points", "value": 36.0}));
        // Select the earlier blank paragraph's LF itself, with no new glyph insertion.
        r10_edit(&mut s, "type.char", &json!({"attrs": {"leading": null}}));
        let (size, leading) = r11_separator_baseline(inherited);
        let q = r10_query(&mut s, sid, 2..3, 20.0, leading.clone());
        assert_eq!(q["charOverrides"], 1);
        let blank = lines(&s, sid).into_iter().find(|line| line.para == 1).unwrap();
        assert!(blank.glyphs.is_empty());
        assert!((blank.leading - if inherited { 26.0 } else { 24.0 }).abs() < 1e-6);
        r10_edit(&mut s, "type.char", &json!({"attrs": {"size": null}}));
        let q = r10_query(&mut s, sid, 2..3, size, leading);
        assert_eq!(q["charOverrides"], 0);
        let blank = lines(&s, sid).into_iter().find(|line| line.para == 1).unwrap();
        assert!((blank.leading - if inherited { 26.0 } else { 14.4 }).abs() < 1e-6);
        r11_consumed_paragraph(&s, sid, 1, inherited);
        // Clearing the LF must not clear the independently persisted final typing stage.
        for mut back in [r8_zip_reopen(&mut s), r9_legacy_reopen(&s)] {
            r10_query(&mut back, sid, 3..3, 20.0, json!({"kind": "points", "value": 36.0}));
            r10_edit(&mut back, "text.insert", &json!({"text": "é", "raw": true}));
            r10_query(&mut back, sid, 3..5, 20.0, json!({"kind": "points", "value": 36.0}));
            r11_consumed_paragraph(&back, sid, 2, inherited);
            r8_zip_reopen(&mut back);
        }
    }
}

#[test]
fn r11_eof_separator_consumption_handles_newline_only_and_multiline_insertions() {
    for inherited in [false, true] {
        for text in ["\n", "\n\n", "\nTail\n", "Tail\n\n", "\nTail", "\n\nTail"] {
            let (mut s, sid) = r11_separator_fixture(inherited);
            let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
            r10_edit(&mut s, "text.insert", &json!({"text": text, "raw": true}));
            let story = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(story.text, format!("A\n{text}"));
            assert_eq!(story.paras[0], before.paras[0]);
            assert_eq!(story.format_after(0), before.format_after(0), "the original glyph remains exact");
            assert_eq!(story.format_after(1), before.format_after(1), "the original LF remains exact");
            for (offset, _) in text.char_indices() {
                let fmt = story.format_after(2 + offset);
                assert_eq!(fmt.over.size, Some(20.0));
                assert_eq!(fmt.over.leading, Some(Leading::Points(36.0)));
            }
            let ranges = story.para_ranges();
            for (pi, range) in ranges.iter().enumerate().skip(1) {
                if pi + 1 == ranges.len() && range.is_empty() {
                    let pf = &story.paras[pi];
                    assert_eq!(pf.chars.size, Some(20.0));
                    assert_eq!(pf.chars.leading, Some(Leading::Points(36.0)));
                    assert_eq!(pf.eof_inherited_chars, inherited.then(|| before.paras[0].chars.clone()));
                } else {
                    r11_consumed_paragraph(&s, sid, pi, inherited);
                }
            }
            assert!(lines(&s, sid).iter().skip(1).all(|line| (line.leading - 36.0).abs() < 1e-6));
            r8_zip_reopen(&mut s);
            r9_legacy_reopen(&s);
        }
    }
}

#[test]
fn r11_eof_true_final_stage_still_types_and_clears_to_inherited_defaults() {
    // Preservation control: it must also pass with only tests.patch applied.
    for inherited in [false, true] {
        let (mut s, sid) = r11_separator_fixture(inherited);
        r10_edit(&mut s, "text.insert", &json!({"text": "\n\n", "raw": true}));
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        for mut back in [r8_zip_reopen(&mut s), r9_legacy_reopen(&s)] {
            let at = original.len();
            r10_query(&mut back, sid, at..at, 20.0, json!({"kind": "points", "value": 36.0}));
            r10_edit(&mut back, "text.insert", &json!({"text": "é", "raw": true}));
            r10_query(&mut back, sid, at..at + 2, 20.0, json!({"kind": "points", "value": 36.0}));
            r10_edit(&mut back, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
            let (size, leading) = r11_separator_baseline(inherited);
            let q = r10_query(&mut back, sid, at..at + 2, size, leading);
            assert_eq!(q["charOverrides"], 0);
            let story = back.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(story.paras[..story.paras.len() - 1], original.paras[..original.paras.len() - 1]);
            for byte in 0..at {
                assert_eq!(story.format_after(byte), original.format_after(byte));
            }
            r11_consumed_paragraph(&back, sid, original.paras.len() - 1, inherited);
            r8_zip_reopen(&mut back);
            r9_legacy_reopen(&back);
        }
    }
}

#[test]
fn fc2_blank_eof_clear_last_override_preserves_explicit_empty_stage() {
    for reopen in [false, true] {
        let (mut s, _, sid) = frame("A");
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
        s.execute("type.char", &json!({"attrs": {"size": 30}})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 1, "focus": 1})).unwrap();
        s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let pf = before.story(sid).unwrap().paras.last().unwrap();
        assert_eq!(pf.chars.size, Some(30.0));
        assert!(pf.eof_inherited_chars.is_none(), "exercise a stage without a paragraph baseline");
        s.execute("type.char", &json!({"attrs": {"size": null}})).unwrap();
        let cleared = s.doc().unwrap().doc.clone();
        let st = cleared.story(sid).unwrap();
        let pf = st.paras.last().unwrap();
        assert!(pf.chars.is_empty());
        assert!(pf.eof_inherited_chars.as_ref().is_some_and(|a| a.is_empty()));
        assert_eq!(st.chars, before.story(sid).unwrap().chars, "clearing EOF must not rewrite its preceding LF");
        assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(12.0));
        assert!((lines(&s, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *before);
        assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(30.0));
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *cleared);
        if reopen {
            let saved = s.execute("file.serialize", &json!({})).unwrap();
            let mut back = Session::new();
            back.execute("file.openBytes", &json!({"base64": saved["base64"]})).unwrap();
            assert_eq!(*back.doc().unwrap().doc, *cleared);
            back.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 2})).unwrap();
            s = back;
        }
        assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(12.0));
        assert!((lines(&s, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
        s.execute("text.insert", &json!({"text": "B", "raw": true})).unwrap();
        let d = &s.doc().unwrap().doc;
        let st = d.story(sid).unwrap();
        assert_eq!(st.text, "A\nB");
        assert_eq!(st.format_after(1).over.size, Some(30.0));
        assert_eq!(st.format_after(2).over.size, None);
        assert_eq!(d.styles.resolve_char(&d.styles.resolve_para(&st.paras[1]).1, st.format_after(2)).size, 12.0);
        assert!((lines(&s, sid).last().unwrap().leading - 14.4).abs() < 1e-6);
        d.check().unwrap();
    }
}

#[test]
fn fc2_warichu_toggle_reads_blank_eof_stage() {
    for preceding_on in [false, true] {
        let (mut s, _, sid) = frame("A");
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
        s.execute("type.warichu", &json!({"on": preceding_on})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 1, "focus": 1})).unwrap();
        s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        s.execute("type.warichu", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras[1].chars.warichu, Some(!preceding_on));
        let first = s.doc().unwrap().doc.clone();
        s.execute("type.warichu", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras[1].chars.warichu, Some(preceding_on));
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().chars, before.story(sid).unwrap().chars);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *first);
        s.execute("edit.redo", &json!({})).unwrap();
        let saved = s.execute("file.serialize", &json!({})).unwrap();
        let mut back = Session::new();
        back.execute("file.openBytes", &json!({"base64": saved["base64"]})).unwrap();
        back.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 2})).unwrap();
        back.execute("type.warichu", &json!({})).unwrap();
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().paras[1].chars.warichu, Some(!preceding_on));
        back.execute("text.insert", &json!({"text": "BC", "raw": true})).unwrap();
        let st = back.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.format_after(1).over.warichu, Some(preceding_on));
        assert_eq!(st.format_after(2).over.warichu, Some(!preceding_on));
        back.doc().unwrap().doc.check().unwrap();
    }
}

fn r35_unstaged_eof(attrs: serde_json::Value) -> (Session, StoryId) {
    let (mut s, _, sid) = frame("A\n");
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 2})).unwrap();
    s.execute("type.char", &json!({"attrs": attrs})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 2})).unwrap();
    let pf = s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap();
    assert!(pf.chars.is_empty() && pf.eof_inherited_chars.is_none(), "range formatting must leave the EOF unstaged");
    (s, sid)
}

#[test]
fn r35_unstaged_eof_partial_format_retains_size_through_typing_history_and_reopen() {
    let (mut s, sid) = r35_unstaged_eof(json!({"size": 24}));
    let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
    assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(24.0));
    r10_edit(&mut s, "type.char", &json!({"attrs": {"tracking": 75}}));
    let expected = designcraft_doc::CharAttrs { size: Some(24.0), tracking: Some(75.0), ..Default::default() };
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().chars, expected);
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["chars"]["size"], json!(24.0));
    assert_eq!(q["chars"]["tracking"], json!(75.0));
    let mut back = r8_zip_reopen(&mut s);
    back.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 2})).unwrap();
    r10_edit(&mut back, "text.insert", &json!({"text": "X", "raw": true}));
    let st = back.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\nX");
    assert_eq!(st.format_after(2).over, expected);
    assert!(st.paras.last().unwrap().chars.is_empty(), "the populated paragraph must not retain typing attrs as defaults");
    r8_prior_unchanged(&back, sid, &prior);
    let saved = r8_zip_reopen(&mut back);
    assert_eq!(saved.doc().unwrap().doc.story(sid).unwrap().format_after(2).over, expected);
}

#[test]
fn r35_unstaged_eof_sparse_clear_preserves_other_attrs_and_live_named_style() {
    let (mut s, sid) = r35_unstaged_eof(json!({"size": 24, "tracking": 75}));
    s.execute("style.character.create", &json!({"name": "R35 Parent", "chars": {"size": 18}})).unwrap();
    s.execute("style.character.create", &json!({"name": "R35 Child", "basedOn": "R35 Parent"})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 2})).unwrap();
    s.execute("style.character.apply", &json!({"name": "R35 Child"})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 2})).unwrap();
    r10_edit(&mut s, "type.char", &json!({"attrs": {"size": null}}));
    let expected = designcraft_doc::CharAttrs { tracking: Some(75.0), ..Default::default() };
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().chars, expected);
    assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(18.0));
    r10_edit(&mut s, "style.character.edit", &json!({"name": "R35 Parent", "chars": {"size": 30}}));
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["characterStyle"], json!("R35 Child"));
    assert_eq!(q["chars"]["size"], json!(30.0));
    assert_eq!(q["chars"]["tracking"], json!(75.0));
    assert_eq!(q["charOverrides"], json!(1));
    r10_edit(&mut s, "text.insert", &json!({"text": "é", "raw": true}));
    let mut back = r8_zip_reopen(&mut s);
    let st = back.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\né");
    assert_eq!(st.format_after(2).style, "R35 Child");
    assert_eq!(st.format_after(2).over, expected);
    assert_eq!(st.format_after(1).over.size, Some(24.0), "clearing the EOF must preserve the LF override");
    back.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 4})).unwrap();
    r10_edit(&mut back, "style.character.edit", &json!({"name": "R35 Parent", "chars": {"size": 32}}));
    assert_eq!(back.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(32.0));
}

#[test]
fn r35_existing_or_explicitly_cleared_eof_stage_is_not_reseeded() {
    for clear_all in [false, true] {
        let (mut s, sid) = r35_unstaged_eof(json!({"size": 24, "tracking": 50}));
        r10_edit(&mut s, "type.char", &json!({"attrs": {"size": 30}}));
        let clear = if clear_all { json!({"size": null, "tracking": null}) } else { json!({"size": null}) };
        r10_edit(&mut s, "type.char", &json!({"attrs": clear}));
        let pf = s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap();
        assert_eq!(pf.chars.size, None);
        assert_eq!(pf.chars.tracking, if clear_all { None } else { Some(50.0) });
        assert_eq!(pf.eof_inherited_chars, clear_all.then(designcraft_doc::CharAttrs::default));
        let mut back = r8_zip_reopen(&mut s);
        back.execute("text.select", &json!({"story": sid.0, "anchor": 2, "focus": 2})).unwrap();
        r10_edit(&mut back, "type.char", &json!({"attrs": {"baselineShift": 2}}));
        let expected =
            designcraft_doc::CharAttrs { tracking: if clear_all { None } else { Some(50.0) }, baseline_shift: Some(2.0), ..Default::default() };
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().chars, expected);
        assert_eq!(back.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(12.0));
        r10_edit(&mut back, "text.insert", &json!({"text": "X", "raw": true}));
        let st = back.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.format_after(2).over, expected);
        assert_eq!(st.format_after(1).over.size, Some(24.0));
        assert_eq!(st.format_after(1).over.tracking, Some(50.0));
    }
}

#[test]
fn r35_first_eof_stage_drops_transient_change_marks() {
    for mark in [ChangeMark::Inserted, ChangeMark::Deleted] {
        let (mut s, sid) = r35_unstaged_eof(json!({"size": 24, "change": mark}));
        r10_edit(&mut s, "type.char", &json!({"attrs": {"tracking": 75}}));
        let expected = designcraft_doc::CharAttrs { size: Some(24.0), tracking: Some(75.0), ..Default::default() };
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().chars, expected);
        r10_edit(&mut s, "text.insert", &json!({"text": "X", "raw": true}));
        let st = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.format_after(1).over.change, Some(mark), "the LF retains its original change annotation");
        assert_eq!(st.format_after(2).over, expected, "untracked typing must not inherit the LF's change annotation");
    }
}

//! Bounded review22 editing corrections, exercised only through public commands.

use designcraft_doc::{ANCHOR_MARK, CharFormat, Leading, NOTE_MARK, ParaFormat, StoryId, TextSel, XREF_MARK};
use designcraft_engine::Session;
use serde_json::{Value, json};

fn document(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": anchor, "focus": focus})).unwrap();
}

fn frame(s: &mut Session, text: &str, y: f64) -> StoryId {
    let result = s.execute("frame.create", &json!({"rect": [40.0, y, 440.0, y + 180.0], "content": "text", "text": text})).unwrap();
    StoryId(result["story"].as_u64().unwrap())
}

fn edit(s: &mut Session, command: &str, params: Value) -> Value {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let result = s.execute(command, &params).unwrap();
    let after = document(s);
    let selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "one public edit owns one history entry");
    assert_eq!(&s.journal[journal..], &[(command.to_string(), params)], "retain command identity without nested dispatch");
    s.doc().unwrap().doc.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(document(s), before, "Undo restores all text, formats and marker metadata");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(document(s), after, "Redo restores the complete document");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn native_copies(s: &mut Session) -> [Session; 2] {
    let expected = document(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let zip = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"));
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    [zip, legacy].map(|bytes| {
        let mut back = Session::new();
        back.execute("file.openBytes", &json!({"base64": designcraft_engine::cmd::base64_encode(&bytes)})).unwrap();
        assert_eq!(document(&back), expected, "native ZIP and legacy JSON preserve the full document");
        back.doc().unwrap().doc.check().unwrap();
        back
    })
}

fn metrics(s: &mut Session, sid: StoryId, start: usize, end: usize, size: f64, leading: Leading) -> Value {
    select(s, sid, start, end);
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["size"], json!(size));
    assert_eq!(attrs["chars"]["leading"], serde_json::to_value(leading).unwrap());
    attrs
}

fn staged(text: &str, inherited: bool) -> (Session, StoryId, ParaFormat) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let sid = frame(&mut s, text, 40.0);
    select(&mut s, sid, 0, text.len());
    if inherited {
        s.execute("style.paragraph.create", &json!({"name": "Review22 baseline", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
            .unwrap();
        s.execute("style.paragraph.apply", &json!({"name": "Review22 baseline"})).unwrap();
        s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    }
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
    assert!(baseline.eof_inherited_chars.is_none());
    if inherited {
        assert_eq!(baseline.chars.size, Some(24.0));
        assert_eq!(baseline.chars.leading, Some(Leading::Points(26.0)));
    } else {
        assert!(baseline.chars.is_empty());
    }
    s.execute("style.character.create", &json!({"name": "Review22 live", "chars": {"underline": true}})).unwrap();
    s.execute("style.character.apply", &json!({"name": "Review22 live"})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}})).unwrap();
    select(&mut s, sid, text.len(), text.len());
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 30}}})).unwrap();
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, format!("{text}\n"));
    assert_eq!(story.format_after(text.len()).over.size, Some(15.0));
    assert_eq!(story.format_after(text.len()).over.leading, Some(Leading::Points(18.0)));
    assert_eq!(story.paras[0], baseline);
    assert_eq!(story.paras[1].chars.size, Some(20.0));
    assert_eq!(story.paras[1].chars.leading, Some(Leading::Points(30.0)));
    assert_eq!(story.paras[1].eof_inherited_chars.as_ref(), inherited.then_some(&baseline.chars));
    (s, sid, baseline)
}

fn type_and_clear(s: &mut Session, sid: StoryId, baseline: &ParaFormat, size: f64, leading: Leading) {
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let at = before.len();
    metrics(s, sid, at, at, size, leading);
    edit(s, "text.insert", json!({"text": "é", "raw": true}));
    let typed = metrics(s, sid, at, at + "é".len(), size, leading);
    assert_eq!(typed["characterStyle"], json!(before.char_format_at(at).style));
    edit(s, "type.char", json!({"attrs": {"size": null, "leading": null}}));
    let expected = s.doc().unwrap().doc.styles.resolve_para(baseline).1;
    let cleared = metrics(s, sid, at, at + "é".len(), expected.size, expected.leading);
    assert_eq!(cleared["charOverrides"], 0);
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, format!("{}é", before.text));
    assert_eq!(story.paras.last(), Some(baseline), "the populated paragraph retains its genuine defaults");
    for (offset, _) in before.text.char_indices() {
        assert_eq!(story.format_after(offset), before.format_after(offset), "typing/clearing preserves every prior byte format");
    }
    assert_eq!(story.anchors, before.anchors);
    assert_eq!(story.xrefs, before.xrefs);
    assert_eq!(story.editorial, before.editorial);
    if before.char_format_at(at).style == "Review22 live" {
        assert_eq!(cleared["chars"]["underline"], true, "the independent named character style stays live");
    }
    native_copies(s);
}

#[test]
fn r22_editing_same_story_earlier_source_consumes_eof_target() {
    for inherited in [false, true] {
        for selected in [false, true] {
            let (mut s, sid, baseline) = staged("éA", inherited);
            let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
            let [zip, legacy] = native_copies(&mut s);
            for mut back in [s, zip, legacy] {
                // Also exercise a reversed UTF-8 selection before the target.
                let removed = if selected { "éA".len() } else { 0 };
                select(&mut back, sid, removed, 0);
                let result = edit(&mut back, "xref.insert", json!({"story": sid.0, "para": 1, "format": "Page Number"}));
                let anchor = result["target"].as_u64().unwrap();
                let at = before.len() - removed + XREF_MARK.len_utf8();
                assert_eq!(result["pos"], 0, "a later target does not shift the source caret");
                let d = &back.doc().unwrap().doc;
                let story = d.story(sid).unwrap();
                let prefix = if selected { "" } else { "éA" };
                assert_eq!(story.text, format!("{XREF_MARK}{prefix}\n{ANCHOR_MARK}"));
                assert_eq!(d.find_anchor(anchor), Some((sid, at)));
                assert_eq!(story.anchors.len(), 1);
                assert_eq!(story.anchors[0].name, "");
                assert_eq!(story.xrefs.len(), 1);
                assert_eq!(story.xrefs[0].target, anchor);
                assert_eq!(story.xrefs[0].format, "Page Number");
                assert_eq!(story.paras.last(), Some(&baseline));
                let expected =
                    CharFormat { style: before.char_format_at(before.len()).style.clone(), over: before.paras.last().unwrap().chars.clone() };
                assert_eq!(story.format_after(at), &expected, "the target marker consumes its own stage");
                for (offset, _) in before.text.char_indices().filter(|(offset, _)| *offset >= removed) {
                    assert_eq!(story.format_after(offset - removed + XREF_MARK.len_utf8()), before.format_after(offset));
                }
                let caret = back.doc().unwrap().selection.text.unwrap();
                assert_eq!((caret.story, caret.anchor, caret.focus, caret.cell), (sid, XREF_MARK.len_utf8(), XREF_MARK.len_utf8(), None));
                native_copies(&mut back);
                type_and_clear(&mut back, sid, &baseline, 20.0, Leading::Points(30.0));
            }
        }
    }
}

#[test]
fn r22_editing_same_story_source_eof_keeps_restored_baseline() {
    let (mut s, sid, baseline) = staged("A", true);
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        select(&mut back, sid, 2, 2);
        let result = edit(&mut back, "xref.insert", json!({"story": sid.0, "para": 1, "format": "Page Number"}));
        let xref_at = 2 + ANCHOR_MARK.len_utf8();
        let story = back.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, format!("A\n{ANCHOR_MARK}{XREF_MARK}"));
        assert_eq!(story.paras.last(), Some(&baseline), "do not consume the restored 24/26 baseline twice");
        assert_eq!(story.format_after(2), before.format_after(1), "retain the same-EOF target anchor's existing LF policy");
        assert_eq!(story.format_after(xref_at).over, before.paras[1].chars);
        assert_eq!(result["pos"], xref_at);
        assert_eq!(back.doc().unwrap().doc.find_anchor(result["target"].as_u64().unwrap()), Some((sid, 2)));
        let caret = back.doc().unwrap().selection.text.unwrap();
        assert_eq!((caret.anchor, caret.focus), (xref_at + XREF_MARK.len_utf8(), xref_at + XREF_MARK.len_utf8()));
        type_and_clear(&mut back, sid, &baseline, 20.0, Leading::Points(30.0));
    }
}

#[test]
fn r22_editing_reused_anchor_preserves_unrelated_eof_stage() {
    for same_story in [false, true] {
        let (mut s, target, baseline) = staged("Target", true);
        select(&mut s, target, 0, 0);
        let anchor = edit(&mut s, "anchor.create", json!({"name": "Keep destination"}))["id"].as_u64().unwrap();
        let source = if same_story { target } else { frame(&mut s, "Source", 250.0) };
        let other = frame(&mut s, "Unrelated", 460.0);
        let other_before = s.doc().unwrap().doc.story(other).unwrap().clone();
        for params in [json!({"anchor": anchor}), json!({"story": target.0, "para": 0}), json!({"paragraph": "Target"})] {
            let target_before = s.doc().unwrap().doc.story(target).unwrap().clone();
            let at = if same_story { ANCHOR_MARK.len_utf8() } else { 0 };
            select(&mut s, source, at, at);
            let result = edit(&mut s, "xref.insert", params);
            assert_eq!(result["target"], anchor);
            assert_eq!(result["pos"], at, "anchor reuse adds no destination bytes or source offset shift");
            let d = &s.doc().unwrap().doc;
            let story = d.story(target).unwrap();
            assert_eq!(d.find_anchor(anchor), Some((target, 0)));
            assert_eq!(story.anchors, target_before.anchors);
            assert_eq!(story.paras, target_before.paras, "a separate EOF stage is untouched by reuse");
            if !same_story {
                assert_eq!(story, &target_before, "the other target story is entirely unchanged");
            }
            assert_eq!(d.story(other).unwrap(), &other_before);
            let end = story.len();
            metrics(&mut s, target, end, end, 20.0, Leading::Points(30.0));
        }
        native_copies(&mut s);
        type_and_clear(&mut s, target, &baseline, 20.0, Leading::Points(30.0));
    }
}

#[test]
fn r22_editing_plain_same_story_target_keeps_lf_format_and_offsets() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let sid = frame(&mut s, "A\n", 40.0);
    select(&mut s, sid, 0, 2);
    s.execute("type.char", &json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}})).unwrap();
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    assert!(before.paras[1].chars.is_empty());
    assert!(before.paras[1].eof_inherited_chars.is_none());
    select(&mut s, sid, 0, 0);
    let result = edit(&mut s, "xref.insert", json!({"story": sid.0, "para": 1, "format": "Page Number"}));
    let at = 2 + XREF_MARK.len_utf8();
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, format!("{XREF_MARK}A\n{ANCHOR_MARK}"));
    assert_eq!(story.paras, before.paras);
    assert_eq!(story.format_after(at), before.format_after(1));
    assert_eq!(s.doc().unwrap().doc.find_anchor(result["target"].as_u64().unwrap()), Some((sid, at)));
    assert_eq!(result["pos"], 0);
    type_and_clear(&mut s, sid, &before.paras[1], 15.0, Leading::Points(18.0));
}

fn note_fixture() -> (Session, StoryId, ParaFormat, u64) {
    let (mut s, sid, baseline) = staged("A", true);
    select(&mut s, sid, 0, 0);
    s.execute("note.new", &json!({"text": "Keep é", "author": "Reviewer 李"})).unwrap();
    let at = s.doc().unwrap().doc.story(sid).unwrap().len();
    select(&mut s, sid, at, at);
    let note = s.execute("note.new", &json!({"text": "Remove only this", "author": "Other"})).unwrap()["id"].as_u64().unwrap();
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.paras.last(), Some(&baseline));
    assert_eq!(story.format_after(at).over.size, Some(20.0));
    assert_eq!(story.format_after(at).over.leading, Some(Leading::Points(30.0)));
    assert_eq!(story.editorial.len(), 2);
    (s, sid, baseline, note)
}

#[test]
fn r22_editing_note_delete_restages_final_eof_with_metadata_and_native_history() {
    let (mut s, sid, baseline, note) = note_fixture();
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        let selection = back.doc().unwrap().selection.clone();
        edit(&mut back, "note.delete", json!({"story": sid.0, "id": note}));
        assert_eq!(back.doc().unwrap().selection, selection, "keep the explicit command's existing selection policy");
        let story = back.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, format!("{NOTE_MARK}A\n"));
        assert_eq!(story.editorial, before.editorial[..1]);
        assert_eq!(story.editorial[0].text, "Keep é");
        assert_eq!(story.editorial[0].author, "Reviewer 李");
        for (offset, _) in story.text.char_indices() {
            assert_eq!(story.format_after(offset), before.format_after(offset));
        }
        let end = story.len();
        metrics(&mut back, sid, end, end, 15.0, Leading::Points(18.0));
        let story = back.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.paras.last().unwrap().eof_inherited_chars.as_ref(), Some(&baseline.chars));
        native_copies(&mut back);
        type_and_clear(&mut back, sid, &baseline, 15.0, Leading::Points(18.0));
    }
}

#[test]
fn r22_editing_note_delete_prefix_preserves_existing_stage() {
    let (mut s, sid, baseline) = staged("A", true);
    select(&mut s, sid, 0, 0);
    let note = s.execute("note.new", &json!({"text": "Prefix"})).unwrap()["id"].as_u64().unwrap();
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    edit(&mut s, "note.delete", json!({"story": sid.0, "id": note}));
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, "A\n");
    assert_eq!(story.paras, before.paras, "the surviving final LF retains its existing stage");
    assert_eq!(story.format_after(1), before.format_after(1 + NOTE_MARK.len_utf8()));
    type_and_clear(&mut s, sid, &baseline, 20.0, Leading::Points(30.0));
}

#[test]
fn r22_editing_note_delete_plain_and_non_eof_controls() {
    for text in ["A", "A\n"] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let sid = frame(&mut s, text, 40.0);
        select(&mut s, sid, 0, text.len());
        s.execute("type.char", &json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}})).unwrap();
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        select(&mut s, sid, text.len(), text.len());
        let note = s.execute("note.new", &json!({"text": "Plain"})).unwrap()["id"].as_u64().unwrap();
        edit(&mut s, "note.delete", json!({"story": sid.0, "id": note}));
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, text);
        assert_eq!(story.chars, before.chars);
        assert!(story.editorial.is_empty());
        if !text.ends_with('\n') {
            assert_eq!(story.paras, before.paras, "ordinary paragraphs do not acquire an EOF stage");
        }
        type_and_clear(&mut s, sid, before.paras.last().unwrap(), 15.0, Leading::Points(18.0));
    }
}

fn selected_child(s: &mut Session, note: bool) -> TextSel {
    let sid = frame(s, "", 300.0);
    select(s, sid, 0, 0);
    if note {
        s.execute("footnote.insert", &json!({"text": "Child\n"})).unwrap();
    } else {
        s.execute("table.insert", &json!({"rows": 1, "cols": 1, "width": 300})).unwrap();
        s.execute("text.insert", &json!({"text": "Child\n", "raw": true})).unwrap();
    }
    s.execute("text.move", &json!({"dir": "storyStart"})).unwrap();
    s.execute("text.move", &json!({"dir": "storyEnd", "extend": true})).unwrap();
    let t = s.doc().unwrap().selection.text.unwrap();
    assert_eq!((t.story, t.range()), (sid, 0..6));
    assert!(t.cell.is_some());
    t
}

#[test]
fn r22_editing_explicit_note_delete_preserves_selected_children_and_unrelated_story() {
    for note_child in [false, true] {
        let (mut s, sid, baseline, note) = note_fixture();
        let other = frame(&mut s, "Unrelated", 510.0);
        let other_before = s.doc().unwrap().doc.story(other).unwrap().clone();
        let child = selected_child(&mut s, note_child);
        let child_before = s.doc().unwrap().doc.text_story(child.story, child.cell).unwrap().clone();
        let parent_before = s.doc().unwrap().doc.story(child.story).unwrap().clone();
        let selection = s.doc().unwrap().selection.clone();
        edit(&mut s, "note.delete", json!({"story": sid.0, "id": note}));
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.story(other).unwrap(), &other_before);
        assert_eq!(d.story(child.story).unwrap(), &parent_before, "an explicit outer target never mutates the selected child's parent");
        assert_eq!(d.text_story(child.story, child.cell).unwrap(), &child_before);
        assert_eq!(s.doc().unwrap().selection, selection);
        native_copies(&mut s);
        type_and_clear(&mut s, sid, &baseline, 15.0, Leading::Points(18.0));
    }
}

fn rejected(s: &mut Session, command: &str, params: Value) {
    let before = document(s);
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    assert!(s.execute(command, &params).is_err());
    assert_eq!(document(s), before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert_eq!(s.journal.len(), journal);
}

#[test]
fn r22_editing_xref_rejects_children_without_consuming_target() {
    for note in [false, true] {
        let (mut s, sid, _) = staged("Target", true);
        selected_child(&mut s, note);
        rejected(&mut s, "xref.insert", json!({"story": sid.0, "para": 1, "format": "Page Number"}));
    }
}

#[test]
fn r22_editing_note_delete_missing_id_is_transactional() {
    let (mut s, sid, _, _) = note_fixture();
    rejected(&mut s, "note.delete", json!({"story": sid.0, "id": 9999}));
}

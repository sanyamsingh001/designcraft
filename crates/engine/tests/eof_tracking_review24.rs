//! D05/D06 regressions: public tracked edits must preserve genuine paragraph defaults.

use designcraft_doc::{ChangeMark, Leading, ParaFormat, StoryId};
use designcraft_engine::Session;
use serde_json::{Value, json};

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": anchor, "focus": focus})).unwrap();
}

fn document(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn edit(s: &mut Session, command: &str, params: Value) -> Value {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let result = s.execute(command, &params).unwrap();
    let after = document(s);
    let selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "{command}: one history entry");
    assert_eq!(&s.journal[journal..], &[(command.to_string(), params)], "retain the owning command identity");
    s.doc().unwrap().doc.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(document(s), before, "{command}: Undo restores text, formats and change marks");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(document(s), after, "{command}: Redo restores the entire document");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn baseline(s: &mut Session) {
    s.execute("style.paragraph.create", &json!({"name": "Tracking baseline", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
        .unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "Tracking baseline"})).unwrap();
    s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
}

fn source(text: &str, inherited: bool) -> (Session, StoryId, ParaFormat) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let frame = s.execute("frame.create", &json!({"rect": [40, 40, 440, 440], "content": "text", "text": text})).unwrap();
    let sid = StoryId(frame["story"].as_u64().unwrap());
    select(&mut s, sid, 0, text.len());
    if inherited {
        baseline(&mut s);
    }
    let para = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
    assert!(para.eof_inherited_chars.is_none());
    assert_eq!(para.chars.size, inherited.then_some(24.0));
    assert_eq!(para.chars.leading, inherited.then_some(Leading::Points(26.0)));
    s.execute("style.character.create", &json!({"name": "Tracking live", "chars": {"underline": true}})).unwrap();
    s.execute("style.character.apply", &json!({"name": "Tracking live"})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}})).unwrap();
    (s, sid, para)
}

fn tracked_tail(inserted: bool, inherited: bool) -> (Session, StoryId, ParaFormat) {
    let (mut s, sid, para) = source(if inserted { "A" } else { "A\nX" }, inherited);
    if inserted {
        select(&mut s, sid, 1, 1);
        s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 30}}})).unwrap();
    }
    s.execute("changes.track", &json!({"on": true})).unwrap();
    if inserted {
        s.execute("text.insert", &json!({"text": "X", "raw": true})).unwrap();
    } else {
        select(&mut s, sid, 2, 3);
        s.execute("text.delete", &json!({})).unwrap();
    }
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\nX");
    assert_eq!(st.paras.last(), Some(&para), "fixture contains genuine paragraph defaults, not an EOF stage");
    assert_eq!(st.format_after(2).over.change, Some(if inserted { ChangeMark::Inserted } else { ChangeMark::Deleted }));
    (s, sid, para)
}

fn type_and_clear(s: &mut Session, sid: StoryId, para: &ParaFormat) {
    s.execute("changes.track", &json!({"on": false})).unwrap();
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let at = before.len();
    select(s, sid, at, at);
    s.execute("text.insert", &json!({"text": "z", "raw": true})).unwrap();
    select(s, sid, at, at + 1);
    let typed = s.execute("type.selectionAttrs", &json!({})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    let expected = s.doc().unwrap().doc.styles.resolve_para(para).1;
    assert_eq!(attrs["chars"]["size"], json!(expected.size), "clearing direct attrs must reveal the original paragraph baseline");
    assert_eq!(attrs["chars"]["leading"], serde_json::to_value(expected.leading).unwrap());
    assert_eq!(attrs["chars"]["underline"], true, "keep the independent named character style");
    assert_eq!(typed["chars"]["size"], 15.0);
    assert_eq!(typed["chars"]["leading"], json!({"kind": "points", "value": 18.0}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.paras.last(), Some(para));
    assert_eq!(st.text, format!("{}z", before.text));
    for (at, _) in before.text.char_indices() {
        assert_eq!(st.format_after(at), before.format_after(at), "existing bytes retain their formats and change marks");
    }
}

fn exposed_eof(s: &mut Session, sid: StoryId, para: &ParaFormat) {
    let st = s.doc().unwrap().doc.story(sid).unwrap().clone();
    assert_eq!(st.text, "A\n");
    assert!(s.execute("changes.list", &json!({})).unwrap().as_array().unwrap().is_empty());
    let t = s.doc().unwrap().selection.text.unwrap();
    assert_eq!((t.anchor, t.focus), (2, 2));
    type_and_clear(s, sid, para);
    assert_eq!(&st.paras[0], para);
    let eof = st.paras.last().unwrap();
    assert_eq!(eof.eof_inherited_chars.as_ref(), (!para.chars.is_empty()).then_some(&para.chars));
    assert_eq!(eof.chars.size, Some(15.0));
    assert_eq!(eof.chars.leading, Some(Leading::Points(18.0)));
    assert_eq!(eof.chars.change, None, "an empty paragraph must not carry a change mark");
    assert_eq!(st.format_after(1).over.change, None);
}

fn resolve_tail(command: &str, accept: bool) {
    for inherited in [true, false] {
        let (mut s, sid, para) = tracked_tail(!accept, inherited);
        let params = if command.ends_with("All") { json!({"story": sid.0}) } else { json!({"story": sid.0, "start": 2}) };
        let result = edit(&mut s, command, params);
        if command.ends_with("All") {
            assert_eq!(result["changes"], 1);
        } else {
            assert_eq!(result, json!({"start": 2, "end": 3}));
        }
        exposed_eof(&mut s, sid, &para);
    }
}

#[test]
fn d05_accept_tail_preserves_defaults() {
    resolve_tail("changes.accept", true);
}

#[test]
fn d05_accept_all_tail_preserves_defaults() {
    resolve_tail("changes.acceptAll", true);
}

#[test]
fn d05_reject_tail_preserves_defaults() {
    resolve_tail("changes.reject", false);
}

#[test]
fn d05_reject_all_tail_preserves_defaults() {
    resolve_tail("changes.rejectAll", false);
}

fn erase_tail(command: &str) {
    for inherited in [true, false] {
        let (mut s, sid, para) = tracked_tail(true, inherited);
        select(&mut s, sid, 3, 2);
        let params = if command == "text.insert" { json!({"text": "", "raw": true}) } else { json!({}) };
        let result = edit(&mut s, command, params);
        if command == "edit.cut" {
            assert_eq!(result["text"], "X");
        }
        exposed_eof(&mut s, sid, &para);
    }
}

#[test]
fn d06_selected_delete_preserves_defaults() {
    erase_tail("text.delete");
}

#[test]
fn d06_clear_preserves_defaults() {
    erase_tail("edit.clear");
}

#[test]
fn d06_cut_preserves_defaults() {
    erase_tail("edit.cut");
}

#[test]
fn d06_empty_replacement_preserves_defaults() {
    erase_tail("text.insert");
}

#[test]
fn d06_forward_and_backward_delete_preserve_defaults() {
    for forward in [false, true] {
        let (mut s, sid, para) = tracked_tail(true, true);
        let at = if forward { 2 } else { 3 };
        select(&mut s, sid, at, at);
        edit(&mut s, "text.delete", json!({"forward": forward}));
        exposed_eof(&mut s, sid, &para);
    }
}

#[test]
fn tracking_nonempty_replacement_consumes_intermediate_stage_and_keeps_inserted_marks() {
    for inherited in [true, false] {
        for text in ["Y", "Y\n", "Y\nZ"] {
            let (mut s, sid, para) = tracked_tail(true, inherited);
            select(&mut s, sid, 2, 3);
            let result = edit(&mut s, "text.insert", json!({"text": text, "raw": true}));
            assert_eq!(result["pos"], 2 + text.len());
            let st = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(st.text, format!("A\n{text}"));
            for (at, _) in text.char_indices() {
                let fmt = st.format_after(2 + at);
                assert_eq!(fmt.over.change, Some(ChangeMark::Inserted));
                assert_eq!(fmt.over.size, Some(15.0), "replacement keeps the surviving LF's insertion format");
            }
            for (pi, range) in st.para_ranges().iter().enumerate() {
                if !range.is_empty() {
                    assert_eq!(&st.paras[pi], &para, "populated replacement paragraphs must not retain an EOF stage");
                }
            }
            if text.ends_with('\n') {
                let eof = st.paras.last().unwrap();
                assert_eq!(eof.eof_inherited_chars.as_ref(), inherited.then_some(&para.chars));
                assert_eq!(eof.chars.change, None);
            }
            type_and_clear(&mut s, sid, &para);
        }
    }
}

#[test]
fn tracking_mixed_original_and_inserted_text_retains_deleted_originals() {
    let (mut s, sid, para) = source("A\nQ", true);
    s.execute("changes.track", &json!({"on": true})).unwrap();
    select(&mut s, sid, 3, 3);
    s.execute("text.insert", &json!({"text": "X", "raw": true})).unwrap();
    select(&mut s, sid, 2, 4);
    edit(&mut s, "text.insert", json!({"text": "Y\n", "raw": true}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\nQY\n");
    assert_eq!(st.format_after(2).over.change, Some(ChangeMark::Deleted));
    assert_eq!(st.format_after(3).over.change, Some(ChangeMark::Inserted));
    assert_eq!(st.format_after(4).over.change, Some(ChangeMark::Inserted));
    assert_eq!(st.paras[1], para);
    assert_eq!(st.paras.last().unwrap().eof_inherited_chars.as_ref(), Some(&para.chars));
    assert_eq!(st.paras.last().unwrap().chars.change, None);
    edit(&mut s, "changes.rejectAll", json!({}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\nQ");
    assert_eq!(st.paras.last(), Some(&para));
    assert!(s.execute("changes.list", &json!({})).unwrap().as_array().unwrap().is_empty());
}

#[test]
fn tracking_prefix_deletion_preserves_existing_eof_typing_choice() {
    let (mut s, sid, _) = source("A", true);
    select(&mut s, sid, 1, 1);
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 30}}})).unwrap();
    let eof = s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().clone();
    s.execute("changes.track", &json!({"on": true})).unwrap();
    select(&mut s, sid, 1, 1);
    s.execute("text.insert", &json!({"text": "X", "raw": true})).unwrap();
    select(&mut s, sid, 1, 2);
    edit(&mut s, "text.delete", json!({}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\n");
    assert_eq!(st.paras.last(), Some(&eof), "surviving final LF keeps its explicit EOF typing choice");
}

#[test]
fn tracking_child_deletions_keep_their_existing_untracked_policy() {
    for note in [false, true] {
        for command in ["text.delete", "edit.clear", "edit.cut", "text.insert"] {
            let (mut s, sid, _) = source("A", false);
            select(&mut s, sid, 1, 1);
            if note {
                s.execute("footnote.insert", &json!({"text": "A\nX"})).unwrap();
            } else {
                s.execute("table.insert", &json!({"rows": 1, "cols": 1})).unwrap();
                s.execute("text.insert", &json!({"text": "A\nX", "raw": true})).unwrap();
            }
            s.execute("text.move", &json!({"dir": "storyStart"})).unwrap();
            s.execute("text.move", &json!({"dir": "storyEnd", "extend": true})).unwrap();
            baseline(&mut s);
            let child = s.doc().unwrap().selection.text.unwrap();
            assert!(child.cell.is_some());
            let para = s.doc().unwrap().doc.text_story(sid, child.cell).unwrap().paras[1].clone();
            assert_eq!(para.chars.size, Some(24.0));
            assert!(para.eof_inherited_chars.is_none());
            s.execute("changes.track", &json!({"on": true})).unwrap();
            s.execute("text.move", &json!({"dir": "storyEnd"})).unwrap();
            s.execute("text.move", &json!({"dir": "left", "extend": true})).unwrap();
            let params = if command == "text.insert" { json!({"text": "", "raw": true}) } else { json!({}) };
            edit(&mut s, command, params);
            let st = s.doc().unwrap().doc.text_story(sid, child.cell).unwrap();
            assert_eq!(st.text, "A\n");
            assert_eq!(st.paras.last(), Some(&para), "child paragraph defaults are never outer EOF staging");
            assert!(st.runs().all(|(_, f)| f.over.change.is_none()));
            s.execute("text.insert", &json!({"text": "z", "raw": true})).unwrap();
            s.execute("text.move", &json!({"dir": "left", "extend": true})).unwrap();
            s.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
            let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
            assert_eq!(attrs["chars"]["size"], 24.0);
            assert_eq!(attrs["chars"]["leading"], json!({"kind": "points", "value": 26.0}));
            assert_eq!(s.doc().unwrap().doc.text_story(sid, child.cell).unwrap().paras.last(), Some(&para));
        }
    }
}

#[test]
fn resolving_kept_changes_only_removes_the_marks() {
    for accept in [false, true] {
        for all in [false, true] {
            let (mut s, sid, _) = tracked_tail(accept, true);
            let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
            let command = match (accept, all) {
                (true, true) => "changes.acceptAll",
                (true, false) => "changes.accept",
                (false, true) => "changes.rejectAll",
                (false, false) => "changes.reject",
            };
            let params = if all { json!({"story": sid.0}) } else { json!({"story": sid.0, "start": 2}) };
            edit(&mut s, command, params);
            let st = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(st.text, before.text);
            assert_eq!(st.paras, before.paras);
            assert!(st.runs().all(|(_, f)| f.over.change.is_none()));
            let mut expected = before.format_after(2).clone();
            expected.over.change = None;
            assert_eq!(st.format_after(2), &expected);
        }
    }
}

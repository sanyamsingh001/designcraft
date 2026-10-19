//! D09-D12: replacements exposing outer EOF retain genuine paragraph defaults.

use designcraft_doc::{Leading, Story, StoryId};
use designcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn document(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    run(s, "text.select", json!({"story": sid.0, "anchor": anchor, "focus": focus}));
}

fn story(s: &Session, sid: StoryId) -> &Story {
    s.doc().unwrap().doc.story(sid).unwrap()
}

fn fixture(text: &str, overrides: bool) -> (Session, StoryId, Story) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"pages": 1}));
    let sid = StoryId(run(&mut s, "frame.create", json!({"rect": [40, 40, 440, 440], "content": "text", "text": text}))["story"].as_u64().unwrap());
    select(&mut s, sid, 0, text.len());
    run(&mut s, "style.paragraph.create", json!({"name": "Review24 baseline", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}));
    run(&mut s, "style.paragraph.apply", json!({"name": "Review24 baseline"}));
    run(&mut s, "style.breakLink", json!({"kind": "paragraph"}));
    if overrides {
        select(&mut s, sid, 0, 2);
        run(&mut s, "type.char", json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}}));
    }
    let original = story(&s, sid).clone();
    assert!(original.paras.iter().all(|p| p.chars.size == Some(24.0) && p.eof_inherited_chars.is_none()));
    (s, sid, original)
}

fn edit(s: &mut Session, command: &str, params: Value) {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    run(s, command, params);
    let after = document(s);
    let selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    run(s, "edit.undo", json!({}));
    assert_eq!(document(s), before);
    run(s, "edit.redo", json!({}));
    assert_eq!(document(s), after);
    assert_eq!(s.doc().unwrap().selection, selection);
    s.doc().unwrap().doc.check().unwrap();
}

fn observe(s: &mut Session, sid: StoryId, original: &Story, already_typed: bool) {
    let saved = run(s, "file.serialize", json!({}));
    let zip = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"));
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    let mut copies = [zip, legacy].map(|bytes| {
        let mut copy = Session::new();
        run(&mut copy, "file.openBytes", json!({"base64": designcraft_engine::cmd::base64_encode(&bytes)}));
        assert_eq!(document(&copy), document(s));
        copy
    });
    let baseline = original.paras.last().unwrap();
    for state in std::iter::once(s).chain(copies.iter_mut()) {
        if already_typed {
            assert_eq!(story(state, sid).text, "A\n ");
        } else {
            assert_eq!(story(state, sid).text, "A\n");
            let eof = story(state, sid).paras.last().unwrap();
            assert_eq!(eof.eof_inherited_chars.as_ref(), Some(&baseline.chars));
            assert_eq!(eof.chars, original.format_after(1).over);
            select(state, sid, 2, 2);
            edit(state, "text.insert", json!({"text": "é", "raw": true}));
        }
        let end = story(state, sid).len();
        select(state, sid, 2, end);
        edit(state, "type.char", json!({"attrs": {"size": null, "leading": null}}));
        let attrs = run(state, "type.selectionAttrs", json!({}));
        assert_eq!(attrs["chars"]["size"], 24.0);
        assert_eq!(attrs["chars"]["leading"], json!({"kind": "points", "value": 26.0}));
        assert_eq!(attrs["charOverrides"], 0);
        assert_eq!(story(state, sid).paras.last(), Some(baseline));
        assert_eq!(story(state, sid).paras[0], original.paras[0]);
        for at in 0..2 {
            assert_eq!(story(state, sid).format_after(at), original.format_after(at), "prefix/LF unchanged");
        }
    }
}

#[test]
fn r24_d09_table_delete_preserves_eof_defaults() {
    for overrides in [false, true] {
        let (mut s, sid, original) = fixture("A\n\nX", overrides);
        select(&mut s, sid, 3, 4);
        let table = run(&mut s, "table.insert", json!({"rows": 1, "cols": 1}));
        edit(&mut s, "table.delete", json!({"story": sid.0, "table": table["table"]}));
        assert!(story(&s, sid).tables.is_empty());
        observe(&mut s, sid, &original, false);
    }
}

#[test]
fn r24_d10_empty_table_conversion_preserves_eof_defaults() {
    for overrides in [false, true] {
        let (mut s, sid, original) = fixture("A\nX", overrides);
        select(&mut s, sid, 2, 3);
        let table = run(&mut s, "table.insert", json!({"rows": 1, "cols": 1}));
        select(&mut s, sid, 5, 6);
        run(&mut s, "text.delete", json!({}));
        edit(&mut s, "table.convertToText", json!({"story": sid.0, "table": table["table"]}));
        assert!(story(&s, sid).tables.is_empty());
        observe(&mut s, sid, &original, false);
    }
}

#[test]
fn r24_d11_empty_selected_placement_preserves_eof_defaults() {
    for overrides in [false, true] {
        let (mut s, sid, original) = fixture("A\nX", overrides);
        select(&mut s, sid, 2, 3);
        edit(&mut s, "file.place", json!({"name": "empty.txt", "base64": ""}));
        observe(&mut s, sid, &original, false);
    }
}

#[test]
fn r24_d12_empty_autocorrection_preserves_eof_defaults() {
    for overrides in [false, true] {
        let (mut s, sid, original) = fixture("A\nx", overrides);
        run(&mut s, "prefs.set", json!({"autocorrect": true, "autocorrectList": [["x", ""]]}));
        select(&mut s, sid, 3, 3);
        edit(&mut s, "text.insert", json!({"text": " ", "raw": false}));
        observe(&mut s, sid, &original, true);
    }
}

#[test]
fn r24_nonempty_formatted_placement_matches_interior_replacement() {
    let mut results = Vec::new();
    for text in ["A\nX", "A\nXY"] {
        let (mut s, sid, original) = fixture(text, true);
        select(&mut s, sid, 2, 3);
        let bytes = designcraft_engine::cmd::base64_encode(br"{\rtf1\ansi {\i em} {\b strong}}");
        edit(&mut s, "file.place", json!({"name": "formatted.rtf", "base64": bytes}));
        assert_eq!(story(&s, sid).text, if text.ends_with('Y') { "A\nem strongY" } else { "A\nem strong" });
        assert_eq!(story(&s, sid).paras, original.paras, "formatted insertion keeps the full destination paragraph");
        if text.ends_with('Y') {
            let end = story(&s, sid).len();
            select(&mut s, sid, end - 1, end);
            run(&mut s, "text.delete", json!({}));
        }
        let st = story(&s, sid);
        for at in 0..2 {
            assert_eq!(st.format_after(at), original.format_after(at));
        }
        assert_ne!(st.format_after(2), st.format_after(5), "emphasis remains formatted");
        results.push((st.text.clone(), st.chars.clone(), st.paras.clone()));
    }
    assert_eq!(results[0], results[1], "tail and interior placement preserve identical complete formats");
}

#[test]
fn r24_autocorrection_in_children_preserves_ordinary_paragraphs() {
    for note in [false, true] {
        for correction in ["", "word"] {
            let (mut s, sid, _) = fixture("A\nX", true);
            select(&mut s, sid, 2, 3);
            if note {
                run(&mut s, "footnote.insert", json!({"text": "A\nx"}));
            } else {
                run(&mut s, "table.insert", json!({"rows": 1, "cols": 1}));
                run(&mut s, "text.insert", json!({"text": "A\nx", "raw": true}));
            }
            run(&mut s, "text.move", json!({"dir": "storyEnd"}));
            run(&mut s, "style.paragraph.apply", json!({"name": "Review24 baseline"}));
            run(&mut s, "style.breakLink", json!({"kind": "paragraph"}));
            let t = s.doc().unwrap().selection.text.unwrap();
            assert!(t.cell.is_some());
            let before = s.doc().unwrap().doc.text_story(sid, t.cell).unwrap().clone();
            let outer = story(&s, sid).clone();
            run(&mut s, "prefs.set", json!({"autocorrect": true, "autocorrectList": [["x", correction]]}));
            edit(&mut s, "text.insert", json!({"text": " ", "raw": false}));
            let child = s.doc().unwrap().doc.text_story(sid, t.cell).unwrap();
            assert_eq!(child.text, format!("A\n{correction} "));
            assert_eq!(child.paras, before.paras);
            assert_eq!(child.paras[1].chars.leading, Some(Leading::Points(26.0)));
            assert!(child.paras.iter().all(|p| p.eof_inherited_chars.is_none()));
            assert_eq!(story(&s, sid).text, outer.text);
            assert_eq!(story(&s, sid).paras, outer.paras);
            assert_eq!(story(&s, sid).chars, outer.chars);
        }
    }
}

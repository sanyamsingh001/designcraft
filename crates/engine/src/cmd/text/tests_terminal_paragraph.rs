//! Focused terminal-paragraph regressions using only current APIs.
use std::sync::Arc;
use designcraft_doc::{Leading, StoryId};
use serde_json::{Value, json};
use crate::Session;

fn fixture(text: &str) -> (Session, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("frame.create", &json!({"rect": [40, 40, 440, 500], "content": "text", "text": text})).unwrap();
    let sid = StoryId(r["story"].as_u64().unwrap());
    select(&mut s, sid, text.len(), text.len());
    (s, sid)
}
fn select(s: &mut Session, sid: StoryId, a: usize, b: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": a, "focus": b})).unwrap();
}
fn format(s: &mut Session, attrs: Value) {
    s.execute("type.char", &json!({"attrs": attrs})).unwrap();
}
fn attrs(s: &mut Session) -> Value {
    s.execute("type.selectionAttrs", &json!({})).unwrap()
}
fn open(bytes: &[u8]) -> Session {
    let mut s = Session::new();
    s.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(bytes)})).unwrap();
    s
}
fn reopen(s: &mut Session) -> Session {
    let native = s.execute("file.serialize", &json!({})).unwrap();
    let bytes = crate::cmd::file::base64_decode(native["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"));
    let back = open(&bytes);
    assert_eq!(serde_json::to_value(&*back.doc().unwrap().doc).unwrap(), serde_json::to_value(&*s.doc().unwrap().doc).unwrap());
    assert!(back.typing_format.is_none());
    back
}
fn styled_fixture(text: &str) -> (Session, StoryId) {
    assert!(text.ends_with('\n'));
    let (mut s, sid) = fixture(text);
    s.execute("style.character.create", &json!({"name": "Tail", "chars": {"size": 23}})).unwrap();
    select(&mut s, sid, 0, text.len());
    s.execute("style.character.apply", &json!({"name": "Tail"})).unwrap();
    // Existing native paragraph defaults, below a live character style.
    let mut native = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    let pi = text.bytes().filter(|b| *b == b'\n').count();
    native["stories"][sid.0.to_string()]["paras"][pi]["chars"] = json!({"size": 11});
    let mut s = open(&serde_json::to_vec(&native).unwrap());
    select(&mut s, sid, text.len(), text.len());
    assert_eq!(attrs(&mut s)["chars"]["size"], 23.0);
    (s, sid)
}

#[test]
fn terminal_defaults_style_direct_clear_and_live_style_survive_native_reopen() {
    let (mut s, sid) = styled_fixture("é\n");
    let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
    format(&mut s, json!({"size": 31}));
    format(&mut s, json!({"leading": {"kind": "points", "value": 36}}));
    assert_eq!(attrs(&mut s)["chars"]["size"], 31.0);
    let mut s = reopen(&mut s);
    select(&mut s, sid, 3, 3);
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.insert_format_at(3).over.size, Some(31.0), "native EOF format must survive without Session.typing_format");
    assert_eq!(st.insert_format_at(3).over.leading, Some(Leading::Points(36.0)));
    assert_eq!(st.insert_format_at(3).style, "Tail");
    assert_eq!(st.text, prior.text);
    assert_eq!(st.chars, prior.chars);
    assert_eq!(st.paras[0], prior.paras[0]);
    assert_eq!(st.paras[1].chars.size, Some(11.0));
    assert!(st.chars.iter().all(|r| r.len > 0));
    format(&mut s, json!({"size": null}));
    let mut s = reopen(&mut s);
    select(&mut s, sid, 3, 3);
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.insert_format_at(3).over.size, None);
    assert_eq!(st.insert_format_at(3).over.leading, Some(Leading::Points(36.0)));
    assert_eq!(st.insert_format_at(3).style, "Tail");
    assert_eq!(st.paras[1].chars.size, Some(11.0));
    assert_eq!(st.chars, prior.chars);
    assert_eq!(attrs(&mut s)["chars"]["size"], 23.0);
    s.execute("style.character.edit", &json!({"name": "Tail", "chars": {"size": 29}})).unwrap();
    let mut s = reopen(&mut s);
    select(&mut s, sid, 3, 3);
    assert_eq!(attrs(&mut s)["chars"]["size"], 29.0);
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.insert_format_at(3).style, "Tail");
    assert_eq!(st.insert_format_at(3).over.size, None);
    assert_eq!(st.paras[1].chars.size, Some(11.0));
    assert_eq!(st.chars, prior.chars);
    s.doc().unwrap().doc.check().unwrap();

    // Clearing the only inherited direct attribute must remain distinct from the LF.
    let (mut s, sid) = styled_fixture("é\n");
    select(&mut s, sid, 0, 3);
    format(&mut s, json!({"size": 31}));
    select(&mut s, sid, 3, 3);
    let runs = s.doc().unwrap().doc.story(sid).unwrap().chars.clone();
    format(&mut s, json!({"size": null}));
    let mut s = reopen(&mut s);
    select(&mut s, sid, 3, 3);
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.format_after(2).over.size, Some(31.0));
    assert_eq!(st.insert_format_at(3).over.size, None);
    assert_eq!(st.insert_format_at(3).style, "Tail");
    assert_eq!(st.paras[1].chars.size, Some(11.0));
    assert_eq!(st.chars, runs);
    assert!(st.chars.iter().all(|r| r.len > 0));
    assert_eq!(attrs(&mut s)["chars"]["size"], 23.0);
    s.doc().unwrap().doc.check().unwrap();
}

#[test]
fn terminal_metrics_and_utf8_typing_keep_defaults_and_neighbor_runs() {
    let (mut s, sid) = styled_fixture("Café\n");
    let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
    format(&mut s, json!({"size": 31}));
    format(&mut s, json!({"leading": {"kind": "points", "value": 36}}));
    select(&mut s, sid, 0, 0);
    select(&mut s, sid, 6, 6);
    assert!(s.typing_format.is_none());
    let composed = designcraft_compose::compose_story(&s.doc().unwrap().doc, sid, &Default::default());
    let blank = composed.frames[0].lines.last().unwrap();
    assert_eq!(blank.range, 6..6);
    assert!((blank.leading - 36.0).abs() < 1e-6);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().chars, prior.chars);
    for text in ["Z", "\n", "é"] {
        s.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
        if text == "\n" {
            let cs = designcraft_compose::compose_story(&s.doc().unwrap().doc, sid, &Default::default());
            let blank = cs.frames[0].lines.last().unwrap();
            assert_eq!(blank.range, 8..8);
            assert!((blank.leading - 36.0).abs() < 1e-6);
        }
    }
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "Café\nZ\né");
    assert_eq!(st.format_after(5), prior.format_after(5));
    for pos in [6, 7, 8] {
        assert_eq!(st.format_after(pos).style, "Tail");
        assert_eq!(st.format_after(pos).over.size, Some(31.0));
        assert_eq!(st.format_after(pos).over.leading, Some(Leading::Points(36.0)));
    }
    assert_eq!(st.paras[1].chars.size, Some(11.0));
    assert_eq!(st.paras[2].chars.size, Some(11.0));
    assert_eq!(st.chars.iter().map(|r| r.len).sum::<usize>(), st.len());
    assert!(st.chars.iter().all(|r| r.len > 0));
    assert_eq!(s.doc().unwrap().selection.text.unwrap().range(), 10..10);
    s.doc().unwrap().doc.check().unwrap();
}

#[test]
fn terminal_format_and_reopened_typing_have_exact_undo_redo() {
    let (mut s, sid) = styled_fixture("é\n");
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    format(&mut s, json!({"size": 31}));
    let formatted = s.doc().unwrap().doc.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(formatted.story(sid).unwrap().insert_format_at(3).over.size, Some(31.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &formatted));
    let mut back = reopen(&mut s);
    select(&mut back, sid, 3, 3);
    let before = back.doc().unwrap().doc.clone();
    back.execute("text.insert", &json!({"text": "字", "raw": true})).unwrap();
    let typed = back.doc().unwrap().doc.clone();
    assert_eq!(typed.story(sid).unwrap().text, "é\n字");
    assert_eq!(typed.story(sid).unwrap().format_after(3).over.size, Some(31.0));
    back.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&back.doc().unwrap().doc, &before));
    back.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&back.doc().unwrap().doc, &typed));
    back.doc().unwrap().doc.check().unwrap();
}

#[test]
fn terminal_scope_preserves_interior_lf_ordinary_carets_empty_story_and_cell_policy() {
    for (text, at) in [("AB", 1), ("AB", 2), ("A\nB", 2)] {
        let (mut s, sid) = fixture(text);
        select(&mut s, sid, at, at);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        format(&mut s, json!({"size": 31}));
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap(), &before);
        assert_eq!(attrs(&mut s)["chars"]["size"], 31.0);
    }
    let (mut s, sid) = fixture("A\n\nB");
    let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
    select(&mut s, sid, 2, 2);
    format(&mut s, json!({"leading": {"kind": "points", "value": 40}}));
    select(&mut s, sid, 0, 0);
    select(&mut s, sid, 2, 2);
    assert!(s.typing_format.is_none());
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.insert_format_at(2).over.leading, Some(Leading::Points(40.0)));
    assert_eq!(st.format_after(1), prior.format_after(1));
    assert_eq!(st.format_after(3), prior.format_after(3));
    let cs = designcraft_compose::compose_story(&s.doc().unwrap().doc, sid, &Default::default());
    assert_eq!(cs.frames[0].lines.iter().find(|l| l.range == (2..2)).unwrap().leading, 40.0);
    let (mut s, sid) = fixture("A\n");
    format(&mut s, json!({"tateChuYoko": true}));
    select(&mut s, sid, 1, 2);
    assert_eq!(attrs(&mut s)["chars"]["tateChuYoko"], false);
    s.execute("type.tateChuYoko", &json!({})).unwrap();
    assert_eq!(attrs(&mut s)["chars"]["tateChuYoko"], true, "the selected LF uses its own run");
    let (mut s, sid) = fixture("");
    format(&mut s, json!({"size": 31}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.chars.len(), 1);
    assert_eq!(st.chars[0].len, 0);
    assert_eq!(st.chars[0].format.over.size, Some(31.0));
    s.execute("table.insert", &json!({"rows": 1, "cols": 1})).unwrap();
    s.execute("table.setCell", &json!({"row": 0, "col": 0, "text": "A\n"})).unwrap();
    let table = *s.doc().unwrap().doc.story(sid).unwrap().tables.keys().next().unwrap();
    s.execute("text.move", &json!({"dir": "storyEnd"})).unwrap();
    let caret = s.doc().unwrap().selection.text.unwrap();
    assert_eq!(caret.cell, Some(designcraft_doc::CellAddr { table, row: 0, col: 0 }));
    assert_eq!((caret.story, caret.anchor, caret.focus), (sid, 2, 2));
    let before = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    format(&mut s, json!({"size": 42}));
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), before);
    assert_eq!(attrs(&mut s)["chars"]["size"], 42.0);
    let pending = super::typing_format(&s).unwrap();
    assert_eq!(pending.at, caret);
    assert_eq!(pending.attrs.size, Some(42.0));
    s.doc().unwrap().doc.check().unwrap();
}

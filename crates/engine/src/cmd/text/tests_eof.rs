//! Native blank-EOF typing regressions; commands drive edits, queries, save/reopen and history.
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
    back
}

#[test]
fn partial_eof_edits_preserve_neighbor_runs_and_continue_typing() {
    let (mut s, sid) = fixture("Café\n");
    select(&mut s, sid, 0, 6);
    format(&mut s, json!({"size": 15, "leading": {"kind": "points", "value": 18}}));
    select(&mut s, sid, 6, 6);
    let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
    format(&mut s, json!({"size": 20}));
    assert_eq!(attrs(&mut s)["chars"]["leading"], json!({"kind": "points", "value": 18.0}));
    format(&mut s, json!({"leading": {"kind": "points", "value": 36}}));
    let after = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(after.text, prior.text);
    assert_eq!(after.chars, prior.chars);
    assert_eq!(after.paras[0], prior.paras[0]);
    assert!(after.chars.iter().all(|r| r.len > 0));
    let composed = s.cache.get(&s.doc().unwrap().doc, sid, None);
    let blank = composed.frames[0].lines.last().unwrap();
    assert!(blank.range.is_empty());
    assert!((blank.leading - 36.0).abs() < 1e-6);
    assert_eq!(attrs(&mut s)["chars"]["size"], 20.0);
    s.execute("text.insert", &json!({"text": "Z", "raw": true})).unwrap();
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    let composed = s.cache.get(&s.doc().unwrap().doc, sid, None);
    assert!((composed.frames[0].lines.last().unwrap().leading - 36.0).abs() < 1e-6);
    assert_eq!(attrs(&mut s)["chars"]["size"], 20.0);
    s.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "Café\nZ\né");
    assert_eq!(st.format_after(6).over.size, Some(20.0));
    assert_eq!(st.format_after(8).over.leading, Some(Leading::Points(36.0)));
    assert!(st.paras.iter().all(|p| p.eof_inherited_chars.is_none()));
    select(&mut s, sid, 6, 10);
    format(&mut s, json!({"size": null, "leading": null}));
    assert_eq!(attrs(&mut s)["chars"]["size"], 12.0, "consumed typing attrs must not become paragraph defaults");
    s.doc().unwrap().doc.check().unwrap();
}

#[test]
fn explicit_empty_eof_stage_survives_native_reopen_and_differs_from_unstaged() {
    let (mut s, sid) = fixture("A\n");
    select(&mut s, sid, 0, 2);
    format(&mut s, json!({"size": 15}));
    select(&mut s, sid, 2, 2);
    let old = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    assert!(!String::from_utf8(old.clone()).unwrap().contains("eofInheritedChars"));
    let mut unstaged = open(&old);
    select(&mut unstaged, sid, 2, 2);
    assert_eq!(attrs(&mut unstaged)["chars"]["size"], 15.0);
    format(&mut s, json!({"size": null}));
    let pf = s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap();
    assert!(pf.chars.is_empty() && pf.eof_inherited_chars.as_ref().unwrap().is_empty());
    let mut back = reopen(&mut s);
    select(&mut back, sid, 2, 2);
    assert_eq!(attrs(&mut back)["chars"]["size"], 12.0);
    back.execute("text.insert", &json!({"text": "X"})).unwrap();
    let st = back.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.format_after(0).over.size, Some(15.0));
    assert_eq!(st.format_after(2).over.size, None);
    assert!(st.paras[1].eof_inherited_chars.is_none());
    back.doc().unwrap().doc.check().unwrap();
}

#[test]
fn eof_keeps_live_character_style_above_genuine_paragraph_defaults() {
    let (mut s, sid) = fixture("A\n");
    s.execute("style.character.create", &json!({"name": "Live", "chars": {"size": 30}})).unwrap();
    select(&mut s, sid, 0, 2);
    s.execute("style.character.apply", &json!({"name": "Live"})).unwrap();
    // A legacy/native paragraph can carry real direct defaults below its character style.
    let mut legacy = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    legacy["stories"][sid.0.to_string()]["paras"][1]["chars"] = json!({"size": 24});
    let mut s = open(&serde_json::to_vec(&legacy).unwrap());
    select(&mut s, sid, 2, 2);
    format(&mut s, json!({"leading": {"kind": "points", "value": 36}}));
    assert_eq!(attrs(&mut s)["chars"]["size"], 30.0);
    s.execute("style.character.edit", &json!({"name": "Live", "chars": {"size": 32}})).unwrap();
    assert_eq!(attrs(&mut s)["chars"]["size"], 32.0);
    let mut s = reopen(&mut s);
    select(&mut s, sid, 2, 2);
    assert_eq!(attrs(&mut s)["chars"]["size"], 32.0);
    s.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.paras[1].chars.size, Some(24.0));
    assert_eq!(st.format_after(2).style, "Live");
    assert_eq!(st.format_after(2).over.size, None);
    select(&mut s, sid, 2, 4);
    format(&mut s, json!({"leading": null}));
    assert_eq!(attrs(&mut s)["chars"]["size"], 32.0);
    s.execute("style.character.edit", &json!({"name": "Live", "chars": {"size": null}})).unwrap();
    assert_eq!(attrs(&mut s)["chars"]["size"], 24.0);
}

#[test]
fn eof_format_and_typing_have_exact_document_undo_redo_after_native_reopen() {
    let (mut s, sid) = fixture("A\n");
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    format(&mut s, json!({"size": 20}));
    let formatted = s.doc().unwrap().doc.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &formatted));
    let mut back = reopen(&mut s);
    select(&mut back, sid, 2, 2);
    let before = back.doc().unwrap().doc.clone();
    back.execute("text.insert", &json!({"text": "é", "raw": true})).unwrap();
    let typed = back.doc().unwrap().doc.clone();
    assert_eq!(typed.story(sid).unwrap().text, "A\né");
    back.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&back.doc().unwrap().doc, &before));
    back.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&back.doc().unwrap().doc, &typed));
    back.doc().unwrap().doc.check().unwrap();
}

#[test]
fn ordinary_carets_empty_stories_and_cell_eof_keep_their_existing_policy() {
    for (text, at) in [("AB", 1), ("A\n\nB", 2)] {
        let (mut s, sid) = fixture(text);
        select(&mut s, sid, at, at);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        format(&mut s, json!({"size": 20}));
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap(), &before);
    }
    let (mut range, sid) = fixture("A\n");
    format(&mut range, json!({"tateChuYoko": true}));
    select(&mut range, sid, 1, 2);
    range.execute("type.tateChuYoko", &json!({})).unwrap();
    assert_eq!(attrs(&mut range)["chars"]["tateChuYoko"], true, "a selected final LF reads its own run, not the staged EOF caret");
    let (mut s, sid) = fixture("");
    format(&mut s, json!({"size": 20}));
    assert_eq!(attrs(&mut s)["chars"]["size"], 20.0);
    assert!(s.doc().unwrap().doc.story(sid).unwrap().paras[0].eof_inherited_chars.is_none());
    s.execute("table.insert", &json!({"rows": 1, "cols": 1})).unwrap();
    s.execute("table.setCell", &json!({"row": 0, "col": 0, "text": "A\n"})).unwrap();
    let table = *s.doc().unwrap().doc.story(sid).unwrap().tables.keys().next().unwrap();
    let cell = json!({"table": table, "row": 0, "col": 0});
    s.execute("text.select", &json!({"story": sid.0, "cell": cell, "anchor": 2, "focus": 2})).unwrap();
    assert!(s.doc().unwrap().selection.text.unwrap().cell.is_some());
    let before = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    format(&mut s, json!({"size": 42}));
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), before);
    s.doc().unwrap().doc.check().unwrap();
}

//! Preservation controls: named character formats must stay live across Enter and UTF-8 typing.
//! These assertions are intended to PASS unchanged on the immutable BEFORE and refined AFTER.

use designcraft_doc::{ChangeMark, CharAttrs, CharFormat, StoryId};
use serde_json::{Value, json};

use crate::Session;

fn full_doc(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, a: usize, b: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": a, "focus": b})).unwrap();
}

fn fixture(parent: bool, tracking: bool) -> (Session, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let f = s.execute("frame.create", &json!({"rect": [36, 36, 436, 236], "content": "text", "text": "Tail"})).unwrap();
    let sid = StoryId(f["story"].as_u64().unwrap());
    if parent {
        s.execute("style.character.create", &json!({"name": "R8 Parent", "chars": {"size": 18, "leading": {"kind": "points", "value": 24}}}))
            .unwrap();
        s.execute("style.character.create", &json!({"name": "R8 Inherited", "basedOn": "R8 Parent"})).unwrap();
    } else {
        s.execute("style.character.create", &json!({"name": "R8 Inherited", "chars": {"size": 18, "leading": {"kind": "points", "value": 24}}}))
            .unwrap();
    }
    select(&mut s, sid, 0, 4);
    s.execute("style.character.apply", &json!({"name": "R8 Inherited"})).unwrap();
    if tracking {
        s.execute("changes.track", &json!({"on": true})).unwrap();
    }
    select(&mut s, sid, 4, 4);
    (s, sid)
}

fn undo_redo(s: &mut Session, before: &Value, after: &Value) {
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(&full_doc(s), before, "undo restores the entire document");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(&full_doc(s), after, "redo restores the entire document");
}

fn zip_reopen(s: &mut Session) -> Session {
    let full = full_doc(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let bytes = crate::cmd::file::base64_decode(saved["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"), "public native ZIP serialization");
    let mut back = Session::new();
    back.execute("file.openBytes", &json!({"base64": saved["base64"]})).unwrap();
    assert_eq!(full_doc(&back), full);
    back.doc().unwrap().doc.check().unwrap();
    back
}

fn bound_format(s: &mut Session, sid: StoryId, a: usize, b: usize, tracking: bool, size: f64, leading: f64) {
    select(s, sid, a, b);
    let expected =
        CharFormat { style: "R8 Inherited".into(), over: CharAttrs { change: tracking.then_some(ChangeMark::Inserted), ..Default::default() } };
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    for (offset, _) in st.text[a..b].char_indices() {
        assert_eq!(st.format_after(a + offset), &expected, "complete named format at UTF-8 byte {}", a + offset);
    }
    let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(q["characterStyle"], "R8 Inherited");
    assert_eq!(q["charOverrides"], json!(usize::from(tracking)), "inherited size/leading must not become direct overrides");
    assert_eq!(q["chars"]["size"], json!(size));
    assert_eq!(q["chars"]["leading"], json!({"kind": "points", "value": leading}));
    s.doc().unwrap().doc.check().unwrap();
}

#[test]
fn r8_preserve_named_character_binding_after_enter_and_style_edit() {
    for tracking in [false, true] {
        let (mut s, sid) = fixture(false, tracking);
        for text in ["\n", "é"] {
            let before = full_doc(&s);
            s.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
            let after = full_doc(&s);
            undo_redo(&mut s, &before, &after);
        }
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "Tail\né");
        bound_format(&mut s, sid, 5, 7, tracking, 18.0, 24.0);
        let before_edit = full_doc(&s);
        s.execute("style.character.edit", &json!({"name": "R8 Inherited", "chars": {"size": 30, "leading": {"kind": "points", "value": 40}}}))
            .unwrap();
        bound_format(&mut s, sid, 5, 7, tracking, 30.0, 40.0);
        let after_edit = full_doc(&s);
        undo_redo(&mut s, &before_edit, &after_edit);
        bound_format(&mut s, sid, 0, 4, false, 30.0, 40.0);
        let mut back = zip_reopen(&mut s);
        bound_format(&mut back, sid, 5, 7, tracking, 30.0, 40.0);
        let before = full_doc(&back);
        back.execute("style.character.edit", &json!({"name": "R8 Inherited", "chars": {"size": 32, "leading": {"kind": "points", "value": 42}}}))
            .unwrap();
        bound_format(&mut back, sid, 5, 7, tracking, 32.0, 42.0);
        let after = full_doc(&back);
        undo_redo(&mut back, &before, &after);
    }
}

#[test]
fn r8_preserve_based_on_binding_partial_override_clear_utf8_and_history_zip() {
    for tracking in [false, true] {
        let (mut s, sid) = fixture(true, tracking);
        for text in ["\n", "é", "à"] {
            let before = full_doc(&s);
            s.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
            let after = full_doc(&s);
            undo_redo(&mut s, &before, &after);
        }
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "Tail\néà");
        bound_format(&mut s, sid, 5, 9, tracking, 18.0, 24.0);
        select(&mut s, sid, 5, 7);
        let before_override = full_doc(&s);
        s.execute("type.char", &json!({"attrs": {"size": 22, "leading": {"kind": "points", "value": 35}}})).unwrap();
        let overridden = full_doc(&s);
        undo_redo(&mut s, &before_override, &overridden);
        let before_edit = full_doc(&s);
        s.execute("style.character.edit", &json!({"name": "R8 Parent", "chars": {"size": 30, "leading": {"kind": "points", "value": 40}}})).unwrap();
        let after_edit = full_doc(&s);
        undo_redo(&mut s, &before_edit, &after_edit);
        bound_format(&mut s, sid, 7, 9, tracking, 30.0, 40.0);
        bound_format(&mut s, sid, 0, 4, false, 30.0, 40.0);
        select(&mut s, sid, 5, 7);
        let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
        assert_eq!(q["characterStyle"], "R8 Inherited");
        assert_eq!(q["chars"]["size"], json!(22.0));
        assert_eq!(q["chars"]["leading"], json!({"kind": "points", "value": 35.0}));
        assert_eq!(q["charOverrides"], json!(2 + usize::from(tracking)));
        let before_clear = full_doc(&s);
        s.execute("type.char", &json!({"attrs": {"size": null}})).unwrap();
        let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
        assert_eq!(q["characterStyle"], "R8 Inherited");
        assert_eq!(q["chars"]["size"], json!(30.0));
        assert_eq!(q["chars"]["leading"], json!({"kind": "points", "value": 35.0}));
        let partial = full_doc(&s);
        undo_redo(&mut s, &before_clear, &partial);
        let mut back = zip_reopen(&mut s);
        select(&mut back, sid, 5, 7);
        let before_clear = full_doc(&back);
        back.execute("type.char", &json!({"attrs": {"leading": null}})).unwrap();
        bound_format(&mut back, sid, 5, 7, tracking, 30.0, 40.0);
        bound_format(&mut back, sid, 7, 9, tracking, 30.0, 40.0);
        let cleared = full_doc(&back);
        undo_redo(&mut back, &before_clear, &cleared);
        // Editing the parent after clears/reopen must still reach the newly typed bytes.
        let before_edit = full_doc(&back);
        back.execute("style.character.edit", &json!({"name": "R8 Parent", "chars": {"size": 34, "leading": {"kind": "points", "value": 44}}}))
            .unwrap();
        bound_format(&mut back, sid, 5, 9, tracking, 34.0, 44.0);
        bound_format(&mut back, sid, 0, 4, false, 34.0, 44.0);
        let after_edit = full_doc(&back);
        undo_redo(&mut back, &before_edit, &after_edit);
        if tracking {
            let changes = back.execute("changes.list", &json!({})).unwrap();
            assert_eq!(changes.as_array().unwrap().len(), 1);
            assert_eq!(changes[0]["text"], "\néà");
            assert_eq!(changes[0]["kind"], "inserted");
        }
        zip_reopen(&mut back);
    }
}

#[test]
fn r8_preserve_complete_direct_format_across_enter_and_parent_edit_while_blank() {
    for tracking in [false, true] {
        let (mut s, sid) = fixture(true, tracking);
        select(&mut s, sid, 0, 4);
        s.execute("type.char", &json!({"attrs": {"size": 22, "leading": {"kind": "points", "value": 35}}})).unwrap();
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        select(&mut s, sid, 4, 4);
        let before = full_doc(&s);
        s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
        let after = full_doc(&s);
        undo_redo(&mut s, &before, &after);
        // A style edit while the continuation is still blank must not freeze inherited values.
        let before_edit = full_doc(&s);
        s.execute("style.character.edit", &json!({"name": "R8 Parent", "chars": {"size": 30, "leading": {"kind": "points", "value": 40}}})).unwrap();
        let after_edit = full_doc(&s);
        undo_redo(&mut s, &before_edit, &after_edit);
        for text in ["é", "à"] {
            let before = full_doc(&s);
            s.execute("text.insert", &json!({"text": text, "raw": true})).unwrap();
            let after = full_doc(&s);
            undo_redo(&mut s, &before, &after);
        }
        let expected = CharFormat {
            style: "R8 Inherited".into(),
            over: CharAttrs {
                size: Some(22.0),
                leading: Some(designcraft_doc::Leading::Points(35.0)),
                change: tracking.then_some(ChangeMark::Inserted),
                ..Default::default()
            },
        };
        let st = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.text, "Tail\néà");
        assert_eq!(st.format_after(5), &expected);
        assert_eq!(st.format_after(7), &expected);
        assert_eq!(st.format_after(0), original.format_after(0), "preceding run data stays exact through typing and parent edits");
        select(&mut s, sid, 5, 7);
        let q = s.execute("type.selectionAttrs", &json!({})).unwrap();
        assert_eq!(q["characterStyle"], "R8 Inherited");
        assert_eq!(q["charOverrides"], json!(2 + usize::from(tracking)));
        let mut back = zip_reopen(&mut s);
        select(&mut back, sid, 5, 7);
        let before_clear = full_doc(&back);
        back.execute("type.char", &json!({"attrs": {"size": null, "leading": null}})).unwrap();
        bound_format(&mut back, sid, 5, 7, tracking, 30.0, 40.0);
        let after_clear = full_doc(&back);
        undo_redo(&mut back, &before_clear, &after_clear);
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().format_after(7), &expected, "clearing é preserves à's complete direct format");
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().format_after(0), original.format_after(0));
        let before_edit = full_doc(&back);
        back.execute("style.character.edit", &json!({"name": "R8 Parent", "chars": {"size": 34, "leading": {"kind": "points", "value": 44}}}))
            .unwrap();
        bound_format(&mut back, sid, 5, 7, tracking, 34.0, 44.0);
        let after_edit = full_doc(&back);
        undo_redo(&mut back, &before_edit, &after_edit);
        zip_reopen(&mut back);
    }
}

//! C5 EOF-CONSUMER-01: public toggle commands read the persisted outer EOF stage.

use designcraft_doc::{CharAttrs, Story, StoryId};
use designcraft_engine::Session;
use serde_json::{Value, json};

const TOGGLES: [(&str, &str); 2] = [("type.tateChuYoko", "tateChuYoko"), ("type.kenten", "kenten")];

fn full_doc(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, start: usize, end: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": start, "focus": end})).unwrap();
}

fn fixture(text: &str) -> (Session, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let frame = s.execute("frame.create", &json!({"rect": [40, 40, 440, 500], "content": "text", "text": text})).unwrap();
    let sid = StoryId(frame["story"].as_u64().unwrap());
    select(&mut s, sid, text.len(), text.len());
    (s, sid)
}

fn edit(s: &mut Session, command: &str, params: Value) {
    let before = full_doc(s);
    let depth = s.doc().unwrap().history.undo.len();
    s.execute(command, &params).unwrap();
    let after = full_doc(s);
    let after_selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "{command}: one command, one history entry");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(full_doc(s), before, "{command}: undo restores the complete document");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(full_doc(s), after, "{command}: redo restores the complete document");
    assert_eq!(s.doc().unwrap().selection, after_selection);
    s.doc().unwrap().doc.check().unwrap();
}

fn native_copies(s: &mut Session) -> [Session; 2] {
    let before = full_doc(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let zip = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"));
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    let mut copies = [Session::new(), Session::new()];
    for (copy, bytes) in copies.iter_mut().zip([zip, legacy]) {
        copy.execute("file.openBytes", &json!({"base64": designcraft_engine::cmd::base64_encode(&bytes)})).unwrap();
        assert_eq!(full_doc(copy), before, "native ZIP and legacy readers preserve the complete document");
        copy.doc().unwrap().doc.check().unwrap();
    }
    copies
}

fn direct(attrs: &CharAttrs, field: &str) -> Value {
    serde_json::to_value(attrs).unwrap()[field].clone()
}

fn assert_stage(s: &mut Session, sid: StoryId, field: &str, expected: bool, original: &Story) {
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(direct(&story.paras.last().unwrap().chars, field), json!(expected), "{field}: persisted typing choice");
    assert_eq!(story.text, original.text);
    assert_eq!(story.chars, original.chars, "a blank EOF toggle must not format the preceding LF");
    assert_eq!(story.paras[..story.paras.len() - 1], original.paras[..original.paras.len() - 1]);
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"][field], json!(expected), "{field}: effective caret query agrees with the stage");
}

#[test]
fn r22_eof_toggles_on_off_on_history_native_and_typing() {
    for (command, field) in TOGGLES {
        let (mut s, sid) = fixture("A\n");
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        for expected in [true, false, true] {
            edit(&mut s, command, json!({}));
            assert_stage(&mut s, sid, field, expected, &original);
        }
        for mut back in native_copies(&mut s) {
            select(&mut back, sid, 2, 2);
            assert_stage(&mut back, sid, field, true, &original);
            edit(&mut back, command, json!({}));
            assert_stage(&mut back, sid, field, false, &original);
            edit(&mut back, "text.insert", json!({"text": "B", "raw": true}));
            let story = back.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(story.text, "A\nB");
            assert_eq!(direct(&story.format_after(2).over, field), json!(false), "typed bytes own the off choice");
            assert_eq!(story.format_after(1), original.format_after(1));
            assert_eq!(story.paras.last().unwrap(), original.paras.last().unwrap(), "typing consumes only the stage");
        }
    }
}

#[test]
fn r22_eof_toggles_explicit_on_is_authoritative_even_when_already_on() {
    for (command, field) in TOGGLES {
        let (mut s, sid) = fixture("A\n");
        select(&mut s, sid, 0, 2);
        s.execute(command, &json!({"on": true})).unwrap();
        select(&mut s, sid, 2, 2);
        let original = s.doc().unwrap().doc.story(sid).unwrap().clone();
        for (params, expected) in [(json!({"on": false}), false), (json!({}), true), (json!({"on": true}), true), (json!({}), false)] {
            edit(&mut s, command, params);
            assert_stage(&mut s, sid, field, expected, &original);
        }
    }
}

#[test]
fn r22_eof_toggles_selected_lf_ordinary_range_caret_and_empty_story_controls() {
    for (command, field) in TOGGLES {
        let (mut s, sid) = fixture("A\n");
        s.execute(command, &json!({"on": true})).unwrap();
        select(&mut s, sid, 1, 2);
        for expected in [true, false, true] {
            s.execute(command, &json!({})).unwrap();
            let story = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(direct(&story.format_after(1).over, field), json!(expected), "selected LF reads its own run");
            assert_eq!(direct(&story.paras.last().unwrap().chars, field), json!(true), "selected LF leaves the stage intact");
            assert_eq!(direct(&story.format_after(0).over, field), Value::Null);
        }
        for text in ["AB", ""] {
            let (mut s, sid) = fixture(text);
            select(&mut s, sid, 0, usize::from(!text.is_empty()));
            for expected in [true, false, true] {
                s.execute(command, &json!({})).unwrap();
                let story = s.doc().unwrap().doc.story(sid).unwrap();
                assert_eq!(direct(&story.char_format_at(1.min(text.len())).over, field), json!(expected));
            }
            if !text.is_empty() {
                select(&mut s, sid, 1, 1);
                let before = full_doc(&s);
                s.execute(command, &json!({})).unwrap();
                assert_eq!(full_doc(&s), before, "ordinary nonempty-story caret keeps the existing no-stage policy");
            }
        }
    }
}

fn child_caret_toggle(s: &mut Session, sid: StoryId, command: &str, field: &str, expected_value: bool) {
    let mut expected = (*s.doc().unwrap().doc).clone();
    // Existing text_story_mut access bumps the containing story, even for a child caret no-op.
    let outer = expected.story_mut(sid).unwrap();
    outer.rev = outer.rev.wrapping_add(1);
    s.execute(command, &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, expected, "child caret changes only the containing story revision");
    assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"][field], json!(expected_value));
}

#[test]
fn r22_eof_toggles_table_and_note_controls() {
    for (command, field) in TOGGLES {
        for note in [false, true] {
            let (mut s, sid) = fixture("");
            if note {
                s.execute("footnote.insert", &json!({"text": "A\n"})).unwrap();
            } else {
                s.execute("table.insert", &json!({"rows": 1, "cols": 1, "width": 300})).unwrap();
                s.execute("text.insert", &json!({"text": "A\n", "raw": true})).unwrap();
            }
            let cell = s.doc().unwrap().selection.text.unwrap().cell.unwrap();
            s.execute("text.move", &json!({"dir": "storyEnd"})).unwrap();
            for _ in 0..2 {
                child_caret_toggle(&mut s, sid, command, field, false);
            }
            s.execute("text.move", &json!({"dir": "storyStart"})).unwrap();
            s.execute("text.move", &json!({"dir": "storyEnd", "extend": true})).unwrap();
            let selection = s.doc().unwrap().selection.text.unwrap();
            assert_eq!((selection.story, selection.cell, selection.range()), (sid, Some(cell), 0..2));
            for expected in [true, false, true] {
                s.execute(command, &json!({})).unwrap();
                let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap();
                assert_eq!(direct(&child.format_after(0).over, field), json!(expected));
                assert_eq!(direct(&child.format_after(1).over, field), json!(expected));
            }
            s.execute("text.move", &json!({"dir": "storyEnd"})).unwrap();
            let child = s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap().clone();
            child_caret_toggle(&mut s, sid, command, field, true);
            assert_eq!(s.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap(), &child, "child EOF remains unstaged");
            assert!(child.paras.last().unwrap().chars.is_empty());
        }
    }
}

//! IDML retains blank-EOF typing overrides separately from paragraph defaults.
use designcraft_doc::{Story, StoryId};
use designcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn document(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn story(s: &Session, sid: StoryId) -> &Story {
    s.doc().unwrap().doc.story(sid).unwrap()
}

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    run(s, "text.select", json!({"story": sid.0, "anchor": anchor, "focus": focus}));
}

fn size(s: &mut Session) -> f64 {
    run(s, "type.selectionAttrs", json!({}))["chars"]["size"].as_f64().unwrap()
}

fn staged(inherited: bool, named: bool) -> (Session, StoryId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"pages": 1}));
    let sid = StoryId(run(&mut s, "frame.create", json!({"rect": [40, 40, 440, 440], "content": "text", "text": "A"}))["story"].as_u64().unwrap());
    select(&mut s, sid, 0, 1);
    if inherited {
        run(&mut s, "style.paragraph.create", json!({"name": "EOF baseline", "chars": {"size": 24}}));
        run(&mut s, "style.paragraph.apply", json!({"name": "EOF baseline"}));
        run(&mut s, "style.breakLink", json!({"kind": "paragraph"}));
        assert_eq!(story(&s, sid).paras[0].chars.size, Some(24.0));
    }
    if named {
        run(&mut s, "style.character.create", json!({"name": "EOF live", "chars": {"size": 20}}));
        run(&mut s, "style.character.apply", json!({"name": "EOF live"}));
    }
    select(&mut s, sid, 1, 1);
    run(&mut s, "text.insert", json!({"text": "\n", "raw": true}));
    let runs = story(&s, sid).chars.clone();
    run(&mut s, "type.char", json!({"attrs": {"size": 30}}));
    assert_eq!(story(&s, sid).text, "A\n");
    assert_eq!(story(&s, sid).chars, runs, "staging must not restyle the actual LF");
    assert_eq!(size(&mut s), 30.0);
    (s, sid)
}

fn reopen(s: &mut Session, native: bool) -> (Session, StoryId) {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let bytes = run(s, if native { "file.serialize" } else { "file.exportIdml" }, json!({}));
    assert_eq!(document(s), before, "export is read-only");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    let mut back = Session::new();
    run(&mut back, if native { "file.openBytes" } else { "file.openIdml" }, json!({"base64": bytes["base64"]}));
    if native {
        assert_eq!(document(&back), before, "native persistence retains all document state");
    }
    let sid = back.doc().unwrap().doc.stories.values().find(|st| st.text == "A\n").unwrap().id;
    select(&mut back, sid, 2, 2);
    back.doc().unwrap().doc.check().unwrap();
    (back, sid)
}

fn edit(s: &mut Session, command: &str, params: Value) {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    run(s, command, params);
    let after = document(s);
    let selection_after = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    run(s, "edit.undo", json!({}));
    assert_eq!(document(s), before);
    // Command history retains the post-edit selection when restoring the document.
    assert_eq!(s.doc().unwrap().selection, selection_after);
    run(s, "edit.redo", json!({}));
    assert_eq!(document(s), after);
    assert_eq!(s.doc().unwrap().selection, selection_after);
}

fn type_and_clear(s: &mut Session, sid: StoryId, expected_clear: f64) {
    select(s, sid, 2, 2);
    assert_eq!(size(s), 30.0, "EOF query reads the restored stage");
    let before = story(s, sid).clone();
    edit(s, "text.insert", json!({"text": "B", "raw": true}));
    select(s, sid, 2, 3);
    assert_eq!(size(s), 30.0, "next typing consumes the restored stage");
    assert!(story(s, sid).paras[1].eof_inherited_chars.is_none());
    edit(s, "type.char", json!({"attrs": {"size": null}}));
    assert_eq!(size(s), expected_clear, "clear exposes the genuine baseline below the live character style");
    assert_eq!(story(s, sid).text, "A\nB");
    assert_eq!(story(s, sid).paras[0], before.paras[0]);
    for at in 0..2 {
        assert_eq!(story(s, sid).format_after(at), before.format_after(at), "prefix/LF runs are unchanged by typing and clear");
    }
    s.doc().unwrap().doc.check().unwrap();
}

#[test]
fn blank_eof_idml_retains_stage_and_genuine_baseline() {
    for inherited in [false, true] {
        let (mut source, sid) = staged(inherited, false);
        let original = story(&source, sid).paras[1].clone();
        let (mut back, sid) = reopen(&mut source, false);
        assert_eq!(story(&back, sid).paras[1].chars, original.chars);
        assert_eq!(story(&back, sid).paras[1].eof_inherited_chars, original.eof_inherited_chars);
        let (mut native, native_sid) = reopen(&mut back, true);
        let expected = if inherited { 24.0 } else { 12.0 };
        type_and_clear(&mut back, sid, expected);
        type_and_clear(&mut native, native_sid, expected);
    }
}

#[test]
fn a_second_idml_roundtrip_retains_the_blank_eof_layers() {
    let (mut source, _) = staged(true, false);
    let (mut once, sid) = reopen(&mut source, false);
    let original = story(&once, sid).paras[1].clone();
    let (mut twice, sid) = reopen(&mut once, false);
    assert_eq!(story(&twice, sid).paras[1].chars, original.chars);
    assert_eq!(story(&twice, sid).paras[1].eof_inherited_chars, original.eof_inherited_chars);
    type_and_clear(&mut twice, sid, 24.0);
}

#[test]
fn idml_eof_clear_keeps_the_character_style_binding_live() {
    let (mut source, _) = staged(true, true);
    let (mut back, sid) = reopen(&mut source, false);
    assert_eq!(story(&back, sid).format_after(1).style, "EOF live");
    assert_eq!(story(&back, sid).paras[1].eof_inherited_chars.as_ref().unwrap().size, Some(24.0));
    run(&mut back, "style.character.edit", json!({"name": "EOF live", "chars": {"size": 28}}));
    type_and_clear(&mut back, sid, 28.0);
    assert_eq!(story(&back, sid).paras[1].chars.size, Some(24.0));
    assert_eq!(story(&back, sid).format_after(2).style, "EOF live");
}

//! Find/Change attributes on a new final LF must also reach its blank-EOF typing stage.

use designcraft_doc::{ParaFormat, Story, StoryId};
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

fn attrs(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) -> Value {
    select(s, sid, anchor, focus);
    run(s, "type.selectionAttrs", json!({}))
}

fn size(s: &mut Session, sid: StoryId, anchor: usize, focus: usize, expected: f64) {
    assert_eq!(attrs(s, sid, anchor, focus)["chars"]["size"], json!(expected));
}

fn edit(s: &mut Session, command: &str, params: Value) -> Value {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let result = run(s, command, params);
    let after = document(s);
    let selection_after = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
    run(s, "edit.undo", json!({}));
    assert_eq!(document(s), before, "Undo restores every document field, including the genuine EOF baseline");
    // The established history contract retains the command's post-edit selection.
    assert_eq!(s.doc().unwrap().selection, selection_after);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    run(s, "edit.redo", json!({}));
    assert_eq!(document(s), after, "Redo restores every document field");
    assert_eq!(s.doc().unwrap().selection, selection_after);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn reopen(s: &mut Session) -> Session {
    let expected = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let saved = run(s, "file.serialize", json!({}));
    assert_eq!(document(s), expected, "native serialization is read-only");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    let mut back = Session::new();
    run(&mut back, "file.openBytes", json!({"base64": saved["base64"]}));
    assert_eq!(document(&back), expected, "native reopen preserves the complete document");
    back.doc().unwrap().doc.check().unwrap();
    back
}

fn fixture(text: &str, genuine: bool, named: bool) -> (Session, StoryId, ParaFormat) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"pages": 1}));
    let frame = run(&mut s, "frame.create", json!({"rect": [40, 40, 440, 340], "content": "text", "text": text}));
    let sid = StoryId(frame["story"].as_u64().unwrap());
    select(&mut s, sid, 0, text.len());
    if genuine {
        run(&mut s, "style.paragraph.create", json!({"name": "F1 baseline", "chars": {"size": 24}}));
        run(&mut s, "style.paragraph.apply", json!({"name": "F1 baseline"}));
        run(&mut s, "style.breakLink", json!({"kind": "paragraph"}));
    }
    let baseline = story(&s, sid).paras[0].clone();
    assert_eq!(baseline.chars.size, genuine.then_some(24.0));
    assert!(baseline.eof_inherited_chars.is_none());
    if named {
        run(&mut s, "style.character.create", json!({"name": "F1 parent", "chars": {"size": 20, "underline": true}}));
        run(&mut s, "style.character.create", json!({"name": "F1 child", "basedOn": "F1 parent"}));
        run(&mut s, "style.character.apply", json!({"name": "F1 child"}));
    }
    run(&mut s, "type.char", json!({"attrs": {"size": 15}}));
    (s, sid, baseline)
}

fn type_and_clear(s: &mut Session, sid: StoryId, baseline: &ParaFormat, typing_size: f64, clear_size: f64) {
    let prior = story(s, sid).clone();
    let at = prior.len();
    size(s, sid, at, at, typing_size);
    edit(s, "text.insert", json!({"text": "é", "raw": true}));
    size(s, sid, at, at + "é".len(), typing_size);
    edit(s, "type.char", json!({"attrs": {"size": null}}));
    size(s, sid, at, at + "é".len(), clear_size);
    assert_eq!(story(s, sid).paras.last(), Some(baseline), "typing consumes the stage, never promotes it to paragraph defaults");
    for (offset, _) in prior.text.char_indices() {
        assert_eq!(story(s, sid).format_after(offset), prior.format_after(offset), "typing and clearing preserve every prior run");
    }
    reopen(s);
}

#[test]
fn final_lf_change_attrs_reach_eof_and_next_typing_without_replacing_baseline() {
    for genuine in [false, true] {
        for tracking in [false, true] {
            let (mut s, sid, baseline) = fixture("A\nX", genuine, false);
            run(&mut s, "changes.track", json!({"on": tracking}));
            select(&mut s, sid, 3, 0);
            let selection = s.doc().unwrap().selection.clone();
            let prefix = story(&s, sid).format_after(0).clone();
            let result = edit(&mut s, "find.change", json!({"story": sid.0, "find": "X", "change": "^p", "attrs": {"size": 30}}));
            assert_eq!(result["count"], 1);
            assert_eq!(s.doc().unwrap().selection, selection, "Find/Change retains its selection policy");
            assert_eq!(story(&s, sid).text, "A\n\n");
            assert_eq!(story(&s, sid).format_after(0), &prefix);
            assert_eq!(story(&s, sid).format_after(1), &prefix);
            assert_eq!(story(&s, sid).paras[0], baseline);
            let eof = story(&s, sid).paras.last().unwrap();
            assert_eq!(eof.chars.size, Some(30.0));
            assert_eq!(eof.eof_inherited_chars.as_ref(), genuine.then_some(&baseline.chars));
            assert_eq!(run(&mut s, "changes.list", json!({})), json!([]), "Find/Change does not acquire tracked replacement semantics");
            size(&mut s, sid, 2, 3, 30.0);
            size(&mut s, sid, 3, 3, 30.0);
            let mut back = reopen(&mut s);
            run(&mut back, "changes.track", json!({"on": false}));
            type_and_clear(&mut back, sid, &baseline, 30.0, if genuine { 24.0 } else { 12.0 });
        }
    }
}

#[test]
fn eof_change_attrs_keep_named_character_style_live_through_clear_and_reopen() {
    for genuine in [false, true] {
        let (mut s, sid, baseline) = fixture("AX", genuine, true);
        edit(&mut s, "find.change", json!({"story": sid.0, "find": "X", "change": "^p", "attrs": {"size": 30}}));
        let mut back = reopen(&mut s);
        edit(&mut back, "style.character.edit", json!({"name": "F1 parent", "chars": {"size": 28, "underline": false}}));
        for (a, b) in [(1, 2), (2, 2)] {
            let effective = attrs(&mut back, sid, a, b);
            assert_eq!(effective["chars"]["size"], 30.0);
            assert_eq!(effective["chars"]["underline"], false, "unrelated style attributes remain live below sparse replacement attrs");
            assert_eq!(effective["characterStyle"], "F1 child");
        }
        type_and_clear(&mut back, sid, &baseline, 30.0, 28.0);
        assert_eq!(story(&back, sid).format_after(2).style, "F1 child");
        edit(&mut back, "style.character.edit", json!({"name": "F1 parent", "chars": {"size": 34}}));
        size(&mut back, sid, 2, 4, 34.0);
        size(&mut back, sid, 1, 2, 30.0);
        reopen(&mut back);
    }
}

#[test]
fn clearing_consumed_eof_attrs_reveals_a_live_paragraph_style() {
    let (mut s, sid, _) = fixture("AX", false, false);
    run(&mut s, "style.paragraph.create", json!({"name": "F1 live paragraph", "chars": {"size": 12}}));
    run(&mut s, "style.paragraph.apply", json!({"name": "F1 live paragraph"}));
    let baseline = story(&s, sid).paras[0].clone();
    edit(&mut s, "find.change", json!({"story": sid.0, "find": "X", "change": "^p", "attrs": {"size": 30}}));
    let mut back = reopen(&mut s);
    type_and_clear(&mut back, sid, &baseline, 30.0, 12.0);
    edit(&mut back, "style.paragraph.edit", json!({"name": "F1 live paragraph", "chars": {"size": 22}}));
    size(&mut back, sid, 2, 4, 22.0);
    size(&mut back, sid, 1, 2, 30.0);
    reopen(&mut back);
}

#[test]
fn non_tail_newline_change_does_not_touch_existing_blank_eof() {
    for genuine in [false, true] {
        let (mut s, sid, baseline) = fixture("AXB", genuine, false);
        select(&mut s, sid, 3, 3);
        run(&mut s, "text.insert", json!({"text": "\n", "raw": true}));
        run(&mut s, "type.char", json!({"attrs": {"size": 45}}));
        let eof = story(&s, sid).paras.last().unwrap().clone();
        let final_lf = story(&s, sid).format_after(3).clone();
        edit(&mut s, "find.change", json!({"story": sid.0, "find": "X", "change": "^p", "attrs": {"size": 30}}));
        assert_eq!(story(&s, sid).text, "A\nB\n");
        assert_eq!(story(&s, sid).paras.last(), Some(&eof));
        assert_eq!(story(&s, sid).format_after(3), &final_lf);
        size(&mut s, sid, 1, 2, 30.0);
        size(&mut s, sid, 2, 3, 15.0);
        size(&mut s, sid, 4, 4, 45.0);
        let mut back = reopen(&mut s);
        type_and_clear(&mut back, sid, &baseline, 45.0, if genuine { 24.0 } else { 12.0 });
    }
}

#[test]
fn omitted_empty_and_null_change_attrs_keep_existing_merge_policy() {
    for genuine in [false, true] {
        for change_attrs in [None, Some(Value::Null), Some(json!({})), Some(json!({"size": null}))] {
            let (mut s, sid, baseline) = fixture("AX", genuine, false);
            let mut params = json!({"story": sid.0, "find": "X", "change": "^p"});
            if let Some(value) = change_attrs {
                params["attrs"] = value;
            }
            edit(&mut s, "find.change", params);
            size(&mut s, sid, 1, 2, 15.0);
            size(&mut s, sid, 2, 2, 15.0);
            let eof = story(&s, sid).paras.last().unwrap();
            assert_eq!(eof.chars.size, Some(15.0));
            assert_eq!(eof.eof_inherited_chars.as_ref(), genuine.then_some(&baseline.chars));
            let mut back = reopen(&mut s);
            type_and_clear(&mut back, sid, &baseline, 15.0, if genuine { 24.0 } else { 12.0 });
        }
    }
}

#[test]
fn ordinary_replacement_attrs_do_not_create_a_blank_eof_stage() {
    for genuine in [false, true] {
        let (mut s, sid, baseline) = fixture("AX", genuine, false);
        edit(&mut s, "find.change", json!({"story": sid.0, "find": "X", "change": "Z", "attrs": {"size": 30}}));
        assert_eq!(story(&s, sid).text, "AZ");
        assert_eq!(story(&s, sid).paras, vec![baseline.clone()]);
        size(&mut s, sid, 1, 2, 30.0);
        size(&mut s, sid, 2, 2, 30.0);
        let mut back = reopen(&mut s);
        type_and_clear(&mut back, sid, &baseline, 30.0, if genuine { 24.0 } else { 12.0 });
    }
}

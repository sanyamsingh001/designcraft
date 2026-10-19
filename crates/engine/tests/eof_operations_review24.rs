//! Bounded public-command regressions for Change Case and table creation at a blank outer EOF.

use designcraft_doc::{Leading, ParaFormat, Story, StoryId, TABLE_ANCHOR};
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

fn edit(s: &mut Session, command: &str, params: Value) -> Value {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let result = run(s, command, params.clone());
    let after = document(s);
    let selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    assert_eq!(&s.journal[journal..], &[(command.to_string(), params)], "one public command owns the edit");
    s.doc().unwrap().doc.check().unwrap();
    run(s, "edit.undo", json!({}));
    assert_eq!(document(s), before, "Undo restores the full document, including EOF baseline and stage");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    run(s, "edit.redo", json!({}));
    assert_eq!(document(s), after, "Redo restores the full document");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn reopen(s: &mut Session) -> Session {
    let expected = document(s);
    let saved = run(s, "file.serialize", json!({}));
    let bytes = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"));
    let mut back = Session::new();
    run(&mut back, "file.openBytes", json!({"base64": saved["base64"]}));
    assert_eq!(document(&back), expected, "native ZIP reopen preserves the full document");
    back.doc().unwrap().doc.check().unwrap();
    back
}

// The 24pt fixture has genuine direct paragraph defaults, independently of the EOF stage.
fn fixture(text: &str, inherited: bool, blank_eof: bool) -> (Session, StoryId, ParaFormat) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"pages": 1}));
    let frame = run(&mut s, "frame.create", json!({"rect": [40, 40, 440, 340], "content": "text", "text": text}));
    let sid = StoryId(frame["story"].as_u64().unwrap());
    select(&mut s, sid, 0, text.len());
    if inherited {
        run(&mut s, "style.paragraph.create", json!({"name": "Review24 baseline", "chars": {"size": 24}}));
        run(&mut s, "style.paragraph.apply", json!({"name": "Review24 baseline"}));
        run(&mut s, "style.breakLink", json!({"kind": "paragraph"}));
    }
    let mut baseline = story(&s, sid).paras.last().unwrap().clone();
    assert!(baseline.eof_inherited_chars.is_none());
    assert_eq!(baseline.chars.size, inherited.then_some(24.0));
    run(
        &mut s,
        "style.character.create",
        json!({"name": "Review24 parent", "chars": {"underline": true, "leading": {"kind": "points", "value": 32}}}),
    );
    run(&mut s, "style.character.create", json!({"name": "Review24 child", "basedOn": "Review24 parent"}));
    run(&mut s, "style.character.apply", json!({"name": "Review24 child"}));
    if blank_eof {
        select(&mut s, sid, text.len(), text.len());
        run(&mut s, "text.insert", json!({"text": "\n", "raw": true}));
        run(&mut s, "type.para", json!({"attrs": {"spaceBefore": 7}}));
        baseline.para = story(&s, sid).paras.last().unwrap().para.clone();
        assert_eq!(baseline.para.space_before, Some(7.0));
        run(&mut s, "type.char", json!({"attrs": {"size": 30}}));
        let eof = story(&s, sid).paras.last().unwrap();
        assert_eq!(eof.chars.size, Some(30.0));
        assert_eq!(eof.eof_inherited_chars.as_ref(), inherited.then_some(&baseline.chars));
        assert_eq!(story(&s, sid).char_format_at(text.len()).over.size, None, "the LF itself was not restyled");
    }
    (s, sid, baseline)
}

fn metrics(s: &mut Session, sid: StoryId, a: usize, b: usize, size: f64, leading: f64) -> Value {
    select(s, sid, a, b);
    let attrs = run(s, "type.selectionAttrs", json!({}));
    assert_eq!(attrs["chars"]["size"], json!(size));
    assert_eq!(attrs["chars"]["leading"], serde_json::to_value(Leading::Points(leading)).unwrap());
    assert_eq!(attrs["chars"]["underline"], true);
    assert_eq!(attrs["characterStyle"], "Review24 child");
    attrs
}

fn type_and_clear(s: &mut Session, sid: StoryId, baseline: &ParaFormat, inherited: bool, leading: f64) {
    run(s, "changes.track", json!({"on": false}));
    let before = story(s, sid).clone();
    let at = before.len();
    metrics(s, sid, at, at, 30.0, leading);
    edit(s, "text.insert", json!({"text": "é", "raw": true}));
    metrics(s, sid, at, at + "é".len(), 30.0, leading);
    edit(s, "type.char", json!({"attrs": {"size": null}}));
    let attrs = metrics(s, sid, at, at + "é".len(), if inherited { 24.0 } else { 12.0 }, leading);
    assert_eq!(attrs["charOverrides"], 0);
    assert_eq!(story(s, sid).paras.last(), Some(baseline));
    for (offset, _) in before.text.char_indices() {
        assert_eq!(story(s, sid).format_after(offset), before.format_after(offset), "prior byte formats survive typing and clear");
    }
    reopen(s);
}

#[test]
fn change_case_retains_full_blank_eof_and_clearable_baseline() {
    for inherited in [false, true] {
        let (mut s, sid, mut baseline) = fixture("a", inherited, true);
        if !inherited {
            run(&mut s, "style.paragraph.create", json!({"name": "Review24 EOF live", "chars": {"size": 12}}));
            run(&mut s, "style.paragraph.apply", json!({"name": "Review24 EOF live"}));
            baseline.style = "Review24 EOF live".to_string();
        }
        let eof = story(&s, sid).paras.last().unwrap().clone();
        let lf = story(&s, sid).format_after(1).clone();
        select(&mut s, sid, 0, 2);
        edit(&mut s, "type.changeCase", json!({"case": "upper"}));
        assert_eq!(story(&s, sid).text, "A\n");
        assert_eq!(story(&s, sid).paras.last(), Some(&eof));
        assert_eq!(story(&s, sid).format_after(1), &lf);
        let mut back = reopen(&mut s);
        type_and_clear(&mut back, sid, &baseline, inherited, 32.0);
        if !inherited {
            edit(&mut back, "style.paragraph.edit", json!({"name": "Review24 EOF live", "chars": {"size": 19}}));
            metrics(&mut back, sid, 2, 4, 19.0, 32.0);
            metrics(&mut back, sid, 0, 1, 12.0, 32.0);
            reopen(&mut back);
        }
    }
}

#[test]
fn change_case_unicode_and_partial_selection_keep_live_style_without_new_tracking() {
    for partial in [false, true] {
        let (mut s, sid, baseline) = fixture("aßı", true, true);
        let before = story(&s, sid).clone();
        run(&mut s, "changes.track", json!({"on": true}));
        // Reverse selection; ß expands to SS while dotless ı contracts in UTF-8.
        select(&mut s, sid, before.len() - usize::from(partial), 0);
        edit(&mut s, "type.changeCase", json!({"case": "upper"}));
        assert_eq!(story(&s, sid).text, "ASSI\n");
        assert_eq!(story(&s, sid).paras.last(), before.paras.last());
        let selection = s.doc().unwrap().selection.text.unwrap();
        assert_eq!((selection.anchor, selection.focus), (5 - usize::from(partial), 0));
        for (offset, _) in story(&s, sid).text.char_indices() {
            assert_eq!(story(&s, sid).format_after(offset), before.format_after(0));
        }
        assert_eq!(run(&mut s, "changes.list", json!({})), json!([]), "case mapping does not acquire new tracked-edit semantics");
        let mut back = reopen(&mut s);
        edit(&mut back, "style.character.edit", json!({"name": "Review24 parent", "chars": {"leading": {"kind": "points", "value": 40}}}));
        type_and_clear(&mut back, sid, &baseline, true, 40.0);
    }
}

#[test]
fn table_creation_consumes_anchor_stage_but_keeps_true_eof_and_cell_baseline() {
    for command in ["table.insert", "table.convertFromText"] {
        for inherited in [false, true] {
            let (mut s, sid, baseline) = fixture("A", inherited, true);
            let eof = story(&s, sid).paras.last().unwrap().clone();
            let prior = story(&s, sid).clone();
            let result = edit(&mut s, command, json!({"rows": 1, "cols": 1}));
            let tid = result["table"].as_u64().unwrap();
            let st = story(&s, sid);
            assert_eq!(st.text, format!("A\n{TABLE_ANCHOR}\n"));
            let mut anchor_para = baseline.clone();
            anchor_para.table = Some(tid);
            assert_eq!(st.paras[1], anchor_para, "populated anchor retains genuine defaults, not staged defaults");
            assert_eq!(st.paras.last(), Some(&eof), "true EOF retains the exact stage and baseline");
            for (offset, _) in prior.text.char_indices() {
                assert_eq!(st.format_after(offset), prior.format_after(offset));
            }
            for offset in [2, 2 + TABLE_ANCHOR.len_utf8()] {
                assert_eq!(st.format_after(offset).style, "Review24 child");
                assert_eq!(st.format_after(offset).over, eof.chars, "anchor and generated LF own the stage as a run override");
            }
            let cell = &st.tables[&tid].cells[0].text;
            assert_eq!(cell.paras, vec![baseline.clone()]);
            assert_eq!(cell.char_format_at(0), prior.char_format_at(prior.len()), "cell typing policy remains independent of the outer stage");
            edit(&mut s, "table.setCell", json!({"story": sid.0, "table": tid, "row": 0, "col": 0, "text": "Z"}));
            let mut back = reopen(&mut s);
            select(&mut back, sid, 2, 2);
            edit(&mut back, "table.convertToText", json!({}));
            assert_eq!(story(&back, sid).text, "A\nZ\n");
            assert!(story(&back, sid).tables.is_empty());
            assert_eq!(story(&back, sid).paras[1], baseline);
            assert_eq!(story(&back, sid).paras.last(), Some(&eof));
            metrics(&mut back, sid, 2, 3, 30.0, 32.0);
            edit(&mut back, "type.char", json!({"attrs": {"size": null}}));
            let attrs = metrics(&mut back, sid, 2, 3, if inherited { 24.0 } else { 12.0 }, 32.0);
            assert_eq!(attrs["charOverrides"], 0);
            assert_eq!(story(&back, sid).paras.last(), Some(&eof));
            let mut reopened = reopen(&mut back);
            edit(&mut reopened, "style.character.edit", json!({"name": "Review24 parent", "chars": {"leading": {"kind": "points", "value": 40}}}));
            metrics(&mut reopened, sid, 2, 3, if inherited { 24.0 } else { 12.0 }, 40.0);
            type_and_clear(&mut reopened, sid, &baseline, inherited, 40.0);
        }
    }
}

#[test]
fn ordinary_selected_tail_is_not_mistaken_for_an_existing_eof_stage() {
    for command in ["table.insert", "table.convertFromText"] {
        let (mut s, sid, baseline) = fixture("A\nX", true, false);
        let cf = story(&s, sid).format_after(2).clone();
        select(&mut s, sid, 2, 3);
        let result = edit(&mut s, command, json!({"rows": 1, "cols": 1}));
        let tid = result["table"].as_u64().unwrap();
        let st = story(&s, sid);
        let mut anchor_para = baseline.clone();
        anchor_para.table = Some(tid);
        assert_eq!(st.paras[1], anchor_para);
        assert_eq!(st.paras.last(), Some(&baseline));
        assert_eq!(st.format_after(2), &cf);
        assert_eq!(st.tables[&tid].cells[0].text.paras[0], baseline);
        edit(&mut s, "table.setCell", json!({"story": sid.0, "table": tid, "text": "Z"}));
        select(&mut s, sid, 2, 2);
        edit(&mut s, "table.convertToText", json!({}));
        assert_eq!(story(&s, sid).text, "A\nZ\n");
        let attrs = metrics(&mut s, sid, 2, 3, 24.0, 32.0);
        assert_eq!(attrs["charOverrides"], 0);
        reopen(&mut s);
    }
}

#[test]
fn change_case_keeps_ordinary_and_child_story_policy() {
    let (mut s, sid, baseline) = fixture("aßı", true, false);
    select(&mut s, sid, 0, "aßı".len());
    edit(&mut s, "type.changeCase", json!({"case": "upper"}));
    assert_eq!(story(&s, sid).text, "ASSI");
    assert_eq!(story(&s, sid).paras, vec![baseline]);
    reopen(&mut s);

    for child in ["table", "footnote"] {
        let (mut s, sid, _) = fixture("A", true, false);
        select(&mut s, sid, 1, 1);
        if child == "table" {
            run(&mut s, "table.insert", json!({"rows": 1, "cols": 1}));
        } else {
            run(&mut s, "footnote.insert", json!({"text": ""}));
        }
        let cell = s.doc().unwrap().selection.text.unwrap().cell;
        assert!(cell.is_some());
        run(&mut s, "text.insert", json!({"text": "aßı\n", "raw": true}));
        let before = s.doc().unwrap().doc.text_story(sid, cell).unwrap().clone();
        assert!(before.paras.iter().all(|pf| pf.eof_inherited_chars.is_none()));
        let mut unchanged = (*s.doc().unwrap().doc).clone();
        // Mutable child access bumps the parent revision even when formatting is unchanged.
        let outer = unchanged.story_mut(sid).unwrap();
        outer.rev = outer.rev.wrapping_add(1);
        run(&mut s, "type.char", json!({"attrs": {"size": 30}}));
        assert_eq!(document(&s), serde_json::to_value(&unchanged).unwrap(), "a nonempty child EOF does not adopt outer EOF staging");
        run(&mut s, "text.move", json!({"dir": "storyStart", "extend": true}));
        edit(&mut s, "type.changeCase", json!({"case": "upper"}));
        let after = s.doc().unwrap().doc.text_story(sid, cell).unwrap();
        assert_eq!(after.text, "ASSI\n");
        assert_eq!(after.paras, before.paras);
        for (offset, _) in after.text.char_indices() {
            assert_eq!(after.format_after(offset), before.format_after(0));
        }
        assert_eq!(s.doc().unwrap().selection.text.unwrap().cell, cell);
        reopen(&mut s);
    }
}

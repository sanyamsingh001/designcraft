//! Public-command regressions for condition references in blank-EOF typing state.

use designcraft_doc::{CharAttrs, Story, StoryId};
use designcraft_engine::Session;
use serde_json::{Value, json};

const PROBE: &str = "QzxProbe26";
const SENTINEL: &str = "Unaffected26Sentinel";

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

fn edit(s: &mut Session, command: &str, params: Value) {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    run(s, command, params);
    let after = document(s);
    let selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
    run(s, "edit.undo", json!({}));
    assert_eq!(document(s), before, "Undo restores every document field");
    // Session history intentionally retains the post-edit selection on Undo, too.
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    run(s, "edit.redo", json!({}));
    assert_eq!(document(s), after, "Redo restores every document field");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
}

fn reopen(s: &mut Session) -> Session {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let saved = run(s, "file.serialize", json!({}));
    let bytes = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"));
    assert_eq!(document(s), before, "serialization is read-only");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    let mut back = Session::new();
    run(&mut back, "file.openBytes", json!({"base64": saved["base64"]}));
    assert_eq!(document(&back), before, "native reopen retains both EOF slots and all other document fields");
    back.doc().unwrap().doc.check().unwrap();
    back
}

fn names(primary: Option<&str>, mixed: bool) -> Vec<String> {
    primary.into_iter().chain(mixed.then_some("Keep")).map(str::to_string).collect()
}

fn assert_conditions(attrs: &CharAttrs, expected: &[String]) {
    assert_eq!(attrs.conditions.as_deref().unwrap_or_default(), expected);
    if expected.is_empty() {
        assert!(attrs.conditions.is_none(), "removing the final condition clears the sparse override");
    }
}

// Compare every story field except condition references and the permitted revision counter.
// This deliberately makes no assumption about the number of revision increments.
fn without_condition_references(st: &Story) -> Story {
    let mut clean = st.clone();
    clean.rev = 0;
    for run in &mut clean.chars {
        run.format.over.conditions = None;
    }
    for pf in &mut clean.paras {
        pf.chars.conditions = None;
        if let Some(base) = &mut pf.eof_inherited_chars {
            base.conditions = None;
        }
    }
    clean
}

fn assert_visibility(s: &mut Session, visible: bool) {
    let before = document(s);
    let result = run(s, "file.exportHtml", json!({}));
    let html = result["text"].as_str().unwrap();
    assert!(html.contains(SENTINEL), "independent visible text proves the export contains page content");
    assert_eq!(html.contains(PROBE), visible, "the unique typed probe must have the expected exported visibility");
    assert_eq!(document(s), before, "HTML export must not change document state");
}

fn assert_glyph_visibility(s: &Session, sid: StoryId, at: usize, visible: bool) {
    let composed = s.cache.get(&s.doc().unwrap().doc, sid, None);
    let glyphs: Vec<_> = composed
        .frames
        .iter()
        .flat_map(|frame| &frame.lines)
        .flat_map(|line| &line.glyphs)
        .filter(|glyph| (at..at + PROBE.len()).contains(&glyph.byte))
        .collect();
    assert!(!glyphs.is_empty(), "the typed probe must be composed, not overset");
    assert!(glyphs.iter().any(|glyph| glyph.byte == at));
    assert!(glyphs.iter().all(|glyph| glyph.visible == visible), "actual glyph visibility must match the resolved paragraph/run conditions");
}

fn fixture(staged: bool, genuine: bool, mixed: bool) -> (Session, StoryId, StoryId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"pages": 1}));
    let frame = run(&mut s, "frame.create", json!({"rect": [40, 40, 480, 340], "content": "text", "text": "A"}));
    let sid = StoryId(frame["story"].as_u64().unwrap());
    let other = run(&mut s, "frame.create", json!({"rect": [40, 400, 480, 500], "content": "text", "text": SENTINEL}));
    let other_sid = StoryId(other["story"].as_u64().unwrap());
    run(&mut s, "condition.new", json!({"name": "Old"}));
    run(&mut s, "condition.new", json!({"name": "Keep", "visible": false, "color": [11, 22, 33]}));
    run(&mut s, "condition.new", json!({"name": "Spare", "visible": true, "color": [44, 55, 66]}));
    select(&mut s, sid, 0, 1);
    if genuine {
        // Establish actual paragraph defaults through public style commands, not fixture mutation.
        run(
            &mut s,
            "style.paragraph.create",
            json!({"name": "Review26 baseline", "para": {"spaceBefore": 7},
                "chars": {"size": 24, "tracking": 17, "conditions": names(Some("Old"), mixed)}}),
        );
        run(&mut s, "style.paragraph.apply", json!({"name": "Review26 baseline"}));
        run(&mut s, "style.breakLink", json!({"kind": "paragraph"}));
        let pf = &story(&s, sid).paras[0];
        assert_eq!(pf.chars.size, Some(24.0));
        assert_eq!(pf.para.space_before, Some(7.0));
        assert_conditions(&pf.chars, &names(Some("Old"), mixed));
    }
    run(&mut s, "condition.apply", json!({"name": "Old", "only": true}));
    if mixed {
        run(&mut s, "condition.apply", json!({"name": "Keep"}));
    }
    run(&mut s, "type.char", json!({"attrs": {"size": 30, "underline": true}}));
    select(&mut s, sid, 1, 1);
    if staged {
        run(&mut s, "text.insert", json!({"text": "\n", "raw": true}));
        let pf = story(&s, sid).paras.last().unwrap();
        assert_conditions(&pf.chars, &names(Some("Old"), mixed));
        assert_eq!(pf.chars.size, Some(30.0));
        assert_eq!(pf.eof_inherited_chars.is_some(), genuine);
        if genuine {
            let base = pf.eof_inherited_chars.as_ref().unwrap();
            assert_conditions(base, &names(Some("Old"), mixed));
            assert_eq!(base.size, Some(24.0));
        }
    } else {
        assert!(story(&s, sid).paras[0].eof_inherited_chars.is_none());
    }
    (reopen(&mut s), sid, other_sid)
}

fn exercise(staged: bool, genuine: bool, mixed: bool, rename: bool) {
    let (mut s, sid, other_sid) = fixture(staged, genuine, mixed);
    let before = story(&s, sid).clone();
    let unrelated = story(&s, other_sid).clone();
    let styles = serde_json::to_value(&s.doc().unwrap().doc.styles).unwrap();
    let unaffected_conditions: Vec<_> = s.doc().unwrap().doc.conditions.iter().filter(|c| c.name != "Old").cloned().collect();
    let at = before.len();
    select(&mut s, sid, at, at);
    if rename {
        edit(&mut s, "condition.options", json!({"name": "Old", "to": "New", "visible": false}));
    } else {
        edit(&mut s, "condition.delete", json!({"name": "Old"}));
    }
    let expected = names(rename.then_some("New"), mixed);
    let after = story(&s, sid);
    assert_eq!(without_condition_references(after), without_condition_references(&before), "all unrelated text and formatting survive");
    for run in &after.chars {
        assert_conditions(&run.format.over, &expected);
    }
    for (i, pf) in after.paras.iter().enumerate() {
        if genuine || (staged && i + 1 == after.paras.len()) {
            assert_conditions(&pf.chars, &expected);
        } else {
            assert_conditions(&pf.chars, &[]);
        }
        if let Some(base) = &pf.eof_inherited_chars {
            assert_conditions(base, &expected);
        }
    }
    assert_eq!(without_condition_references(story(&s, other_sid)), without_condition_references(&unrelated));
    assert!(story(&s, other_sid).chars.iter().all(|r| r.format.over.conditions.is_none()));
    assert!(story(&s, other_sid).paras.iter().all(|pf| pf.chars.conditions.is_none() && pf.eof_inherited_chars.is_none()));
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc.styles).unwrap(), styles, "style definitions are outside this reference rewrite");
    let still_unaffected: Vec<_> = s.doc().unwrap().doc.conditions.iter().filter(|c| c.name == "Keep" || c.name == "Spare").cloned().collect();
    assert_eq!(still_unaffected, unaffected_conditions);
    let conditions = run(&mut s, "condition.list", json!({}));
    assert!(!conditions.as_array().unwrap().iter().any(|c| c["name"] == "Old"));
    if !rename {
        // Reusing the removed name must not silently reattach the old stage or baseline.
        edit(&mut s, "condition.new", json!({"name": "Old", "visible": false}));
    }
    s = reopen(&mut s);
    let pre_typing = story(&s, sid).clone();
    let restored_para = pre_typing.paras.last().unwrap().clone();
    select(&mut s, sid, at, at);
    edit(&mut s, "text.insert", json!({"text": PROBE, "raw": true}));
    assert_eq!(story(&s, sid).text, format!("{}{PROBE}", before.text));
    assert_conditions(&story(&s, sid).format_after(at).over, &expected);
    let selection = s.doc().unwrap().selection.text.unwrap();
    assert_eq!((selection.anchor, selection.focus), (at + PROBE.len(), at + PROBE.len()));
    for (offset, _) in pre_typing.text.char_indices() {
        assert_eq!(story(&s, sid).format_after(offset), pre_typing.format_after(offset), "typing preserves complete prefix formats");
    }
    let populated = story(&s, sid).paras.last().unwrap();
    assert!(populated.eof_inherited_chars.is_none());
    let baseline = if staged { restored_para.eof_inherited_chars.unwrap_or_default() } else { restored_para.chars };
    assert_eq!(populated.chars, baseline, "typing restores the genuine paragraph defaults");
    // Keep starts hidden; a retained visible condition would incorrectly mask the rename bug.
    assert_visibility(&mut s, !rename && !mixed);
    s = reopen(&mut s);
    assert_visibility(&mut s, !rename && !mixed);
    if mixed {
        edit(&mut s, "condition.options", json!({"name": "Keep", "visible": true}));
        assert_visibility(&mut s, true);
        edit(&mut s, "condition.options", json!({"name": "Keep", "visible": false}));
        assert_visibility(&mut s, false);
    }
    select(&mut s, sid, at, at + PROBE.len());
    let attrs = run(&mut s, "type.selectionAttrs", json!({}));
    assert_eq!(attrs["chars"]["size"], json!(30.0));
    assert_eq!(attrs["chars"]["underline"], true);
    // Expose the saved paragraph layer, so fixing only pf.chars cannot pass this test.
    edit(&mut s, "type.char", json!({"attrs": {"size": null, "conditions": null}}));
    let attrs = run(&mut s, "type.selectionAttrs", json!({}));
    assert_eq!(attrs["chars"]["size"], json!(if genuine { 24.0 } else { 12.0 }));
    assert_eq!(attrs["chars"]["underline"], true, "clearing conditions/size leaves unrelated direct attributes");
    assert_eq!(attrs["chars"]["conditions"], json!(if genuine { expected } else { Vec::<String>::new() }));
    // HTML currently filters run overrides; actual glyphs resolve the paragraph layer too.
    assert_glyph_visibility(&s, sid, at, !genuine || (!rename && !mixed));
    s = reopen(&mut s);
    assert_glyph_visibility(&s, sid, at, !genuine || (!rename && !mixed));
}

#[test]
fn staged_condition_rename_updates_typing_and_genuine_baseline() {
    for genuine in [false, true] {
        for mixed in [false, true] {
            exercise(true, genuine, mixed, true);
        }
    }
}

#[test]
fn staged_condition_delete_and_recreate_cannot_resurrect_references() {
    for genuine in [false, true] {
        for mixed in [false, true] {
            exercise(true, genuine, mixed, false);
        }
    }
}

#[test]
fn ordinary_condition_rename_preserves_formatting_and_history() {
    for genuine in [false, true] {
        for mixed in [false, true] {
            exercise(false, genuine, mixed, true);
        }
    }
}

#[test]
fn ordinary_condition_delete_and_recreate_preserves_formatting_and_history() {
    for genuine in [false, true] {
        for mixed in [false, true] {
            exercise(false, genuine, mixed, false);
        }
    }
}

#[test]
fn r32_deleting_only_eof_condition_keeps_empty_stage_and_live_style() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({}));
    let frame = run(&mut s, "frame.create", json!({"rect": [40, 40, 440, 300], "content": "text", "text": "A\n"}));
    let sid = StoryId(frame["story"].as_u64().unwrap());
    run(&mut s, "condition.new", json!({"name": "Old"}));
    run(&mut s, "style.character.create", json!({"name": "R32 live", "chars": {"size": 18, "leading": {"kind": "points", "value": 22}}}));
    select(&mut s, sid, 0, 2);
    run(&mut s, "style.character.apply", json!({"name": "R32 live"}));
    run(&mut s, "type.char", json!({"attrs": {"size": 30, "leading": {"kind": "points", "value": 36}}}));
    let lf = story(&s, sid).format_after(1).clone();
    select(&mut s, sid, 2, 2);
    // Keep this a condition-only stage by explicitly clearing the LF metric overrides.
    run(&mut s, "type.char", json!({"attrs": {"conditions": ["Old"], "size": null, "leading": null}}));
    let pf = &story(&s, sid).paras[1];
    assert_eq!(pf.chars, CharAttrs { conditions: Some(vec!["Old".into()]), ..Default::default() });
    assert!(pf.eof_inherited_chars.is_none());
    let before = run(&mut s, "type.selectionAttrs", json!({}));
    assert_eq!(before["chars"]["size"], json!(18.0));
    assert_eq!(before["chars"]["leading"], json!({"kind": "points", "value": 22.0}));

    edit(&mut s, "condition.delete", json!({"name": "Old"}));
    let pf = &story(&s, sid).paras[1];
    assert!(pf.chars.is_empty());
    assert_eq!(pf.eof_inherited_chars, Some(CharAttrs::default()));
    assert_eq!(story(&s, sid).format_after(1), &lf);
    let cleared = run(&mut s, "type.selectionAttrs", json!({}));
    assert_eq!(cleared["chars"]["size"], before["chars"]["size"]);
    assert_eq!(cleared["chars"]["leading"], before["chars"]["leading"]);
    assert_eq!(cleared["chars"]["conditions"], json!([]));
    assert_eq!(cleared["characterStyle"], "R32 live");

    edit(&mut s, "style.character.edit", json!({"name": "R32 live", "chars": {"size": 20, "leading": {"kind": "points", "value": 26}}}));
    let live = run(&mut s, "type.selectionAttrs", json!({}));
    assert_eq!(live["chars"]["size"], json!(20.0));
    assert_eq!(live["chars"]["leading"], json!({"kind": "points", "value": 26.0}));
    edit(&mut s, "text.insert", json!({"text": "Z", "raw": true}));
    let typed = run(&mut s, "type.selectionAttrs", json!({}));
    assert_eq!(typed["chars"], live["chars"]);
    assert_eq!(typed["characterStyle"], "R32 live");
    assert_eq!(story(&s, sid).text, "A\nZ");
    assert_eq!(story(&s, sid).format_after(1), &lf, "the real LF keeps its direct metrics and named style");
    assert!(story(&s, sid).format_after(2).over.is_empty());
    assert!(story(&s, sid).paras[1].eof_inherited_chars.is_none());
}

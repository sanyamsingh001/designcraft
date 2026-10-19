//! C5 EOF-CONSUMER-05/06: public-command marker regressions and boundary controls.

use designcraft_doc::{ANCHOR_MARK, CharFormat, INDEX_MARK, ParaFormat, Story, StoryId, XREF_MARK};
use serde_json::{Value, json};

use crate::Session;

fn document(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": anchor, "focus": focus})).unwrap();
}

fn frame(s: &mut Session, text: &str, y: f64) -> StoryId {
    let result = s.execute("frame.create", &json!({"rect": [40.0, y, 440.0, y + 180.0], "content": "text", "text": text})).unwrap();
    StoryId(result["story"].as_u64().unwrap())
}

fn query_size(s: &mut Session, sid: StoryId, start: usize, end: usize, size: f64) -> Value {
    select(s, sid, start, end);
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["size"], json!(size));
    attrs
}

fn edit(s: &mut Session, command: &str, params: &Value) -> Value {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let result = s.execute(command, params).unwrap();
    let after = document(s);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "{command} owns exactly one history entry");
    s.doc().unwrap().doc.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(document(s), before, "{command}: Undo restores both stories, runs, stages and marker metadata");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(document(s), after, "{command}: Redo restores the complete document");
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn native_copies(s: &mut Session) -> [Session; 2] {
    let expected = document(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let zip = crate::cmd::file::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"));
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    let mut copies = [Session::new(), Session::new()];
    for (back, bytes) in copies.iter_mut().zip([zip, legacy]) {
        back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
        assert_eq!(document(back), expected, "both native readers preserve the full document");
        back.doc().unwrap().doc.check().unwrap();
    }
    copies
}

fn staged_story(s: &mut Session, text: &str, y: f64, inherited: bool, size: f64) -> (StoryId, ParaFormat) {
    let sid = frame(s, text, y);
    select(s, sid, 0, text.len());
    if inherited {
        // Flatten a genuine paragraph baseline before staging. Nondefault leading keeps
        // inherited attrs nonempty even though 12 pt equals the default character size.
        let name = format!("R22 baseline {text}");
        s.execute("style.paragraph.create", &json!({"name": name, "chars": {"size": 12, "leading": {"kind": "points", "value": 18}}})).unwrap();
        s.execute("style.paragraph.apply", &json!({"name": name})).unwrap();
        s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    }
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
    assert_eq!(s.doc().unwrap().doc.styles.resolve_para(&baseline).1.size, 12.0);
    assert!(baseline.eof_inherited_chars.is_none());
    assert_eq!(!baseline.chars.is_empty(), inherited);
    s.execute("type.char", &json!({"attrs": {"size": 15}})).unwrap();
    select(s, sid, text.len(), text.len());
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    let end = text.len() + 1;
    select(s, sid, end, end);
    s.execute("type.char", &json!({"attrs": {"size": size}})).unwrap();
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, format!("{text}\n"));
    assert_eq!(story.format_after(end - 1).over.size, Some(15.0), "actual LF differs from the stage");
    assert_eq!(story.paras[0], baseline);
    assert_eq!(story.paras[1].chars.size, Some(size));
    assert_eq!(story.paras[1].eof_inherited_chars.as_ref(), inherited.then_some(&baseline.chars));
    query_size(s, sid, end, end, size);
    (sid, baseline)
}

fn assert_marker(s: &Session, sid: StoryId, before: &Story, baseline: &ParaFormat, marker: char) {
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    let at = before.len();
    let expected = CharFormat { style: before.char_format_at(at).style.clone(), over: before.paras.last().unwrap().chars.clone() };
    assert_eq!(story.text, format!("{}{marker}", before.text));
    assert_eq!(story.format_after(at), &expected, "the inserted marker owns the EOF stage, not the 15-point LF format");
    assert_eq!(story.paras.last(), Some(baseline), "inserting bytes restores the genuine 12-point paragraph baseline");
    assert!(story.paras.last().unwrap().eof_inherited_chars.is_none());
    assert_eq!(story.paras[0], before.paras[0]);
    for (offset, _) in before.text.char_indices() {
        assert_eq!(story.format_after(offset), before.format_after(offset), "existing text and the LF keep their complete formats");
    }
}

fn type_and_clear(s: &mut Session, sid: StoryId, before: &Story, baseline: &ParaFormat, marker: char, size: f64) {
    assert_marker(s, sid, before, baseline, marker);
    let at = before.len() + marker.len_utf8();
    query_size(s, sid, at, at, size);
    edit(s, "text.insert", &json!({"text": "é", "raw": true}));
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, format!("{}{marker}é", before.text));
    assert_eq!(query_size(s, sid, at, at + "é".len(), size)["charOverrides"], 1);
    edit(s, "type.char", &json!({"attrs": {"size": null}}));
    assert_eq!(query_size(s, sid, at, at + "é".len(), 12.0)["charOverrides"], 0);
    // Clear the marker itself as well: the stage must not survive as paragraph defaults.
    query_size(s, sid, before.len(), at, size);
    edit(s, "type.char", &json!({"attrs": {"size": null}}));
    assert_eq!(query_size(s, sid, before.len(), at, 12.0)["charOverrides"], 0);
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.paras.last(), Some(baseline));
    assert!(story.format_after(at).over.is_empty());
    assert!(story.format_after(before.len()).over.is_empty());
    assert_eq!(story.format_after(before.len() - 1), before.format_after(before.len() - 1));
    for mut back in native_copies(s) {
        assert_eq!(query_size(&mut back, sid, before.len(), at + "é".len(), 12.0)["charOverrides"], 0);
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().paras.last(), Some(baseline));
    }
}

#[test]
fn r22_index_eof_marker_consumes_stage_with_native_history_and_clear() {
    for inherited in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (sid, baseline) = staged_story(&mut s, "A", 40.0, inherited, 20.0);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            select(&mut back, sid, 2, 2);
            let result = edit(&mut back, "index.addReference", &json!({"topics": ["Type", "Markers"], "sort": ["type", "markers"]}));
            assert_eq!(result["pos"], 2);
            let caret = back.doc().unwrap().selection.text.unwrap();
            let end = 2 + INDEX_MARK.len_utf8();
            assert_eq!((caret.story, caret.anchor, caret.focus, caret.cell), (sid, end, end, None));
            assert_marker(&back, sid, &before, &baseline, INDEX_MARK);
            let story = back.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(story.index_refs.len(), 1);
            assert_eq!(story.index_refs[0].topics, vec!["Type", "Markers"]);
            assert_eq!(story.index_refs[0].sort, vec!["type", "markers"]);
            assert_eq!(story.index_refs[0].range, designcraft_doc::index::IndexRange::CurrentPage);
            for mut reopened in native_copies(&mut back) {
                type_and_clear(&mut reopened, sid, &before, &baseline, INDEX_MARK, 20.0);
            }
            type_and_clear(&mut back, sid, &before, &baseline, INDEX_MARK, 20.0);
        }
    }
}

#[test]
fn r22_implicit_target_eof_anchor_consumes_its_own_stage_with_native_history_and_clear() {
    for inherited in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let (target, target_base) = staged_story(&mut s, "T", 40.0, inherited, 20.0);
        let (source, source_base) = staged_story(&mut s, "S", 260.0, false, 28.0);
        let target_before = s.doc().unwrap().doc.story(target).unwrap().clone();
        let source_before = s.doc().unwrap().doc.story(source).unwrap().clone();
        assert_ne!(target, source);
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            select(&mut back, source, 2, 2);
            let result = edit(&mut back, "xref.insert", &json!({"story": target.0, "para": 1, "format": "Page Number"}));
            let anchor = result["target"].as_u64().unwrap();
            assert_eq!(result["pos"], 2, "creating an anchor in another story cannot shift the source caret");
            let caret = back.doc().unwrap().selection.text.unwrap();
            let end = 2 + XREF_MARK.len_utf8();
            assert_eq!((caret.story, caret.anchor, caret.focus, caret.cell), (source, end, end, None));
            assert_marker(&back, target, &target_before, &target_base, ANCHOR_MARK);
            assert_marker(&back, source, &source_before, &source_base, XREF_MARK);
            let d = &back.doc().unwrap().doc;
            assert_eq!(d.find_anchor(anchor), Some((target, 2)));
            assert_eq!(d.story(target).unwrap().anchors.len(), 1);
            assert_eq!(d.anchor(anchor).unwrap().name, "", "retain paragraph_anchor's blank-paragraph name");
            let refs = &d.story(source).unwrap().xrefs;
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target, anchor);
            assert_eq!(refs[0].format, "Page Number");
            query_size(&mut back, source, end, end, 28.0);
            let source_after = back.doc().unwrap().doc.story(source).unwrap().clone();
            for mut reopened in native_copies(&mut back) {
                type_and_clear(&mut reopened, target, &target_before, &target_base, ANCHOR_MARK, 20.0);
                assert_eq!(reopened.doc().unwrap().doc.story(source).unwrap(), &source_after);
            }
            type_and_clear(&mut back, target, &target_before, &target_base, ANCHOR_MARK, 20.0);
            assert_eq!(back.doc().unwrap().doc.story(source).unwrap(), &source_after);
        }
    }
}

#[test]
fn r22_index_before_reversed_unicode_selection_preserves_selection_text_and_unrelated_eof_stage() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (sid, _) = staged_story(&mut s, "éZ", 40.0, true, 20.0);
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    select(&mut s, sid, "éZ".len(), 0);
    let result = edit(&mut s, "index.addReference", &json!({}));
    assert_eq!(result["pos"], 0);
    let shift = INDEX_MARK.len_utf8();
    let caret = s.doc().unwrap().selection.text.unwrap();
    assert_eq!((caret.story, caret.anchor, caret.focus), (sid, "éZ".len() + shift, shift));
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, format!("{INDEX_MARK}éZ\n"));
    assert_eq!(story.slice(caret.range()), "éZ");
    assert_eq!(story.index_refs[0].topics, vec!["éZ"]);
    assert_eq!(story.format_after(0), before.char_format_at(0));
    assert_eq!(story.paras, before.paras, "inserting before existing text must not consume a separate EOF stage");
    for (offset, _) in before.text.char_indices() {
        assert_eq!(story.format_after(offset + shift), before.format_after(offset));
    }
    query_size(&mut s, sid, 0, shift, 15.0);
    let end = before.len() + shift;
    query_size(&mut s, sid, end, end, 20.0);
}

#[test]
fn r22_existing_target_anchor_reuse_preserves_target_and_its_separate_eof_stage() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (target, _) = staged_story(&mut s, "Target", 40.0, true, 20.0);
    select(&mut s, target, 0, 0);
    let anchor = edit(&mut s, "anchor.create", &json!({"name": "Keep this anchor"}))["id"].as_u64().unwrap();
    let before = s.doc().unwrap().doc.story(target).unwrap().clone();
    let source = frame(&mut s, "S", 260.0);
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        for params in [json!({"story": target.0, "para": 0}), json!({"paragraph": "arget"}), json!({"anchor": anchor})] {
            let at = back.doc().unwrap().doc.story(source).unwrap().len();
            select(&mut back, source, at, at);
            let result = edit(&mut back, "xref.insert", &params);
            assert_eq!(result["target"], anchor);
            assert_eq!(result["pos"], at);
            assert_eq!(back.doc().unwrap().doc.story(target).unwrap(), &before, "reuse changes no target bytes, metadata or stage");
            assert_eq!(back.doc().unwrap().doc.find_anchor(anchor), Some((target, 0)));
            query_size(&mut back, target, before.len(), before.len(), 20.0);
        }
        assert_eq!(back.doc().unwrap().doc.story(source).unwrap().xrefs.len(), 3);
    }
}

#[test]
fn r22_same_story_implicit_anchor_preserves_caret_and_selected_replacement_offsets() {
    for (anchor, focus, params, expected, xref_at, target_at) in [
        (3, 3, json!({"para": 0}), format!("{ANCHOR_MARK}H\nS{XREF_MARK}"), 6, 0),
        (0, 0, json!({"para": 1}), format!("{XREF_MARK}H\n{ANCHOR_MARK}S"), 0, 5),
        (1, 0, json!({"paragraph": "H"}), format!("{ANCHOR_MARK}{XREF_MARK}\nS"), 3, 0),
    ] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let sid = frame(&mut s, "H\nS", 40.0);
        let mut params = params;
        params["story"] = json!(sid.0);
        select(&mut s, sid, anchor, focus);
        let result = edit(&mut s, "xref.insert", &params);
        let id = result["target"].as_u64().unwrap();
        assert_eq!(result["pos"], xref_at);
        let d = &s.doc().unwrap().doc;
        assert_eq!(d.story(sid).unwrap().text, expected);
        assert_eq!(d.find_anchor(id), Some((sid, target_at)));
        assert_eq!(d.story(sid).unwrap().anchors.len(), 1);
        assert_eq!(d.story(sid).unwrap().xrefs[0].target, id);
        let caret = s.doc().unwrap().selection.text.unwrap();
        assert_eq!((caret.story, caret.anchor, caret.focus), (sid, xref_at + XREF_MARK.len_utf8(), xref_at + XREF_MARK.len_utf8()));
    }
}

#[test]
fn r22_unstaged_blank_eof_markers_keep_the_actual_lf_format() {
    for implicit_target in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1})).unwrap();
        let sid = frame(&mut s, "A\n", 40.0);
        select(&mut s, sid, 1, 2);
        s.execute("type.char", &json!({"attrs": {"size": 15}})).unwrap();
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        assert!(before.paras[1].chars.is_empty());
        assert!(before.paras[1].eof_inherited_chars.is_none());
        let marker = if implicit_target {
            let source = frame(&mut s, "S", 260.0);
            select(&mut s, source, 1, 1);
            edit(&mut s, "xref.insert", &json!({"story": sid.0, "para": 1}));
            ANCHOR_MARK
        } else {
            select(&mut s, sid, 2, 2);
            edit(&mut s, "index.addReference", &json!({"topic": "Ordinary"}));
            INDEX_MARK
        };
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, format!("A\n{marker}"));
        assert_eq!(story.format_after(2), before.format_after(1));
        assert_eq!(story.paras, before.paras);
        query_size(&mut s, sid, 2, 2 + marker.len_utf8(), 15.0);
        edit(&mut s, "type.char", &json!({"attrs": {"size": null}}));
        assert_eq!(query_size(&mut s, sid, 2, 2 + marker.len_utf8(), 12.0)["charOverrides"], 0);
    }
}

#[test]
fn r22_same_story_eof_destination_does_not_consume_the_restored_baseline_twice() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let (sid, baseline) = staged_story(&mut s, "A", 40.0, true, 20.0);
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    select(&mut s, sid, 2, 2);
    let result = edit(&mut s, "xref.insert", &json!({"story": sid.0, "para": 1}));
    let anchor_at = 2;
    let xref_at = anchor_at + ANCHOR_MARK.len_utf8();
    assert_eq!(result["pos"], xref_at);
    let d = &s.doc().unwrap().doc;
    let story = d.story(sid).unwrap();
    assert_eq!(story.text, format!("A\n{ANCHOR_MARK}{XREF_MARK}"));
    assert_eq!(d.find_anchor(result["target"].as_u64().unwrap()), Some((sid, anchor_at)));
    assert_eq!(story.paras.last(), Some(&baseline), "source preparation already restored these genuine defaults");
    assert_eq!(story.format_after(1), before.format_after(1));
    assert_eq!(story.format_after(anchor_at), before.format_after(1), "retain the existing same-story target-anchor policy");
    assert_eq!(story.format_after(xref_at).over.size, Some(20.0));
    let end = xref_at + XREF_MARK.len_utf8();
    let caret = s.doc().unwrap().selection.text.unwrap();
    assert_eq!((caret.story, caret.anchor, caret.focus), (sid, end, end));
    query_size(&mut s, sid, xref_at, end, 20.0);
    edit(&mut s, "type.char", &json!({"attrs": {"size": null}}));
    assert_eq!(query_size(&mut s, sid, xref_at, end, 12.0)["charOverrides"], 0);
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["leading"], json!({"kind": "points", "value": 18.0}));
    for mut back in native_copies(&mut s) {
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().paras.last(), Some(&baseline));
        query_size(&mut back, sid, xref_at, end, 12.0);
    }
}

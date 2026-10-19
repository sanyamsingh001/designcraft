//! C5 EOF-CONSUMER-03/04: outer range replacements and data placeholders consume typing stages.

use designcraft_doc::{CharFormat, Leading, PlaceholderAnchor, PlaceholderRole, Story, StoryId};
use serde_json::{Value, json};

use crate::Session;

fn full_doc(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, range: std::ops::Range<usize>) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": range.start, "focus": range.end})).unwrap();
}

fn frame(s: &mut Session, text: &str) -> StoryId {
    let r = s.execute("frame.create", &json!({"rect": [40, 40, 440, 500], "content": "text", "text": text})).unwrap();
    StoryId(r["story"].as_u64().unwrap())
}

fn edit(s: &mut Session, command: &str, params: &Value) -> Value {
    let before = full_doc(s);
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let result = s.execute(command, params).unwrap();
    let after = full_doc(s);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "{command} owns one history entry");
    assert_eq!(&s.journal[journal..], &[(command.to_string(), params.clone())], "retain the public command identity");
    s.doc().unwrap().doc.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(full_doc(s), before, "{command}: Undo restores the complete document");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(full_doc(s), after, "{command}: Redo restores the complete document");
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn native_copies(s: &mut Session) -> [Session; 2] {
    let full = full_doc(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let zip = crate::cmd::file::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"), "use the actual native ZIP serializer");
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    let mut copies = [Session::new(), Session::new()];
    for (back, bytes) in copies.iter_mut().zip([zip, legacy]) {
        back.execute("file.openBytes", &json!({"base64": crate::cmd::file::base64_encode(&bytes)})).unwrap();
        assert_eq!(full_doc(back), full, "native reopen preserves the complete document");
        back.doc().unwrap().doc.check().unwrap();
    }
    copies
}

fn staged(inherited: bool) -> (Session, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let sid = frame(&mut s, "A");
    select(&mut s, sid, 0..1);
    if inherited {
        s.execute("style.paragraph.create", &json!({"name": "R22 baseline", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
            .unwrap();
        s.execute("style.paragraph.apply", &json!({"name": "R22 baseline"})).unwrap();
        s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    }
    s.execute("style.character.create", &json!({"name": "R22 live", "chars": {"underline": true}})).unwrap();
    s.execute("style.character.apply", &json!({"name": "R22 live"})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}})).unwrap();
    select(&mut s, sid, 1..1);
    s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 30}}})).unwrap();
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "A\n");
    assert_eq!(st.format_after(1).over.size, Some(15.0));
    assert_eq!(st.format_after(1).over.leading, Some(Leading::Points(18.0)));
    assert_eq!(st.paras[1].chars.size, Some(20.0));
    assert_eq!(st.paras[1].chars.leading, Some(Leading::Points(30.0)));
    assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(20.0));
    // These commands have always made literal, untracked replacements, even with tracking on.
    s.execute("changes.track", &json!({"on": true})).unwrap();
    (s, sid)
}

fn expected_format(before: &Story) -> CharFormat {
    CharFormat { style: before.char_format_at(2).style.clone(), over: before.paras[1].chars.clone() }
}

fn consumed(s: &Session, sid: StoryId, before: &Story, text: &str, expected: &CharFormat) {
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, format!("A\n{text}"));
    assert_eq!(st.format_after(0), before.format_after(0));
    assert_eq!(st.format_after(1), before.format_after(1), "do not reformat the actual LF");
    assert_eq!(st.paras[0], before.paras[0]);
    assert_eq!(st.paras[1], before.paras[0], "the populated paragraph regains its genuine baseline");
    for (offset, _) in text.char_indices() {
        assert_eq!(st.format_after(2 + offset), expected, "inserted bytes own the staged attrs and live style");
        assert!(st.format_after(2 + offset).over.change.is_none(), "do not start tracking these existing consumers");
    }
}

fn clear_to_baseline(s: &mut Session, sid: StoryId, end: usize, inherited: bool) {
    let lf = s.doc().unwrap().doc.story(sid).unwrap().format_after(1).clone();
    select(s, sid, 2..end);
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["size"], json!(20.0));
    assert_eq!(attrs["chars"]["leading"], json!({"kind": "points", "value": 30.0}));
    edit(s, "type.char", &json!({"attrs": {"size": null, "leading": null}}));
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    let leading = if inherited { json!({"kind": "points", "value": 26.0}) } else { json!({"kind": "auto"}) };
    assert_eq!(attrs["chars"]["size"], json!(if inherited { 24.0 } else { 12.0 }), "clearing must not expose a leaked EOF stage");
    assert_eq!(attrs["chars"]["leading"], leading);
    assert_eq!(attrs["chars"]["underline"], true, "the named character style remains live");
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().format_after(1), &lf);
}

fn data_source(s: &mut Session) -> u64 {
    let csv = crate::cmd::base64_encode(b"Name\nAda\n");
    let result = edit(s, "data.source.select", &json!({"bytes": csv, "name": "r22-people.csv"}));
    assert_eq!(result["records"], 1);
    assert_eq!(result["fields"], json!([{"name": "Name", "kind": "text"}]));
    let source = &s.doc().unwrap().doc.data_merge.sources[0];
    assert_eq!(source.rows, vec![vec!["Ada".to_string()]]);
    source.id
}

fn placeholder(s: &Session, id: u64, source_id: u64, sid: StoryId, start: usize, end: usize) {
    let ph = s.doc().unwrap().doc.data_merge.placeholders.iter().find(|p| p.id == id).unwrap();
    assert_eq!(ph.source_id, source_id);
    assert_eq!(ph.field, "Name");
    assert_eq!(ph.role, PlaceholderRole::Text);
    assert_eq!(ph.anchor, PlaceholderAnchor::Text { story: sid, start, end });
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text.get(start..end), Some("<<Name>>"));
}

#[test]
fn r22_find_zero_width_eof_consumes_stage_with_attrs_history_and_native() {
    for inherited in [false, true] {
        let (mut s, sid) = staged(inherited);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        // An explicit story target must work independently of the canvas selection.
        select(&mut s, sid, 0..1);
        let selection = s.doc().unwrap().selection.clone();
        let text = "é\"";
        let result = edit(
            &mut s,
            "find.change",
            &json!({"story": sid.0, "grep": true, "find": r"(?P<end>\z)", "change": "${end}é\"", "attrs": {"tracking": 7}}),
        );
        assert_eq!(result, json!({"count": 1}));
        assert_eq!(s.doc().unwrap().selection, selection);
        let mut expected = expected_format(&before);
        expected.over.tracking = Some(7.0);
        consumed(&s, sid, &before, text, &expected);
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            consumed(&back, sid, &before, text, &expected);
            clear_to_baseline(&mut back, sid, 2 + text.len(), inherited);
            assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().format_after(2).over.tracking, Some(7.0));
        }
    }
}

#[test]
fn r22_spelling_equal_endpoints_consume_stage_with_history_and_native() {
    for inherited in [false, true] {
        let (mut s, sid) = staged(inherited);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        select(&mut s, sid, 0..1);
        let selection = s.doc().unwrap().selection.clone();
        let text = "é\"";
        assert_eq!(edit(&mut s, "spelling.change", &json!({"story": sid.0, "start": 2, "end": 2, "to": text})), Value::Null);
        assert_eq!(s.doc().unwrap().selection, selection);
        let expected = expected_format(&before);
        consumed(&s, sid, &before, text, &expected);
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            consumed(&back, sid, &before, text, &expected);
            clear_to_baseline(&mut back, sid, 2 + text.len(), inherited);
        }
    }
}

#[test]
fn r22_data_placeholder_consumes_stage_from_real_source_with_history_and_native() {
    for inherited in [false, true] {
        let (mut s, sid) = staged(inherited);
        let source = data_source(&mut s);
        select(&mut s, sid, 2..2);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let result = edit(&mut s, "data.placeholder.add", &json!({"field": "Name"}));
        let id = result["id"].as_u64().unwrap();
        let expected = expected_format(&before);
        consumed(&s, sid, &before, "<<Name>>", &expected);
        placeholder(&s, id, source, sid, 2, 10);
        let [zip, legacy] = native_copies(&mut s);
        for mut back in [s, zip, legacy] {
            consumed(&back, sid, &before, "<<Name>>", &expected);
            placeholder(&back, id, source, sid, 2, 10);
            clear_to_baseline(&mut back, sid, 10, inherited);
            placeholder(&back, id, source, sid, 2, 10);
        }
    }
}

#[test]
fn r22_find_nonempty_regex_and_spelling_keep_byte_formats_and_pending_eof_control() {
    for spelling in [false, true] {
        let (mut s, sid) = staged(false);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        select(&mut s, sid, 0..1);
        let mut expected = before.format_after(0).clone();
        if spelling {
            edit(&mut s, "spelling.change", &json!({"story": sid.0, "start": 0, "end": 1, "to": "Aé\""}));
        } else {
            let result = edit(
                &mut s,
                "find.change",
                &json!({"story": sid.0, "grep": true, "find": "(A)", "change": "$1é\"", "attrs": {"size": 17, "tracking": 7}}),
            );
            assert_eq!(result, json!({"count": 1}));
            expected.over.size = Some(17.0);
            expected.over.tracking = Some(7.0);
        }
        let st = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(st.text, "Aé\"\n", "keep capture expansion and literal Unicode/quotes");
        for offset in [0, 1, 3] {
            assert_eq!(st.format_after(offset), &expected);
        }
        assert_eq!(st.format_after(4), before.format_after(1));
        assert_eq!(st.paras, before.paras, "editing actual bytes must not consume the separate EOF stage");
        assert_eq!(s.execute("changes.list", &json!({})).unwrap(), json!([]));
        select(&mut s, sid, 5..5);
        assert_eq!(s.execute("type.selectionAttrs", &json!({})).unwrap()["chars"]["size"], json!(20.0));
        native_copies(&mut s);
    }
}

#[test]
fn r22_equal_non_eof_and_empty_replacements_keep_existing_policy_control() {
    for spelling in [false, true] {
        for at in [1, 2] {
            let (mut s, sid) = staged(false);
            let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
            // Empty replacement at blank EOF must leave the entire stage available.
            let text = if at == 2 { "" } else { "é\"" };
            if spelling {
                edit(&mut s, "spelling.change", &json!({"story": sid.0, "start": at, "end": at, "to": text}));
            } else {
                let pattern = if at == 2 { r"\z" } else { "$" };
                let result = edit(&mut s, "find.change", &json!({"story": sid.0, "grep": true, "find": pattern, "change": text, "first": true}));
                assert_eq!(result, json!({"count": 1}));
            }
            let st = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(st.text, if at == 2 { "A\n" } else { "Aé\"\n" });
            assert_eq!(st.paras, before.paras);
            assert!(st.runs().all(|(_, fmt)| fmt == before.format_after(0)), "ordinary insertions still borrow the byte run");
        }
    }
}

#[test]
fn r22_data_duplicate_placeholders_shift_exact_ranges_and_preview_real_record_control() {
    let (mut s, sid) = staged(false);
    let source = data_source(&mut s);
    let first = edit(&mut s, "data.placeholder.add", &json!({"field": "Name", "story": sid.0, "at": 2}))["id"].as_u64().unwrap();
    let second = edit(&mut s, "data.placeholder.add", &json!({"field": "Name", "story": sid.0, "at": 2}))["id"].as_u64().unwrap();
    assert_ne!(first, second);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, "A\n<<Name>><<Name>>");
    // Identical marker text prevents sync_placeholders from repairing an incorrect shift by a unique match.
    placeholder(&s, first, source, sid, 10, 18);
    placeholder(&s, second, source, sid, 2, 10);
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.format_after(2).over.size, Some(15.0), "the second insertion is an ordinary interior position");
    assert_eq!(st.format_after(10).over.size, Some(20.0), "the first marker keeps its consumed EOF format");
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        placeholder(&back, first, source, sid, 10, 18);
        placeholder(&back, second, source, sid, 2, 10);
        let template = full_doc(&back);
        back.execute("data.preview", &json!({"record": 1})).unwrap();
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().text, "A\nAdaAda", "both placeholders bind to the genuine source row");
        back.execute("data.preview.stop", &json!({})).unwrap();
        assert_eq!(full_doc(&back), template, "preview leaves the template and its ranges intact");
        edit(&mut back, "data.placeholder.remove", &json!({"id": second}));
        assert_eq!(back.doc().unwrap().doc.data_merge.placeholders.len(), 1);
        placeholder(&back, first, source, sid, 2, 10);
        assert_eq!(back.doc().unwrap().doc.story(sid).unwrap().format_after(2).over.size, Some(20.0));
    }
}

#[test]
fn r22_find_mixed_matches_preserve_reverse_order_capture_expansion_and_attrs() {
    let (mut s, sid) = staged(false);
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let result =
        edit(&mut s, "find.change", &json!({"story": sid.0, "grep": true, "find": r"(?P<a>A)|\z", "change": "${a}é\"", "attrs": {"tracking": 7}}));
    assert_eq!(result, json!({"count": 2}));
    let st = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(st.text, "Aé\"\né\"", "matches retain their original offsets and expand independently");
    let mut prefix = before.format_after(0).clone();
    prefix.over.tracking = Some(7.0);
    let mut tail = expected_format(&before);
    tail.over.tracking = Some(7.0);
    for offset in [0, 1, 3] {
        assert_eq!(st.format_after(offset), &prefix);
    }
    assert_eq!(st.format_after(4), before.format_after(1));
    for offset in [5, 7] {
        assert_eq!(st.format_after(offset), &tail);
    }
    assert_eq!(st.paras[1], before.paras[0]);
    native_copies(&mut s);
}

#[test]
fn r22_explicit_outer_consumers_leave_selected_cell_and_note_bodies_intact_control() {
    for note in [false, true] {
        for command in ["find.change", "spelling.change", "data.placeholder.add"] {
            let (mut s, sid) = staged(false);
            if command == "data.placeholder.add" {
                data_source(&mut s);
            }
            let outer_before = s.doc().unwrap().doc.story(sid).unwrap().clone();
            let child_sid = frame(&mut s, "");
            select(&mut s, child_sid, 0..0);
            if note {
                s.execute("footnote.insert", &json!({"text": "child\n"})).unwrap();
            } else {
                s.execute("table.insert", &json!({"rows": 1, "cols": 1, "width": 300})).unwrap();
                s.execute("text.insert", &json!({"text": "child\n", "raw": true})).unwrap();
            }
            let selected = s.doc().unwrap().selection.text.unwrap();
            assert_eq!(selected.story, child_sid);
            let cell = selected.cell.unwrap();
            let child_before = s.doc().unwrap().doc.text_story(child_sid, Some(cell)).unwrap().clone();
            let parent_before = s.doc().unwrap().doc.story(child_sid).unwrap().clone();
            let params = match command {
                "find.change" => json!({"story": sid.0, "grep": true, "find": r"\z", "change": "B"}),
                "spelling.change" => json!({"story": sid.0, "start": 2, "end": 2, "to": "B"}),
                _ => json!({"field": "Name", "story": sid.0, "at": 2}),
            };
            edit(&mut s, command, &params);
            let d = &s.doc().unwrap().doc;
            assert_eq!(d.text_story(child_sid, Some(cell)).unwrap(), &child_before);
            assert_eq!(d.story(child_sid).unwrap(), &parent_before);
            assert_eq!(s.doc().unwrap().selection.text, Some(selected));
            let text = if command == "data.placeholder.add" { "<<Name>>" } else { "B" };
            consumed(&s, sid, &outer_before, text, &expected_format(&outer_before));
        }
    }
}

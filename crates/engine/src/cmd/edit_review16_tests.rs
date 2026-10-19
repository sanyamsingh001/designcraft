//! C6 clipboard output/ownership controls; no RF test helper dependency.
use std::sync::Arc;

use designcraft_doc::{Document, ItemId, OBJECT_MARK, StoryId};
use serde_json::{Value, json};

use crate::Session;

fn doc(s: &Session) -> Arc<Document> {
    s.doc().unwrap().doc.clone()
}

fn fixture(command: &str, observed: bool) -> (Session, StoryId, ItemId, String) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let made = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    if observed {
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 3})).unwrap();
        assert_eq!(s.execute("edit.copy", &json!({})).unwrap()["text"], "OLD");
    }
    let made = s.execute("frame.create", &json!({"rect": [100,100,140,140], "content": "unassigned", "shape": "ellipse"})).unwrap();
    let item = ItemId(made["id"].as_u64().unwrap());
    s.execute("selection.set", &json!({"ids": [item.0]})).unwrap();
    let before = doc(&s);
    let undo = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let result = s.execute(command, &json!({})).unwrap();
    let marker = result["clipboardText"].as_str().expect("object Copy/Cut must publish its system text").to_string();
    assert!(!marker.is_empty());
    assert!(result.get("text").is_none(), "the ownership marker is not copied story text");
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last().unwrap().0, command);
    if command == "edit.cut" {
        assert_eq!(result["deleted"], 1, "Cut keeps its existing result field");
        assert!(doc(&s).item(item).is_none());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    } else {
        assert!(Arc::ptr_eq(&before, &doc(&s)));
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    (s, sid, item, marker)
}

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": anchor, "focus": focus})).unwrap();
}

fn paste(s: &mut Session, sid: StoryId, params: Value, expected: &str, anchored: bool) {
    let before = doc(s);
    let undo = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let result = s.execute("edit.paste", &params).unwrap();
    let after = doc(s);
    assert_eq!(after.story(sid).unwrap().text, expected);
    assert_eq!(result.get("anchored").is_some(), anchored);
    assert_eq!(after.story(sid).unwrap().objects.len(), usize::from(anchored));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last(), Some(&("edit.paste".to_string(), params)));
    after.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*doc(s), *before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*doc(s), *after);
}

#[test]
fn r16_c6_copy_cut_output_replaces_unobserved_stale_system_text() {
    for command in ["edit.copy", "edit.cut"] {
        let (mut s, sid, _, marker) = fixture(command, false);
        // The UI publishes this result, replacing preexisting unobserved STALE.
        select(&mut s, sid, 3, 3);
        paste(&mut s, sid, json!({"text": marker}), &format!("OLD{OBJECT_MARK}"), true);
    }
}

#[test]
fn r16_c6_external_text_after_copy_cut_never_uses_cached_stale_equality() {
    for command in ["edit.copy", "edit.cut"] {
        for observed in [false, true] {
            for (text, expected) in [("OLD", "OOLDD"), ("NEW external", "ONEW externalD"), ("", "OD"), ("\"NEW\"\r\ntext", "O\"NEW\"\ntextD")] {
                let (mut s, sid, _, _) = fixture(command, observed);
                select(&mut s, sid, 1, 2);
                paste(&mut s, sid, json!({"text": text}), expected, false);
            }
        }
    }
}

#[test]
fn r16_c6_no_payload_remains_an_internal_object_paste() {
    for command in ["edit.copy", "edit.cut"] {
        let (mut s, sid, _, _) = fixture(command, false);
        select(&mut s, sid, 3, 3);
        paste(&mut s, sid, json!({}), &format!("OLD{OBJECT_MARK}"), true);
    }
}

fn select_child_text(s: &mut Session) {
    s.execute("text.move", &json!({"dir": "storyStart"})).unwrap();
    s.execute("text.move", &json!({"dir": "storyEnd", "extend": true})).unwrap();
}

fn rejects_without_edit(s: &mut Session, command: &str, marker: &str) {
    let before = doc(s);
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    assert!(s.execute(command, &json!({"text": marker})).is_err());
    assert!(Arc::ptr_eq(&before, &doc(s)), "rejected marker must not edit the document");
    assert_eq!(s.doc().unwrap().selection, selection, "do not delete or collapse the selection");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert_eq!(s.journal.len(), journal);
}

#[test]
fn r16_c6_current_marker_never_becomes_plain_text_or_child_text() {
    for copy in ["edit.copy", "edit.cut"] {
        let (mut s, sid, _, marker) = fixture(copy, false);
        select(&mut s, sid, 1, 2);
        rejects_without_edit(&mut s, "edit.pasteWithoutFormatting", &marker);
        assert_eq!(doc(&s).story(sid).unwrap().text, "OLD");

        for note in [false, true] {
            for command in ["edit.paste", "edit.pasteWithoutFormatting"] {
                let (mut s, sid, _, marker) = fixture(copy, false);
                select(&mut s, sid, 3, 3);
                if note {
                    s.execute("footnote.insert", &json!({"text": "CHILD"})).unwrap();
                } else {
                    s.execute("table.insert", &json!({"rows": 1, "cols": 1, "width": 160})).unwrap();
                    s.execute("text.insert", &json!({"text": "CHILD", "raw": true})).unwrap();
                }
                select_child_text(&mut s);
                let target = s.doc().unwrap().selection.text.unwrap();
                assert!(target.cell.is_some());
                assert_eq!(target.range(), 0..5);
                rejects_without_edit(&mut s, command, &marker);
                let before = doc(&s);
                let undo = s.doc().unwrap().history.undo.len();
                let journal = s.journal.len();
                s.execute(command, &json!({"text": "NEW external"})).unwrap();
                let after = doc(&s);
                assert_eq!(after.text_story(sid, target.cell).unwrap().text, "NEW external");
                assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
                assert_eq!(s.journal.len(), journal + 1);
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(*doc(&s), *before);
            }
        }
    }
}

#[test]
fn r16_c6_ineligible_object_payload_rejects_its_marker_before_text_edit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let frame = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(frame["story"].as_u64().unwrap());
    s.execute("selection.set", &json!({"ids": [frame["id"]]})).unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    let marker = result["clipboardText"].as_str().unwrap();
    select(&mut s, sid, 1, 2);
    rejects_without_edit(&mut s, "edit.paste", marker);
}

#[test]
fn r16_c6_generation_matching_is_exact_and_text_copy_supersedes_it() {
    let (mut s, sid, item, first) = fixture("edit.copy", false);
    s.execute("selection.set", &json!({"ids": [item.0]})).unwrap();
    let second = s.execute("edit.copy", &json!({})).unwrap()["clipboardText"].as_str().unwrap().to_string();
    assert_ne!(first, second, "each object copy must own a fresh identity");
    select(&mut s, sid, 3, 3);
    paste(&mut s, sid, json!({"text": first}), &format!("OLD{first}"), false);

    let (mut s, sid, _, marker) = fixture("edit.copy", false);
    select(&mut s, sid, 3, 3);
    let external = format!("{marker}!");
    paste(&mut s, sid, json!({"text": external}), &format!("OLD{external}"), false);

    // Actual text can equal a previous marker. A later text Copy owns its normal formatted slice.
    let (mut s, sid, _, marker) = fixture("edit.copy", false);
    s.execute("story.setText", &json!({"story": sid.0, "text": marker})).unwrap();
    select(&mut s, sid, 0, marker.len());
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], marker);
    assert!(result.get("clipboardText").is_none());
    let target = s.execute("frame.create", &json!({"rect": [320,36,560,300], "content": "text", "text": "TARGET"})).unwrap();
    s.execute("text.select", &json!({"story": target["story"], "anchor": 6, "focus": 6})).unwrap();
    let pasted = s.execute("edit.paste", &json!({"text": marker})).unwrap();
    assert_eq!(pasted["formatted"], true);
    let got = s.execute("story.get", &json!({"story": target["story"]})).unwrap();
    assert_eq!(got["text"], format!("TARGET{marker}"));
}

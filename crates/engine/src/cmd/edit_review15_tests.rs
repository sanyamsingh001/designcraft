//! REVIEW15 C6 clipboard regressions, independent of the RF topic.
use std::sync::Arc;

use designcraft_compose::{self as compose, Cache, ComposeOptions, ComposedStory};
use designcraft_doc::{Document, ItemId, StoryId};
use serde_json::{Value, json};

use crate::Session;

fn doc(s: &Session) -> Arc<Document> {
    s.doc().unwrap().doc.clone()
}

fn full_doc(d: &Document) -> Value {
    serde_json::to_value(d).unwrap()
}

/// Independent direct composition with the same consumed indices as the renderer.
/// Default ComposeOptions are intentionally NOT a reference oracle for running headers.
fn fresh(d: &Document, sid: StoryId) -> ComposedStory {
    let bank = Cache::new();
    let story = d.story(sid).unwrap();
    let options = ComposeOptions {
        running: compose::vars::has_running(d, story).then(|| bank.running_index(d)),
        xrefs: compose::xref::has_xrefs(story).then(|| bank.xref_index(d)),
        ..Default::default()
    };
    compose::compose_story(d, sid, &options)
}

fn same_composition(a: &ComposedStory, b: &ComposedStory) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"), "all composition fields must agree");
}

fn public_native_controls(s: &mut Session, sid: StoryId) {
    let expected = doc(s);
    expected.check().unwrap();
    let reference = fresh(&expected, sid);
    same_composition(&reference, &s.cache.get(&expected, sid, None));
    let story = s.execute("story.get", &json!({"story": sid.0})).unwrap();
    assert_eq!(story["overset"], json!(reference.overset_at));
    let inspect = s.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(inspect["pageCount"], json!(expected.page_count()));
    let summary = inspect["stories"].as_array().unwrap().iter().find(|v| v["id"] == json!(sid.0)).unwrap();
    assert_eq!(summary["overset"], json!(reference.is_overset()));
    let serialized = s.execute("file.serialize", &json!({})).unwrap();
    let bytes = crate::cmd::base64_decode(serialized["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"), "fixture must exercise native ZIP, not an empty fallback");
    let loaded = designcraft_format::load(&bytes).unwrap();
    assert_eq!(loaded, *expected, "the full native document must survive");
    assert_eq!(full_doc(&loaded), full_doc(&expected));
    let mut reopened = Session::new();
    reopened.execute("file.openBytes", &json!({"base64": serialized["base64"]})).unwrap();
    assert_eq!(*doc(&reopened), *expected);
    same_composition(&reference, &reopened.cache.get(&doc(&reopened), sid, None));
    assert!(Arc::ptr_eq(&doc(s), &expected), "queries/serialization must be read-only");
}

fn history(s: &mut Session, sid: StoryId, before: &Arc<Document>, after: &Arc<Document>, count: usize) {
    assert_eq!(s.doc().unwrap().history.undo.len(), count + 1, "one user edit owns one undo entry");
    assert!(Arc::ptr_eq(&s.doc().unwrap().history.undo.last().unwrap().doc, before));
    public_native_controls(s, sid);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(s), before));
    assert_eq!(full_doc(&doc(s)), full_doc(before));
    public_native_controls(s, sid);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(s), after));
    assert_eq!(full_doc(&doc(s)), full_doc(after));
    public_native_controls(s, sid);
}

fn select_all(s: &mut Session, sid: StoryId) {
    let end = doc(s).story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": end})).unwrap();
}

// REVIEW15 C6: explicit external text must outrank an older object clipboard;
// unchanged system text left by object Copy/Cut must still paste the object.
fn r15_c6_clipboard_fixture(copied: Option<&str>, command: &str) -> (Session, StoryId, Option<String>) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let text = copied.unwrap_or("OLD");
    let made = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": text})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    if copied.is_some() {
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
        assert_eq!(s.execute("edit.copy", &json!({})).unwrap()["text"], text);
    }
    let made = s.execute("frame.create", &json!({"rect": [100,100,140,140], "content": "unassigned", "shape": "ellipse"})).unwrap();
    let item = ItemId(made["id"].as_u64().unwrap());
    s.execute("selection.set", &json!({"ids": [item.0]})).unwrap();
    let before = doc(&s);
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let copied = s.execute(command, &json!({})).unwrap();
    let marker = copied.get("clipboardText").and_then(Value::as_str).map(str::to_string);
    let after = doc(&s);
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last().unwrap().0, command);
    if command == "edit.cut" {
        assert!(after.item(item).is_none());
        history(&mut s, sid, &before, &after, count);
    } else {
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(s.doc().unwrap().history.undo.len(), count);
    }
    (s, sid, marker)
}

fn r15_c6_paste(s: &mut Session, sid: StoryId, range: std::ops::Range<usize>, command: &str, supplied: Option<&str>, expected: &str, anchored: bool) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": range.start, "focus": range.end})).unwrap();
    let before = doc(s);
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let params = supplied.map_or_else(|| json!({}), |text| json!({"text": text}));
    let result = s.execute(command, &params).unwrap();
    let after = doc(s);
    assert_eq!(after.story(sid).unwrap().text, expected, "the requested clipboard source must win");
    assert_eq!(result.get("anchored").is_some(), anchored);
    let added = usize::from(anchored);
    assert_eq!(after.story(sid).unwrap().objects.len(), before.story(sid).unwrap().objects.len() + added);
    if anchored {
        assert_eq!(result["anchored"], 1);
        assert_eq!(after.story(sid).unwrap().objects.last().unwrap().item.shape, designcraft_doc::Shape::Oval);
    }
    assert_eq!(after.story(sid).unwrap().frames, before.story(sid).unwrap().frames);
    assert_eq!(after.page_count(), before.page_count());
    assert_eq!(s.journal.len(), journal + 1, "one Paste, no nested text.insert journal entry");
    assert_eq!(s.journal.last(), Some(&(command.to_string(), params)));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, if command == "edit.paste" { "Paste" } else { "Paste without Formatting" });
    history(s, sid, &before, &after, count);
}

#[test]
fn r15_c6_newer_external_text_after_object_cut_caret() {
    // Exact formal-review trigger, using only public commands.
    let (mut s, sid, _) = r15_c6_clipboard_fixture(Some("OLD"), "edit.cut");
    r15_c6_paste(&mut s, sid, 3..3, "edit.paste", Some("NEW external"), "OLDNEW external", false);
}

#[test]
fn r15_c6_newer_external_text_after_object_cut_range() {
    for (text, expected) in [("NEW external", "ONEW externalD"), ("", "OD"), ("\"NEW\"\r\ntext", "O\"NEW\"\ntextD")] {
        let (mut s, sid, _) = r15_c6_clipboard_fixture(Some("OLD"), "edit.cut");
        r15_c6_paste(&mut s, sid, 1..2, "edit.paste", Some(text), expected, false);
    }
}

#[test]
fn r15_c6_object_paste_without_payload_controls() {
    for command in ["edit.cut", "edit.copy"] {
        for copied in [None, Some("OLD")] {
            let (mut s, sid, _) = r15_c6_clipboard_fixture(copied, command);
            r15_c6_paste(&mut s, sid, 3..3, "edit.paste", None, &format!("OLD{}", designcraft_doc::OBJECT_MARK), true);
        }
    }
}

#[test]
fn r15_c6_stale_system_text_preserves_object_paste() {
    // Use the actual object CopyText output; the fallback keeps this control runnable BEFORE.
    for command in ["edit.cut", "edit.copy"] {
        for (copied, supplied) in [("OLD", "OLD"), ("OLD\ntext", "OLD\r\ntext")] {
            let (mut s, sid, marker) = r15_c6_clipboard_fixture(Some(copied), command);
            let at = copied.len();
            r15_c6_paste(
                &mut s,
                sid,
                at..at,
                "edit.paste",
                Some(marker.as_deref().unwrap_or(supplied)),
                &format!("{copied}{}", designcraft_doc::OBJECT_MARK),
                true,
            );
        }
    }
}

#[test]
fn r15_c6_repeated_external_paste_keeps_newer_text() {
    let (mut s, sid, _) = r15_c6_clipboard_fixture(Some("OLD"), "edit.cut");
    r15_c6_paste(&mut s, sid, 3..3, "edit.paste", Some("NEW"), "OLDNEW", false);
    r15_c6_paste(&mut s, sid, 6..6, "edit.paste", Some("NEW"), "OLDNEWNEW", false);
}

#[test]
fn r15_c6_recut_publishes_current_object_ownership() {
    let (mut s, sid, _) = r15_c6_clipboard_fixture(Some("OLD"), "edit.cut");
    r15_c6_paste(&mut s, sid, 3..3, "edit.paste", Some("NEW"), "OLDNEW", false);
    let made = s.execute("frame.create", &json!({"rect": [100,100,140,140], "content": "unassigned", "shape": "ellipse"})).unwrap();
    let item = ItemId(made["id"].as_u64().unwrap());
    s.execute("selection.set", &json!({"ids": [item.0]})).unwrap();
    let before = doc(&s);
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let copied = s.execute("edit.cut", &json!({})).unwrap();
    let marker = copied.get("clipboardText").and_then(Value::as_str).unwrap_or("NEW");
    let after = doc(&s);
    assert!(after.item(item).is_none());
    assert_eq!(s.journal.len(), journal + 1);
    history(&mut s, sid, &before, &after, count);
    r15_c6_paste(&mut s, sid, 6..6, "edit.paste", Some(marker), &format!("OLDNEW{}", designcraft_doc::OBJECT_MARK), true);
}

#[test]
fn r15_c6_explicit_text_without_prior_text_copy_control() {
    let (mut s, sid, _) = r15_c6_clipboard_fixture(None, "edit.cut");
    r15_c6_paste(&mut s, sid, 3..3, "edit.paste", Some("NEW external"), "OLDNEW external", false);
}

#[test]
fn r15_c6_plain_paste_uses_one_history_and_journal() {
    let (mut s, sid, _) = r15_c6_clipboard_fixture(Some("OLD"), "edit.cut");
    r15_c6_paste(&mut s, sid, 1..2, "edit.pasteWithoutFormatting", Some("OLD"), "OOLDD", false);
}

#[test]
fn r15_c6_formatted_copy_after_object_cut_retains_note_payload() {
    for supplied in [false, true] {
        let (mut s, sid, _) = r15_c6_clipboard_fixture(Some("OLD"), "edit.cut");
        s.execute("text.select", &json!({"story": sid.0, "anchor": 3, "focus": 3})).unwrap();
        s.execute("footnote.insert", &json!({"text": "note payload"})).unwrap();
        select_all(&mut s, sid);
        s.execute("type.char", &json!({"attrs": {"tracking": 75}})).unwrap();
        assert_eq!(s.execute("edit.copy", &json!({})).unwrap()["text"], "OLD");
        let made = s.execute("frame.create", &json!({"rect": [320,36,560,300], "content": "text", "text": "Target: "})).unwrap();
        let target = StoryId(made["story"].as_u64().unwrap());
        s.execute("text.select", &json!({"story": target.0, "anchor": 8, "focus": 8})).unwrap();
        let before = doc(&s);
        let count = s.doc().unwrap().history.undo.len();
        let journal = s.journal.len();
        let params = if supplied { json!({"text": "OLD"}) } else { json!({}) };
        let result = s.execute("edit.paste", &params).unwrap();
        let after = doc(&s);
        let pasted = after.story(target).unwrap();
        assert_eq!(result["formatted"], true);
        assert!(result.get("anchored").is_none());
        assert_eq!(pasted.text, format!("Target: OLD{}", designcraft_doc::FOOTNOTE_REF));
        assert_eq!(pasted.notes.len(), 1);
        assert_eq!(pasted.notes[0].text.text, "note payload");
        assert_eq!(pasted.format_after(8).over.tracking, Some(75.0));
        assert!(pasted.objects.is_empty());
        assert_eq!(after.story(sid), before.story(sid));
        assert_eq!(s.journal.len(), journal + 1);
        assert_eq!(s.journal.last(), Some(&("edit.paste".to_string(), params)));
        history(&mut s, target, &before, &after, count);
        public_native_controls(&mut s, sid);
    }
}

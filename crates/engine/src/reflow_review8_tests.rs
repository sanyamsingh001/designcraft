//! REVIEW8 reflow regressions. Only public Session/compose/format APIs, usable on the pinned BEFORE tree.
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

fn r9_object_clipboard(command: &str, earlier_text: bool) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let made = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    if earlier_text {
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 3})).unwrap();
        assert_eq!(s.execute("edit.copy", &json!({})).unwrap()["text"], "OLD");
    }
    let shape = s.execute("frame.create", &json!({"rect": [100,100,140,140], "content": "unassigned", "shape": "ellipse"})).unwrap();
    let id = ItemId(shape["id"].as_u64().unwrap());
    s.execute("selection.set", &json!({"ids": [id.0]})).unwrap();
    assert!(s.doc().unwrap().selection.text.is_none());
    let before = doc(&s);
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute(command, &json!({})).unwrap();
    let after = doc(&s);
    if command == "edit.cut" {
        assert!(after.item(id).is_none());
        assert_eq!(s.journal.len(), journal + 1, "one public Cut, no nested Clear journal");
        assert_eq!(s.journal.last().unwrap().0, "edit.cut");
        history(&mut s, sid, &before, &after, count);
    } else {
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(s.doc().unwrap().history.undo.len(), count);
    }
    s.execute("text.select", &json!({"story": sid.0, "anchor": 3, "focus": 3})).unwrap();
    let paste_before = doc(&s);
    let paste_count = s.doc().unwrap().history.undo.len();
    let result = s.execute("edit.paste", &json!({})).unwrap();
    let pasted = doc(&s);
    assert_eq!(result["anchored"], 1, "defect oracle: latest object clipboard must win over old formatted text");
    assert_eq!(pasted.story(sid).unwrap().text, format!("OLD{}", designcraft_doc::OBJECT_MARK));
    assert_eq!(pasted.story(sid).unwrap().objects.len(), 1);
    assert_eq!(pasted.story(sid).unwrap().objects[0].item.shape, before.item(id).unwrap().shape);
    history(&mut s, sid, &paste_before, &pasted, paste_count);
}

#[test]
fn r9_reflow_object_cut_replaces_old_formatted_clipboard_public_paste_single_journal() {
    r9_object_clipboard("edit.cut", true);
}

#[test]
fn r9_reflow_object_copy_and_cut_without_text_clipboard_controls() {
    r9_object_clipboard("edit.copy", true);
    r9_object_clipboard("edit.cut", false);
}

#[test]
fn r9_reflow_formatted_text_cut_and_plain_paste_controls() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let made = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    select_all(&mut s, sid);
    s.execute("type.char", &json!({"attrs": {"tracking": 75}})).unwrap();
    let before = doc(&s);
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    assert_eq!(s.execute("edit.cut", &json!({})).unwrap()["text"], "OLD");
    let cut = doc(&s);
    assert!(cut.story(sid).unwrap().is_empty());
    assert_eq!(s.journal.len(), journal + 1);
    history(&mut s, sid, &before, &cut, count);
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    assert_eq!(s.execute("edit.paste", &json!({})).unwrap()["formatted"], true);
    assert_eq!(doc(&s).story(sid).unwrap().text, "OLD");
    assert_eq!(doc(&s).story(sid).unwrap().format_after(0).over.tracking, Some(75.0));
    s.execute("text.select", &json!({"story": sid.0, "anchor": 3, "focus": 3})).unwrap();
    s.execute("edit.pasteWithoutFormatting", &json!({"text": " plain"})).unwrap();
    assert_eq!(doc(&s).story(sid).unwrap().text, "OLD plain");
    public_native_controls(&mut s, sid);
}

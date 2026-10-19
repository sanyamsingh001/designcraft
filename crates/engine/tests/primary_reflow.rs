use std::sync::Arc;

use designcraft_doc::{Document, ItemId, StoryId};
use designcraft_engine::{Session, compose};
use serde_json::json;

#[derive(Clone, Copy, Debug)]
enum Removal {
    Frame,
    Page,
    Cut,
}

fn primary_chain() -> (Session, StoryId, ItemId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1, "primaryTextFrame": true})).unwrap();
    let sid = s.doc().unwrap().doc.settings.primary_story.unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    s.execute("text.insert", &json!({"text": "Words keep coming and the page fills up quickly with them. ".repeat(250)})).unwrap();
    let d = &s.doc().unwrap().doc;
    let last = *d.story(sid).unwrap().frames.last().unwrap();
    assert!(d.page_count() >= 3);
    assert!(!compose::compose_story(d, sid, &Default::default()).is_overset());
    let mut probe = (**d).clone();
    probe.remove_item(last).unwrap();
    assert!(compose::compose_story(&probe, sid, &Default::default()).is_overset(), "the removed frame holds text");
    (s, sid, last)
}

fn remove(s: &mut Session, frame: ItemId, kind: Removal) {
    match kind {
        Removal::Frame => s.execute("edit.clear", &json!({"ids": [frame.0]})).unwrap(),
        Removal::Page => {
            let page = s.doc().unwrap().doc.page_of_item(frame).unwrap();
            s.execute("layout.pages.delete", &json!({"pages": [page]})).unwrap()
        }
        Removal::Cut => {
            s.execute("selection.set", &json!({"ids": [frame.0]})).unwrap();
            s.execute("edit.cut", &json!({})).unwrap()
        }
    };
}

fn assert_removed(s: &Session, before: &Document, sid: StoryId, frame: ItemId, pages: usize) {
    let d = &s.doc().unwrap().doc;
    assert!(d.item(frame).is_none());
    assert_eq!(d.page_count(), pages);
    assert_eq!(d.settings.primary_story, Some(sid));
    let story = d.story(sid).unwrap();
    let old = before.story(sid).unwrap();
    assert_eq!(story.text, old.text);
    assert_eq!(story.frames, old.frames.iter().copied().filter(|f| *f != frame).collect::<Vec<_>>());
    assert!(compose::compose_story(d, sid, &Default::default()).is_overset());
    d.check().unwrap();
}

#[test]
fn explicit_primary_removals_survive_unrelated_edits_with_a_cold_cache() {
    for kind in [Removal::Frame, Removal::Page, Removal::Cut] {
        let (mut s, sid, last) = primary_chain();
        let before = s.doc().unwrap().doc.clone();
        let pages = before.page_count() - usize::from(matches!(kind, Removal::Page));
        remove(&mut s, last, kind);
        assert_removed(&s, &before, sid, last, pages);
        let removed = s.doc().unwrap().doc.clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &removed));

        s.cache.clear();
        assert!(s.cache.composed_for(&removed, sid).is_none());
        s.execute("frame.create", &json!({"rect": [10, 10, 20, 20], "content": "unassigned"})).unwrap();
        assert_removed(&s, &before, sid, last, pages);
        s.execute("frame.create", &json!({"rect": [10, 30, 100, 60], "content": "text", "text": "Other story", "caret": false})).unwrap();
        assert_removed(&s, &before, sid, last, pages);
        assert!(s.cache.composed_for(&s.doc().unwrap().doc, sid).is_none(), "unrelated commands do not compose the primary story");
        assert!(s.prefs.smart_text_reflow);
    }
}

#[test]
fn primary_content_and_frame_changes_resume_reflow_in_one_undo_step() {
    for frame_change in [false, true] {
        let (mut s, sid, last) = primary_chain();
        remove(&mut s, last, Removal::Page);
        let removed = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        if frame_change {
            let first = removed.story(sid).unwrap().frames[0];
            let inset = removed.item(first).unwrap().text_frame().unwrap().options.inset.map(|v| v + 1.0);
            s.execute("object.textFrameOptions", &json!({"ids": [first.0], "inset": inset})).unwrap();
        } else {
            let end = removed.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
            s.execute("text.insert", &json!({"text": " More text."})).unwrap();
        }
        let flowed = s.doc().unwrap().doc.clone();
        assert!(flowed.page_count() > removed.page_count());
        assert!(!compose::compose_story(&flowed, sid, &Default::default()).is_overset());
        assert!(flowed.item(last).is_none());
        assert_eq!(flowed.settings.primary_story, Some(sid));
        let expected = removed.story(sid).unwrap().text.clone() + if frame_change { "" } else { " More text." };
        assert_eq!(flowed.story(sid).unwrap().text, expected);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        flowed.check().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &removed));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &flowed));
    }
}

#[test]
fn deleting_primary_text_still_prunes_empty_pages() {
    for command in ["text.delete", "edit.clear", "edit.cut"] {
        let (mut s, sid, _) = primary_chain();
        let before = s.doc().unwrap().doc.clone();
        let end = before.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": end})).unwrap();
        s.execute(command, &json!({})).unwrap();
        let empty = s.doc().unwrap().doc.clone();
        assert_eq!(empty.page_count(), 1, "{command}");
        assert_eq!(empty.settings.primary_story, Some(sid));
        assert_eq!(empty.story(sid).unwrap().frames.len(), 1);
        assert!(empty.story(sid).unwrap().text.is_empty());
        empty.check().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &empty));
    }
}

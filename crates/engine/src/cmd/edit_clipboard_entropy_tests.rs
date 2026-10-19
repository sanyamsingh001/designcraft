//! C6 entropy failure/publication ordering; no RF helper dependency.
use std::sync::Arc;

use designcraft_doc::ItemId;
use serde_json::json;

use crate::Session;

fn selected_object() -> (Session, ItemId) {
    let mut session = Session::new();
    session.execute("file.new", &json!({})).unwrap();
    let frame = session.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    session.execute("text.select", &json!({"story": frame["story"], "anchor": 0, "focus": 3})).unwrap();
    assert_eq!(session.execute("edit.copy", &json!({})).unwrap()["text"], "OLD");
    let made = session.execute("frame.create", &json!({"rect": [100,100,140,140], "content": "unassigned", "shape": "ellipse"})).unwrap();
    let item = ItemId(made["id"].as_u64().unwrap());
    session.execute("selection.set", &json!({"ids": [item.0]})).unwrap();
    (session, item)
}

#[test]
fn r17_c6_entropy_failure_precedes_cut_and_preserves_clipboards() {
    let (mut session, item) = selected_object();
    let before = session.doc().unwrap().doc.clone();
    let selection = session.doc().unwrap().selection.clone();
    let undo = session.doc().unwrap().history.undo.len();
    let journal = session.journal.len();
    let text = session.text_clipboard.clone().unwrap();
    assert!(session.object_clipboard_nonce.is_none());
    let result = super::copy_objects_with_entropy(&mut session, Some(&json!({})), |_| Err(getrandom::Error::UNSUPPORTED));
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&before, &session.doc().unwrap().doc));
    assert!(session.doc().unwrap().doc.item(item).is_some());
    assert_eq!(session.doc().unwrap().selection, selection);
    assert_eq!(session.doc().unwrap().history.undo.len(), undo);
    assert_eq!(session.journal.len(), journal);
    assert!(session.clipboard.is_none());
    assert!(session.object_clipboard_marker.is_none());
    assert!(session.object_clipboard_nonce.is_none());
    let retained = session.text_clipboard.as_ref().unwrap();
    assert!(Arc::ptr_eq(&text.0, &retained.0));
    assert_eq!(text.1, retained.1);
    // A later successful public Cut still owns exactly one deletion/history entry.
    let copied = session.execute("edit.cut", &json!({})).unwrap();
    assert!(copied["clipboardText"].as_str().is_some());
    assert_eq!(copied["deleted"], 1);
    assert!(session.doc().unwrap().doc.item(item).is_none());
    assert_eq!(session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(session.journal.len(), journal + 1);
}

#[test]
fn r17_c6_instance_nonce_is_reused_with_fresh_generations() {
    let (mut session, _) = selected_object();
    let first = session.execute("edit.copy", &json!({})).unwrap();
    let before = session.doc().unwrap().doc.clone();
    let nonce = session.object_clipboard_nonce.unwrap();
    // Already initialized Sessions do not reacquire entropy for each generation.
    let second = super::copy_objects_with_entropy(&mut session, None, |_| Err(getrandom::Error::UNSUPPORTED)).unwrap();
    assert_eq!(session.object_clipboard_nonce, Some(nonce));
    assert!(Arc::ptr_eq(&before, &session.doc().unwrap().doc));
    let first = first["clipboardText"].as_str().unwrap();
    let second = second["clipboardText"].as_str().unwrap();
    let (first_namespace, first_generation) = first.rsplit_once(':').unwrap();
    let (second_namespace, second_generation) = second.rsplit_once(':').unwrap();
    assert_eq!(first_namespace, second_namespace);
    assert_ne!(first_generation, second_generation);
}

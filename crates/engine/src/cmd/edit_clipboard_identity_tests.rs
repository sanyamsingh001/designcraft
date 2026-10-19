//! Native fresh-process regression for C6 marker namespaces; public commands only.
use std::process::Command;

use designcraft_doc::{ItemId, StoryId};
use serde_json::{Value, json};

use crate::Session;

const WORKER: &str = "cmd::edit::clipboard_identity_tests::r17_identity_process_worker";
const OUTPUT: &str = "C6_IDENTITY_RESULT:";

fn fixture(shape: &str) -> (Session, StoryId, ItemId) {
    let mut session = Session::new();
    session.execute("file.new", &json!({})).unwrap();
    let frame = session.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let story = StoryId(frame["story"].as_u64().unwrap());
    let rect = if shape == "ellipse" { [100, 100, 140, 140] } else { [100, 100, 180, 160] };
    let mut params = json!({"rect": rect, "content": "unassigned"});
    if shape == "ellipse" {
        params["shape"] = json!("ellipse");
    }
    let made = session.execute("frame.create", &params).unwrap();
    let item = ItemId(made["id"].as_u64().unwrap());
    session.execute("selection.set", &json!({"ids": [item.0]})).unwrap();
    (session, story, item)
}

#[test]
fn r17_identity_process_worker() {
    let Ok(shape) = std::env::var("DESIGNCRAFT_C6_IDENTITY_WORKER") else { return };
    let command = std::env::var("DESIGNCRAFT_C6_IDENTITY_COMMAND").unwrap();
    let (mut session, story, _) = fixture(&shape);
    let copied = session.execute(&command, &json!({})).unwrap();
    let marker = copied["clipboardText"].as_str().unwrap().to_string();
    let mut output = json!({"marker": marker});
    if let Ok(foreign) = std::env::var("DESIGNCRAFT_C6_FOREIGN_MARKER") {
        session.execute("text.select", &json!({"story": story.0, "anchor": 1, "focus": 2})).unwrap();
        let before = session.doc().unwrap().doc.clone();
        let undo = session.doc().unwrap().history.undo.len();
        let journal = session.journal.len();
        let pasted = session.execute("edit.paste", &json!({"text": foreign})).unwrap();
        let after = session.doc().unwrap().doc.clone();
        output["text"] = json!(after.story(story).unwrap().text);
        output["anchored"] = pasted.get("anchored").cloned().unwrap_or(Value::Null);
        output["objects"] = json!(after.story(story).unwrap().objects.len());
        assert_eq!(session.doc().unwrap().history.undo.len(), undo + 1);
        assert_eq!(session.journal.len(), journal + 1);
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *before);
        session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*session.doc().unwrap().doc, *after);
    }
    println!("{OUTPUT}{output}");
}

fn fresh_process(shape: &str, command: &str, foreign: Option<&str>) -> Value {
    let mut worker = Command::new(std::env::current_exe().unwrap());
    worker.args(["--exact", WORKER, "--nocapture", "--test-threads=1"]);
    worker.env("DESIGNCRAFT_C6_IDENTITY_WORKER", shape);
    worker.env("DESIGNCRAFT_C6_IDENTITY_COMMAND", command);
    worker.env_remove("DESIGNCRAFT_C6_FOREIGN_MARKER");
    if let Some(foreign) = foreign {
        worker.env("DESIGNCRAFT_C6_FOREIGN_MARKER", foreign);
    }
    let output = worker.output().unwrap();
    assert!(output.status.success(), "worker failed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result = stdout.lines().find_map(|line| line.split_once(OUTPUT).map(|(_, result)| result)).expect("worker must report its actual marker");
    serde_json::from_str(result).unwrap()
}

#[test]
fn r17_c6_fresh_process_marker_cannot_authorize_local_objects() {
    for command in ["edit.copy", "edit.cut"] {
        // Each worker starts with the same allocator/document trace in its own process.
        let foreign = fresh_process("rectangle", command, None);
        let foreign_marker = foreign["marker"].as_str().unwrap();
        let local = fresh_process("ellipse", command, Some(foreign_marker));
        assert_ne!(local["marker"], foreign["marker"], "independent owners must not repeat a process-local identity");
        assert!(local["anchored"].is_null(), "foreign output must not select the local ellipse");
        assert_eq!(local["objects"], 0);
        assert_eq!(local["text"], format!("O{foreign_marker}D"));
    }
}

//! C6 owns this module. It uses public commands and the shared clipboard output path.
//! No C4 canvas harness or RF test helpers are required.
use designcraft_doc::{OBJECT_MARK, StoryId};
use designcraft_engine::Session;
use egui::{Context, OutputCommand, RawInput};
use serde_json::{Value, json};

use crate::{DesignApp, Services, control, menus};

fn fixture() -> (DesignApp, StoryId) {
    let mut session = Session::new();
    session.execute("file.new", &json!({})).unwrap();
    let frame = session.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(frame["story"].as_u64().unwrap());
    let item = session.execute("frame.create", &json!({"rect": [100,100,140,140], "content": "unassigned", "shape": "ellipse"})).unwrap();
    session.execute("selection.set", &json!({"ids": [item["id"]]})).unwrap();
    (DesignApp::new(session, Services::default()), sid)
}

fn output(app: &mut DesignApp, ctx: &Context) -> Vec<String> {
    // Even the initial font-loading UI frame must deliver queued command output.
    let mut full = ctx.run_ui(RawInput::default(), |ui| app.ui(ui));
    full.textures_delta.clear();
    full.platform_output
        .commands
        .into_iter()
        .filter_map(|command| match command {
            OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn r16_c6_object_output_reaches_menu_control_and_direct_dispatch() {
    for command in ["edit.copy", "edit.cut"] {
        for route in ["direct", "menu", "control"] {
            let (mut app, sid) = fixture();
            let ctx = Context::default();
            match route {
                "direct" => {
                    app.run(command, json!({})).unwrap();
                }
                "menu" => menus::activate(&mut app, command, &Value::Null),
                _ => {
                    let (request, _) = control::ControlRequest::new("ui.menu.invoke", json!({"command": command}));
                    assert!(matches!(control::handle(&mut app, &ctx, &request), control::Outcome::Done(_)));
                }
            }
            assert!(app.ui.status.is_empty());
            let writes = output(&mut app, &ctx);
            assert_eq!(writes.len(), 1, "{route} {command} must produce exactly one clipboard write");
            let mut system_clipboard = "STALE".to_string();
            assert_ne!(writes[0], system_clipboard);
            system_clipboard.clone_from(&writes[0]);
            assert!(output(&mut app, &ctx).is_empty(), "idle frames must not repeat a clipboard write");
            app.run("text.select", json!({"story": sid.0, "anchor": 3, "focus": 3})).unwrap();
            let undo = app.session.doc().unwrap().history.undo.len();
            let journal = app.session.journal.len();
            let result = app.run("edit.paste", json!({"text": system_clipboard})).unwrap();
            assert_eq!(result["anchored"], 1);
            assert_eq!(app.session.doc().unwrap().doc.story(sid).unwrap().text, format!("OLD{OBJECT_MARK}"));
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
            assert_eq!(app.session.journal.len(), journal + 1);
            assert!(output(&mut app, &ctx).is_empty(), "Paste must not rewrite the system clipboard");
        }
    }
}

#[test]
fn r16_c6_shared_output_preserves_text_results_and_last_copy_wins() {
    for command in ["edit.copy", "edit.cut"] {
        let (mut app, sid) = fixture();
        let ctx = Context::default();
        app.run("edit.copy", json!({})).unwrap();
        app.run("text.select", json!({"story": sid.0, "anchor": 0, "focus": 3})).unwrap();
        let result = app.run(command, json!({})).unwrap();
        assert_eq!(result["text"], "OLD");
        assert!(result.get("clipboardText").is_none());
        assert_eq!(output(&mut app, &ctx), vec!["OLD"]);
        assert!(output(&mut app, &ctx).is_empty());
    }
}

#[test]
fn r16_c6_failed_copy_keeps_prior_pending_output_and_adds_no_write() {
    let (mut app, _) = fixture();
    let ctx = Context::default();
    let copied = app.run("edit.copy", json!({})).unwrap();
    let marker = copied["clipboardText"].as_str().unwrap().to_string();
    app.run("edit.deselectAll", json!({})).unwrap();
    assert!(app.run("edit.copy", json!({})).is_err());
    assert_eq!(output(&mut app, &ctx), vec![marker]);
    assert!(output(&mut app, &ctx).is_empty());
}

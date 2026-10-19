//! Shared egui clipboard request/delivery tests; no native menu or rendering backend.
use std::sync::Arc;

use designcraft_doc::{OBJECT_MARK, StoryId};
use egui::{Context, Event, FullOutput, OutputCommand, RawInput};
use serde_json::{Value, json};

use crate::{DesignApp, Services, menus};

fn fixture(object: bool) -> (DesignApp, StoryId, String) {
    let mut app = DesignApp::new(designcraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({})).unwrap();
    let frame = app.run("frame.create", json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(frame["story"].as_u64().unwrap());
    if object {
        let item = app.run("frame.create", json!({"rect": [100,100,140,140], "content": "unassigned"})).unwrap();
        app.run("selection.set", json!({"ids": [item["id"]]})).unwrap();
    } else {
        app.run("text.select", json!({"story": sid.0, "anchor": 0, "focus": 3})).unwrap();
    }
    let copied = app.run("edit.copy", json!({})).unwrap();
    let text = copied.get("clipboardText").or_else(|| copied.get("text")).unwrap().as_str().unwrap().to_string();
    app.run("text.select", json!({"story": sid.0, "anchor": 1, "focus": 2})).unwrap();
    app.session.set_tool("type");
    app.view_mut().unwrap();
    assert!(app.session.wants_text());
    (app, sid, text)
}

fn frame(app: &mut DesignApp, ctx: &Context, events: Vec<Event>, paste_menu: bool) -> FullOutput {
    let mut output = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
        app.flush_clipboard_request(ui.ctx());
        app.flush_clipboard_output(ui.ctx());
        if paste_menu {
            // The same activation used by the in-window menu, with its default null params.
            menus::activate(app, "edit.paste", &Value::Null);
        }
        let (rect, response) = ui.allocate_exact_size(egui::vec2(800.0, 600.0), egui::Sense::click_and_drag());
        super::handle_input(app, ui, &response, rect);
        app.flush_clipboard_output(ui.ctx());
    });
    // These request/delivery tests have no renderer to consume the font atlas.
    output.textures_delta.clear();
    output
}

fn requests(out: &FullOutput) -> usize {
    out.viewport_output.values().flat_map(|output| &output.commands).filter(|cmd| matches!(cmd, egui::ViewportCommand::RequestPaste)).count()
}

#[test]
fn fc2_menu_paste_waits_for_external_payload_before_replacing_selection() {
    for object in [false, true] {
        for text in ["NEW external", ""] {
            let (mut app, sid, _) = fixture(object);
            let ctx = Context::default();
            frame(&mut app, &ctx, vec![], false); // Publish the earlier copy.
            let before = app.session.doc().unwrap().doc.clone();
            let selection = app.session.doc().unwrap().selection.clone();
            let undo = app.session.doc().unwrap().history.undo.len();
            let journal = app.session.journal.len();
            assert_eq!(requests(&frame(&mut app, &ctx, vec![], true)), 0);
            assert_eq!(requests(&frame(&mut app, &ctx, vec![], false)), 1);
            assert!(Arc::ptr_eq(&before, &app.session.doc().unwrap().doc));
            assert_eq!(app.session.doc().unwrap().selection, selection);
            assert_eq!(app.session.journal.len(), journal);
            assert_eq!(requests(&frame(&mut app, &ctx, vec![Event::Paste(text.into())], false)), 0);
            let st = app.session.doc().unwrap();
            assert_eq!(st.doc.story(sid).unwrap().text, format!("O{text}D"));
            assert!(st.doc.story(sid).unwrap().objects.is_empty());
            assert_eq!(st.history.undo.len(), undo + 1);
            assert_eq!(app.session.journal.len(), journal + 1);
            assert_eq!(app.session.journal.last().unwrap(), &("edit.paste".into(), json!({"text": text})));
            assert!(app.ui.status.is_empty());
        }
    }
}

#[test]
fn fc2_menu_paste_reads_after_pending_copy_and_accepts_its_object_payload() {
    let (mut app, sid, marker) = fixture(true);
    let ctx = Context::default();
    let before = app.session.doc().unwrap().doc.clone();
    menus::activate(&mut app, "edit.paste", &Value::Null);
    let output = frame(&mut app, &ctx, vec![], false);
    assert_eq!(requests(&output), 0, "deliver the queued write before requesting a read");
    assert!(output.platform_output.commands.contains(&OutputCommand::CopyText(marker.clone())));
    assert_eq!(requests(&frame(&mut app, &ctx, vec![], false)), 1);
    assert!(Arc::ptr_eq(&before, &app.session.doc().unwrap().doc));
    frame(&mut app, &ctx, vec![Event::Paste(marker)], false);
    assert_eq!(app.session.doc().unwrap().doc.story(sid).unwrap().text, format!("O{OBJECT_MARK}D"));
    assert_eq!(app.session.doc().unwrap().doc.story(sid).unwrap().objects.len(), 1);
    assert_eq!(requests(&frame(&mut app, &ctx, vec![], false)), 0);
}

#[test]
fn fc2_explicit_internal_paste_does_not_request_system_clipboard() {
    let (mut app, sid, _) = fixture(true);
    let result = app.run("edit.paste", json!({})).unwrap();
    assert_eq!(result["anchored"], 1);
    assert_eq!(app.session.doc().unwrap().doc.story(sid).unwrap().text, format!("O{OBJECT_MARK}D"));
    assert_eq!(requests(&frame(&mut app, &Context::default(), vec![], false)), 0);
}

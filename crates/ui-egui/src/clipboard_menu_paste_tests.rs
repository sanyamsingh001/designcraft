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

fn host_reader(app: &mut DesignApp) -> Arc<std::sync::Mutex<Vec<crate::PasteRequest>>> {
    let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let host_reads = reads.clone();
    app.services.request_paste = Some(Box::new(move |request| {
        host_reads.lock().unwrap().push(request);
    }));
    reads
}

#[test]
fn fc2_menu_paste_uses_host_reader_after_queued_copy() {
    for text in ["external", ""] {
        let (mut app, sid, marker) = fixture(true);
        let reads = host_reader(&mut app);
        let ctx = Context::default();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let journal = app.session.journal.len();
        menus::activate(&mut app, "edit.paste", &Value::Null);
        let output = frame(&mut app, &ctx, vec![], false);
        assert!(output.platform_output.commands.contains(&OutputCommand::CopyText(marker)));
        assert!(reads.lock().unwrap().is_empty(), "publish Copy before asking the host to read");
        assert_eq!(requests(&output), 0);
        assert_eq!(requests(&frame(&mut app, &ctx, vec![], false)), 0, "the web host handles the request instead of eframe");
        assert_eq!(reads.lock().unwrap().len(), 1);
        assert!(Arc::ptr_eq(&before, &app.session.doc().unwrap().doc));
        let request = reads.lock().unwrap().pop().unwrap();
        assert!(app.receive_clipboard_paste(&ctx, request, text.into()));
        let state = app.session.doc().unwrap();
        assert_eq!(state.doc.story(sid).unwrap().text, format!("O{text}D"));
        assert!(state.doc.story(sid).unwrap().objects.is_empty());
        assert_eq!(state.history.undo.len(), undo + 1);
        assert_eq!(app.session.journal.len(), journal + 1);
        assert_eq!(app.session.journal.last().unwrap(), &("edit.paste".into(), json!({"text": text})));
        frame(&mut app, &ctx, vec![], false);
        assert!(reads.lock().unwrap().is_empty());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.doc().unwrap().doc, *before);
    }
}

#[test]
fn fc2_host_paste_cancels_when_its_editing_context_changes() {
    for change in ["document", "selection", "edit", "focus", "dialog", "tool"] {
        let (mut app, sid, _) = fixture(false);
        let reads = host_reader(&mut app);
        let ctx = Context::default();
        frame(&mut app, &ctx, vec![], false);
        frame(&mut app, &ctx, vec![], true);
        frame(&mut app, &ctx, vec![], false);
        let request = reads.lock().unwrap().pop().unwrap();
        match change {
            "document" => {
                app.run("file.new", json!({})).unwrap();
                let created = app.run("frame.create", json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
                app.run("text.select", json!({"story": created["story"], "anchor": 1, "focus": 2})).unwrap();
                app.session.set_tool("type");
            }
            "selection" => {
                app.run("text.select", json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
            }
            "edit" => {
                app.run("text.insert", json!({"text": "changed"})).unwrap();
                // Restore the original selection: document identity must reject this read too.
                app.run("text.select", json!({"story": sid.0, "anchor": 1, "focus": 2})).unwrap();
            }
            "focus" => {
                let mut field = String::from("panel value");
                let mut output = ctx.run_ui(RawInput::default(), |ui| {
                    ui.text_edit_singleline(&mut field).request_focus();
                });
                output.textures_delta.clear();
                assert!(ctx.text_edit_focused());
            }
            "dialog" => app.ui.dialog = Some(crate::dialogs::Dialog::new("textFrameOptions", json!({}))),
            _ => {
                assert_eq!(change, "tool");
                app.session.set_tool("selection");
            }
        }
        let before = app.session.documents().to_vec();
        let active = app.session.active_index();
        let journal = app.session.journal.clone();
        assert!(!app.receive_clipboard_paste(&ctx, request, "external".into()), "{change}");
        assert_eq!(app.session.active_index(), active, "{change}");
        assert_eq!(app.session.journal, journal, "{change}");
        for (old, current) in before.iter().zip(app.session.documents()) {
            assert!(Arc::ptr_eq(&old.doc, &current.doc), "{change}");
            assert_eq!(old.selection, current.selection, "{change}");
            assert_eq!(old.history.undo.len(), current.history.undo.len(), "{change}");
            assert_eq!(old.history.redo.len(), current.history.redo.len(), "{change}");
        }
    }
}

#[test]
fn fc2_host_paste_captures_selection_when_menu_is_activated() {
    let (mut app, sid, _) = fixture(false);
    let reads = host_reader(&mut app);
    let ctx = Context::default();
    menus::activate(&mut app, "edit.paste", &Value::Null);
    app.run("text.select", json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    frame(&mut app, &ctx, vec![], false); // Finish the earlier Copy first.
    frame(&mut app, &ctx, vec![], false);
    assert!(reads.lock().unwrap().is_empty(), "do not capture the new selection at dispatch time");
    assert!(Arc::ptr_eq(&before, &app.session.doc().unwrap().doc));
}

fn full_ui_frame(app: &mut DesignApp, ctx: &Context, events: Vec<Event>, mut after_ui: impl FnMut(&mut DesignApp, &Context)) -> FullOutput {
    let mut raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
        max_texture_side: Some(8192),
        time: Some(ctx.input(|i| i.time) + 1.0 / 60.0),
        focused: true,
        events,
        ..Default::default()
    };
    app.raw_input_hook(&mut raw);
    let mut output = ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx().clone();
        app.logic(&ctx);
        app.ui(ui);
        after_ui(app, &ctx);
    });
    output.textures_delta.clear();
    assert!(app.ui.status.is_empty(), "shared UI error: {}", app.ui.status);
    output
}

fn full_ui_fixture() -> (DesignApp, Context, StoryId) {
    let (mut app, sid, _) = fixture(false);
    let ctx = Context::default();
    for _ in 0..6 {
        full_ui_frame(&mut app, &ctx, vec![], |_, _| {});
    }
    assert!(app.fonts_ready && app.canvas_rect.is_some());
    assert!(app.services.request_paste.is_none());
    assert!(!ctx.text_edit_focused());
    assert!(app.session.wants_text());
    assert_eq!(app.session.doc().unwrap().selection.text.unwrap().range(), 1..2);
    (app, ctx, sid)
}

#[test]
fn fc2_native_paste_cancels_before_dispatch_when_selection_changes() {
    let (mut app, ctx, sid) = full_ui_fixture();
    let before = app.session.doc().unwrap().clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app = app.with_control(rx);
    let (paste, paste_reply) = crate::control::ControlRequest::new("ui.menu.invoke", json!({"command": "edit.paste"}));
    let (select, select_reply) =
        crate::control::ControlRequest::new("engine.execute", json!({"command": "text.select", "params": {"story": sid.0, "anchor": 0, "focus": 1}}));
    tx.send(paste).unwrap();
    tx.send(select).unwrap();
    let output = full_ui_frame(&mut app, &ctx, vec![], |_, _| {});
    assert_eq!(paste_reply.try_recv().unwrap()["ok"], true);
    assert_eq!(select_reply.try_recv().unwrap()["ok"], true);
    let state = app.session.doc().unwrap();
    assert_eq!(state.selection.text.unwrap().range(), 0..1);
    assert!(Arc::ptr_eq(&state.doc, &before.doc));
    assert_eq!(state.history.undo.len(), before.history.undo.len());
    assert_eq!(state.history.redo.len(), before.history.redo.len());
    assert_eq!(requests(&output), 0, "cancel an already-stale target before requesting native clipboard data");
}

#[test]
fn fc2_host_paste_cancels_after_input_in_dispatch_frame() {
    let (mut app, ctx, sid) = full_ui_fixture();
    let reads = host_reader(&mut app);
    menus::activate(&mut app, "edit.paste", &Value::Null);
    let mut events = Vec::new();
    for (key, modifiers) in [(egui::Key::Home, egui::Modifiers::NONE), (egui::Key::ArrowRight, egui::Modifiers::SHIFT)] {
        events.push(Event::ModifiersChanged(modifiers));
        for pressed in [true, false] {
            events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
        }
    }
    events.push(Event::ModifiersChanged(egui::Modifiers::NONE));
    let mut completions = 0;
    let output = full_ui_frame(&mut app, &ctx, events, |app, ctx| {
        // The host request is issued before input; deliver its token after the same UI pass.
        let request = reads.lock().unwrap().pop();
        if let Some(request) = request {
            completions += 1;
            let before = app.session.doc().unwrap().clone();
            let journal = app.session.journal.clone();
            assert_eq!(before.selection.text.unwrap().range(), 0..1);
            assert!(app.session.wants_text() && !ctx.text_edit_focused());
            assert!(!app.receive_clipboard_paste(ctx, request, "external".into()));
            let state = app.session.doc().unwrap();
            assert!(Arc::ptr_eq(&state.doc, &before.doc));
            assert_eq!(state.selection, before.selection);
            assert_eq!(state.history.undo.len(), before.history.undo.len());
            assert_eq!(state.history.redo.len(), before.history.redo.len());
            assert_eq!(app.session.journal, journal);
        }
    });
    assert_eq!(completions, 1, "the original target must be valid when the host request is dispatched");
    assert_eq!(requests(&output), 0);
    assert!(reads.lock().unwrap().is_empty());
    assert_eq!(app.session.doc().unwrap().doc.story(sid).unwrap().text, "OLD");
}

#[test]
fn host_paste_cancels_after_input_in_following_frame() {
    let (mut app, ctx, sid) = full_ui_fixture();
    app.run("text.insert", json!({"text": "temporary"})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    app.run("text.select", json!({"story": sid.0, "anchor": 1, "focus": 2})).unwrap();
    let reads = host_reader(&mut app);
    menus::activate(&mut app, "edit.paste", &Value::Null);
    let dispatch = full_ui_frame(&mut app, &ctx, vec![], |_, _| {});
    assert_eq!(requests(&dispatch), 0);
    assert_eq!(reads.lock().unwrap().len(), 1, "dispatch before the frame containing input and the completed read");
    let mut completed = reads.lock().unwrap().pop();
    let before = app.session.doc().unwrap().clone();
    assert_eq!(before.selection.text.unwrap().range(), 1..2);
    assert!(!before.history.redo.is_empty());
    let mut journal = app.session.journal.clone();

    let mut events = Vec::new();
    for (key, modifiers) in [(egui::Key::Home, egui::Modifiers::NONE), (egui::Key::ArrowRight, egui::Modifiers::SHIFT)] {
        events.push(Event::ModifiersChanged(modifiers));
        for pressed in [true, false] {
            events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
        }
    }
    events.push(Event::ModifiersChanged(egui::Modifiers::NONE));
    let mut deliveries = 0;
    let output = full_ui_frame(&mut app, &ctx, events, |app, ctx| {
        if let Some(request) = completed.take() {
            deliveries += 1;
            assert_eq!(app.session.doc().unwrap().selection.text.unwrap().range(), 0..1);
            assert!(app.session.wants_text() && !ctx.text_edit_focused());
            assert!(!app.receive_clipboard_paste(ctx, request, "external".into()));
        }
    });
    assert_eq!(deliveries, 1);
    assert!(completed.is_none() && reads.lock().unwrap().is_empty());
    assert_eq!(requests(&output), 0);
    let state = app.session.doc().unwrap();
    assert_eq!(state.selection.text.unwrap().range(), 0..1);
    assert_eq!(state.doc.story(sid).unwrap().text, "OLD");
    assert!(Arc::ptr_eq(&state.doc, &before.doc));
    for (actual, expected) in [(&state.history.undo, &before.history.undo), (&state.history.redo, &before.history.redo)] {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.label, expected.label);
            assert_eq!(actual.selection, expected.selection);
            assert!(Arc::ptr_eq(&actual.doc, &expected.doc));
        }
    }
    for (dir, extend) in [("lineStart", false), ("right", true)] {
        journal.push(("text.move".into(), json!({"dir": dir, "extend": extend, "word": false, "far": false})));
    }
    assert_eq!(app.session.journal, journal);
}

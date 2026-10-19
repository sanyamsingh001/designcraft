//! C5 EOF-CONSUMER-02: Ruby initialization and confirmation use the staged reading.

use designcraft_doc::{Story, StoryId};
use designcraft_engine::Session;
use serde_json::{Value, json};

use crate::{DesignApp, Services};

fn full_doc(app: &DesignApp) -> Value {
    serde_json::to_value(&*app.session.doc().unwrap().doc).unwrap()
}

fn select(app: &mut DesignApp, sid: StoryId, start: usize, end: usize) {
    app.run("text.select", json!({"story": sid.0, "anchor": start, "focus": end})).unwrap();
}

fn fixture(text: &str) -> (DesignApp, StoryId) {
    let mut app = DesignApp::new(Session::new(), Services::default());
    app.run("file.new", json!({})).unwrap();
    let f = app.run("frame.create", json!({"rect": [40, 40, 440, 500], "content": "text", "text": text})).unwrap();
    let sid = StoryId(f["story"].as_u64().unwrap());
    select(&mut app, sid, text.len(), text.len());
    (app, sid)
}

fn staged(lf: Option<&str>, reading: &str) -> (DesignApp, StoryId, Story) {
    let (mut app, sid) = fixture("A\n");
    if let Some(lf) = lf {
        select(&mut app, sid, 0, 2);
        app.run("type.ruby", json!({"text": lf})).unwrap();
        select(&mut app, sid, 2, 2);
    }
    let original = app.session.doc().unwrap().doc.story(sid).unwrap().clone();
    app.run("type.ruby", json!({"text": reading})).unwrap();
    (app, sid, original)
}

fn open(app: &mut DesignApp, expected: &str) {
    let before = full_doc(app);
    let selection = app.session.doc().unwrap().selection.clone();
    let depth = app.session.doc().unwrap().history.undo.len();
    let journal = app.session.journal.len();
    app.run("app.rubyDialog", json!({})).unwrap();
    let dialog = app.ui.dialog.as_ref().unwrap();
    assert_eq!(dialog.id, "ruby");
    assert_eq!(dialog.fields.get("text"), Some(&json!(expected)), "Ruby must initialize from the active formatting target");
    assert_eq!(full_doc(app), before, "opening the dialog is read-only");
    let st = app.session.doc().unwrap();
    assert_eq!(st.selection, selection);
    assert_eq!(st.history.undo.len(), depth);
    assert_eq!(app.session.journal.len(), journal);
}

fn assert_stage(app: &mut DesignApp, sid: StoryId, reading: &str, original: &Story) {
    let story = app.session.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.paras.last().unwrap().chars.ruby.as_deref(), Some(reading));
    assert_eq!(story.text, original.text);
    assert_eq!(story.chars, original.chars, "Ruby at EOF must not rewrite the preceding LF");
    assert_eq!(story.paras[..story.paras.len() - 1], original.paras[..original.paras.len() - 1]);
    assert_eq!(app.run("type.selectionAttrs", json!({})).unwrap()["chars"]["ruby"], json!(reading));
}

fn confirm_and_history(app: &mut DesignApp, sid: StoryId, reading: &str, original: &Story) {
    let before = full_doc(app);
    let selection = app.session.doc().unwrap().selection.clone();
    let depth = app.session.doc().unwrap().history.undo.len();
    crate::dialogs::confirm(app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_stage(app, sid, reading, original);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
    let after = full_doc(app);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(full_doc(app), before, "undo restores the exact staged document");
    assert_eq!(app.session.doc().unwrap().selection, selection);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(full_doc(app), after, "redo restores the exact confirmed document");
    assert_eq!(app.session.doc().unwrap().selection, selection);
    app.session.doc().unwrap().doc.check().unwrap();
}

fn native_copies(app: &mut DesignApp) -> [DesignApp; 2] {
    let before = full_doc(app);
    let saved = app.run("file.serialize", json!({})).unwrap();
    let zip = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"));
    let legacy = serde_json::to_vec(&*app.session.doc().unwrap().doc).unwrap();
    [zip, legacy].map(|bytes| {
        let mut back = DesignApp::new(Session::new(), Services::default());
        back.run("file.openBytes", json!({"base64": designcraft_engine::cmd::base64_encode(&bytes)})).unwrap();
        assert_eq!(full_doc(&back), before);
        back.session.doc().unwrap().doc.check().unwrap();
        back
    })
}

#[test]
fn r22_ruby_dialog_staged_initialize_confirm_undo_native_and_typing() {
    for lf in [None, Some("LF reading")] {
        for reading in ["かんじ", ""] {
            let (mut app, sid, original) = staged(lf, reading);
            open(&mut app, reading);
            confirm_and_history(&mut app, sid, reading, &original);
            for mut back in native_copies(&mut app) {
                select(&mut back, sid, 2, 2);
                open(&mut back, reading);
                confirm_and_history(&mut back, sid, reading, &original);
                back.run("text.insert", json!({"text": "B", "raw": true})).unwrap();
                let story = back.session.doc().unwrap().doc.story(sid).unwrap();
                assert_eq!(story.text, "A\nB");
                assert_eq!(story.format_after(2).over.ruby.as_deref(), Some(reading), "typed bytes keep the confirmed reading");
                assert_eq!(story.format_after(1), original.format_after(1));
                assert_eq!(story.paras.last().unwrap(), original.paras.last().unwrap(), "typing consumes the Ruby stage");
            }
        }
    }
}

#[test]
fn r22_ruby_dialog_enter_confirmation_and_edit_restore_previous_reading() {
    let (mut app, sid, original) = staged(Some("LF reading"), "staged reading");
    open(&mut app, "staged reading");
    let before = full_doc(&app);
    let depth = app.session.doc().unwrap().history.undo.len();
    let mut fonts_installed = false;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 800.0)).build_ui_state(
        move |ui, app| {
            if fonts_installed {
                crate::dialogs::show(app, ui.ctx());
            } else {
                crate::theme::install_fonts(ui.ctx(), "en");
                ui.ctx().request_repaint();
                fonts_installed = true;
            }
        },
        app,
    );
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    let app = harness.state_mut();
    assert!(app.ui.dialog.is_none(), "Enter confirms through the actual Ruby dialog UI");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
    assert_stage(app, sid, "staged reading", &original);
    let after = full_doc(app);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(full_doc(app), before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(full_doc(app), after);
    open(app, "staged reading");
    app.ui.dialog.as_mut().unwrap().fields.insert("text".into(), json!("edited reading"));
    confirm_and_history(app, sid, "edited reading", &original);
    app.run("edit.undo", json!({})).unwrap();
    assert_stage(app, sid, "staged reading", &original);
    open(app, "staged reading");
}

#[test]
fn r22_ruby_dialog_selected_lf_ordinary_caret_and_unstaged_eof_controls() {
    let (mut app, sid, _) = staged(Some("LF reading"), "staged reading");
    select(&mut app, sid, 1, 2);
    open(&mut app, "LF reading");
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().chars.ruby.as_deref(), Some("staged reading"));
    select(&mut app, sid, 1, 1);
    open(&mut app, "LF reading");
    for text in ["AB", "A\n"] {
        let (mut app, sid) = fixture(text);
        select(&mut app, sid, 0, text.len());
        app.run("type.ruby", json!({"text": "ordinary reading"})).unwrap();
        select(&mut app, sid, text.len(), text.len());
        assert!(app.session.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().chars.is_empty());
        open(&mut app, "ordinary reading");
    }
    let (mut app, sid) = fixture("AB");
    app.run("style.character.create", json!({"name": "Ruby control", "chars": {"ruby": "named reading"}})).unwrap();
    select(&mut app, sid, 0, 2);
    app.run("style.character.apply", json!({"name": "Ruby control"})).unwrap();
    assert_eq!(app.run("type.selectionAttrs", json!({})).unwrap()["chars"]["ruby"], json!("named reading"));
    open(&mut app, ""); // Existing non-EOF initialization reads direct overrides, not resolved named styles.
}

#[test]
fn r22_ruby_dialog_table_and_note_eof_controls() {
    for note in [false, true] {
        let (mut app, sid) = fixture("");
        if note {
            app.run("footnote.insert", json!({"text": "A\n"})).unwrap();
        } else {
            app.run("table.insert", json!({"rows": 1, "cols": 1, "width": 300})).unwrap();
            app.run("text.insert", json!({"text": "A\n", "raw": true})).unwrap();
        }
        let cell = app.session.doc().unwrap().selection.text.unwrap().cell.unwrap();
        app.run("text.move", json!({"dir": "storyStart"})).unwrap();
        app.run("text.move", json!({"dir": "storyEnd", "extend": true})).unwrap();
        app.run("type.ruby", json!({"text": "child reading"})).unwrap();
        app.run("text.move", json!({"dir": "storyEnd"})).unwrap();
        let child = app.session.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap().clone();
        assert!(child.paras.last().unwrap().chars.is_empty());
        open(&mut app, "child reading");
        let mut expected = (*app.session.doc().unwrap().doc).clone();
        // Preserve the existing containing-story invalidation on mutable child access.
        let outer = expected.story_mut(sid).unwrap();
        outer.rev = outer.rev.wrapping_add(1);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(*app.session.doc().unwrap().doc, expected, "child confirmation changes only the containing story revision");
        assert_eq!(app.session.doc().unwrap().doc.text_story(sid, Some(cell)).unwrap(), &child);
    }
}

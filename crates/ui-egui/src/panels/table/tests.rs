use egui_kittest::{Harness, kittest::Queryable};

use super::*;
use designcraft_doc::{CellAddr, ItemId, Selection, StoryId, TextSel};

fn app_with_frame() -> (DesignApp, u64, u64) {
    let mut app = DesignApp::new(designcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"pages": 1})).unwrap();
    let frame = app.run("frame.create", json!({"rect": [36, 36, 336, 500], "content": "text", "caret": true})).unwrap();
    app.select_tool("type");
    (app, frame["story"].as_u64().unwrap(), frame["id"].as_u64().unwrap())
}

fn app_with_table(rows: usize, cols: usize) -> (DesignApp, u64, u64, u64) {
    let (mut app, story, frame) = app_with_frame();
    let table = app.run("table.insert", json!({"rows": rows, "cols": cols})).unwrap()["table"].as_u64().unwrap();
    (app, story, frame, table)
}

fn caret(app: &mut DesignApp, story: u64, frame: u64, cell: Option<CellAddr>) {
    app.session.doc_mut().unwrap().selection =
        Selection::text(TextSel { story: StoryId(story), frame: Some(ItemId(frame)), cell, anchor: 0, focus: 0 });
}

fn panel(app: DesignApp) -> Harness<'static, DesignApp> {
    Harness::builder().with_size(egui::vec2(640.0, 900.0)).build_ui_state(|ui, app| show(app, ui), app)
}

fn choose_style(harness: &mut Harness<'_, DesignApp>, kind: &str, name: &str) {
    harness.get_by_role_and_label(egui::accesskit::Role::ComboBox, kind).click();
    harness.run();
    harness.get_by_role_and_label(egui::accesskit::Role::Button, name).click();
    harness.run();
}

fn table(app: &mut DesignApp, id: u64) -> Value {
    app.run("table.get", json!({"table": id})).unwrap()
}

#[test]
fn cell_style_picker_preserves_caret_cell_range_and_unselected_formatting() {
    for (r0, c0, r1, c1) in [(0, 1, 0, 1), (1, 0, 2, 1), (0, 0, 2, 2)] {
        let (mut app, story, frame, tid) = app_with_table(3, 3);
        app.run("style.cell.create", json!({"name": "Centered", "vj": "center", "insets": 8})).unwrap();
        if (r0, c0) == (r1, c1) {
            caret(&mut app, story, frame, Some(CellAddr { table: tid, row: r0, col: c0 }));
        } else {
            app.run("table.select", json!({"rows": [r0, r1], "cols": [c0, c1]})).unwrap();
        }
        let selection = app.session.doc().unwrap().selection.clone();
        let before = table(&mut app, tid);
        let mut harness = panel(app);
        choose_style(&mut harness, "Cell Style", "Centered");
        let after = table(harness.state_mut(), tid);
        let before_cells = before["cells"].as_array().unwrap();
        for cell in after["cells"].as_array().unwrap() {
            let (row, col) = (cell["row"].as_u64().unwrap() as usize, cell["col"].as_u64().unwrap() as usize);
            if (r0..=r1).contains(&row) && (c0..=c1).contains(&col) {
                assert_eq!(cell["style"], "Centered");
                assert_eq!(cell["vj"], "center");
                assert_eq!(cell["insets"], json!([8.0, 8.0, 8.0, 8.0]));
            } else {
                assert_eq!(cell, before_cells.iter().find(|c| c["row"] == cell["row"] && c["col"] == cell["col"]).unwrap());
            }
        }
        assert_eq!(harness.state().session.doc().unwrap().selection, selection, "choosing a style keeps the selected range");
        harness.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(table(harness.state_mut(), tid)["cells"], before["cells"], "undo restores the original formatting");
    }
}

#[test]
fn cell_style_picker_changes_only_the_selected_merged_owner() {
    let (mut app, _, _, tid) = app_with_table(2, 3);
    app.run("table.select", json!({"rows": [0, 0], "cols": [1, 2]})).unwrap();
    app.run("table.merge", json!({})).unwrap();
    app.run("table.select", json!({"rows": [0, 0], "cols": [1, 1]})).unwrap();
    app.run("style.cell.create", json!({"name": "Merged", "vj": "bottom"})).unwrap();
    let before = table(&mut app, tid);
    let mut harness = panel(app);
    choose_style(&mut harness, "Cell Style", "Merged");
    let after = table(harness.state_mut(), tid);
    for cell in after["cells"].as_array().unwrap() {
        if cell["row"] == 0 && cell["col"] == 1 {
            assert_eq!(cell["style"], "Merged");
            assert_eq!(cell["vj"], "bottom");
            assert_eq!(cell["colSpan"], 2);
        } else {
            assert_eq!(cell, before["cells"].as_array().unwrap().iter().find(|c| c["row"] == cell["row"] && c["col"] == cell["col"]).unwrap());
        }
    }
}

#[test]
fn new_table_style_button_captures_supported_options_and_region_bindings() {
    let (mut app, _, _) = app_with_frame();
    let source = app.run("table.insert", json!({"rows": 2, "cols": 3, "headerRows": 1, "footerRows": 1})).unwrap()["table"].as_u64().unwrap();
    for (name, vj) in [("Head", "center"), ("Body", "top"), ("Foot", "bottom"), ("Left", "center"), ("Right", "bottom")] {
        app.run("style.cell.create", json!({"name": name, "vj": vj})).unwrap();
    }
    app.run(
        "style.table.create",
        json!({"name": "Source", "header": "Head", "body": "Body", "footer": "Foot", "leftColumn": "Left", "rightColumn": "Right"}),
    )
    .unwrap();
    app.run("style.table.apply", json!({"name": "Source"})).unwrap();
    let options = json!({
        "border": {"weight": 3.0, "color": "[Paper]", "tint": 0.7},
        "altRows": {"first": 2, "firstColor": "[Black]", "firstTint": 0.3, "next": 1, "nextColor": "[Paper]", "nextTint": 0.8, "skipFirst": 1, "skipLast": 1},
        "spaceBefore": 17.0, "spaceAfter": 19.0,
    });
    app.run("table.options", options).unwrap();
    let source_info = table(&mut app, source);
    let mut harness = panel(app);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "New Table Style").click();
    harness.run();
    let saved = harness.state().session.doc().unwrap().doc.styles.table.iter().find(|s| s.name == "Table Style 1").unwrap().clone();
    assert_eq!(saved.header.as_deref(), Some("Head"));
    assert_eq!(saved.body.as_deref(), Some("Body"));
    assert_eq!(saved.footer.as_deref(), Some("Foot"));
    assert_eq!(saved.left_column.as_deref(), Some("Left"));
    assert_eq!(saved.right_column.as_deref(), Some("Right"));
    assert_eq!(table(harness.state_mut(), source)["cells"], source_info["cells"], "saving a style does not restyle the source");
    harness.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(!harness.state().session.doc().unwrap().doc.styles.table.iter().any(|s| s.name == "Table Style 1"));
    harness.state_mut().run("edit.redo", json!({})).unwrap();

    // The picker must apply table styles to the entire destination even with a single-cell caret.
    harness.state_mut().run("frame.create", json!({"rect": [360, 36, 560, 500], "content": "text", "caret": true})).unwrap();
    let destination =
        harness.state_mut().run("table.insert", json!({"rows": 2, "cols": 3, "headerRows": 1, "footerRows": 1})).unwrap()["table"].as_u64().unwrap();
    harness.run();
    choose_style(&mut harness, "Table Style", "Table Style 1");
    let destination_info = table(harness.state_mut(), destination);
    assert_eq!(destination_info["style"], "Table Style 1");
    for field in ["border", "altRows", "spaceBefore", "spaceAfter"] {
        assert_eq!(destination_info["options"][field], source_info["options"][field], "{field} is captured from the live table");
    }
    assert_eq!(destination_info["cells"], source_info["cells"], "all representable cell-region styles are reproduced");
}

fn canvas_frame(ctx: &egui::Context, app: &mut DesignApp, events: Vec<egui::Event>) {
    let input =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0))), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| {
        crate::menus::shortcuts(app, ui.ctx());
        crate::canvas::show(app, ui);
    });
    output.textures_delta.clear();
}

fn tab(ctx: &egui::Context, app: &mut DesignApp, shift: bool) {
    let modifiers = egui::Modifiers { shift, ..Default::default() };
    canvas_frame(
        ctx,
        app,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers },
            egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: false, repeat: false, modifiers },
        ],
    );
}

fn cell(app: &DesignApp) -> (usize, usize) {
    let cell = app.session.doc().unwrap().selection.text.unwrap().cell.unwrap();
    (cell.row, cell.col)
}

#[test]
fn canvas_shift_tab_moves_back_without_adding_rows_and_tab_still_moves_forward() {
    let (mut app, story, frame, tid) = app_with_table(2, 2);
    app.run("table.setCell", json!({"table": tid, "rows": [0, 0], "cols": [0, 0], "text": "First"})).unwrap();
    caret(&mut app, story, frame, Some(CellAddr { table: tid, row: 1, col: 1 }));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, "en");
    canvas_frame(&ctx, &mut app, vec![]);
    canvas_frame(&ctx, &mut app, vec![]);
    for expected in [(1, 0), (0, 1), (0, 0), (0, 0)] {
        tab(&ctx, &mut app, true);
        assert_eq!(cell(&app), expected);
        assert_eq!(table(&mut app, tid)["rows"].as_array().unwrap().len(), 2);
    }
    assert_eq!(app.ui.hidden_panels, 0, "Shift+Tab is cell navigation while typing");
    assert_eq!(app.session.doc().unwrap().selection.text.unwrap().range(), 0..5, "reverse navigation selects the preceding cell's text");
    tab(&ctx, &mut app, false);
    assert_eq!(cell(&app), (0, 1));
    caret(&mut app, story, frame, Some(CellAddr { table: tid, row: 1, col: 1 }));
    tab(&ctx, &mut app, false);
    assert_eq!(cell(&app), (2, 0));
    assert_eq!(table(&mut app, tid)["rows"].as_array().unwrap().len(), 3);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(table(&mut app, tid)["rows"].as_array().unwrap().len(), 2, "forward Tab's added row remains undoable");
}

#[test]
fn canvas_shift_tab_skips_covered_merged_cells() {
    let (mut app, story, frame, tid) = app_with_table(2, 3);
    app.run("table.select", json!({"rows": [0, 0], "cols": [1, 2]})).unwrap();
    app.run("table.merge", json!({})).unwrap();
    caret(&mut app, story, frame, Some(CellAddr { table: tid, row: 1, col: 0 }));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, "en");
    canvas_frame(&ctx, &mut app, vec![]);
    tab(&ctx, &mut app, true);
    assert_eq!(cell(&app), (0, 1), "the preceding merged owner is selected, not its covered cell");
    assert_eq!(table(&mut app, tid)["rows"].as_array().unwrap().len(), 2);
}

#[test]
fn canvas_shift_tab_in_plain_text_keeps_inserting_a_tab() {
    let (mut app, story, _) = app_with_frame();
    app.run("text.insert", json!({"text": "Text"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, "en");
    canvas_frame(&ctx, &mut app, vec![]);
    tab(&ctx, &mut app, true);
    assert_eq!(app.session.doc().unwrap().doc.story(StoryId(story)).unwrap().text, "Text\t");
    tab(&ctx, &mut app, false);
    assert_eq!(app.session.doc().unwrap().doc.story(StoryId(story)).unwrap().text, "Text\t\t");
}

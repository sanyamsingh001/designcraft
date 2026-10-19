//! Issue #174: raw egui events reach the canvas's actual input handler and public Session.
//! No tests inject designcraft_tools::PointerKind::DoubleClick or use ui.pointer.

use std::sync::Arc;

use designcraft_compose::Line;
use designcraft_doc::{ItemId, SpreadRef, StoryId};
use designcraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Sense, vec2};
use serde_json::json;

use super::{Xf, handle_input};
use crate::{DesignApp, Services, View, control};

#[derive(Debug)]
struct Flags {
    pressed: bool,
    released: bool,
    double: bool,
}

struct Canvas {
    ctx: egui::Context,
    app: DesignApp,
    frame: ItemId,
    story: StoryId,
    time: f64,
}

impl Canvas {
    fn new(text: &str) -> Self {
        let mut session = Session::new();
        session.execute("file.new", &json!({})).unwrap();
        let r = session.execute("frame.create", &json!({"rect": [40, 40, 440, 340], "content": "text", "text": text})).unwrap();
        session.set_tool("type");
        let mut app = DesignApp::new(session, Services::default());
        *app.view_mut().unwrap() = View { zoom: 1.0, origin: designcraft_geom::Point::ZERO, fitted: true, rotation: 0 };
        app.ui.rulers = false;
        app.ui.guides = false;
        let mut c = Self {
            ctx: egui::Context::default(),
            app,
            frame: ItemId(r["id"].as_u64().unwrap()),
            story: StoryId(r["story"].as_u64().unwrap()),
            time: 0.0,
        };
        // egui hit-tests the previous frame's widget rectangles.
        c.tick(vec![], Modifiers::NONE);
        c.tick(vec![], Modifiers::NONE);
        c
    }

    fn tick(&mut self, mut events: Vec<Event>, modifiers: Modifiers) -> Flags {
        self.time += 0.025;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        // This is the native app's real control-event drain, one pointer event per frame.
        self.app.raw_input_hook(&mut raw);
        let mut flags = Flags { pressed: false, released: false, double: false };
        let mut output = self.ctx.run_ui(raw, |ui| {
            flags = ui.input(|i| Flags {
                pressed: i.pointer.primary_pressed(),
                released: i.pointer.primary_released(),
                double: i.pointer.button_double_clicked(PointerButton::Primary),
            });
            let rect = ui.available_rect_before_wrap();
            self.app.canvas_rect = Some(rect);
            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            // Same widget allocation and handler as canvas::show; no rendering worker needed.
            handle_input(&mut self.app, ui, &response, rect);
        });
        output.textures_delta.clear();
        assert!(self.app.ui.status.is_empty(), "canvas command error: {}", self.app.ui.status);
        flags
    }

    fn line(&self, para: usize) -> Line {
        let s = &self.app.session;
        s.cache.get(&s.doc().unwrap().doc, self.story, None).frames[0].lines.iter().find(|l| l.para == para).unwrap().clone()
    }

    fn screen(&self, l: &Line, x: f64) -> Pos2 {
        let d = &self.app.session.doc().unwrap().doc;
        let it = d.item(self.frame).unwrap();
        let loc = d.find(self.frame).unwrap();
        let spread = d.parent_xf(&loc) * d.text_xf(it) * designcraft_geom::Point::new(x, l.baseline - l.ascent * 0.5);
        let canvas = self.app.session.layout().xf(SpreadRef::Doc(0)) * spread;
        let p = Xf::new(self.app.canvas_rect.unwrap(), self.app.view().unwrap()).to_screen(canvas);
        assert!(self.app.canvas_rect.unwrap().contains(p), "fixture point must be in the real canvas");
        p
    }

    fn glyph(&self, byte: usize) -> Pos2 {
        let s = &self.app.session;
        let cs = s.cache.get(&s.doc().unwrap().doc, self.story, None);
        let l = cs.frames[0].lines.iter().find(|l| l.glyphs.iter().any(|g| g.byte == byte && g.len > 0)).unwrap();
        let g = l.glyphs.iter().find(|g| g.byte == byte && g.len > 0).unwrap();
        self.screen(l, g.x + g.adv * 0.25)
    }

    fn button(&mut self, p: Pos2, pressed: bool, modifiers: Modifiers) -> Flags {
        self.tick(vec![Event::PointerMoved(p), button(p, pressed, modifiers)], modifiers)
    }

    fn range(&self) -> std::ops::Range<usize> {
        self.app.session.doc().unwrap().selection.text.unwrap().range()
    }

    fn text(&self) -> String {
        self.app.session.doc().unwrap().doc.story(self.story).unwrap().text.clone()
    }

    fn double_click(&mut self, p: Pos2) {
        self.tick(vec![Event::PointerMoved(p)], Modifiers::NONE);
        let down = self.button(p, true, Modifiers::NONE);
        assert!(down.pressed && !down.released && !down.double, "first physical press: {down:?}");
        let up = self.button(p, false, Modifiers::NONE);
        assert!(!up.pressed && up.released && !up.double, "first physical release: {up:?}");
        let down = self.button(p, true, Modifiers::NONE);
        assert!(down.pressed && !down.released && !down.double, "second physical press: {down:?}");
        let up = self.button(p, false, Modifiers::NONE);
        assert!(!up.pressed && up.released && up.double, "second physical release: {up:?}");
    }

    fn drag(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) {
        self.time += 1.0; // A drag/drop control must not inherit double-click timing.
        self.tick(vec![Event::PointerMoved(from)], modifiers);
        self.button(from, true, modifiers);
        self.tick(vec![Event::PointerMoved(to)], modifiers);
        self.button(to, false, modifiers);
    }

    fn control(&mut self, method: &str, params: serde_json::Value) {
        let (request, _receiver) = control::ControlRequest::new(method, params);
        let control::Outcome::Done(response) = control::handle(&mut self.app, &self.ctx, &request) else {
            panic!("expected immediate control response")
        };
        assert_eq!(response["ok"], true, "{response}");
        assert!(!self.app.synthetic.is_empty(), "control must enqueue real egui events");
        while !self.app.synthetic.is_empty() {
            self.tick(vec![], Modifiers::NONE);
        }
    }

    fn format_blank(&mut self, original: &[Line; 3]) {
        assert_eq!(self.range(), 6..7, "the actual canvas drag must select the blank paragraph's newline");
        let d = &self.app.session.doc().unwrap().doc;
        let neighbors = [
            serde_json::to_value(d.story(self.story).unwrap().format_after(5)).unwrap(),
            serde_json::to_value(d.story(self.story).unwrap().format_after(7)).unwrap(),
        ];
        self.app.run("type.char", json!({"attrs": {"leading": {"kind": "points", "value": 36}}})).unwrap();
        let d = &self.app.session.doc().unwrap().doc;
        assert_eq!(
            [
                serde_json::to_value(d.story(self.story).unwrap().format_after(5)).unwrap(),
                serde_json::to_value(d.story(self.story).unwrap().format_after(7)).unwrap()
            ],
            neighbors
        );
        let blank = self.line(1);
        assert!((blank.leading - 36.0).abs() < 1e-6, "actual canvas-selected separator must change composed blank leading: {}", blank.leading);
        assert!(blank.baseline > original[1].baseline + 20.0);
        assert_eq!(self.line(0).baseline, original[0].baseline);
        for i in [0, 2] {
            assert_eq!(self.line(i).leading, original[i].leading);
        }
        assert!(self.line(2).baseline > original[2].baseline + 20.0);
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

#[test]
fn r7a_selection_egui_double_click_is_a_release_control() {
    let mut c = Canvas::new("Hello brave world");
    let p = c.glyph(8);
    c.double_click(p);
    // This test controls egui's signal timing independently of the selected range.
}

#[test]
fn r7a_selection_physical_double_click_selects_word_after_release() {
    for (text, byte) in [("Hello brave world", 8), ("Hello café world", 9)] {
        let mut c = Canvas::new(text);
        let original = c.app.session.doc().unwrap().doc.clone();
        let undo = c.app.session.doc().unwrap().history.undo.len();
        let p = c.glyph(byte);
        c.double_click(p);
        assert_eq!(c.range(), 6..11, "physical second release must select the clicked word in {text:?}");
        c.tick(vec![], Modifiers::NONE);
        assert_eq!(c.range(), 6..11, "word must remain selected on the next frame");
        assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &original));
        assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo, "selecting a word must not mutate the document");
        c.app.run("text.insert", json!({"text": "REPLACED"})).unwrap();
        assert_eq!(c.text(), "Hello REPLACED world", "typing must replace the whole selected word");
        let replaced = c.app.session.doc().unwrap().doc.clone();
        c.app.run("edit.undo", json!({})).unwrap();
        assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &original));
        assert_eq!(c.text(), text);
        c.app.run("edit.redo", json!({})).unwrap();
        assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &replaced));
    }
}

#[test]
fn r7a_selection_coalesced_double_click_selects_word_after_release() {
    for batch in 0..3 {
        let mut c = Canvas::new("Hello brave world");
        let p = c.glyph(8);
        c.tick(vec![Event::PointerMoved(p)], Modifiers::NONE);
        let events = if batch == 0 {
            vec![
                button(p, true, Modifiers::NONE),
                button(p, false, Modifiers::NONE),
                button(p, true, Modifiers::NONE),
                button(p, false, Modifiers::NONE),
            ]
        } else if batch == 1 {
            // The first click and second press coalesce; the physical second release follows.
            let first =
                c.tick(vec![button(p, true, Modifiers::NONE), button(p, false, Modifiers::NONE), button(p, true, Modifiers::NONE)], Modifiers::NONE);
            assert!(first.pressed && first.released && !first.double);
            vec![button(p, false, Modifiers::NONE)]
        } else {
            let first = c.tick(vec![button(p, true, Modifiers::NONE), button(p, false, Modifiers::NONE)], Modifiers::NONE);
            assert!(first.pressed && first.released && !first.double);
            vec![button(p, true, Modifiers::NONE), button(p, false, Modifiers::NONE)]
        };
        let second = c.tick(events, Modifiers::NONE);
        assert_eq!(second.pressed, batch != 1);
        assert!(second.released && second.double, "coalesced egui release signal: {second:?}");
        assert_eq!(c.range(), 6..11, "coalesced double click must keep its word after Up");
        assert!(!c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", c.app.pane)))).unwrap());
        // A later release must not reuse a stale canvas press or collapse the word.
        c.button(p, false, Modifiers::NONE);
        assert_eq!(c.range(), 6..11);
        c.tick(vec![], Modifiers::NONE);
        assert_eq!(c.range(), 6..11);
    }
}

#[test]
fn r7a_selection_repeated_physical_double_click_keeps_word_selection() {
    let mut c = Canvas::new("Hello brave world");
    let p = c.glyph(8);
    c.double_click(p);
    assert_eq!(c.range(), 6..11);
    c.time += 1.0;
    c.double_click(p);
    assert_eq!(c.range(), 6..11, "double-clicking the selected word must not turn its release into a text drop or collapsed caret");
    assert_eq!(c.text(), "Hello brave world");
}

#[test]
fn r7a_selection_physical_text_move_copy_and_click_controls() {
    let mut c = Canvas::new("Hello brave world");
    c.app.session.execute("text.select", &json!({"story": c.story.0, "anchor": 6, "focus": 12})).unwrap();
    let l = c.line(0);
    let from = c.glyph(8);
    let to = c.screen(&l, l.end_x + 24.0);
    let before = c.app.session.doc().unwrap().doc.clone();
    let undo = c.app.session.doc().unwrap().history.undo.len();
    c.drag(from, to, Modifiers::NONE);
    assert_eq!(c.text(), "Hello worldbrave ");
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
    let after = c.app.session.doc().unwrap().doc.clone();
    c.app.run("edit.undo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &before));
    c.app.run("edit.redo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &after));
    let from = c.glyph(13);
    let to = c.glyph(0);
    c.drag(from, to, Modifiers { alt: true, ..Modifiers::NONE });
    assert_eq!(c.text(), "brave Hello worldbrave ");
    let p = c.glyph(2);
    c.time += 1.0;
    c.tick(vec![Event::PointerMoved(p)], Modifiers::NONE);
    c.button(p, true, Modifiers::NONE);
    c.button(p, false, Modifiers::NONE);
    assert_eq!(c.range(), 2..2, "single click inside selection still collapses it");
    assert_eq!(c.text(), "brave Hello worldbrave ");
}

#[test]
fn r7a_selection_physical_blank_paragraph_drag_formats_leading() {
    let mut c = Canvas::new("First\n\nLast");
    let original = [c.line(0), c.line(1), c.line(2)];
    let from = c.screen(&original[1], original[1].end_x);
    let to = c.screen(&original[1], original[1].end_x + 24.0);
    c.drag(from, to, Modifiers::NONE);
    c.format_blank(&original);
}

#[test]
fn r7a_selection_native_control_click_queue_selects_word() {
    let mut c = Canvas::new("Hello world\n\nTail");
    let p = c.glyph(1);
    c.control("ui.click", json!({"x": p.x, "y": p.y, "count": 2}));
    assert_eq!(c.range(), 0..5, "ui.click count=2 must traverse raw_input_hook and the actual canvas handler");
    c.tick(vec![], Modifiers::NONE);
    assert_eq!(c.range(), 0..5);
    c.app.run("text.insert", json!({"text": "REPLACED"})).unwrap();
    assert_eq!(c.text(), "REPLACED world\n\nTail", "same whole-word oracle as Main's qualified Linux ui_shot reproduction");
}

#[test]
fn r7a_selection_native_control_drag_queue_formats_blank_leading() {
    let mut c = Canvas::new("First\n\nLast");
    let original = [c.line(0), c.line(1), c.line(2)];
    let from = c.screen(&original[1], original[1].end_x);
    let to = c.screen(&original[1], original[1].end_x + 24.0);
    c.control("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4}));
    c.format_blank(&original);
}

#[test]
fn r7a_selection_selection_tool_double_click_enters_text_and_finishes_release() {
    for coalesced in [false, true] {
        let mut c = Canvas::new("Hello brave world");
        c.app.session.set_tool("selection");
        let p = c.glyph(8);
        let before = c.app.session.doc().unwrap().doc.clone();
        let undo = c.app.session.doc().unwrap().history.undo.len();
        if coalesced {
            c.tick(vec![Event::PointerMoved(p)], Modifiers::NONE);
            c.tick(
                vec![
                    button(p, true, Modifiers::NONE),
                    button(p, false, Modifiers::NONE),
                    button(p, true, Modifiers::NONE),
                    button(p, false, Modifiers::NONE),
                ],
                Modifiers::NONE,
            );
        } else {
            c.double_click(p);
        }
        assert_eq!(c.app.session.tool_id(), "type", "selection-tool double click must reach its existing text-entry handler on release");
        assert_eq!(c.range(), 8..8);
        assert!(c.app.session.doc().unwrap().interaction.is_none(), "release must leave no pending interaction");
        assert!(!c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", c.app.pane)))).unwrap());
        assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &before));
        assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo);
        c.tick(vec![], Modifiers::NONE);
        assert_eq!(c.range(), 8..8);
    }
}

#[test]
fn r7a_selection_selection_tool_graphic_double_click_then_drag_commits_control() {
    let mut c = Canvas::new("Hello brave world");
    let r = c.app.run("frame.create", json!({"rect": [70, 100, 170, 180], "content": "graphic"})).unwrap();
    let id = ItemId(r["id"].as_u64().unwrap());
    c.app.session.set_tool("selection");
    c.app.ui.snap_to_guides = false;
    c.app.ui.snap_to_document_grid = false;
    c.app.ui.smart_guides = false;
    c.app.ui.align_edges = false;
    c.app.ui.align_centers = false;
    c.app.ui.smart_dimensions = false;
    c.app.ui.smart_spacing = false;
    let before_bounds = c.app.session.doc().unwrap().doc.item(id).unwrap().bounds();
    let canvas = c.app.session.layout().xf(SpreadRef::Doc(0)) * before_bounds.center();
    let from = Xf::new(c.app.canvas_rect.unwrap(), c.app.view().unwrap()).to_screen(canvas);
    c.double_click(from);
    assert_eq!(c.app.session.tool_id(), "selection", "a graphic double click stays in the selection tool");
    assert_eq!(c.app.session.doc().unwrap().selection.items, vec![id]);
    assert!(c.app.session.doc().unwrap().interaction.is_none());
    let before = c.app.session.doc().unwrap().doc.clone();
    let undo = c.app.session.doc().unwrap().history.undo.len();
    let to = from + vec2(60.0, 40.0);
    c.drag(from, to, Modifiers::NONE);
    let after_bounds = c.app.session.doc().unwrap().doc.item(id).unwrap().bounds();
    assert_eq!((after_bounds.x0 - before_bounds.x0, after_bounds.y0 - before_bounds.y0), (60.0, 40.0));
    assert_eq!((after_bounds.width(), after_bounds.height()), (before_bounds.width(), before_bounds.height()));
    assert!(c.app.session.doc().unwrap().interaction.is_none(), "the physical release must commit the move");
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1, "one drag, one history entry");
    let after = c.app.session.doc().unwrap().doc.clone();
    c.app.run("edit.undo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &before));
    c.app.run("edit.redo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &after));
    assert_eq!(c.app.session.doc().unwrap().doc.item(id).unwrap().bounds(), after_bounds);
}

// R8 uses full serialized documents and every history entry, including its selection.
fn r8_full_doc(c: &Canvas) -> serde_json::Value {
    serde_json::to_value(&*c.app.session.doc().unwrap().doc).unwrap()
}

fn r8_history(c: &Canvas) -> serde_json::Value {
    let h = &c.app.session.doc().unwrap().history;
    let entries = |list: &[designcraft_engine::HistoryEntry]| {
        list.iter().map(|e| json!({"label": e.label, "doc": &*e.doc, "selection": e.selection})).collect::<Vec<_>>()
    };
    json!({"undo": entries(&h.undo), "redo": entries(&h.redo), "limit": h.limit})
}

fn r8_spread_point(c: &Canvas, point: designcraft_geom::Point) -> Pos2 {
    let canvas = c.app.session.layout().xf(SpreadRef::Doc(0)) * point;
    let p = Xf::new(c.app.canvas_rect.unwrap(), c.app.view().unwrap()).to_screen(canvas);
    assert!(c.app.canvas_rect.unwrap().contains(p));
    p
}

fn r8_selected_text_corner_double_click_then_type(tool: &str, coalesced: bool) {
    let mut c = Canvas::new("Hello brave world");
    // Keep a real redo branch: a zero-motion double click must not consume or clear it.
    c.app.run("text.select", json!({"story": c.story.0, "anchor": 0, "focus": 0})).unwrap();
    c.app.run("text.insert", json!({"text": "Prior "})).unwrap();
    c.app.run("edit.undo", json!({})).unwrap();
    c.app.run("selection.set", json!({"ids": [c.frame.0]})).unwrap();
    c.app.session.set_tool(tool);
    let label = if tool == "directSelection" { "Move Anchor" } else { "Resize" };
    let bounds = c.app.session.doc().unwrap().doc.item(c.frame).unwrap().bounds();
    // Inside the hit area and within both the resize-handle and anchor tolerances.
    let p = r8_spread_point(&c, designcraft_geom::Point::new(bounds.x0 + 2.0, bounds.y0 + 2.0));
    let before = c.app.session.doc().unwrap().doc.clone();
    let before_doc = r8_full_doc(&c);
    let before_history = r8_history(&c);
    let undo = c.app.session.doc().unwrap().history.undo.len();
    assert_eq!(before_history["redo"].as_array().unwrap().len(), 1);
    c.tick(vec![Event::PointerMoved(p)], Modifiers::NONE);
    if coalesced {
        let flags = c.tick(
            vec![
                button(p, true, Modifiers::NONE),
                button(p, false, Modifiers::NONE),
                button(p, true, Modifiers::NONE),
                button(p, false, Modifiers::NONE),
            ],
            Modifiers::NONE,
        );
        assert!(flags.pressed && flags.released && flags.double, "real coalesced egui double click: {flags:?}");
    } else {
        let flags = c.button(p, true, Modifiers::NONE);
        assert!(flags.pressed && !flags.released && !flags.double);
        assert_eq!(c.app.session.doc().unwrap().interaction.as_ref().unwrap().label, label);
        let flags = c.button(p, false, Modifiers::NONE);
        assert!(!flags.pressed && flags.released && !flags.double);
        assert!(c.app.session.doc().unwrap().interaction.is_none(), "ordinary first release commits its zero-motion interaction");
        let flags = c.button(p, true, Modifiers::NONE);
        assert!(flags.pressed && !flags.released && !flags.double);
        assert_eq!(c.app.session.doc().unwrap().interaction.as_ref().unwrap().label, label);
        let flags = c.button(p, false, Modifiers::NONE);
        assert!(!flags.pressed && flags.released && flags.double, "real physical second release: {flags:?}");
    }
    assert_eq!(c.app.session.tool_id(), "type");
    assert_eq!(c.range(), 0..0);
    assert_eq!(r8_full_doc(&c), before_doc, "entering text must preserve the whole document");
    assert_eq!(r8_history(&c), before_history, "zero motion must preserve every undo/redo entry");
    assert!(!c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", c.app.pane)))).unwrap());
    c.tick(vec![], Modifiers::NONE);
    c.tick(vec![Event::Text("X".into())], Modifiers::NONE);
    assert_eq!(c.text(), "XHello brave world", "typing through egui must reach the new Type tool");
    assert_eq!(c.range(), 1..1);
    let typed = c.app.session.doc().unwrap().doc.clone();
    let typed_doc = r8_full_doc(&c);
    let typed_history = r8_history(&c);
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1, "typing must create its own history entry after the second release");
    assert_eq!(&typed_history["undo"].as_array().unwrap()[..undo], before_history["undo"].as_array().unwrap());
    assert_eq!(typed_history["undo"][undo]["label"], "Type");
    assert_eq!(typed_history["undo"][undo]["doc"], before_doc);
    assert!(typed_history["redo"].as_array().unwrap().is_empty(), "only the new typing edit clears redo");
    assert!(c.app.session.doc().unwrap().interaction.is_none());
    assert_eq!(c.app.run("edit.undo", json!({})).unwrap()["undone"], "Type");
    assert_eq!(r8_full_doc(&c), before_doc, "undo must restore the entire pre-typing document, not remove the frame");
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &before));
    let undone_history = r8_history(&c);
    assert_eq!(undone_history["undo"], before_history["undo"]);
    assert_eq!(undone_history["redo"].as_array().unwrap().len(), 1);
    assert_eq!(undone_history["redo"][0]["doc"], typed_doc);
    assert_eq!(c.app.run("edit.redo", json!({})).unwrap()["redone"], "Type");
    assert_eq!(r8_full_doc(&c), typed_doc);
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &typed));
    assert_eq!(r8_history(&c), typed_history, "redo must reconstruct every typing history entry");
}

#[test]
fn r8_selection_physical_resize_handle_double_click_typing_full_doc_history() {
    r8_selected_text_corner_double_click_then_type("selection", false);
}

#[test]
fn r8_selection_coalesced_resize_handle_double_click_typing_full_doc_history() {
    r8_selected_text_corner_double_click_then_type("selection", true);
}

#[test]
fn r8_selection_physical_direct_anchor_double_click_typing_full_doc_history() {
    r8_selected_text_corner_double_click_then_type("directSelection", false);
}

#[test]
fn r8_selection_coalesced_direct_anchor_double_click_typing_full_doc_history() {
    r8_selected_text_corner_double_click_then_type("directSelection", true);
}

#[test]
fn r8_selection_physical_text_resize_still_commits_full_doc_history() {
    let mut c = Canvas::new("Hello brave world");
    c.app.run("selection.set", json!({"ids": [c.frame.0]})).unwrap();
    c.app.session.set_tool("selection");
    c.app.ui.snap_to_guides = false;
    c.app.ui.snap_to_document_grid = false;
    c.app.ui.smart_guides = false;
    c.app.ui.align_edges = false;
    c.app.ui.align_centers = false;
    c.app.ui.smart_dimensions = false;
    c.app.ui.smart_spacing = false;
    let before_bounds = c.app.session.doc().unwrap().doc.item(c.frame).unwrap().bounds();
    let from = r8_spread_point(&c, designcraft_geom::Point::new(before_bounds.x1, before_bounds.y1));
    let before_doc = r8_full_doc(&c);
    let before_history = r8_history(&c);
    let undo = c.app.session.doc().unwrap().history.undo.len();
    c.drag(from, from + vec2(40.0, 30.0), Modifiers::NONE);
    let after_bounds = c.app.session.doc().unwrap().doc.item(c.frame).unwrap().bounds();
    assert_eq!((after_bounds.x0, after_bounds.y0), (before_bounds.x0, before_bounds.y0));
    let delta = (after_bounds.width() - before_bounds.width(), after_bounds.height() - before_bounds.height());
    // Resize bakes non-integer affine scales into the path; compare document points with the geometry tolerance.
    assert!(
        (delta.0 - 40.0).abs() < designcraft_geom::EPS && (delta.1 - 30.0).abs() < designcraft_geom::EPS,
        "physical resize delta must be (40, 30) within geometry EPS after affine scaling: {delta:?}"
    );
    assert_eq!(c.app.session.tool_id(), "selection");
    assert!(c.app.session.doc().unwrap().interaction.is_none());
    let after_doc = r8_full_doc(&c);
    let after_history = r8_history(&c);
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(&after_history["undo"].as_array().unwrap()[..undo], before_history["undo"].as_array().unwrap());
    assert_eq!(after_history["undo"][undo]["label"], "Resize");
    assert_eq!(after_history["undo"][undo]["doc"], before_doc);
    c.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(r8_full_doc(&c), before_doc);
    assert_eq!(r8_history(&c)["undo"], before_history["undo"]);
    c.app.run("edit.redo", json!({})).unwrap();
    assert_eq!(r8_full_doc(&c), after_doc);
    assert_eq!(r8_history(&c), after_history);
}

// R9 UI-1/UI-2: physical events share one RawInput batch, without synthetic pacing.
fn r9_input_snapshot(c: &Canvas) -> serde_json::Value {
    let st = c.app.session.doc().unwrap();
    json!({"doc": &*st.doc, "selection": st.selection, "history": r8_history(c)})
}

fn r9_input_reference(c: &Canvas) -> Canvas {
    let mut reference = Canvas::new("Hello brave world");
    *reference.app.session.doc_mut().unwrap() = c.app.session.doc().unwrap().clone();
    reference.app.after_engine();
    *reference.app.view_mut().unwrap() = *c.app.view().unwrap();
    reference.frame = c.frame;
    reference.story = c.story;
    assert_eq!(r9_input_snapshot(&reference), r9_input_snapshot(c));
    reference
}

fn r9_input_with_redo(anchor: usize, focus: usize) -> Canvas {
    let mut c = Canvas::new("Hello brave world");
    c.app.run("text.select", json!({"story": c.story.0, "anchor": 0, "focus": 0})).unwrap();
    c.app.run("text.insert", json!({"text": "Prior "})).unwrap();
    c.app.run("edit.undo", json!({})).unwrap();
    c.app.run("text.select", json!({"story": c.story.0, "anchor": anchor, "focus": focus})).unwrap();
    assert_eq!(r8_history(&c)["redo"].as_array().unwrap().len(), 1);
    c
}

fn r9_input_replay(c: &mut Canvas, events: &[Event]) {
    for event in events {
        c.tick(vec![event.clone()], Modifiers::NONE);
    }
}

fn r9_input_finished(c: &Canvas) {
    assert!(c.app.session.doc().unwrap().interaction.is_none());
    assert!(!c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", c.app.pane)))).unwrap());
}

fn r9_input_round_trip(c: &mut Canvas, before: &serde_json::Value) {
    let after = r9_input_snapshot(c);
    c.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(r8_full_doc(c), before["doc"], "undo restores the whole document");
    assert_eq!(r8_history(c)["undo"], before["history"]["undo"]);
    c.app.run("edit.redo", json!({})).unwrap();
    assert_eq!(r9_input_snapshot(c), after, "redo restores document, selection and every history entry");
}

#[test]
fn r9_input_physical_text_then_click_matches_chronological_full_snapshot() {
    let mut c = r9_input_with_redo(0, 0);
    let mut reference = r9_input_reference(&c);
    let before = r9_input_snapshot(&c);
    let p = c.glyph(12);
    let events = vec![Event::Text("Z".into()), Event::PointerMoved(p), button(p, true, Modifiers::NONE), button(p, false, Modifiers::NONE)];
    let flags = c.tick(events.clone(), Modifiers::NONE);
    assert!(flags.pressed && flags.released && !flags.double);
    r9_input_replay(&mut reference, &events);
    assert_eq!(c.text(), "ZHello brave world", "earlier text belongs at the original caret");
    assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference));
    assert_eq!(r8_history(&c)["undo"].as_array().unwrap().len(), before["history"]["undo"].as_array().unwrap().len() + 1);
    assert!(r8_history(&c)["redo"].as_array().unwrap().is_empty());
    r9_input_finished(&c);
    r9_input_round_trip(&mut c, &before);
}

#[test]
fn r9_input_physical_click_then_text_keeps_chronological_control() {
    let mut c = r9_input_with_redo(0, 0);
    let mut reference = r9_input_reference(&c);
    let before = r9_input_snapshot(&c);
    let p = c.glyph(12);
    let events = vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE), button(p, false, Modifiers::NONE), Event::Text("Z".into())];
    c.tick(events.clone(), Modifiers::NONE);
    r9_input_replay(&mut reference, &events);
    assert_eq!(c.text(), "Hello brave Zworld", "later text belongs at the clicked caret");
    assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference));
    r9_input_finished(&c);
    r9_input_round_trip(&mut c, &before);
}

#[test]
fn r9_input_physical_key_and_click_keep_event_order_and_redo() {
    for key_first in [true, false] {
        let mut c = r9_input_with_redo(0, 0);
        let mut reference = r9_input_reference(&c);
        let before = r9_input_snapshot(&c);
        let p = c.glyph(12);
        let key = |pressed| Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        let mut events = vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE), button(p, false, Modifiers::NONE)];
        if key_first {
            events.splice(0..0, [key(true), key(false)]);
        } else {
            events.extend([key(true), key(false)]);
        }
        c.tick(events.clone(), Modifiers::NONE);
        r9_input_replay(&mut reference, &events);
        assert_eq!(c.range(), if key_first { 12..12 } else { 13..13 });
        assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference));
        assert_eq!(r8_full_doc(&c), before["doc"]);
        assert_eq!(r8_history(&c), before["history"], "selection-only raw input retains the entire redo branch");
        r9_input_finished(&c);
    }
}

fn r9_input_release_then_move(previous_frame_down: bool, coalesced_tail: bool) {
    let mut c = r9_input_with_redo(6, 12);
    let mut reference = r9_input_reference(&c);
    let before = r9_input_snapshot(&c);
    let from = c.glyph(8);
    let line = c.line(0);
    let to = c.screen(&line, line.end_x + 24.0);
    assert!(from.distance(to) > 3.0);
    let mut events = vec![Event::PointerMoved(from), button(from, true, Modifiers::NONE)];
    if previous_frame_down {
        c.tick(events.clone(), Modifiers::NONE);
        r9_input_replay(&mut reference, &events);
        events.clear();
    }
    events.push(button(from, false, Modifiers::NONE));
    if coalesced_tail {
        events.push(Event::PointerMoved(to));
    }
    c.tick(events.clone(), Modifiers::NONE);
    r9_input_replay(&mut reference, &events);
    if !coalesced_tail {
        c.tick(vec![Event::PointerMoved(to)], Modifiers::NONE);
        reference.tick(vec![Event::PointerMoved(to)], Modifiers::NONE);
    }
    assert_eq!(c.text(), "Hello brave world", "motion after release cannot move the selected text");
    assert_eq!(c.range(), 8..8, "the caret collapses at the actual release point");
    assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference));
    assert_eq!(r8_full_doc(&c), before["doc"]);
    assert_eq!(r8_history(&c), before["history"], "single click retains every undo/redo entry");
    r9_input_finished(&c);
}

#[test]
fn r9_input_coalesced_down_up_then_move_preserves_document_history_and_redo() {
    r9_input_release_then_move(false, true);
}

#[test]
fn r9_input_previous_frame_down_up_then_move_preserves_document_history_and_redo() {
    r9_input_release_then_move(true, true);
}

#[test]
fn r9_input_release_then_later_frame_motion_is_a_no_held_motion_control() {
    for previous_frame_down in [false, true] {
        r9_input_release_then_move(previous_frame_down, false);
    }
}

#[test]
fn r9_input_coalesced_real_move_and_alt_copy_restore_full_document_history() {
    for copy in [false, true] {
        let mut c = r9_input_with_redo(6, 12);
        let before = r9_input_snapshot(&c);
        let from = c.glyph(8);
        let line = c.line(0);
        let to = c.screen(&line, line.end_x + 24.0);
        let modifiers = Modifiers { alt: copy, ..Modifiers::NONE };
        c.tick(vec![Event::PointerMoved(from), button(from, true, modifiers), Event::PointerMoved(to), button(to, false, modifiers)], modifiers);
        assert_eq!(c.text(), if copy { "Hello brave worldbrave " } else { "Hello worldbrave " });
        assert_eq!(c.range(), if copy { 17..23 } else { 11..17 });
        let history = r8_history(&c);
        let undo = before["history"]["undo"].as_array().unwrap().len();
        assert_eq!(history["undo"].as_array().unwrap().len(), undo + 1);
        assert_eq!(&history["undo"].as_array().unwrap()[..undo], before["history"]["undo"].as_array().unwrap());
        assert!(history["redo"].as_array().unwrap().is_empty());
        r9_input_finished(&c);
        r9_input_round_trip(&mut c, &before);
        // A following ordinary physical click must not retain the copy gesture.
        c.time += 1.0;
        let p = c.glyph(0);
        c.tick(vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE), button(p, false, Modifiers::NONE)], Modifiers::NONE);
        assert_eq!(c.range(), 0..0);
    }
}

#[test]
fn r9_input_alt_release_before_modifier_reset_and_motion_still_copies() {
    let mut c = r9_input_with_redo(6, 12);
    let before = r9_input_snapshot(&c);
    let from = c.glyph(8);
    let line = c.line(0);
    let to = c.screen(&line, line.end_x + 24.0);
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    c.tick(
        vec![
            Event::ModifiersChanged(alt),
            Event::PointerMoved(from),
            button(from, true, alt),
            Event::PointerMoved(to),
            button(to, false, alt),
            Event::ModifiersChanged(Modifiers::NONE),
            Event::PointerMoved(from),
        ],
        Modifiers::NONE,
    );
    assert_eq!(c.text(), "Hello brave worldbrave ", "Alt and the actual target belong to release, before the later reset/move");
    assert_eq!(c.range(), 17..23);
    r9_input_finished(&c);
    r9_input_round_trip(&mut c, &before);
}

#[test]
fn r9_input_coalesced_double_click_uses_release_target_before_later_motion() {
    let mut c = r9_input_with_redo(0, 0);
    let before = r9_input_snapshot(&c);
    let p = c.glyph(8);
    let tail = c.glyph(14);
    let flags = c.tick(
        vec![
            Event::PointerMoved(p),
            button(p, true, Modifiers::NONE),
            button(p, false, Modifiers::NONE),
            button(p, true, Modifiers::NONE),
            button(p, false, Modifiers::NONE),
            Event::PointerMoved(tail),
        ],
        Modifiers::NONE,
    );
    assert!(flags.pressed && flags.released && flags.double);
    assert_eq!(c.range(), 6..11);
    assert_eq!(r8_full_doc(&c), before["doc"]);
    assert_eq!(r8_history(&c), before["history"]);
    r9_input_finished(&c);
    c.tick(vec![], Modifiers::NONE);
    assert_eq!(c.range(), 6..11);
}

#[test]
fn r10_hand_coalesced_motion_matches_separate_frames_and_preserves_document() {
    for coalesced in [false, true] {
        let mut c = r9_input_with_redo(0, 0);
        c.app.session.set_tool("hand");
        let before = r9_input_snapshot(&c);
        let view = *c.app.view().unwrap();
        let from = c.app.canvas_rect.unwrap().min + vec2(100.0, 100.0);
        let mid = from + vec2(20.0, 0.0);
        let to = from + vec2(40.0, 0.0);
        c.tick(vec![Event::PointerMoved(from), button(from, true, Modifiers::NONE)], Modifiers::NONE);
        if coalesced {
            c.tick(vec![Event::PointerMoved(mid), Event::PointerMoved(to)], Modifiers::NONE);
        } else {
            c.tick(vec![Event::PointerMoved(mid)], Modifiers::NONE);
            c.tick(vec![Event::PointerMoved(to)], Modifiers::NONE);
        }
        c.tick(vec![button(to, false, Modifiers::NONE)], Modifiers::NONE);
        let after = *c.app.view().unwrap();
        assert_eq!(after.zoom, view.zoom);
        assert_eq!(after.rotation, view.rotation);
        assert_eq!(after.origin.y, view.origin.y);
        assert_eq!(after.origin.x, view.origin.x - 40.0 / view.zoom, "coalesced={coalesced}: the same screen displacement must pan once");
        assert_eq!(r9_input_snapshot(&c), before, "Hand panning preserves the complete document, selection, Undo and Redo");
        assert!(!c.app.session.tool_busy());
        assert!(c.app.power_zoom.is_none());
        r9_input_finished(&c);
    }
}

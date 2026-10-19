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

// R29-EDIT-01 BEFORE probe: use native RawInput and the real canvas response.
#[test]
fn r30_input_subthreshold_motion_text_before_later_drag_matches_native_replay() {
    let mut candidate = Canvas::new("A\nB");
    let mut reference = Canvas::new("A\nB");
    let line = candidate.line(0);
    assert_eq!(line.range.end, 1);
    let p = candidate.screen(&line, designcraft_compose::caret_x(&line, line.range.end));
    let threshold = candidate.ctx.options(|o| o.input_options.max_click_dist);
    assert!(threshold.is_finite() && threshold > 0.0);
    let small = p + vec2(threshold * 0.5, 0.0);
    let large = p + vec2(threshold + 10.0, 0.0);
    assert!(f64::from(small.x - p.x) > designcraft_geom::EPS);
    assert!(small.distance(p) < threshold, "the early motion must remain below the current threshold");
    assert!(large.distance(p) > threshold, "only the later motion crosses the current threshold");

    for canvas in [&mut candidate, &mut reference] {
        assert_eq!(canvas.ctx.options(|o| o.input_options.max_click_dist), threshold);
        let line = canvas.line(0);
        assert_eq!(line.range.end, 1);
        assert_eq!(canvas.screen(&line, designcraft_compose::caret_x(&line, line.range.end)), p);
        let rect = canvas.app.canvas_rect.unwrap();
        assert!(rect.contains(small) && rect.contains(large));
        let flags = canvas.tick(vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE)], Modifiers::NONE);
        assert!(flags.pressed && !flags.released && !flags.double);
        assert!(canvas.ctx.input(|i| i.pointer.primary_down()));
        assert_eq!(canvas.range(), 1..1, "the separate press frame must place the caret before the LF");
        assert_eq!(canvas.text(), "A\nB");
    }

    let events = vec![Event::PointerMoved(small), Event::Text("Q".into()), Event::PointerMoved(large)];
    candidate.tick(events.clone(), Modifiers::NONE);
    r9_input_replay(&mut reference, &events);

    for canvas in [&mut candidate, &mut reference] {
        assert!(canvas.ctx.input(|i| i.pointer.primary_down()));
        let flags = canvas.tick(vec![button(large, false, Modifiers::NONE)], Modifiers::NONE);
        assert!(!flags.pressed && flags.released);
        r9_input_finished(canvas);
    }

    assert_eq!(reference.text(), "AQ\nB", "chronological input must insert Q before the LF");
    assert_eq!(candidate.text(), reference.text(), "batched motion must not select the LF before the intervening Text event");
}

// R30 compatibility controls: native RawInput, actual widgets, no injected tool/drag state.
#[test]
fn r30_compat_long_hold_keeps_subthreshold_text_drag() {
    for held in [false, true] {
        let mut c = Canvas::new("A\nB");
        let line = c.line(0);
        assert_eq!(line.range.end, 1);
        let p = c.screen(&line, designcraft_compose::caret_x(&line, 1));
        let (distance, duration) = c.ctx.options(|o| (o.input_options.max_click_dist, o.input_options.max_click_duration));
        assert!(distance.is_finite() && distance > 0.0 && duration.is_finite() && duration > 0.1);
        let small = p + vec2(distance * 0.5, 0.0);
        assert!(small.distance(p) < distance && f64::from(small.x - p.x) > designcraft_geom::EPS);
        let flags = c.tick(vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE)], Modifiers::NONE);
        assert!(flags.pressed && !flags.released);
        assert_eq!(c.range(), 1..1);
        let press_time = c.time;
        if held {
            c.time += duration;
            c.tick(vec![], Modifiers::NONE);
            assert_eq!(c.range(), 1..1, "time alone must not emit a tool motion");
        }
        c.tick(vec![Event::PointerMoved(small)], Modifiers::NONE);
        assert!(c.ctx.input(|i| i.pointer.primary_down()));
        assert_eq!(c.time - press_time > duration, held);
        assert_eq!(c.range(), if held { 1..2 } else { 1..1 }, "held={held}: elapsed-time activation preserves the LF selection");
        c.tick(vec![Event::Text("Q".into())], Modifiers::NONE);
        assert_eq!(c.text(), if held { "AQB" } else { "AQ\nB" });
        c.tick(vec![button(small, false, Modifiers::NONE)], Modifiers::NONE);
        r9_input_finished(&c);
    }
}

fn r30_compat_assert_pan(c: &Canvas, before: View, from: Pos2, to: Pos2) {
    let view = c.app.view().unwrap();
    assert_eq!(view.zoom, before.zoom);
    assert_eq!(view.rotation, before.rotation);
    let x = before.origin.x - f64::from(to.x - from.x) / before.zoom;
    let y = before.origin.y - f64::from(to.y - from.y) / before.zoom;
    assert!(
        (view.origin.x - x).abs() < 1e-9 && (view.origin.y - y).abs() < 1e-9,
        "actual pan must follow the current screen position: {view:?}, expected ({x}, {y})"
    );
}

#[test]
fn r30_compat_active_drag_returns_inside_threshold() {
    for coalesced in [false, true] {
        let mut c = r9_input_with_redo(0, 0);
        c.app.session.set_tool("hand");
        let before = r9_input_snapshot(&c);
        let view = *c.app.view().unwrap();
        let distance = c.ctx.options(|o| o.input_options.max_click_dist);
        assert!(distance.is_finite() && distance > 0.0);
        let p = c.app.canvas_rect.unwrap().min + vec2(100.0, 100.0);
        let far = p + vec2(distance + 10.0, 0.0);
        let inside = p + vec2(distance * 0.5, 0.0);
        let later = p + vec2(distance * 0.25, 0.0);
        assert!(far.distance(p) > distance && inside.distance(p) < distance && later.distance(p) < distance);
        let flags = c.tick(vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE)], Modifiers::NONE);
        assert!(flags.pressed && !flags.released);
        let events = vec![Event::PointerMoved(far), Event::PointerMoved(inside)];
        if coalesced {
            c.tick(events, Modifiers::NONE);
        } else {
            r9_input_replay(&mut c, &events);
        }
        r30_compat_assert_pan(&c, view, p, inside);
        c.tick(vec![Event::PointerMoved(later)], Modifiers::NONE);
        r30_compat_assert_pan(&c, view, p, later);
        c.tick(vec![button(later, false, Modifiers::NONE)], Modifiers::NONE);
        assert_eq!(r9_input_snapshot(&c), before);
        assert!(!c.app.session.tool_busy() && c.app.power_zoom.is_none());
        r9_input_finished(&c);
    }
}

// A bounded canvas rectangle leaves room for a real adjacent widget in the same viewport.
// This uses the same raw hook/allocation/handler as Canvas::tick and only observes egui's flag.
fn r30_compat_widget_tick(c: &mut Canvas, rect: egui::Rect, neighbor: bool, mut events: Vec<Event>) -> bool {
    c.time += 0.025;
    events.insert(0, Event::ModifiersChanged(Modifiers::NONE));
    let mut raw =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 700.0))), time: Some(c.time), events, ..Default::default() };
    c.app.raw_input_hook(&mut raw);
    let mut dragged = false;
    let mut output = c.ctx.run_ui(raw, |ui| {
        c.app.canvas_rect = Some(rect);
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if neighbor {
            let _ = ui
                .allocate_rect(egui::Rect::from_min_max(egui::pos2(rect.max.x, rect.min.y), egui::pos2(780.0, rect.max.y)), Sense::click_and_drag());
        }
        dragged = response.dragged_by(PointerButton::Primary);
        handle_input(&mut c.app, ui, &response, rect);
    });
    output.textures_delta.clear();
    assert!(c.app.ui.status.is_empty(), "canvas command error: {}", c.app.ui.status);
    dragged
}

#[test]
fn r30_compat_widget_exit_keeps_hit_margin_and_adjacent_widget_behavior() {
    for neighbor in [false, true] {
        let mut c = r9_input_with_redo(0, 0);
        c.app.session.set_tool("hand");
        let rect = egui::Rect::from_min_max(egui::pos2(40.0, 40.0), egui::pos2(740.0, 640.0));
        for _ in 0..2 {
            r30_compat_widget_tick(&mut c, rect, neighbor, vec![]);
        }
        let before = r9_input_snapshot(&c);
        let view = *c.app.view().unwrap();
        let distance = c.ctx.options(|o| o.input_options.max_click_dist);
        let radius = c.ctx.global_style().interaction.interact_radius;
        assert!(distance.is_finite() && radius.is_finite() && distance > radius && radius > 0.0);
        let inset = (distance - radius) * 0.25;
        let p = egui::pos2(rect.max.x - inset, rect.center().y);
        let near = p + vec2(distance * 0.5, 0.0);
        let outside = egui::pos2(rect.max.x + radius + inset, p.y);
        let returned = p - vec2(inset, 0.0);
        assert!(!rect.contains(near) && rect.distance_sq_to_pos(near) < radius * radius);
        assert!(rect.distance_sq_to_pos(outside) > radius * radius);
        assert!(near.distance(p) < distance && outside.distance(p) < distance && returned.distance(p) < distance);
        r30_compat_widget_tick(&mut c, rect, neighbor, vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE)]);
        assert!(c.ctx.input(|i| i.pointer.primary_down()) && c.app.session.tool_busy());
        let near_drag = r30_compat_widget_tick(&mut c, rect, neighbor, vec![Event::PointerMoved(near)]);
        assert_eq!(near_drag, neighbor, "egui itself distinguishes a nearby canvas hit from an adjacent widget");
        r30_compat_assert_pan(&c, view, p, if neighbor { near } else { p });
        assert!(r30_compat_widget_tick(&mut c, rect, neighbor, vec![Event::PointerMoved(outside)]));
        assert!(c.ctx.input(|i| i.pointer.could_any_button_be_click()), "neither time nor distance may account for this activation");
        r30_compat_assert_pan(&c, view, p, outside);
        r30_compat_widget_tick(&mut c, rect, neighbor, vec![Event::PointerMoved(returned)]);
        r30_compat_assert_pan(&c, view, p, returned);
        r30_compat_widget_tick(&mut c, rect, neighbor, vec![button(returned, false, Modifiers::NONE)]);
        assert_eq!(r9_input_snapshot(&c), before);
        assert!(!c.app.session.tool_busy() && c.app.power_zoom.is_none());
        r9_input_finished(&c);
    }
}

#[test]
fn r30_compat_later_widget_exit_does_not_promote_earlier_text_motion() {
    let mut candidate = Canvas::new("A\nB");
    let mut reference = Canvas::new("A\nB");
    let mut rect = egui::Rect::from_min_max(egui::pos2(40.0, 40.0), egui::pos2(740.0, 640.0));
    for c in [&mut candidate, &mut reference] {
        for _ in 0..2 {
            r30_compat_widget_tick(c, rect, false, vec![]);
        }
    }
    let line = candidate.line(0);
    let p = candidate.screen(&line, designcraft_compose::caret_x(&line, 1));
    let distance = candidate.ctx.options(|o| o.input_options.max_click_dist);
    let radius = candidate.ctx.global_style().interaction.interact_radius;
    assert!(distance.is_finite() && radius.is_finite() && distance > radius && radius > 0.0);
    let inset = (distance - radius) * 0.25;
    rect.max.x = p.x + inset;
    let small = p + vec2(distance * 0.5, 0.0);
    let outside = egui::pos2(rect.max.x + radius + inset, p.y);
    assert!(rect.contains(p) && !rect.contains(small));
    assert!(rect.distance_sq_to_pos(small) < radius * radius && rect.distance_sq_to_pos(outside) > radius * radius);
    assert!(small.distance(p) < distance && outside.distance(p) < distance);
    for c in [&mut candidate, &mut reference] {
        for _ in 0..2 {
            r30_compat_widget_tick(c, rect, false, vec![]);
        }
        let line = c.line(0);
        assert_eq!(line.range.end, 1);
        assert_eq!(c.screen(&line, designcraft_compose::caret_x(&line, 1)), p);
        r30_compat_widget_tick(c, rect, false, vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE)]);
        assert!(c.ctx.input(|i| i.pointer.primary_down()));
        assert_eq!(c.range(), 1..1);
    }
    let events = vec![Event::PointerMoved(small), Event::Text("Q".into()), Event::PointerMoved(outside)];
    assert!(r30_compat_widget_tick(&mut candidate, rect, false, events.clone()));
    for event in events {
        r30_compat_widget_tick(&mut reference, rect, false, vec![event]);
    }
    for c in [&mut candidate, &mut reference] {
        r30_compat_widget_tick(c, rect, false, vec![button(outside, false, Modifiers::NONE)]);
        r9_input_finished(c);
    }
    assert_eq!(reference.text(), "AQ\nB");
    assert_eq!(candidate.text(), reference.text(), "later widget exit must not extend the LF before Text");
}

#[test]
fn r30_compat_hand_hold_keeps_subthreshold_power_zoom_aim() {
    let mut c = r9_input_with_redo(0, 0);
    c.app.session.set_tool("hand");
    let before = r9_input_snapshot(&c);
    let view = *c.app.view().unwrap();
    let rect = c.app.canvas_rect.unwrap();
    let p = rect.center();
    let (distance, duration) = c.ctx.options(|o| (o.input_options.max_click_dist, o.input_options.max_click_duration));
    assert!(distance.is_finite() && distance > 0.0 && duration.is_finite() && duration > 0.0);
    let small = p + vec2(distance * 0.5, 0.0);
    assert!(small.distance(p) < distance && small != p);
    c.tick(vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE)], Modifiers::NONE);
    assert!(c.ctx.input(|i| i.pointer.primary_down()));
    c.time += 0.5;
    c.tick(vec![], Modifiers::NONE);
    assert!(c.app.power_zoom.is_some(), "the independent still-hold timer must start Power Zoom");
    c.time += duration;
    c.tick(vec![], Modifiers::NONE);
    let target = Xf::new(rect, c.app.view().unwrap()).to_canvas(small);
    assert_ne!(c.app.power_zoom.unwrap().1, target, "the small motion must change the actual aim");
    c.tick(vec![Event::PointerMoved(small)], Modifiers::NONE);
    assert_eq!(c.app.power_zoom.unwrap().1, target, "a long-held subthreshold move must still update Power Zoom");
    c.tick(vec![button(small, false, Modifiers::NONE)], Modifiers::NONE);
    let after = *c.app.view().unwrap();
    assert_eq!(after.zoom, view.zoom);
    assert!((after.origin.x - (target.x - f64::from(rect.width()) * 0.5 / view.zoom)).abs() < 1e-9);
    assert!((after.origin.y - (target.y - f64::from(rect.height()) * 0.5 / view.zoom)).abs() < 1e-9);
    assert_eq!(r9_input_snapshot(&c), before);
    assert!(!c.app.session.tool_busy() && c.app.power_zoom.is_none());
    r9_input_finished(&c);
}

// C4 owns these full-app input tests. They use native RawInput and the existing
// document/history snapshots; no clipboard fixture, payload or entropy API is involved.
fn r30_shortcut_native_frame(c: &mut Canvas, events: Vec<Event>) {
    c.time += 1.0 / 60.0;
    let mut raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
        max_texture_side: Some(8192),
        time: Some(c.time),
        focused: true,
        events,
        ..Default::default()
    };
    c.app.raw_input_hook(&mut raw);
    let mut output = c.ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx().clone();
        c.app.logic(&ctx);
        c.app.ui(ui);
    });
    output.textures_delta.clear();
    assert!(c.app.ui.status.is_empty(), "native input command failed: {}", c.app.ui.status);
}

fn r30_shortcut_keys(key: egui::Key, modifiers: Modifiers) -> Vec<Event> {
    vec![
        Event::ModifiersChanged(modifiers),
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
        Event::ModifiersChanged(Modifiers::NONE),
    ]
}

fn r30_shortcut_fixture() -> (Canvas, Vec<ItemId>) {
    let mut c = Canvas::new("AB");
    let other = c.app.run("frame.create", json!({"rect": [470, 40, 530, 100], "content": "unassigned", "shape": "ellipse"})).unwrap();
    let all = vec![c.frame, ItemId(other["id"].as_u64().unwrap())];
    // Seed a real redo branch, then select one real object before the tested input.
    c.app.run("text.select", json!({"story": c.story.0, "anchor": 0, "focus": 0})).unwrap();
    c.app.run("text.insert", json!({"text": "Q"})).unwrap();
    c.app.run("edit.undo", json!({})).unwrap();
    c.app.select_tool("selection");
    c.app.run("selection.set", json!({"ids": [c.frame.0]})).unwrap();
    c.app.integrated_titlebar = true;
    for _ in 0..6 {
        r30_shortcut_native_frame(&mut c, vec![]);
    }
    assert!(c.app.fonts_ready && c.app.canvas_rect.is_some());
    assert!(!c.ctx.text_edit_focused() && c.app.ui.dialog.is_none() && c.app.ui.palette.is_none());
    assert!(c.app.native_shortcuts.is_empty() && c.app.synthetic.is_empty());
    assert!(!c.app.session.tool_busy() && !c.app.session.wants_text());
    assert_eq!(c.app.session.doc().unwrap().selection.items, vec![c.frame]);
    assert_eq!(r8_history(&c)["redo"].as_array().unwrap().len(), 1);
    (c, all)
}

fn r30_shortcut_preserved(c: &Canvas, before: &serde_json::Value) {
    assert_eq!(r8_full_doc(c), before["doc"], "native selection keys preserve the entire Document");
    assert_eq!(r8_history(c), before["history"], "native selection keys preserve every undo/redo entry");
}

fn r30_shortcut_case(keys: &[(egui::Key, Modifiers)], batched: bool, expect_all: bool) {
    let (mut c, all) = r30_shortcut_fixture();
    let before = r9_input_snapshot(&c);
    if batched {
        let events = keys.iter().flat_map(|(key, modifiers)| r30_shortcut_keys(*key, *modifiers)).collect();
        r30_shortcut_native_frame(&mut c, events);
    } else {
        for (key, modifiers) in keys {
            r30_shortcut_native_frame(&mut c, r30_shortcut_keys(*key, *modifiers));
            r30_shortcut_preserved(&c, &before);
            if *key == egui::Key::Escape {
                assert!(c.app.session.doc().unwrap().selection.items.is_empty(), "serial Escape control must clear selection");
            } else if *key == egui::Key::A {
                assert_eq!(c.app.session.doc().unwrap().selection.items, all, "serial Select All control must select both objects");
            }
        }
    }
    r30_shortcut_preserved(&c, &before);
    assert!(c.app.session.doc().unwrap().selection.text.is_none());
    assert!(!c.app.session.tool_busy());
    assert_eq!(
        c.app.session.doc().unwrap().selection.items,
        if expect_all { all } else { vec![] },
        "R30-C4-SELECTION-ORDER ORACLE: native selection follows key order; batched={batched}, keys={keys:?}"
    );
}

#[test]
fn r30_native_escape_then_select_all_matches_serial_object_selection() {
    let command = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    for with_v in [false, true] {
        let mut keys = vec![(egui::Key::Escape, Modifiers::NONE)];
        if with_v {
            keys.push((egui::Key::V, Modifiers::NONE));
        }
        keys.push((egui::Key::A, command));
        for batched in [false, true] {
            r30_shortcut_case(&keys, batched, true);
        }
    }
}

#[test]
fn r30_native_escape_reverse_and_single_key_controls() {
    let command = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    for (keys, expect_all) in [
        (vec![(egui::Key::A, command), (egui::Key::Escape, Modifiers::NONE)], false),
        (vec![(egui::Key::Escape, Modifiers::NONE)], false),
        (vec![(egui::Key::A, command)], true),
    ] {
        for batched in [false, true] {
            r30_shortcut_case(&keys, batched, expect_all);
        }
    }
}

#[test]
fn r30_native_escape_keeps_text_and_shortcut_owners() {
    let command = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    for owner in ["native_menu", "custom_escape", "text"] {
        let (mut c, all) = r30_shortcut_fixture();
        match owner {
            "native_menu" => {
                c.app.native_shortcuts.insert("edit.selectAll".into());
            }
            "custom_escape" => {
                c.app.ui.shortcuts.insert("view.zoomIn".into(), "Escape".into());
            }
            "text" => {
                c.app.select_tool("type");
                c.app.run("text.select", json!({"story": c.story.0, "anchor": 0, "focus": 0})).unwrap();
                assert!(c.app.session.wants_text());
            }
            _ => panic!("unexpected input owner: {owner}"),
        }
        let before = r9_input_snapshot(&c);
        let zoom = c.app.view().unwrap().zoom;
        let mut events = r30_shortcut_keys(egui::Key::Escape, Modifiers::NONE);
        events.extend(r30_shortcut_keys(egui::Key::A, command));
        r30_shortcut_native_frame(&mut c, events);
        r30_shortcut_preserved(&c, &before);
        let expected = match owner {
            "native_menu" => vec![],
            "custom_escape" => all, // Upstream command consumption keeps the bound Escape out of the canvas.
            _ => vec![c.frame],
        };
        assert_eq!(c.app.session.doc().unwrap().selection.items, expected, "existing {owner} ownership");
        if owner == "custom_escape" {
            assert!(c.app.view().unwrap().zoom > zoom, "custom Escape shortcut must still run");
        }
    }
}

#[test]
fn r30_native_escape_keeps_pending_pointer_gesture_order() {
    let command = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    for shared_press_frame in [false, true] {
        let (mut c, _) = r30_shortcut_fixture();
        let before = r9_input_snapshot(&c);
        let point = c.app.session.doc().unwrap().doc.item(c.frame).unwrap().bounds().center();
        let p = r8_spread_point(&c, point);
        let press = vec![Event::PointerMoved(p), button(p, true, Modifiers::NONE)];
        let mut keys = r30_shortcut_keys(egui::Key::Escape, Modifiers::NONE);
        keys.extend(r30_shortcut_keys(egui::Key::A, command));
        if shared_press_frame {
            let mut events = press;
            events.extend(keys);
            r30_shortcut_native_frame(&mut c, events);
        } else {
            r30_shortcut_native_frame(&mut c, press);
            assert!(c.app.session.tool_busy(), "real pointer Down must enter SelectionTool::Pending");
            r30_shortcut_native_frame(&mut c, keys);
        }
        assert!(c.app.session.tool_busy(), "Escape must not reset the pending pointer gesture");
        assert!(c.app.session.doc().unwrap().selection.items.is_empty(), "busy/mixed input retains its existing canvas order");
        r30_shortcut_preserved(&c, &before);
        r30_shortcut_native_frame(&mut c, vec![button(p, false, Modifiers::NONE)]);
        assert!(!c.app.session.tool_busy());
        assert!(c.app.session.doc().unwrap().interaction.is_none());
        r30_shortcut_preserved(&c, &before);
    }
}

#[test]
fn r30_native_shortcuts_respect_modal_palette_and_text_edit_owners() {
    for owner in ["dialog", "palette", "text_edit"] {
        let (mut c, _) = r30_shortcut_fixture();
        let mut field = String::from("owner");
        match owner {
            "dialog" => {
                c.app.ui.dialog = Some(crate::dialogs::Dialog::new("newDocument", json!({})));
            }
            "palette" => {
                c.app.run("app.palette", json!({})).unwrap();
            }
            "text_edit" => {
                for _ in 0..2 {
                    let mut output = c.ctx.run_ui(RawInput::default(), |ui| {
                        ui.text_edit_singleline(&mut field).request_focus();
                    });
                    output.textures_delta.clear();
                }
                assert!(c.ctx.text_edit_focused(), "real egui TextEdit must own keyboard focus");
            }
            _ => panic!("unexpected input owner: {owner}"),
        }
        let before = r9_input_snapshot(&c);
        // Default TextEdit relinquishes focus on Escape before shortcut dispatch.
        // Exercise its actual Ctrl+A ownership; dialogs/palette retain Escape.
        let owner_key = if owner == "text_edit" { egui::Key::A } else { egui::Key::Escape };
        let mut events = if owner == "text_edit" { vec![] } else { r30_shortcut_keys(egui::Key::Escape, Modifiers::NONE) };
        events.extend(r30_shortcut_keys(egui::Key::A, Modifiers { ctrl: true, command: true, ..Modifiers::NONE }));
        let mut raw = RawInput { events, focused: true, ..Default::default() };
        c.app.raw_input_hook(&mut raw);
        let mut owner_key_available = false;
        // Deliberately stop at the menu ownership boundary: rendering/closing dialogs
        // belongs to UI, not the independent C4 projection. The native key stays for its owner.
        let mut output = c.ctx.run_ui(raw, |ui| {
            if owner == "text_edit" {
                assert!(ui.ctx().text_edit_focused(), "TextEdit must own focus at shortcut dispatch");
            }
            crate::menus::shortcuts(&mut c.app, ui.ctx());
            owner_key_available = ui.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { key, pressed: true, .. } if *key == owner_key)));
        });
        output.textures_delta.clear();
        assert!(owner_key_available, "{owner} must retain its native {owner_key:?}");
        assert_eq!(serde_json::to_value(&c.app.session.doc().unwrap().selection).unwrap(), before["selection"]);
        r30_shortcut_preserved(&c, &before);
    }
}

#[test]
fn r30_native_escape_after_focus_loss_keeps_existing_key_order() {
    let (mut c, all) = r30_shortcut_fixture();
    let before = r9_input_snapshot(&c);
    let mut events = r30_shortcut_keys(egui::Key::A, Modifiers { ctrl: true, command: true, ..Modifiers::NONE });
    events.push(Event::WindowFocused(false));
    events.extend(r30_shortcut_keys(egui::Key::Escape, Modifiers::NONE));
    r30_shortcut_native_frame(&mut c, events);
    assert_eq!(c.app.session.doc().unwrap().selection.items, all, "canvas must not dispatch Escape after focus loss");
    r30_shortcut_preserved(&c, &before);
}

// Installed eframe 0.36.2 emits Text before Key on the focused web canvas;
// egui-winit 0.36.2 emits Key before printable Text. Neither order consumes Text.
fn r30_selection_tool_text_events(text_before_key: bool) -> Vec<Event> {
    let key = |pressed| Event::Key { key: egui::Key::V, physical_key: Some(egui::Key::V), pressed, repeat: false, modifiers: Modifiers::NONE };
    let text = Event::Text("v".into());
    if text_before_key { vec![text, key(true), key(false)] } else { vec![key(true), text, key(false)] }
}

fn r30_shortcut_text_case(text_before_key: bool) {
    for batched in [false, true] {
        let (mut c, all) = r30_shortcut_fixture();
        let before = r9_input_snapshot(&c);
        let frames = [
            r30_shortcut_keys(egui::Key::Escape, Modifiers::NONE),
            r30_selection_tool_text_events(text_before_key),
            r30_shortcut_keys(egui::Key::A, Modifiers { ctrl: true, command: true, ..Modifiers::NONE }),
        ];
        if batched {
            r30_shortcut_native_frame(&mut c, frames.into_iter().flatten().collect());
        } else {
            for (index, events) in frames.into_iter().enumerate() {
                r30_shortcut_native_frame(&mut c, events);
                r30_shortcut_preserved(&c, &before);
                if index < 2 {
                    assert!(c.app.session.doc().unwrap().selection.items.is_empty(), "serial Escape/V must leave no selection");
                } else {
                    assert_eq!(c.app.session.doc().unwrap().selection.items, all, "serial Select All must select both objects");
                }
            }
        }
        r30_shortcut_preserved(&c, &before);
        assert_eq!(c.text(), "AB", "Selection tool must not insert its shortcut text");
        assert!(!c.app.session.tool_busy() && !c.app.session.wants_text());
        assert_eq!(
            c.app.session.doc().unwrap().selection.items,
            all,
            "R30-C4-TEXT-KEY-ORDER ORACLE: native selection follows Escape/V/Select All; text_before_key={text_before_key}, batched={batched}"
        );
    }
}

#[test]
fn r30_web_text_before_selection_key_matches_serial() {
    r30_shortcut_text_case(true);
}

#[test]
fn r30_winit_selection_key_before_text_matches_serial() {
    r30_shortcut_text_case(false);
}

#[test]
fn r30_selection_shortcut_text_preserves_typing_custom_and_unpaired_input() {
    let command = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    for text_before_key in [true, false] {
        // A real Type-tool caret must still receive the integration's printable event.
        let (mut c, _) = r30_shortcut_fixture();
        c.app.select_tool("type");
        c.app.run("text.select", json!({"story": c.story.0, "anchor": 0, "focus": 0})).unwrap();
        assert!(c.app.session.wants_text());
        let before = r8_full_doc(&c);
        let undo_count = r8_history(&c)["undo"].as_array().unwrap().len();
        r30_shortcut_native_frame(&mut c, r30_selection_tool_text_events(text_before_key));
        assert_eq!(c.text(), "vAB");
        assert_eq!(c.range(), 1..1);
        assert_eq!(c.app.session.tool_id(), "type");
        assert_eq!(r8_history(&c)["undo"].as_array().unwrap().len(), undo_count + 1);
        assert!(r8_history(&c)["redo"].as_array().unwrap().is_empty());
        c.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(r8_full_doc(&c), before, "one undo must restore the entire pre-typing Document");

        // Even a custom Select All on V owns that key: its Text is not reclassified
        // as the plain Selection-tool shortcut.
        for custom in ["view.zoomIn", "edit.selectAll"] {
            let (mut c, _) = r30_shortcut_fixture();
            c.app.ui.shortcuts.insert(custom.into(), "V".into());
            let before = r9_input_snapshot(&c);
            let zoom = c.app.view().unwrap().zoom;
            let mut events = r30_shortcut_keys(egui::Key::Escape, Modifiers::NONE);
            events.extend(r30_selection_tool_text_events(text_before_key));
            events.extend(r30_shortcut_keys(egui::Key::A, command));
            r30_shortcut_native_frame(&mut c, events);
            assert!(c.app.session.doc().unwrap().selection.items.is_empty(), "custom {custom} keeps existing canvas Escape order");
            if custom == "view.zoomIn" {
                assert!(c.app.view().unwrap().zoom > zoom, "custom V command must run");
            }
            r30_shortcut_preserved(&c, &before);
        }
    }

    // A matching adjacent key is required; arbitrary, unpaired or late Text
    // must not make a previously ineligible mixed-input frame eligible.
    for case in ["unpaired_v", "other_text", "text_after_release"] {
        let (mut c, _) = r30_shortcut_fixture();
        let before = r9_input_snapshot(&c);
        let mut events = r30_shortcut_keys(egui::Key::Escape, Modifiers::NONE);
        match case {
            "unpaired_v" => events.push(Event::Text("v".into())),
            "other_text" => {
                events.push(Event::Text("q".into()));
                events.extend(r30_shortcut_keys(egui::Key::V, Modifiers::NONE));
            }
            "text_after_release" => {
                events.extend(r30_shortcut_keys(egui::Key::V, Modifiers::NONE));
                events.push(Event::Text("v".into()));
            }
            _ => panic!("unexpected input case: {case}"),
        }
        events.extend(r30_shortcut_keys(egui::Key::A, command));
        r30_shortcut_native_frame(&mut c, events);
        assert!(c.app.session.doc().unwrap().selection.items.is_empty(), "{case} keeps existing canvas Escape order");
        r30_shortcut_preserved(&c, &before);
    }
}

// The web producer emits a physical ControlLeft key as well as modifier-state changes.
fn r30_physical_control_key(key: egui::Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

fn r30_web_physical_control_frames() -> [Vec<Event>; 5] {
    let command = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    [
        vec![r30_physical_control_key(egui::Key::Escape, true, Modifiers::NONE), r30_physical_control_key(egui::Key::Escape, false, Modifiers::NONE)],
        r30_selection_tool_text_events(true),
        // eframe::on_keydown calls set_modifiers before emitting ControlLeft.
        vec![Event::ModifiersChanged(command), r30_physical_control_key(egui::Key::ControlLeft, true, command)],
        vec![r30_physical_control_key(egui::Key::A, true, command), r30_physical_control_key(egui::Key::A, false, command)],
        vec![Event::ModifiersChanged(Modifiers::NONE), r30_physical_control_key(egui::Key::ControlLeft, false, Modifiers::NONE)],
    ]
}

#[test]
fn r30_web_physical_control_press_matches_serial_selection() {
    let run = |batched: bool| {
        let (mut c, all) = r30_shortcut_fixture();
        let before = r9_input_snapshot(&c);
        let frames = r30_web_physical_control_frames();
        if batched {
            r30_shortcut_native_frame(&mut c, frames.into_iter().flatten().collect());
        } else {
            for (index, events) in frames.into_iter().enumerate() {
                r30_shortcut_native_frame(&mut c, events);
                r30_shortcut_preserved(&c, &before);
                if index < 3 {
                    assert!(c.app.session.doc().unwrap().selection.items.is_empty(), "serial Escape/V/Control press keeps selection empty");
                } else {
                    assert_eq!(c.app.session.doc().unwrap().selection.items, all, "serial A/release must select both objects");
                }
                if matches!(index, 2 | 3) {
                    assert!(c.ctx.input(|i| i.key_down(egui::Key::ControlLeft) && i.modifiers.ctrl && i.modifiers.command));
                }
            }
        }
        r30_shortcut_preserved(&c, &before);
        assert_eq!(c.text(), "AB", "Selection-tool text must not enter the story");
        assert!(c.app.session.doc().unwrap().selection.text.is_none());
        assert!(!c.app.session.tool_busy() && !c.app.session.wants_text());
        assert!(
            c.ctx.input(|i| {
                i.modifiers == Modifiers::NONE
                    && [egui::Key::Escape, egui::Key::V, egui::Key::A, egui::Key::ControlLeft].into_iter().all(|key| !i.key_down(key))
            }),
            "all physical keys and modifiers must be released"
        );
        c.app.session.doc().unwrap().selection.items.clone()
    };
    let serial = run(false);
    let batched = run(true);
    assert_eq!(batched, serial, "R30-C4-PHYSICAL-CONTROL ORACLE: the producer's physical modifier press must preserve Escape/V/Select All order");
}

#[test]
fn r30_physical_control_preserves_held_escape_and_command_binding() {
    let command = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    // A modifier already held before Escape must not turn Ctrl+Escape into plain Escape.
    let (mut held, _) = r30_shortcut_fixture();
    let before = r9_input_snapshot(&held);
    r30_shortcut_native_frame(&mut held, vec![Event::ModifiersChanged(command), r30_physical_control_key(egui::Key::ControlLeft, true, command)]);
    assert!(held.ctx.input(|i| i.key_down(egui::Key::ControlLeft) && i.modifiers.ctrl));
    r30_shortcut_native_frame(
        &mut held,
        vec![
            r30_physical_control_key(egui::Key::Escape, true, command),
            r30_physical_control_key(egui::Key::Escape, false, command),
            r30_physical_control_key(egui::Key::A, true, command),
            r30_physical_control_key(egui::Key::A, false, command),
        ],
    );
    assert!(held.app.session.doc().unwrap().selection.items.is_empty(), "modified Escape retains the existing canvas order");
    assert!(held.ctx.input(|i| i.key_down(egui::Key::ControlLeft)));
    r30_shortcut_native_frame(
        &mut held,
        vec![Event::ModifiersChanged(Modifiers::NONE), r30_physical_control_key(egui::Key::ControlLeft, false, Modifiers::NONE)],
    );
    assert!(held.app.session.doc().unwrap().selection.items.is_empty());
    assert!(held.ctx.input(|i| !i.key_down(egui::Key::ControlLeft) && i.modifiers == Modifiers::NONE));
    r30_shortcut_preserved(&held, &before);

    // Command lookup still owns a bound physical key and excludes this mixed command batch.
    let (mut bound, _) = r30_shortcut_fixture();
    bound.app.ui.shortcuts.insert("view.zoomIn".into(), "Cmd+ControlLeft".into());
    let before = r9_input_snapshot(&bound);
    let zoom = bound.app.view().unwrap().zoom;
    r30_shortcut_native_frame(&mut bound, r30_web_physical_control_frames().into_iter().flatten().collect());
    assert!(bound.app.view().unwrap().zoom > zoom, "the command bound to ControlLeft must run");
    assert!(bound.app.session.doc().unwrap().selection.items.is_empty(), "bound ControlLeft retains the existing canvas Escape order");
    assert!(bound.ctx.input(|i| !i.key_down(egui::Key::ControlLeft) && i.modifiers == Modifiers::NONE));
    r30_shortcut_preserved(&bound, &before);
}

#[test]
fn fc2_input_double_click_is_classified_at_its_own_release() {
    for split_frame in [false, true] {
        let mut c = Canvas::new("Hello brave world");
        let a = c.glyph(14);
        let b = c.glyph(2);
        let mut events = vec![
            Event::PointerMoved(a),
            button(a, true, Modifiers::NONE),
            button(a, false, Modifiers::NONE),
            button(a, true, Modifiers::NONE),
            button(a, false, Modifiers::NONE),
            Event::Text("X".into()),
        ];
        if split_frame {
            assert!(c.tick(events.clone(), Modifiers::NONE).double);
            assert_eq!(c.text(), "Hello brave X");
            events.clear();
        }
        events.extend([Event::PointerMoved(b), button(b, true, Modifiers::NONE), button(b, false, Modifiers::NONE), Event::Text("!".into())]);
        c.tick(events, Modifiers::NONE);
        assert_eq!(c.text(), "He!llo brave X", "each release must affect only its own word/caret");
        assert_eq!(c.range(), 3..3);
        r9_input_finished(&c);
    }
}

#[test]
fn fc2_input_triple_release_does_not_replay_the_double_click() {
    let mut c = Canvas::new("Hello brave world");
    let p = c.glyph(8);
    let flags = c.tick(
        vec![
            Event::PointerMoved(p),
            button(p, true, Modifiers::NONE),
            button(p, false, Modifiers::NONE),
            button(p, true, Modifiers::NONE),
            button(p, false, Modifiers::NONE),
            button(p, true, Modifiers::NONE),
            button(p, false, Modifiers::NONE),
        ],
        Modifiers::NONE,
    );
    assert!(flags.double, "the frame contains a double release followed by a triple release");
    assert_eq!(c.range(), 8..8);
    c.tick(vec![Event::Text("!".into())], Modifiers::NONE);
    assert_eq!(c.text(), "Hello br!ave world");
    r9_input_finished(&c);
}

// Exercise the merged split-pane caller, including its pre-dispatch focus choice.
// Unlike the earlier canvas-only fixture, retain platform output to count Copy/Cut dispatches.
fn r15_split_frame(c: &mut Canvas, events: Vec<Event>) -> Vec<String> {
    c.time += 1.0 / 60.0;
    let mut raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
        max_texture_side: Some(8192),
        time: Some(c.time),
        focused: true,
        events,
        ..Default::default()
    };
    c.app.raw_input_hook(&mut raw);
    let mut output = c.ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx().clone();
        c.app.logic(&ctx);
        c.app.ui(ui);
    });
    output.textures_delta.clear();
    assert!(c.app.ui.status.is_empty(), "split input command failed: {}", c.app.ui.status);
    output
        .platform_output
        .commands
        .into_iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn r15_split_glyph(c: &mut Canvas, pane: u8, byte: usize) -> Pos2 {
    let previous = c.app.pane;
    c.app.switch_pane(pane);
    let point = c.glyph(byte);
    c.app.switch_pane(previous);
    point
}

fn r15_split_fixture(pane: u8) -> Canvas {
    let mut c = r9_input_with_redo(0, 0);
    c.app.split = true;
    c.app.integrated_titlebar = true;
    for p in [0, 1] {
        c.app.switch_pane(p);
        *c.app.view_mut().unwrap() = View { zoom: 1.0, origin: designcraft_geom::Point::ZERO, fitted: true, rotation: 0 };
    }
    c.app.switch_pane(0);
    for _ in 0..6 {
        assert!(r15_split_frame(&mut c, vec![]).is_empty());
    }
    assert!(c.app.fonts_ready && c.app.canvas_rect.is_some());
    let point = r15_split_glyph(&mut c, pane, 0);
    let click = vec![Event::PointerMoved(point), button(point, true, Modifiers::NONE), button(point, false, Modifiers::NONE)];
    assert!(r15_split_frame(&mut c, click).is_empty());
    assert_eq!((c.app.focus_pane, c.app.pane), (pane, pane), "the real split UI must focus the clicked pane");
    assert!(!c.ctx.text_edit_focused() && c.app.ui.dialog.is_none() && c.app.ui.palette.is_none());
    assert!(c.app.native_shortcuts.is_empty() && c.app.synthetic.is_empty());
    assert_eq!(c.range(), 0..0);
    assert_eq!(r8_history(&c)["redo"].as_array().unwrap().len(), 1);
    r9_input_finished(&c);
    c
}

// Keep a real focused UI field outside the extracted split-canvas dispatcher.
#[test]
fn r51_focused_quick_apply_text_then_split_canvas_hover() {
    let mut c = r15_split_fixture(0);
    let before = r9_input_snapshot(&c);
    let pane_before = (c.app.focus_pane, c.app.pane);
    c.app.switch_pane(1);
    let other_canvas = c.app.canvas_rect.unwrap();
    c.app.switch_pane(pane_before.1);
    // Derive the hover from the actual other-pane rectangle, not a screen coordinate.
    let hover = other_canvas.shrink(1.0).right_top();
    assert!(other_canvas.contains(hover));
    c.app.run("app.palette", json!({})).unwrap();
    // Match the existing Quick Apply focused-field fixture's three UI frames.
    for _ in 0..3 {
        assert!(r15_split_frame(&mut c, vec![]).is_empty());
    }
    assert!(c.app.split && c.app.session.wants_text());
    assert!(c.ctx.text_edit_focused());
    assert_eq!(c.app.ui.palette.as_deref(), Some(""));
    let field_id = c.ctx.memory(|m| m.focused()).unwrap();
    assert!(egui::TextEdit::load_state(&c.ctx, field_id).is_some());
    let palette_rect = c.ctx.memory(|m| m.area_rect(egui::Id::new("palette"))).unwrap();
    assert!(!palette_rect.contains(hover), "the trailing hover must be outside Quick Apply's content");
    assert_eq!((c.app.focus_pane, c.app.pane), pane_before);
    assert_eq!(r9_input_snapshot(&c), before);
    assert!(c.app.synthetic.is_empty());

    // One native batch: Text belongs to the actual field, followed only by canvas hover.
    let events = vec![Event::Text("order51-field".into()), Event::PointerMoved(hover)];
    assert!(r15_split_frame(&mut c, events).is_empty());
    assert_eq!(c.app.ui.palette.as_deref(), Some("order51-field"), "the field receives Text exactly once");
    assert!(c.ctx.text_edit_focused());
    assert_eq!(c.ctx.memory(|m| m.focused()), Some(field_id), "hover cannot transfer field focus");
    assert_eq!(c.ctx.input(|i| i.pointer.hover_pos()), Some(hover));
    assert!(!c.ctx.input(|i| i.pointer.any_down()));
    assert_eq!((c.app.focus_pane, c.app.pane), pane_before, "hover cannot switch keyboard panes");
    assert_eq!(r9_input_snapshot(&c), before, "field input preserves the complete Doc, selection and both history stacks");
    for pane in [0u8, 1] {
        assert_eq!(c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", pane)))), Some(false));
    }
    r9_input_finished(&c);
    assert!(!c.app.session.tool_busy() && c.app.synthetic.is_empty());
}

#[test]
fn r15_split_text_and_paste_dispatch_once_in_either_pane() {
    for pane in [0, 1] {
        for (event, command) in [(Event::Text("Z".into()), "text.insert"), (Event::Paste("Z".into()), "edit.paste")] {
            let mut c = r15_split_fixture(pane);
            let before = r9_input_snapshot(&c);
            let mut reference = r9_input_reference(&c);
            reference.app.run(command, json!({"text": "Z"})).unwrap();
            assert!(r15_split_frame(&mut c, vec![event]).is_empty());
            assert_eq!(c.text(), "ZHello brave world", "{command} must insert once in pane {pane}");
            assert_eq!(c.range(), 1..1);
            assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference), "{command}, pane {pane}");
            // Plain Paste inherits the public command's history semantics on this topic.
            // Keep Text's one-edit contract; both events must match exactly one command.
            if command == "text.insert" {
                assert_eq!(r8_history(&c)["undo"].as_array().unwrap().len(), before["history"]["undo"].as_array().unwrap().len() + 1);
            }
            assert!(r8_history(&c)["redo"].as_array().unwrap().is_empty());
            r9_input_finished(&c);
            let after = r9_input_snapshot(&c);
            c.app.run("edit.undo", json!({})).unwrap();
            reference.app.run("edit.undo", json!({})).unwrap();
            assert_eq!(r8_full_doc(&c), before["doc"], "{command} undo restores the full document in pane {pane}");
            assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference), "{command} undo, pane {pane}");
            if command == "text.insert" {
                assert_eq!(r8_history(&c)["undo"], before["history"]["undo"]);
            }
            c.app.run("edit.redo", json!({})).unwrap();
            reference.app.run("edit.redo", json!({})).unwrap();
            assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference), "{command} redo, pane {pane}");
            assert_eq!(r9_input_snapshot(&c), after, "{command} redo restores the full snapshot in pane {pane}");
        }
    }
}

#[test]
fn r15_split_copy_preserves_history_and_cut_is_one_edit_in_either_pane() {
    for pane in [0, 1] {
        let mut c = r15_split_fixture(pane);
        c.app.run("text.select", json!({"story": c.story.0, "anchor": 6, "focus": 11})).unwrap();
        let before = r9_input_snapshot(&c);
        let mut reference = r9_input_reference(&c);
        reference.app.run("edit.copy", json!({})).unwrap();
        assert_eq!(r15_split_frame(&mut c, vec![Event::Copy]), vec!["brave".to_string()]);
        assert_eq!(r9_input_snapshot(&c), before, "Copy preserves the document, selection and complete undo/redo history");
        reference.app.run("edit.cut", json!({})).unwrap();
        assert_eq!(r15_split_frame(&mut c, vec![Event::Cut]), vec!["brave".to_string()]);
        assert_eq!(c.text(), "Hello  world");
        assert_eq!(c.range(), 6..6);
        assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference));
        assert_eq!(r8_history(&c)["undo"].as_array().unwrap().len(), before["history"]["undo"].as_array().unwrap().len() + 1);
        assert!(r8_history(&c)["redo"].as_array().unwrap().is_empty());
        r9_input_finished(&c);
        r9_input_round_trip(&mut c, &before);
    }
}

#[test]
fn r15_split_pane_switch_keeps_click_and_text_chronological() {
    for target in [0, 1] {
        for text_first in [false, true] {
            let mut c = r15_split_fixture(1 - target);
            let mut serial = r15_split_fixture(1 - target);
            *serial.app.session.doc_mut().unwrap() = c.app.session.doc().unwrap().clone();
            serial.frame = c.frame;
            serial.story = c.story;
            serial.app.views = c.app.views.clone();
            serial.app.after_engine();
            let before = r9_input_snapshot(&c);
            assert_eq!(r9_input_snapshot(&serial), before);
            let point = r15_split_glyph(&mut c, target, 12);
            assert_eq!(r15_split_glyph(&mut serial, target, 12), point);
            let mut events = vec![Event::PointerMoved(point), button(point, true, Modifiers::NONE), button(point, false, Modifiers::NONE)];
            if text_first {
                events.insert(0, Event::Text("Z".into()));
            } else {
                events.push(Event::Text("Z".into()));
            }
            assert!(r15_split_frame(&mut c, events.clone()).is_empty());
            for event in events {
                assert!(r15_split_frame(&mut serial, vec![event]).is_empty());
            }
            assert_eq!((c.app.focus_pane, c.app.pane), (target, target));
            assert_eq!((serial.app.focus_pane, serial.app.pane), (target, target));
            assert_eq!(c.text(), if text_first { "ZHello brave world" } else { "Hello brave Zworld" });
            if !text_first {
                assert_eq!(c.range(), 13..13);
            }
            assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&serial), "pane {target}, text_first={text_first}");
            assert_eq!(r8_history(&c)["undo"].as_array().unwrap().len(), before["history"]["undo"].as_array().unwrap().len() + 1);
            assert!(r8_history(&c)["redo"].as_array().unwrap().is_empty());
            for pane in [0u8, 1] {
                assert_eq!(c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", pane)))), Some(false));
            }
            r9_input_finished(&c);
            r9_input_finished(&serial);
            r9_input_round_trip(&mut c, &before);
        }
    }
}

// RUN47: an intervening click in another pane interrupts a double-click sequence.
fn r47_split_click_fixture(pane: u8) -> Canvas {
    let mut c = r15_split_fixture(pane);
    let before = r9_input_snapshot(&c);
    // Age the fixture's focus click beyond the actual egui double/triple-click windows.
    c.time += c.ctx.options(|o| o.input_options.max_double_click_delay) * 3.0;
    assert!(r15_split_frame(&mut c, vec![]).is_empty());
    assert_eq!(r9_input_snapshot(&c), before);
    c
}

fn r47_split_click(c: &mut Canvas, pane: u8, point: Pos2) -> bool {
    assert!(
        r15_split_frame(c, vec![Event::PointerMoved(point), button(point, true, Modifiers::NONE), button(point, false, Modifiers::NONE)]).is_empty()
    );
    assert_eq!((c.app.focus_pane, c.app.pane), (pane, pane));
    r9_input_finished(c);
    c.ctx.input(|i| i.pointer.button_double_clicked(PointerButton::Primary))
}

fn r47_split_interrupted_chain(pane: u8) {
    let mut c = r47_split_click_fixture(pane);
    let before_clicks = r9_input_snapshot(&c);
    let a = r15_split_glyph(&mut c, pane, 8);
    let b = r15_split_glyph(&mut c, 1 - pane, 12);
    let (distance, delay) = c.ctx.options(|o| (o.input_options.max_click_dist, o.input_options.max_double_click_delay));
    assert!(a.distance(b) > distance, "the actual pane glyphs must break egui's spatial click chain");

    assert!(!r47_split_click(&mut c, pane, a));
    assert_eq!(c.range(), 8..8);
    let first_release = c.time;
    assert!(!r47_split_click(&mut c, 1 - pane, b));
    assert_eq!(c.range(), 12..12);
    assert_eq!(r15_split_glyph(&mut c, pane, 8), a, "the return target must be the same actual glyph position");
    assert!(!r47_split_click(&mut c, pane, a), "egui sees the intervening click in the other pane");
    assert!(c.time - first_release < delay, "the return is within the actual double-click window");
    assert_eq!(c.range(), 8..8, "pane {pane}: A/B/A are three single clicks; returning to A must not select its word");
    assert_eq!(r8_full_doc(&c), before_clicks["doc"]);
    assert_eq!(r8_history(&c), before_clicks["history"], "clicks preserve every undo/redo entry");

    let before_type = r9_input_snapshot(&c);
    let mut reference = r9_input_reference(&c);
    reference.app.run("text.insert", json!({"text": "Z"})).unwrap();
    assert!(r15_split_frame(&mut c, vec![Event::Text("Z".into())]).is_empty());
    assert_eq!(c.text(), "Hello brZave world", "typing inserts at the returned caret instead of replacing brave");
    assert_eq!(c.range(), 9..9);
    assert_eq!(r9_input_snapshot(&c), r9_input_snapshot(&reference));
    assert_eq!(r8_history(&c)["undo"].as_array().unwrap().len(), before_type["history"]["undo"].as_array().unwrap().len() + 1);
    assert!(r8_history(&c)["redo"].as_array().unwrap().is_empty());
    r9_input_finished(&c);
    r9_input_round_trip(&mut c, &before_type);
}

#[test]
fn r47_split_left_right_left_keeps_single_caret() {
    r47_split_interrupted_chain(0);
}

#[test]
fn r47_split_right_left_right_keeps_single_caret() {
    r47_split_interrupted_chain(1);
}

#[test]
fn r47_split_same_pane_double_and_triple_click_controls() {
    for pane in [0, 1] {
        let mut c = r47_split_click_fixture(pane);
        let before = r9_input_snapshot(&c);
        let point = r15_split_glyph(&mut c, pane, 8);
        let delay = c.ctx.options(|o| o.input_options.max_double_click_delay);
        assert!(!r47_split_click(&mut c, pane, point));
        assert_eq!(c.range(), 8..8);
        let first_release = c.time;
        assert!(r47_split_click(&mut c, pane, point), "ordinary same-pane double-click remains recognized");
        assert!(c.time - first_release < delay);
        assert_eq!(c.range(), 6..11, "double-click selects brave in pane {pane}");
        assert_eq!(r8_full_doc(&c), before["doc"]);
        assert_eq!(r8_history(&c), before["history"]);

        assert!(!r47_split_click(&mut c, pane, point), "the third release is not another double-click");
        assert!(c.time - first_release < delay * 2.0);
        assert_eq!(c.range(), 8..8, "retain the existing triple-release caret behavior in pane {pane}");
        assert_eq!(r8_full_doc(&c), before["doc"]);
        assert_eq!(r8_history(&c), before["history"]);
    }
}

// Physical batches preserve event order across split panes.
fn r51_split_finished(c: &Canvas) {
    r9_input_finished(c);
    assert!(!c.ctx.input(|i| i.pointer.any_down()));
    assert!(!c.app.session.tool_busy());
    assert!(c.app.synthetic.is_empty());
    for pane in [0u8, 1] {
        assert_eq!(c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", pane)))), Some(false));
    }
}

fn r51_split_tail_case(clicked_pane: u8, tail_pane: u8, tail_click: bool) {
    let mut batched = r47_split_click_fixture(1 - clicked_pane);
    let mut serial = r47_split_click_fixture(1 - clicked_pane);
    *serial.app.session.doc_mut().unwrap() = batched.app.session.doc().unwrap().clone();
    serial.frame = batched.frame;
    serial.story = batched.story;
    serial.app.views = batched.app.views.clone();
    serial.app.after_engine();
    let before = r9_input_snapshot(&batched);
    assert_eq!(r9_input_snapshot(&serial), before);
    assert_eq!(batched.text(), "Hello brave world");
    assert_eq!(batched.range(), 0..0);
    assert_eq!((batched.app.focus_pane, batched.app.pane), (1 - clicked_pane, 1 - clicked_pane));
    let point = r15_split_glyph(&mut batched, clicked_pane, 12);
    let tail = r15_split_glyph(&mut batched, tail_pane, 0);
    assert_eq!(r15_split_glyph(&mut serial, clicked_pane, 12), point);
    assert_eq!(r15_split_glyph(&mut serial, tail_pane, 0), tail);
    assert!(point.distance(tail) > batched.ctx.options(|o| o.input_options.max_click_dist));
    let mut events = vec![Event::PointerMoved(point), button(point, true, Modifiers::NONE), button(point, false, Modifiers::NONE)];

    // Establish the serial click target and its complete no-edit/history precondition.
    for event in &events {
        assert!(r15_split_frame(&mut serial, vec![event.clone()]).is_empty());
    }
    assert_eq!((serial.app.focus_pane, serial.app.pane), (clicked_pane, clicked_pane));
    assert_eq!(serial.range(), 12..12);
    assert_eq!(r8_full_doc(&serial), before["doc"]);
    assert_eq!(r8_history(&serial), before["history"]);
    r51_split_finished(&serial);
    let mut reference = r9_input_reference(&serial);
    reference.app.run("text.insert", json!({"text": "Z"})).unwrap();
    assert!(r15_split_frame(&mut serial, vec![Event::Text("Z".into())]).is_empty());
    assert_eq!(r9_input_snapshot(&serial), r9_input_snapshot(&reference));
    assert_eq!(serial.text(), "Hello brave Zworld");
    assert_eq!(serial.range(), 13..13);
    let typed_doc = r8_full_doc(&serial);
    let typed_history = r8_history(&serial);
    events.push(Event::Text("Z".into()));
    let mut tail_events = vec![Event::PointerMoved(tail)];
    if tail_click {
        tail_events.extend([button(tail, true, Modifiers::NONE), button(tail, false, Modifiers::NONE)]);
    }
    for event in &tail_events {
        assert!(r15_split_frame(&mut serial, vec![event.clone()]).is_empty());
    }
    events.extend(tail_events);
    let expected_pane = if tail_click { tail_pane } else { clicked_pane };
    assert_eq!((serial.app.focus_pane, serial.app.pane), (expected_pane, expected_pane));
    assert_eq!(serial.range(), if tail_click { 0..0 } else { 13..13 });
    assert_eq!(r8_full_doc(&serial), typed_doc, "the trailing pointer input makes no document edit");
    assert_eq!(r8_history(&serial), typed_history, "hover/click does not rewrite any history entry");

    // The complete native batch is passed once through raw_input_hook + logic + ui.
    assert!(r15_split_frame(&mut batched, events).is_empty());
    let actual = r9_input_snapshot(&batched);
    let expected = r9_input_snapshot(&serial);
    let actual_focus = (batched.app.focus_pane, batched.app.pane);
    let expected_focus = (serial.app.focus_pane, serial.app.pane);
    for c in [&mut batched, &mut serial] {
        r51_split_finished(c);
        assert_eq!(r8_history(c)["undo"].as_array().unwrap().len(), before["history"]["undo"].as_array().unwrap().len() + 1);
        assert!(r8_history(c)["redo"].as_array().unwrap().is_empty());
        // Both paths conserve the complete original document and restore their
        // exact post-input state on Redo.
        r9_input_round_trip(c, &before);
        r51_split_finished(c);
    }
    assert_eq!(actual, expected, "split batch differs from serial input: clicked pane {clicked_pane}, tail pane {tail_pane}, click {tail_click}");
    assert_eq!(actual_focus, expected_focus, "a hover preserves focus; a later click transfers it");
}

#[test]
fn r51_split_right_click_text_left_hover() {
    r51_split_tail_case(1, 0, false);
}

#[test]
fn r51_split_right_click_text_left_click() {
    r51_split_tail_case(1, 0, true);
}

#[test]
fn r51_split_left_click_text_right_hover() {
    r51_split_tail_case(0, 1, false);
}

#[test]
fn r51_split_left_click_text_right_click() {
    r51_split_tail_case(0, 1, true);
}

#[test]
fn r51_split_same_pane_tail_controls() {
    for pane in [0, 1] {
        for click in [false, true] {
            r51_split_tail_case(pane, pane, click);
        }
    }
}

struct FlyoutHeaderDrag {
    canvas: Canvas,
    header_id: egui::Id,
    flyout: egui::Rect,
    end: Pos2,
    before: serde_json::Value,
    after_press: serde_json::Value,
    canvas_down: bool,
}

// Use the real flyout header and retain its held drag for release or cancellation.
fn r51_split_flyout_header_drag() -> FlyoutHeaderDrag {
    let mut c = r15_split_fixture(1);
    c.app.ui.control_bar = false;
    c.app.ui.dock_expanded = false;
    c.app.ui.open_panel = Some("info".into());
    for _ in 0..3 {
        assert!(r15_split_frame(&mut c, vec![]).is_empty());
    }
    assert!(c.app.split && c.app.ui.docking.is_none() && c.app.ui.floating.is_empty());
    assert!(!c.ctx.text_edit_focused() && c.app.ui.dialog.is_none() && c.app.ui.palette.is_none());
    assert!(c.app.native_shortcuts.is_empty() && c.app.synthetic.is_empty());

    // dock::flyout's real Frame child and drag header, measured after full UI layout.
    let flyout_id = egui::Id::new("panel_flyout");
    let layer = egui::LayerId::new(egui::Order::Foreground, flyout_id);
    let header_id = flyout_id.with(egui::IdSalt::new("child")).with("flyhead");
    let header = c.ctx.viewport(|v| v.prev_pass.widgets.get(header_id).copied()).unwrap();
    let flyout = c.ctx.memory(|m| m.area_rect(flyout_id)).unwrap();
    assert_eq!(header.layer_id, layer);
    assert!(header.enabled && header.sense.senses_drag() && !header.sense.senses_click());
    let p = egui::pos2(header.interact_rect.left() + 1.0, header.interact_rect.center().y);
    let radius = c.ctx.global_style().interaction.interact_radius;
    assert!(header.interact_rect.contains(p) && flyout.contains(p));
    assert!(p.x > flyout.left() && p.x - flyout.left() < radius);
    let canvas_rect = c.app.canvas_rect.unwrap();
    assert!(canvas_rect.shrink(radius).contains(p), "the measured header edge overlaps the real right canvas");
    let canvas_ids = c.ctx.viewport(|v| {
        let widgets = &v.prev_pass.widgets;
        widgets
            .layer_ids()
            .flat_map(|layer| widgets.get_layer(layer))
            .filter(|w| w.rect == canvas_rect && w.sense.senses_click() && w.sense.senses_drag())
            .map(|w| w.id)
            .collect::<Vec<_>>()
    });
    assert_eq!(canvas_ids.len(), 1);
    let canvas_id = canvas_ids[0];

    // Put byte 12 beneath the measured edge by panning only; start with caret 0.
    let glyph = c.glyph(12);
    let view = c.app.view_mut().unwrap();
    assert_eq!(view.rotation, 0);
    view.origin =
        designcraft_geom::Point::new(view.origin.x + (glyph.x - p.x) as f64 / view.zoom, view.origin.y + (glyph.y - p.y) as f64 / view.zoom);
    assert!(r15_split_frame(&mut c, vec![]).is_empty());
    assert!(c.glyph(12).distance(p) < 0.1);
    assert_eq!(c.range(), 0..0);
    let before = r9_input_snapshot(&c);

    // Use egui's actual winning hit records, not the candidate's ownership helper.
    let assert_foreground_owner = |c: &Canvas| {
        c.ctx.viewport(|v| {
            assert!(v.hits.click.is_none(), "egui's top drag-only header suppresses the underlying click target");
            let drag = v.hits.drag.unwrap();
            assert_eq!((drag.id, drag.layer_id), (header_id, layer));
            assert!(v.hits.contains_pointer.iter().any(|w| w.id == canvas_id), "containment alone does not confer ownership");
            assert!(!v.interact_widgets.hovered.contains(&canvas_id));
        });
    };
    assert!(r15_split_frame(&mut c, vec![Event::PointerMoved(p)]).is_empty());
    assert_foreground_owner(&c);
    assert_eq!(r9_input_snapshot(&c), before);
    assert!(r15_split_frame(&mut c, vec![button(p, true, Modifiers::NONE)]).is_empty());
    assert_foreground_owner(&c);
    assert_eq!(c.ctx.viewport(|v| v.interact_widgets.dragged), Some(header_id));
    let after_press = r9_input_snapshot(&c);
    let canvas_down = c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", 1u8)))).unwrap();

    // Complete the real tear-off before checking the document oracle.
    let end = p + vec2(40.0, 0.0);
    assert!(canvas_rect.contains(end) && end.distance(p) > 24.0);
    assert!(r15_split_frame(&mut c, vec![Event::PointerMoved(end)]).is_empty());
    assert_eq!(c.ctx.viewport(|v| v.interact_widgets.dragged), Some(header_id));
    FlyoutHeaderDrag { canvas: c, header_id, flyout, end, before, after_press, canvas_down }
}

// A foreground flyout drag must not also start the underlying split-canvas tool.
#[test]
fn r51_split_flyout_header_edge_preserves_document_selection() {
    let FlyoutHeaderDrag { canvas: mut c, header_id, flyout, end, before, after_press, canvas_down } = r51_split_flyout_header_drag();
    assert!(r15_split_frame(&mut c, vec![button(end, false, Modifiers::NONE)]).is_empty());
    assert_eq!(c.ctx.viewport(|v| v.interact_widgets.drag_stopped), Some(header_id));
    assert!(c.app.ui.open_panel.is_none());
    let layout = c.app.ui.docking.as_ref().unwrap();
    let floated = layout.floating.iter().find(|f| f.panels.iter().any(|id| id == "info")).unwrap();
    assert_eq!(&floated.rect[..2], &[end.x - 20.0, end.y - 10.0]);
    let (_, at) = c.app.ui.floating.iter().find(|(id, _)| id == "info").unwrap();
    assert!(egui::pos2(at[0], at[1]).distance(flyout.min) > 1.0, "the real panel moved into its floating position");
    assert!(!c.ctx.input(|i| i.pointer.any_down()) && !c.app.session.tool_busy());
    assert!(c.app.synthetic.is_empty());
    r9_input_finished(&c);
    for pane in [0u8, 1] {
        assert_eq!(c.ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("canvas_pointer_down", pane)))), Some(false));
    }

    assert_eq!(after_press["doc"], before["doc"]);
    assert_eq!(after_press["history"], before["history"]);
    let after = r9_input_snapshot(&c);
    assert_eq!(after["doc"], before["doc"]);
    assert_eq!(after["history"], before["history"]);
    assert_eq!(after_press["selection"], before["selection"], "foreground press leaked into the canvas; canvas_down={canvas_down}");
    assert!(!canvas_down, "the document tool must not capture a foreground-owned press");
    assert_eq!(after, before, "tearing off a panel preserves the complete Doc, selection and both history stacks");
}

#[test]
fn split_flyout_escape_and_release_cancels_tear_off() {
    let FlyoutHeaderDrag { canvas: mut c, header_id, end, before, .. } = r51_split_flyout_header_drag();
    let escape = |pressed| Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    // Both events reach the full UI in the same frame: egui cancels this held drag.
    assert!(r15_split_frame(&mut c, vec![escape(true), button(end, false, Modifiers::NONE)]).is_empty());
    assert_eq!(c.ctx.viewport(|v| v.interact_widgets.drag_stopped), Some(header_id));
    assert!(c.ctx.viewport(|v| v.interact_widgets.dragged).is_none());
    assert_eq!(c.app.ui.open_panel.as_deref(), Some("info"), "Escape must cancel tear-off even when release is in the same frame");
    assert!(c.app.ui.docking.is_none() && c.app.ui.floating.is_empty());
    let origin_id = header_id.with(("press_origin", "info"));
    assert!(c.ctx.data(|d| d.get_temp::<Pos2>(origin_id)).is_none(), "cancellation must consume the held origin");
    r51_split_finished(&c);
    assert_eq!(r8_full_doc(&c), before["doc"]);
    assert_eq!(r8_history(&c), before["history"]);
    // Escape may switch tools or change selection through the existing global path.
    // A later ordinary header drag must preserve that resulting state exactly.
    let after_cancel = r9_input_snapshot(&c);
    let header = c.ctx.viewport(|v| v.prev_pass.widgets.get(header_id).copied()).unwrap();
    let retry = header.interact_rect.center();
    let retry_end = retry + vec2(40.0, 0.0);
    assert!(header.interact_rect.contains(retry));
    assert!(c.app.canvas_rect.unwrap().contains(retry_end) && retry.distance(retry_end) > 24.0);
    assert!(r15_split_frame(&mut c, vec![escape(false), Event::PointerMoved(retry)]).is_empty());
    assert!(r15_split_frame(&mut c, vec![button(retry, true, Modifiers::NONE)]).is_empty());
    assert_eq!(c.ctx.viewport(|v| v.interact_widgets.dragged), Some(header_id));
    // Also cover the final move and release coalesced into one frame.
    assert!(r15_split_frame(&mut c, vec![Event::PointerMoved(retry_end), button(retry_end, false, Modifiers::NONE)]).is_empty());
    assert_eq!(c.ctx.viewport(|v| v.interact_widgets.drag_stopped), Some(header_id));
    assert!(c.app.ui.open_panel.is_none());
    let floated = c.app.ui.docking.as_ref().unwrap().floating.iter().find(|f| f.panels.iter().any(|id| id == "info")).unwrap();
    assert_eq!(&floated.rect[..2], &[retry_end.x - 20.0, retry_end.y - 10.0]);
    let (_, at) = c.app.ui.floating.iter().find(|(id, _)| id == "info").unwrap();
    assert_eq!(*at, [retry_end.x - 20.0, retry_end.y - 10.0]);
    assert!(c.ctx.data(|d| d.get_temp::<Pos2>(origin_id)).is_none());
    r51_split_finished(&c);
    assert_eq!(r9_input_snapshot(&c), after_cancel, "the fresh successful drag preserves the post-Escape Doc, selection and both history stacks");
}

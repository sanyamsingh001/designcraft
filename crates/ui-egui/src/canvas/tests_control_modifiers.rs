//! Synthetic controls must supply global modifiers to the actual canvas input handler.
//! Idle/control frames deliberately contain no fixture-generated ModifiersChanged events.

use std::sync::Arc;

use designcraft_doc::{ItemId, SpreadRef, StoryId};
use designcraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Sense, vec2};
use serde_json::{Value, json};

use super::{Xf, handle_input};
use crate::{DesignApp, Services, View, control};

#[derive(Debug)]
struct Frame {
    pressed: bool,
    released: bool,
    down: bool,
    modifiers: Modifiers,
}

struct Canvas {
    ctx: egui::Context,
    app: DesignApp,
    frame: ItemId,
    story: StoryId,
    time: f64,
}

impl Canvas {
    fn new() -> Self {
        let mut session = Session::new();
        session.execute("file.new", &json!({})).unwrap();
        let r = session.execute("frame.create", &json!({"rect": [40, 40, 440, 340], "content": "text", "text": "Hello brave world"})).unwrap();
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
        c.tick(vec![]);
        c.tick(vec![]);
        c
    }

    fn tick(&mut self, events: Vec<Event>) -> Frame {
        self.time += 0.025;
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        self.app.raw_input_hook(&mut raw);
        let mut frame = Frame { pressed: false, released: false, down: false, modifiers: Modifiers::NONE };
        let mut output = self.ctx.run_ui(raw, |ui| {
            frame = ui.input(|i| Frame {
                pressed: i.pointer.primary_pressed(),
                released: i.pointer.primary_released(),
                down: i.pointer.primary_down(),
                modifiers: i.modifiers,
            });
            let rect = ui.available_rect_before_wrap();
            self.app.canvas_rect = Some(rect);
            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            // Same widget allocation and handler as canvas::show, without its renderer.
            handle_input(&mut self.app, ui, &response, rect);
        });
        output.textures_delta.clear();
        assert!(self.app.ui.status.is_empty(), "canvas command error: {}", self.app.ui.status);
        frame
    }

    fn screen(&self, x: f64, y: f64) -> Pos2 {
        let d = &self.app.session.doc().unwrap().doc;
        let it = d.item(self.frame).unwrap();
        let loc = d.find(self.frame).unwrap();
        let spread = d.parent_xf(&loc) * d.text_xf(it) * designcraft_geom::Point::new(x, y);
        let canvas = self.app.session.layout().xf(SpreadRef::Doc(0)) * spread;
        let p = Xf::new(self.app.canvas_rect.unwrap(), self.app.view().unwrap()).to_screen(canvas);
        assert!(self.app.canvas_rect.unwrap().contains(p), "fixture point must be in the real canvas");
        p
    }

    fn glyph(&self, byte: usize) -> Pos2 {
        let s = &self.app.session;
        let cs = s.cache.get(&s.doc().unwrap().doc, self.story, None);
        let line = cs.frames[0].lines.iter().find(|l| l.glyphs.iter().any(|g| g.byte == byte && g.len > 0)).unwrap();
        let glyph = line.glyphs.iter().find(|g| g.byte == byte && g.len > 0).unwrap();
        self.screen(glyph.x + glyph.adv * 0.25, line.baseline - line.ascent * 0.5)
    }

    fn end(&self) -> Pos2 {
        let s = &self.app.session;
        let cs = s.cache.get(&s.doc().unwrap().doc, self.story, None);
        let line = cs.frames[0].lines.last().unwrap();
        self.screen(line.end_x + 24.0, line.baseline - line.ascent * 0.5)
    }

    fn select(&mut self, anchor: usize, focus: usize) {
        self.app.session.execute("text.select", &json!({"story": self.story.0, "anchor": anchor, "focus": focus})).unwrap();
    }

    fn range(&self) -> std::ops::Range<usize> {
        self.app.session.doc().unwrap().selection.text.unwrap().range()
    }

    fn text(&self) -> String {
        self.app.session.doc().unwrap().doc.story(self.story).unwrap().text.clone()
    }

    fn queue(&mut self, method: &str, params: Value) {
        self.time += 1.0; // Separate gestures must not inherit double-click timing.
        let (request, _receiver) = control::ControlRequest::new(method, params);
        let control::Outcome::Done(response) = control::handle(&mut self.app, &self.ctx, &request) else {
            panic!("expected immediate control response")
        };
        assert_eq!(response["ok"], true, "{response}");
        assert!(!self.app.synthetic.is_empty());
    }

    fn drain(&mut self) -> Vec<Frame> {
        let mut frames = vec![];
        while !self.app.synthetic.is_empty() {
            assert!(frames.len() < 32, "bounded test gesture must finish draining");
            frames.push(self.tick(vec![]));
        }
        frames
    }

    fn drag(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) -> Vec<Frame> {
        self.queue(
            "ui.drag",
            json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4,
            "alt": modifiers.alt, "shift": modifiers.shift}),
        );
        self.drain()
    }

    fn click(&mut self, at: Pos2) {
        self.queue("ui.click", json!({"x": at.x, "y": at.y}));
        let frames = self.drain();
        assert_gesture(&frames, Modifiers::NONE, 3);
    }

    fn physical_drag_without_modifier_change(&mut self, from: Pos2, to: Pos2) {
        self.time += 1.0;
        self.tick(vec![Event::PointerMoved(from)]);
        self.tick(vec![button(from, true, Modifiers::NONE)]);
        self.tick(vec![Event::PointerMoved(to)]);
        self.tick(vec![button(to, false, Modifiers::NONE)]);
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

fn assert_gesture(frames: &[Frame], modifiers: Modifiers, count: usize) {
    // Counts observe egui's processed state; collapsing the queue into one frame cannot pass.
    assert_eq!(frames.len(), count, "one queued pointer event per frame, with a separate cleanup for modified gestures");
    assert_eq!(frames.iter().filter(|f| f.pressed).count(), 1);
    assert_eq!(frames.iter().filter(|f| f.released).count(), 1);
    assert!(frames.iter().all(|f| !(f.pressed && f.released)), "press and release must remain separate: {frames:?}");
    assert!(frames.iter().filter(|f| f.pressed).all(|f| f.down));
    assert!(frames.iter().filter(|f| f.released).all(|f| !f.down));
    let pointer_frames = if modifiers == Modifiers::NONE { frames } else { &frames[..frames.len() - 1] };
    for frame in pointer_frames {
        assert_eq!(frame.modifiers, modifiers, "gesture modifiers must survive motion and release");
    }
    if modifiers != Modifiers::NONE {
        let cleanup = frames.last().unwrap();
        assert_eq!(cleanup.modifiers, Modifiers::NONE);
        assert!(!cleanup.pressed && !cleanup.released && !cleanup.down);
    }
}

#[test]
fn r7a_control_modifiers_alt_drag_copies_then_physical_plain_drag_moves() {
    let mut c = Canvas::new();
    c.select(6, 12);
    let before = c.app.session.doc().unwrap().doc.clone();
    let undo = c.app.session.doc().unwrap().history.undo.len();
    let frames = c.drag(c.glyph(8), c.end(), Modifiers { alt: true, ..Modifiers::NONE });
    assert_eq!(c.text(), "Hello brave worldbrave ", "ui.drag Alt must copy selected text through raw_input_hook and the canvas release handler");
    assert_eq!(c.range(), 17..23);
    assert_gesture(&frames, Modifiers { alt: true, ..Modifiers::NONE }, 8);
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
    let copied = c.app.session.doc().unwrap().doc.clone();
    c.app.run("edit.undo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &before));
    c.app.run("edit.redo", json!({})).unwrap();
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &copied));
    c.select(17, 23);
    // No ModifiersChanged(NONE) from the fixture: cleanup must prevent sticky Alt.
    c.physical_drag_without_modifier_change(c.glyph(18), c.glyph(0));
    assert_eq!(c.text(), "brave Hello brave world", "a subsequent physical plain drag must move the copy, not copy it again");
    assert_eq!(c.range(), 0..6);
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 2);
    assert!(!c.tick(vec![]).down);
}

#[test]
fn r7a_control_modifiers_plain_drag_moves_and_plain_click_collapses() {
    let mut c = Canvas::new();
    c.select(6, 12);
    let undo = c.app.session.doc().unwrap().history.undo.len();
    let frames = c.drag(c.glyph(8), c.end(), Modifiers::NONE);
    assert_eq!(c.text(), "Hello worldbrave ", "ordinary ui.drag must still move selected text");
    assert_eq!(c.range(), 11..17);
    assert_gesture(&frames, Modifiers::NONE, 7);
    c.click(c.glyph(13));
    assert_eq!(c.range(), 13..13, "ordinary ui.click must still collapse the selection");
    assert_eq!(c.text(), "Hello worldbrave ");
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
}

#[test]
fn r7a_control_modifiers_shift_drag_extends_existing_anchor() {
    let mut c = Canvas::new();
    c.select(0, 0);
    let before = c.app.session.doc().unwrap().doc.clone();
    let undo = c.app.session.doc().unwrap().history.undo.len();
    let frames = c.drag(c.glyph(6), c.glyph(10), Modifiers { shift: true, ..Modifiers::NONE });
    assert_eq!(c.range(), 0..10, "ui.drag Shift must extend the existing anchor instead of placing a new caret at byte 6");
    let selection = c.app.session.doc().unwrap().selection.text.unwrap();
    assert_eq!((selection.anchor, selection.focus), (0, 10));
    assert_gesture(&frames, Modifiers { shift: true, ..Modifiers::NONE }, 8);
    c.click(c.glyph(12));
    assert_eq!(c.range(), 12..12, "a later ordinary click must not retain synthetic Shift");
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &before));
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn r7a_control_modifiers_cleanup_keeps_physical_modifier_change() {
    let mut c = Canvas::new();
    c.select(6, 12);
    let from = c.glyph(8);
    let to = c.end();
    c.queue("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "alt": true}));
    for _ in 0..7 {
        c.tick(vec![]);
    }
    assert_eq!(c.text(), "Hello brave worldbrave ", "release must copy before the cleanup frame");
    assert_eq!(c.app.synthetic.len(), 1, "release must leave only the cleanup marker");
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    c.tick(vec![Event::ModifiersChanged(shift)]);
    assert!(c.app.synthetic.is_empty());
    c.tick(vec![]); // Empty synthetic queues must leave physical Shift held.
    c.time += 1.0;
    let p = c.glyph(0);
    c.tick(vec![Event::PointerMoved(p)]);
    c.tick(vec![button(p, true, shift)]);
    c.tick(vec![button(p, false, shift)]);
    assert_eq!(c.range(), 0..17, "physical Shift arriving on the cleanup frame must extend the copied selection's anchor");
    assert_eq!(c.text(), "Hello brave worldbrave ");
}

#[test]
fn r7a_control_modifiers_cleanup_does_not_drain_queued_click() {
    let mut c = Canvas::new();
    c.select(6, 12);
    let from = c.glyph(8);
    let to = c.end();
    let click = c.glyph(0);
    c.queue("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "alt": true}));
    c.queue("ui.click", json!({"x": click.x, "y": click.y}));
    let frames = c.drain();
    assert_eq!(c.text(), "Hello brave worldbrave ", "queued Alt drag must copy before the following ordinary click");
    assert_eq!(frames.len(), 11, "the modifier cleanup must consume its marker without batching the following click");
    assert_gesture(&frames[..8], Modifiers { alt: true, ..Modifiers::NONE }, 8);
    assert_gesture(&frames[8..], Modifiers::NONE, 3);
    assert_eq!(c.range(), 0..0, "the following ordinary click must reach the canvas after cleanup");
}

#[test]
fn r7a_control_modifiers_refinement_held_shift_survives_alt_copy() {
    let mut c = Canvas::new();
    c.select(6, 12);
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    // The ordinary physical hook must cache this even with no synthetic queue.
    c.tick(vec![Event::ModifiersChanged(shift)]);
    let frames = c.drag(c.glyph(8), c.end(), Modifiers { alt: true, ..Modifiers::NONE });
    assert_eq!(c.text(), "Hello brave worldbrave ", "the restoration regression starts after a correct scripted Alt copy");
    assert_eq!(c.range(), 17..23);
    c.time += 1.0;
    let p = c.glyph(0);
    // Shift remains physically held; no new ModifiersChanged event is supplied.
    c.tick(vec![Event::PointerMoved(p)]);
    c.tick(vec![button(p, true, shift)]);
    c.tick(vec![button(p, false, shift)]);
    assert_eq!(c.range(), 0..17, "the physical Shift click must extend the copied selection's anchor without a fresh modifier event");
    assert_eq!(c.text(), "Hello brave worldbrave ");
    assert_eq!(frames.len(), 8, "Alt differing from physical Shift needs a separate restoration frame");
    assert_eq!(frames.last().unwrap().modifiers, shift);
}

#[test]
fn r7a_control_modifiers_refinement_plain_click_restores_held_shift() {
    let mut c = Canvas::new();
    c.select(0, 0);
    let before = c.app.session.doc().unwrap().doc.clone();
    let undo = c.app.session.doc().unwrap().history.undo.len();
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    c.tick(vec![Event::ModifiersChanged(shift)]);
    let at = c.glyph(6);
    c.queue("ui.click", json!({"x": at.x, "y": at.y}));
    let frames = c.drain();
    assert_eq!(c.range(), 6..6, "an explicitly plain scripted click must collapse while physical Shift is held");
    c.time += 1.0;
    let p = c.glyph(10);
    c.tick(vec![Event::PointerMoved(p)]);
    c.tick(vec![button(p, true, shift)]);
    c.tick(vec![button(p, false, shift)]);
    assert_eq!(c.range(), 6..10, "the next physical Shift click must extend the scripted caret without a fresh modifier event");
    assert_eq!(frames.len(), 4, "plain NONE differing from held Shift needs a restoration frame");
    assert!(frames[..3].iter().all(|f| f.modifiers == Modifiers::NONE));
    assert_eq!(frames.last().unwrap().modifiers, shift);
    assert!(Arc::ptr_eq(&c.app.session.doc().unwrap().doc, &before));
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn r7a_control_modifiers_refinement_native_release_during_drag_stays_released() {
    let mut c = Canvas::new();
    c.select(6, 12);
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    c.tick(vec![Event::ModifiersChanged(shift)]);
    let from = c.glyph(8);
    let to = c.end();
    c.queue("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "alt": true}));
    c.tick(vec![]); // Initial motion.
    c.tick(vec![]); // Press with scripted Alt.
    c.tick(vec![Event::ModifiersChanged(Modifiers::NONE)]); // Physical Shift released during motion.
    let rest = c.drain();
    assert_eq!(c.text(), "Hello brave worldbrave ", "physical release must not remove the scripted copy flag from the later pointer release");
    c.time += 1.0;
    let p = c.glyph(0);
    c.tick(vec![Event::PointerMoved(p)]);
    c.tick(vec![button(p, true, Modifiers::NONE)]);
    c.tick(vec![button(p, false, Modifiers::NONE)]);
    assert_eq!(c.range(), 0..0, "cleanup must not revive physical Shift that was released during the scripted drag");
    assert_eq!(c.text(), "Hello brave worldbrave ");
    assert_eq!(rest.len(), 5);
    assert_eq!(rest.last().unwrap().modifiers, Modifiers::NONE);
}

#[test]
fn r7a_control_modifiers_refinement_focus_loss_does_not_revive_native_hold() {
    for event_loss in [false, true] {
        let mut c = Canvas::new();
        c.select(6, 12);
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        c.tick(vec![Event::ModifiersChanged(shift)]);
        let from = c.glyph(8);
        let to = c.end();
        c.queue("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "alt": true}));
        for _ in 0..7 {
            c.tick(vec![]);
        }
        assert_eq!(c.text(), "Hello brave worldbrave ", "the focus-loss regression starts after a correct scripted Alt copy");
        assert_eq!(c.app.synthetic.len(), 1, "restoration is pending until the next frame");
        // Exercise both host signals on the pending restoration frame, without changing
        // the original harness or injecting a fixture-generated modifier reset.
        c.time += 0.025;
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 700.0))),
            time: Some(c.time),
            focused: event_loss,
            events: if event_loss { vec![Event::WindowFocused(false)] } else { vec![] },
            ..Default::default()
        };
        c.app.raw_input_hook(&mut raw);
        let mut output = c.ctx.run_ui(raw, |ui| {
            let rect = ui.available_rect_before_wrap();
            c.app.canvas_rect = Some(rect);
            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            handle_input(&mut c.app, ui, &response, rect);
        });
        output.textures_delta.clear();
        assert!(c.app.ui.status.is_empty(), "canvas command error: {}", c.app.ui.status);
        assert!(c.app.synthetic.is_empty());
        c.tick(vec![Event::WindowFocused(true)]);
        // A later ordinary gesture also exposes a stale cached hold even when egui's
        // WindowFocused(false) event cleared its global modifiers by itself.
        let at = c.glyph(6);
        c.queue("ui.click", json!({"x": at.x, "y": at.y}));
        let frames = c.drain();
        assert_eq!(c.range(), 6..6);
        c.time += 1.0;
        let p = c.glyph(0);
        c.tick(vec![Event::PointerMoved(p)]);
        c.tick(vec![button(p, true, Modifiers::NONE)]);
        c.tick(vec![button(p, false, Modifiers::NONE)]);
        assert_eq!(c.range(), 0..0, "focus loss must not revive physical Shift after refocus (WindowFocused event: {event_loss})");
        assert_eq!(c.text(), "Hello brave worldbrave ");
        assert_eq!(frames.len(), 3, "focus loss leaves native NONE, so a plain scripted click retains its original three frames");
    }
}

fn r8_snapshot(c: &Canvas) -> Value {
    let st = c.app.session.doc().unwrap();
    let entries = |list: &[designcraft_engine::HistoryEntry]| {
        list.iter().map(|e| json!({"label": e.label, "doc": &*e.doc, "selection": e.selection})).collect::<Vec<_>>()
    };
    json!({"doc": &*st.doc, "selection": st.selection,
        "history": {"undo": entries(&st.history.undo), "redo": entries(&st.history.redo), "limit": st.history.limit}})
}

fn r8_reference(c: &Canvas) -> Canvas {
    let mut reference = Canvas::new();
    // Share the exact baseline, including creation time and all history documents.
    *reference.app.session.doc_mut().unwrap() = c.app.session.doc().unwrap().clone();
    reference.app.after_engine();
    *reference.app.view_mut().unwrap() = *c.app.view().unwrap();
    reference.frame = c.frame;
    reference.story = c.story;
    assert_eq!(r8_snapshot(&reference), r8_snapshot(c));
    reference
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum R8TextPrefix {
    Native,
    KeyTail,
    ImmediateControl,
}

fn r8_prepare_text_prefix(c: &mut Canvas, prefix: R8TextPrefix) {
    assert!(c.app.synthetic.is_empty());
    assert!(!c.ctx.text_edit_focused() && c.app.ui.dialog.is_none() && c.app.ui.palette.is_none());
    assert!(c.app.session.wants_text());
    assert_eq!(c.text(), "Hello brave world");
    assert_eq!(c.range(), 0..0);
    let before = r8_snapshot(c);
    match prefix {
        R8TextPrefix::Native => {} // Delivered as real RawInput in the next frame, not into the synthetic queue.
        R8TextPrefix::KeyTail => c.queue("ui.key", json!({"key": "A", "text": "Z"})),
        R8TextPrefix::ImmediateControl => {
            c.time += 1.0;
            let undo = c.app.session.doc().unwrap().history.undo.len();
            let (request, _receiver) = control::ControlRequest::new("ui.text", json!({"text": "Z"}));
            let control::Outcome::Done(response) = control::handle(&mut c.app, &c.ctx, &request) else {
                panic!("expected immediate control response")
            };
            assert_eq!(response["ok"], true, "{response}");
            assert!(c.app.synthetic.is_empty(), "explicit-caret ui.text must complete without queuing input");
            assert_eq!(c.text(), "ZHello brave world", "ui.text must insert before any input frame advances");
            assert_eq!(c.range(), 1..1);
            assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
        }
    }
    if prefix != R8TextPrefix::ImmediateControl {
        assert_eq!(r8_snapshot(c), before, "native/queued text must not edit before its input frame");
    }
}

fn r8_deliver_text_prefix(c: &mut Canvas, prefix: R8TextPrefix) -> Option<Frame> {
    if prefix == R8TextPrefix::KeyTail {
        let before = r8_snapshot(c);
        r8_assert_nonpointer(&c.tick(vec![]));
        assert_eq!(c.text(), "Hello brave world", "the key pair precedes its queued text tail");
        assert_eq!(r8_snapshot(c), before);
    }
    let frame = match prefix {
        R8TextPrefix::Native => Some(c.tick(vec![Event::Text("Z".into())])),
        R8TextPrefix::KeyTail => Some(c.tick(vec![])),
        R8TextPrefix::ImmediateControl => None,
    };
    if let Some(frame) = &frame {
        r8_assert_nonpointer(frame);
    }
    assert_eq!(c.text(), "ZHello brave world", "text must insert at the old caret before the pointer press");
    frame
}

fn r8_assert_nonpointer(frame: &Frame) {
    assert!(!frame.pressed && !frame.released && !frame.down, "text/key input must precede the pointer press: {frame:?}");
}

fn r8_assert_restored_gesture(frames: &[Frame], scripted: Modifiers, native: Modifiers) {
    assert_eq!(frames.len(), 8, "seven pointer frames and one restoration frame");
    assert_eq!(frames.iter().filter(|f| f.pressed).count(), 1);
    assert_eq!(frames.iter().filter(|f| f.released).count(), 1);
    assert!(frames.iter().all(|f| !(f.pressed && f.released)));
    assert!(frames.iter().filter(|f| f.pressed).all(|f| f.down));
    assert!(frames.iter().filter(|f| f.released).all(|f| !f.down));
    assert!(frames[..7].iter().all(|f| f.modifiers == scripted), "scripted modifiers must reach every motion and release: {frames:?}");
    r8_assert_nonpointer(&frames[7]);
    assert_eq!(frames[7].modifiers, native, "restore the held native modifier without a fresh native event");
}

fn r8_round_trip_last_edit(c: &mut Canvas, before_doc: &Value) {
    let after = r8_snapshot(c);
    c.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(&r8_snapshot(c)["doc"], before_doc, "undo must restore the entire preceding document");
    c.app.run("edit.redo", json!({})).unwrap();
    assert_eq!(r8_snapshot(c), after, "redo must restore the full document, selection, and every undo/redo entry");
}

fn r8_text_or_key_tail_before_click(prefix: R8TextPrefix) {
    let mut c = Canvas::new();
    c.select(0, 0);
    let mut reference = r8_reference(&c);
    let baseline = r8_snapshot(&c);
    let undo = c.app.session.doc().unwrap().history.undo.len();
    let click = c.glyph(12);
    // Queue the click before delivering native/key-tail text. ui.text already completes at its explicit caret.
    r8_prepare_text_prefix(&mut c, prefix);
    c.queue("ui.click", json!({"x": click.x, "y": click.y}));
    let prefix_frame = r8_deliver_text_prefix(&mut c, prefix);
    assert_eq!(c.range(), 1..1);
    assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
    // Native text precedes the hook's first motion in this same frame; count that motion exactly once.
    let mut frames = if prefix == R8TextPrefix::Native { vec![prefix_frame.unwrap()] } else { vec![] };
    frames.extend(c.drain());
    assert_gesture(&frames, Modifiers::NONE, 3);
    // A serial reference supplies the exact caret oracle at the unchanged screen position.
    r8_prepare_text_prefix(&mut reference, prefix);
    let _ = r8_deliver_text_prefix(&mut reference, prefix);
    assert!(reference.app.synthetic.is_empty());
    reference.queue("ui.click", json!({"x": click.x, "y": click.y}));
    reference.drain();
    assert_eq!(r8_snapshot(&c), r8_snapshot(&reference), "pipelining must match serial replay of the same native/control inputs");
    assert!(c.range().start > 1 && c.range().is_empty());
    assert!(c.app.session.doc().unwrap().interaction.is_none());
    r8_round_trip_last_edit(&mut c, &baseline["doc"]);
}

#[test]
fn r8_control_pipeline_text_before_click_full_doc_history_and_pacing() {
    for prefix in [R8TextPrefix::Native, R8TextPrefix::ImmediateControl] {
        r8_text_or_key_tail_before_click(prefix);
    }
}

#[test]
fn r8_control_pipeline_key_text_tail_before_click_full_doc_history_and_pacing() {
    r8_text_or_key_tail_before_click(R8TextPrefix::KeyTail);
}

#[test]
fn r8_control_pipeline_text_or_key_tail_before_alt_drag_restores_held_shift() {
    for prefix in [R8TextPrefix::Native, R8TextPrefix::KeyTail, R8TextPrefix::ImmediateControl] {
        let mut c = Canvas::new();
        c.select(0, 0);
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        let alt = Modifiers { alt: true, ..Modifiers::NONE };
        c.tick(vec![Event::ModifiersChanged(shift)]);
        let mut reference = r8_reference(&c);
        reference.tick(vec![Event::ModifiersChanged(shift)]);
        let undo = c.app.session.doc().unwrap().history.undo.len();
        let from = c.glyph(0);
        let to = c.end();
        let drag = json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "alt": true});
        // Prepare text, then pipeline the selection key and Alt drag before the first frame.
        r8_prepare_text_prefix(&mut c, prefix);
        c.queue("ui.key", json!({"key": "left", "shift": true}));
        c.queue("ui.drag", drag.clone());
        // Native/key-tail text shares the Shift-Left frame; immediate ui.text has already inserted.
        let selection_frame = r8_deliver_text_prefix(&mut c, prefix).unwrap_or_else(|| c.tick(vec![]));
        r8_assert_nonpointer(&selection_frame);
        assert_eq!(selection_frame.modifiers, shift);
        assert_eq!(c.text(), "ZHello brave world");
        assert_eq!(c.range(), 0..1);
        let before_copy = r8_snapshot(&c);
        let frames = c.drain();
        r8_assert_restored_gesture(&frames, alt, shift);
        assert_eq!(c.text(), "ZHello brave worldZ", "the selection created by the queued key must be copied after text insertion");
        assert_eq!(c.range(), 18..19);
        assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 2);
        r8_prepare_text_prefix(&mut reference, prefix);
        let _ = r8_deliver_text_prefix(&mut reference, prefix);
        assert!(reference.app.synthetic.is_empty());
        reference.queue("ui.key", json!({"key": "left", "shift": true}));
        reference.drain();
        reference.queue("ui.drag", drag);
        reference.drain();
        assert_eq!(r8_snapshot(&c), r8_snapshot(&reference));
        r8_round_trip_last_edit(&mut c, &before_copy["doc"]);
        assert_eq!(c.tick(vec![]).modifiers, shift);
        c.time += 1.0;
        let p = c.glyph(0);
        c.tick(vec![Event::PointerMoved(p)]);
        c.tick(vec![button(p, true, shift)]);
        c.tick(vec![button(p, false, shift)]);
        assert_eq!(c.range(), 0..18, "held native Shift must still extend after the pipeline without another ModifiersChanged");
        assert_eq!(c.text(), "ZHello brave worldZ");
    }
}

#[test]
fn r8_control_pipeline_text_or_key_tail_before_shift_drag_restores_held_alt() {
    for prefix in [R8TextPrefix::Native, R8TextPrefix::KeyTail, R8TextPrefix::ImmediateControl] {
        let mut c = Canvas::new();
        c.select(0, 0);
        let alt = Modifiers { alt: true, ..Modifiers::NONE };
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        c.tick(vec![Event::ModifiersChanged(alt)]);
        let mut reference = r8_reference(&c);
        reference.tick(vec![Event::ModifiersChanged(alt)]);
        let baseline = r8_snapshot(&c);
        let undo = c.app.session.doc().unwrap().history.undo.len();
        let from = c.glyph(6);
        let to = c.glyph(10);
        let drag = json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "shift": true});
        r8_prepare_text_prefix(&mut c, prefix);
        c.queue("ui.drag", drag.clone());
        let prefix_frame = r8_deliver_text_prefix(&mut c, prefix);
        // A native text frame also contains the first scripted motion and therefore its Shift.
        assert_eq!(c.ctx.input(|i| i.modifiers), if prefix == R8TextPrefix::Native { shift } else { alt });
        assert_eq!(c.text(), "ZHello brave world");
        assert_eq!(c.range(), 1..1);
        let mut frames = if prefix == R8TextPrefix::Native { vec![prefix_frame.unwrap()] } else { vec![] };
        frames.extend(c.drain());
        r8_assert_restored_gesture(&frames, shift, alt);
        let sel = c.app.session.doc().unwrap().selection.text.unwrap();
        assert_eq!(sel.anchor, 1, "Shift drag must extend the caret left by the preceding text");
        assert!(sel.focus > sel.anchor);
        assert_eq!(c.text(), "ZHello brave world");
        assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
        r8_prepare_text_prefix(&mut reference, prefix);
        let _ = r8_deliver_text_prefix(&mut reference, prefix);
        assert!(reference.app.synthetic.is_empty());
        reference.queue("ui.drag", drag);
        reference.drain();
        assert_eq!(r8_snapshot(&c), r8_snapshot(&reference));
        r8_round_trip_last_edit(&mut c, &baseline["doc"]);
        assert_eq!(c.tick(vec![]).modifiers, alt);
        c.time += 1.0;
        let p = c.glyph(0);
        c.tick(vec![Event::PointerMoved(p)]);
        c.tick(vec![button(p, true, alt)]);
        c.tick(vec![button(p, false, alt)]);
        assert_eq!(c.range(), 0..0, "synthetic Shift must be gone while native Alt remains held");
        assert_eq!(c.text(), "ZHello brave world");
    }
}

#[test]
fn r8_control_pipeline_focus_loss_restoration_preserves_following_click() {
    for prefix in [R8TextPrefix::Native, R8TextPrefix::KeyTail, R8TextPrefix::ImmediateControl] {
        let mut c = Canvas::new();
        c.select(0, 0);
        let alt = Modifiers { alt: true, ..Modifiers::NONE };
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        c.tick(vec![Event::ModifiersChanged(alt)]);
        let from = c.glyph(6);
        let to = c.glyph(10);
        let click = c.glyph(0);
        r8_prepare_text_prefix(&mut c, prefix);
        c.queue("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "shift": true}));
        c.queue("ui.click", json!({"x": click.x, "y": click.y}));
        let prefix_frame = r8_deliver_text_prefix(&mut c, prefix);
        assert_eq!(c.text(), "ZHello brave world");
        assert_eq!(c.range(), 1..1);
        let after_text = r8_snapshot(&c);
        let mut frames = if prefix == R8TextPrefix::Native { vec![prefix_frame.unwrap()] } else { vec![] };
        for _ in frames.len()..7 {
            frames.push(c.tick(vec![]));
        }
        assert!(frames.iter().all(|f| f.modifiers == shift));
        assert_eq!(c.app.synthetic.len(), 4, "one restoration marker and the untouched three-event click");
        let cleanup = c.tick(vec![Event::WindowFocused(false)]);
        r8_assert_nonpointer(&cleanup);
        assert_eq!(cleanup.modifiers, Modifiers::NONE, "focus loss must discard the cached native Alt hold");
        frames.push(cleanup);
        r8_assert_restored_gesture(&frames, shift, Modifiers::NONE);
        assert_eq!(c.app.synthetic.len(), 3, "restoration must not consume the next queued click");
        let motion = c.tick(vec![Event::WindowFocused(true)]);
        let mut click_frames = vec![motion];
        click_frames.extend(c.drain());
        assert_gesture(&click_frames, Modifiers::NONE, 3);
        assert_eq!(c.range(), 0..0);
        assert_eq!(c.text(), "ZHello brave world");
        assert_eq!(c.tick(vec![]).modifiers, Modifiers::NONE);
        assert_eq!(r8_snapshot(&c)["doc"], after_text["doc"]);
        assert_eq!(r8_snapshot(&c)["history"], after_text["history"]);
    }
}

#[test]
fn r8_control_pipeline_text_or_key_tail_before_plain_or_alt_drag_pacing() {
    for prefix in [R8TextPrefix::Native, R8TextPrefix::KeyTail, R8TextPrefix::ImmediateControl] {
        for scripted in [Modifiers::NONE, Modifiers { alt: true, ..Modifiers::NONE }] {
            let mut c = Canvas::new();
            c.select(0, 0);
            let mut reference = r8_reference(&c);
            let baseline = r8_snapshot(&c);
            let undo = c.app.session.doc().unwrap().history.undo.len();
            let from = c.glyph(6);
            let to = c.glyph(10);
            let drag = json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 4, "alt": scripted.alt});
            r8_prepare_text_prefix(&mut c, prefix);
            c.queue("ui.drag", drag.clone());
            let prefix_frame = r8_deliver_text_prefix(&mut c, prefix);
            assert_eq!(c.text(), "ZHello brave world");
            assert_eq!(c.range(), 1..1);
            let mut frames = if prefix == R8TextPrefix::Native { vec![prefix_frame.unwrap()] } else { vec![] };
            frames.extend(c.drain());
            if scripted == Modifiers::NONE {
                assert_gesture(&frames, Modifiers::NONE, 7);
            } else {
                r8_assert_restored_gesture(&frames, scripted, Modifiers::NONE);
            }
            // This gesture starts without a selected range; Alt must still reach egui
            // without changing ordinary text selection into a move or a copy.
            assert!(!c.range().is_empty());
            assert_eq!(c.text(), "ZHello brave world");
            assert_eq!(c.app.session.doc().unwrap().history.undo.len(), undo + 1);
            r8_prepare_text_prefix(&mut reference, prefix);
            let _ = r8_deliver_text_prefix(&mut reference, prefix);
            assert!(reference.app.synthetic.is_empty());
            reference.queue("ui.drag", drag);
            reference.drain();
            assert_eq!(r8_snapshot(&c), r8_snapshot(&reference));
            r8_round_trip_last_edit(&mut c, &baseline["doc"]);
        }
    }
}

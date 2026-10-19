//! Public command/tool regressions for issue #174. No private placement helpers are called.

use std::sync::Arc;

use designcraft_compose::{ComposedStory, Line};
use designcraft_doc::{ItemId, SpreadRef, StoryId};
use designcraft_geom::Point;
use designcraft_tools::{PointerEvent, PointerKind};
use serde_json::{Value, json};

use crate::{Session, ViewInfo};

fn fixture(text: &str, width: f64) -> (Session, ItemId, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("frame.create", &json!({"rect": [40, 40, 40.0 + width, 400], "content": "text", "text": text})).unwrap();
    s.set_tool("type");
    (s, ItemId(r["id"].as_u64().unwrap()), StoryId(r["story"].as_u64().unwrap()))
}

fn composed(s: &Session, sid: StoryId) -> Arc<ComposedStory> {
    s.cache.get(&s.doc().unwrap().doc, sid, None)
}

fn line(s: &Session, sid: StoryId, para: usize) -> Line {
    composed(s, sid).frames[0].lines.iter().find(|l| l.para == para).unwrap().clone()
}

fn spread_point(s: &Session, fid: ItemId, inner: Point) -> Point {
    let d = &s.doc().unwrap().doc;
    let loc = d.find(fid).unwrap();
    d.parent_xf(&loc) * d.text_xf(d.item(fid).unwrap()) * inner
}

fn point(s: &Session, fid: ItemId, l: &Line, x: f64) -> Point {
    spread_point(s, fid, Point::new(x, l.baseline - l.ascent * 0.5))
}

fn glyph_point(s: &Session, fid: ItemId, sid: StoryId, byte: usize) -> Point {
    let cs = composed(s, sid);
    let l = cs.frames[0].lines.iter().find(|l| l.glyphs.iter().any(|g| g.byte == byte && g.len > 0)).unwrap();
    let g = l.glyphs.iter().find(|g| g.byte == byte && g.len > 0).unwrap();
    point(s, fid, l, g.x + g.adv * 0.25)
}

fn pointer(s: &mut Session, kind: PointerKind, spread: Point) {
    let canvas = s.layout().xf(SpreadRef::Doc(0)) * spread;
    s.pointer(&PointerEvent::new(kind, canvas.x, canvas.y), ViewInfo::at_zoom(1.0)).unwrap();
}

fn range(s: &Session) -> std::ops::Range<usize> {
    s.doc().unwrap().selection.text.unwrap().range()
}

fn chars(s: &Session, sid: StoryId, byte: usize) -> Value {
    serde_json::to_value(s.doc().unwrap().doc.story(sid).unwrap().format_after(byte)).unwrap()
}

fn assert_blank_metrics(s: &Session, sid: StoryId, original: &[Line; 3]) {
    let now = [line(s, sid, 0), line(s, sid, 1), line(s, sid, 2)];
    assert!(now[1].glyphs.is_empty(), "fixture must retain its blank paragraph");
    assert!((now[1].leading - 40.0).abs() < 1e-6, "selected newline must supply blank-line leading: {}", now[1].leading);
    assert!(now[1].baseline > original[1].baseline + 20.0, "blank paragraph must consume the new leading");
    assert!(now[2].baseline > original[2].baseline + 20.0, "following text must move with the blank line");
    assert_eq!(now[0].baseline, original[0].baseline);
    for i in [0, 2] {
        assert_eq!((now[i].leading, now[i].ascent, now[i].descent), (original[i].leading, original[i].ascent, original[i].descent));
        assert_eq!(
            now[i].glyphs.iter().map(|g| (g.gid, g.byte, g.adv)).collect::<Vec<_>>(),
            original[i].glyphs.iter().map(|g| (g.gid, g.byte, g.adv)).collect::<Vec<_>>()
        );
    }
}

#[test]
fn r7a_selection_word_command_survives_release_inside_prior_selection() {
    let (mut s, fid, sid) = fixture("Hello brave world", 400.0);
    let p = glyph_point(&s, fid, sid, 8);
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 17})).unwrap();
    s.execute("text.selectWord", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
    assert_eq!(range(&s), 6..11, "word must come from the clicked byte, not the previous selection focus");
    s.execute("text.release", &json!({"frame": fid.0, "point": [p.x, p.y], "moved": false})).unwrap();
    assert_eq!(range(&s), 6..11, "release must retain word selection");
}

#[test]
fn r7a_selection_blank_newline_drag_formats_roundtrip_and_history() {
    let (mut s, fid, sid) = fixture("First\n\nLast", 400.0);
    let original = [line(&s, sid, 0), line(&s, sid, 1), line(&s, sid, 2)];
    let start = point(&s, fid, &original[1], original[1].end_x);
    let end = point(&s, fid, &original[1], original[1].end_x + 24.0);
    pointer(&mut s, PointerKind::Down, start);
    pointer(&mut s, PointerKind::Drag, end);
    pointer(&mut s, PointerKind::Up, end);
    assert_eq!(range(&s), 6..7, "horizontal drag must select only the middle blank paragraph's newline");
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let neighbors = [chars(&s, sid, 0), chars(&s, sid, 5), chars(&s, sid, 7)];
    s.execute("type.char", &json!({"attrs": {"leading": {"kind": "points", "value": 40}, "size": 20}})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(chars(&s, sid, 6)["over"]["size"], json!(20.0));
    assert_eq!([chars(&s, sid, 0), chars(&s, sid, 5), chars(&s, sid, 7)], neighbors, "neighbor character runs must be unchanged");
    assert_blank_metrics(&s, sid, &original);
    let after = s.doc().unwrap().doc.clone();
    let bytes = s.execute("file.serialize", &json!({})).unwrap();
    let mut reopened = Session::new();
    reopened.execute("file.openBytes", &json!({"name": after.title, "base64": bytes["base64"]})).unwrap();
    assert_eq!(serde_json::to_value(&*reopened.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
    assert_blank_metrics(&reopened, sid, &original);
    #[cfg(not(target_arch = "wasm32"))]
    {
        struct Saved(std::path::PathBuf);
        impl Drop for Saved {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
                if let Some(dir) = self.0.parent() {
                    let _ = std::fs::remove_dir(dir);
                }
            }
        }
        let dir = std::env::temp_dir().join(format!("designcraft-issue174-newline-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let saved = Saved(dir.join("selection.designcraft"));
        s.execute("file.saveAs", &json!({"path": saved.0.to_string_lossy()})).unwrap();
        let mut native = Session::new();
        native.execute("file.open", &json!({"path": saved.0.to_string_lossy()})).unwrap();
        assert_eq!(serde_json::to_value(&*native.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
        assert_blank_metrics(&native, sid, &original);
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
    assert_eq!(line(&s, sid, 1).leading, original[1].leading);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &after));
    assert_blank_metrics(&s, sid, &original);
}

#[test]
fn r7a_selection_explicit_newline_format_changes_composed_blank_leading() {
    let (mut s, _, sid) = fixture("First\n\nLast", 400.0);
    let original = [line(&s, sid, 0), line(&s, sid, 1), line(&s, sid, 2)];
    s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 7})).unwrap();
    let neighbors = [chars(&s, sid, 5), chars(&s, sid, 7)];
    s.execute("type.char", &json!({"attrs": {"leading": {"kind": "points", "value": 40}, "size": 20}})).unwrap();
    assert_eq!(
        chars(&s, sid, 6)["over"]["leading"],
        json!({"kind": "points", "value": 40.0}),
        "the existing nonempty range formatter already stores separator attributes"
    );
    assert_eq!([chars(&s, sid, 5), chars(&s, sid, 7)], neighbors);
    assert_blank_metrics(&s, sid, &original);
}

#[test]
fn r7a_selection_drag_past_nonblank_hard_end_includes_newline() {
    let (mut s, fid, sid) = fixture("First\nLast", 400.0);
    let l = line(&s, sid, 0);
    let start = glyph_point(&s, fid, sid, 0);
    let end = point(&s, fid, &l, l.end_x + 24.0);
    pointer(&mut s, PointerKind::Down, start);
    pointer(&mut s, PointerKind::Drag, end);
    pointer(&mut s, PointerKind::Up, end);
    assert_eq!(range(&s), 0..6, "hard paragraph separator belongs to a drag beyond the end");
}

#[test]
fn r7a_selection_drag_to_next_paragraph_already_selects_blank_newline() {
    let (mut s, fid, sid) = fixture("First\n\nLast", 400.0);
    let blank = line(&s, sid, 1);
    let start = point(&s, fid, &blank, blank.end_x);
    let end = glyph_point(&s, fid, sid, 7);
    pointer(&mut s, PointerKind::Down, start);
    pointer(&mut s, PointerKind::Drag, end);
    pointer(&mut s, PointerKind::Up, end);
    assert_eq!(range(&s), 6..7, "cross-line drag is a pre-existing newline-selection control");
}

#[test]
fn r7a_selection_ordinary_placement_and_exact_end_exclude_newline() {
    for text in ["First\nLast", "First\n\nLast"] {
        let (mut s, fid, sid) = fixture(text, 400.0);
        let pi = usize::from(text.contains("\n\n"));
        let l = line(&s, sid, pi);
        for x in [l.end_x, l.end_x + 24.0] {
            let p = point(&s, fid, &l, x);
            s.execute("edit.deselectAll", &json!({})).unwrap();
            s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
            assert_eq!(range(&s), l.range.end..l.range.end, "ordinary placement stays before newline");
        }
        let p = point(&s, fid, &l, l.end_x);
        s.execute("text.extendTo", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
        assert_eq!(range(&s), l.range.end..l.range.end, "an exact endpoint does not select a separator");
    }
}

#[test]
fn r7a_selection_soft_wrap_and_forced_break_keep_existing_hits() {
    for (text, width, forced) in [("one two three four five six seven eight nine ten", 90.0, false), ("First\u{2028}Last", 400.0, true)] {
        let (mut s, fid, sid) = fixture(text, width);
        let cs = composed(&s, sid);
        assert!(cs.frames[0].lines.len() > 1, "fixture must actually break lines");
        let l = &cs.frames[0].lines[0];
        assert!(!l.last_in_para);
        let inner = Point::new(l.end_x + 24.0, l.baseline - l.ascent * 0.5);
        let expected = designcraft_compose::hit(&cs, 0, inner).unwrap();
        assert_ne!(text.as_bytes().get(expected), Some(&b'\n'));
        if forced {
            assert!(text.contains('\u{2028}'));
        }
        let end = spread_point(&s, fid, inner);
        let start = glyph_point(&s, fid, sid, 0);
        pointer(&mut s, PointerKind::Down, start);
        pointer(&mut s, PointerKind::Drag, end);
        pointer(&mut s, PointerKind::Up, end);
        assert_eq!(range(&s), 0..expected, "extension must preserve the established non-paragraph hit");
    }
}

fn sidecar_bidi_fixture(text: &str, rtl_paragraph: bool) -> (Session, ItemId, StoryId) {
    const FAMILY: &str = "DC R7a Selection Sidecar Bidi";
    static ADDED: std::sync::Once = std::sync::Once::new();
    ADDED.call_once(|| {
        let glyphs = [
            ('\u{05d0}', 'X'),
            ('\u{05d1}', 'X'),
            ('\u{05d2}', 'X'),
            ('a', 'a'),
            ('b', 'b'),
            ('c', 'c'),
            (' ', ' '),
            ('T', 'T'),
            ('i', 'i'),
            ('l', 'l'),
        ];
        let bytes = designcraft_fonts::testing::font_mapping(FAMILY, &glyphs).unwrap();
        designcraft_fonts::FontDb::global().add_font(bytes);
    });
    let (mut s, fid, sid) = fixture(text, 400.0);
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
    s.execute("type.char", &json!({"attrs": {"fontFamily": FAMILY, "fontStyle": "Regular"}})).unwrap();
    s.execute(
        "type.para",
        &json!({"attrs": {
            "direction": if rtl_paragraph { "rightToLeft" } else { "leftToRight" },
            "align": if rtl_paragraph { "right" } else { "left" }
        }}),
    )
    .unwrap();
    s.execute("edit.deselectAll", &json!({})).unwrap();
    (s, fid, sid)
}

fn sidecar_check_rtl_separator(text: &str, rtl_paragraph: bool) {
    let (mut s, fid, sid) = sidecar_bidi_fixture(text, rtl_paragraph);
    let cs = composed(&s, sid);
    let l = line(&s, sid, 0);
    let b = text.find('\n').unwrap();
    assert_eq!(l.range.end, b);
    assert!(l.last_in_para);
    assert!(l.glyphs.iter().filter(|g| g.len > 0).all(|g| g.gid != 0 && g.adv > 0.0 && g.face.family == "DC R7a Selection Sidecar Bidi"));
    let last = l.glyphs.iter().rev().find(|g| g.len > 0).unwrap();
    assert!(last.rtl, "fixture must end in a source RTL cluster");
    let (fi, cx, _, _, _) = designcraft_compose::caret(&cs, b).unwrap();
    assert_eq!(fi, 0);
    assert!((cx - last.x).abs() < 1e-6);
    assert!(cx < l.end_x - 1e-6, "logical endpoint differs from right pen edge");

    // Stay within the endpoint's public hit region. In the mixed case, the
    // Latin/space run is immediately to the left, so do not cross its midpoint.
    let mut nearest_other = f64::INFINITY;
    for pos in l.glyphs.iter().filter(|g| g.len > 0).flat_map(|g| [g.byte, g.byte + g.len]).chain([l.range.start, l.range.end]) {
        let distance = (designcraft_compose::caret_x(&l, pos) - cx).abs();
        if distance > 1e-6 {
            nearest_other = nearest_other.min(distance);
        }
    }
    assert!(nearest_other.is_finite());
    let delta = (nearest_other * 0.25).min(2.0);
    let inner = Point::new(cx - delta, l.baseline - l.ascent * 0.5);
    assert_eq!(designcraft_compose::hit(&cs, 0, inner), Some(b), "public hit fixture gate");
    assert!(inner.x < cx && inner.x < l.end_x);
    let beyond = spread_point(&s, fid, inner);
    let exact = point(&s, fid, &l, cx);
    let inside_inner = Point::new(cx + delta, inner.y);
    assert_eq!(designcraft_compose::hit(&cs, 0, inside_inner), Some(b));
    let inside = spread_point(&s, fid, inside_inner);

    // Ordinary placement and the exact endpoint must retain compose::hit.
    for p in [exact, beyond, inside] {
        s.execute("edit.deselectAll", &json!({})).unwrap();
        s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
        assert_eq!(range(&s), b..b);
    }
    s.execute("text.select", &json!({"story": sid.0, "anchor": b, "focus": b})).unwrap();
    s.execute("text.extendTo", &json!({"frame": fid.0, "point": [exact.x, exact.y]})).unwrap();
    assert_eq!(range(&s), b..b, "exact RTL endpoint excludes the separator");
    s.execute("text.extendTo", &json!({"frame": fid.0, "point": [inside.x, inside.y]})).unwrap();
    assert_eq!(range(&s), b..b, "the interior side of the RTL endpoint excludes the separator");

    let original_doc = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    s.execute("text.extendTo", &json!({"frame": fid.0, "point": [beyond.x, beyond.y]})).unwrap();
    assert_eq!(range(&s), 0..b + 1, "extension beyond the logical RTL end must include the real hard separator");
    s.execute(
        "text.release",
        &json!({
            "frame": fid.0, "point": [beyond.x, beyond.y], "moved": true
        }),
    )
    .unwrap();
    assert_eq!(range(&s), 0..b + 1);
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &original_doc));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn r7a_sidecar_rtl_hard_separator_extension() {
    sidecar_check_rtl_separator("\u{05d0}\u{05d1}\u{05d2}\nTail", true);
}

#[test]
fn r7a_sidecar_mixed_tail_rtl_hard_separator_extension() {
    sidecar_check_rtl_separator("abc \u{05d0}\u{05d1}\u{05d2}\nTail", false);
}

#[test]
fn r7a_sidecar_transformed_and_vertical_ltr_controls() {
    for vertical in [false, true] {
        for transformed in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", &json!({})).unwrap();
            // Origin zero keeps the vertical quarter turn's exact logical endpoint exact.
            let r = s
                .execute(
                    "frame.create",
                    &json!({
                        "rect": [0, 0, 400, 400], "content": "text", "text": "First\nLast", "vertical": vertical
                    }),
                )
                .unwrap();
            let fid = ItemId(r["id"].as_u64().unwrap());
            let sid = StoryId(r["story"].as_u64().unwrap());
            s.set_tool("type");
            if transformed {
                // Reflect and translate across the line; the logical advance axis is unchanged.
                s.execute(
                    "transform.flip",
                    &json!({
                        "ids": [fid.0], "axis": if vertical { "horizontal" } else { "vertical" }
                    }),
                )
                .unwrap();
                s.execute(
                    "transform.move",
                    &json!({
                        "ids": [fid.0], "dx": if vertical { 32 } else { 0 }, "dy": if vertical { 0 } else { 32 }
                    }),
                )
                .unwrap();
            }
            let cs = composed(&s, sid);
            assert_eq!(cs.frames[0].vertical, vertical);
            let l = line(&s, sid, 0);
            assert!(l.last_in_para);
            assert_eq!(l.range.end, 5);
            assert!(l.glyphs.iter().filter(|g| g.len > 0).all(|g| !g.rtl));
            let (fi, end_x, _, _, _) = designcraft_compose::caret(&cs, 5).unwrap();
            assert_eq!(fi, 0);
            assert_eq!(end_x, l.end_x);
            let exact = point(&s, fid, &l, end_x);
            let beyond = point(&s, fid, &l, end_x + 24.0);
            let doc = &s.doc().unwrap().doc;
            let loc = doc.find(fid).unwrap();
            let xf = doc.parent_xf(&loc) * doc.text_xf(doc.item(fid).unwrap());
            for (p, x) in [(exact, end_x), (beyond, end_x + 24.0)] {
                let inner = xf.inverse() * p;
                assert_eq!(inner.x, x, "fixture must preserve exact logical x through the transform");
                assert_eq!(designcraft_compose::hit(&cs, 0, inner), Some(5));
                s.execute("edit.deselectAll", &json!({})).unwrap();
                s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
                assert_eq!(range(&s), 5..5, "ordinary placement retains compose::hit");
            }
            s.execute("text.extendTo", &json!({"frame": fid.0, "point": [exact.x, exact.y]})).unwrap();
            assert_eq!(range(&s), 5..5, "exact transformed/vertical endpoint excludes the newline");
            let original = s.doc().unwrap().doc.clone();
            let undo = s.doc().unwrap().history.undo.len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
            s.execute("text.extendTo", &json!({"frame": fid.0, "point": [beyond.x, beyond.y]})).unwrap();
            assert_eq!(range(&s), 0..6, "LTR extension beyond the hard end includes its newline");
            s.execute("text.release", &json!({"frame": fid.0, "point": [beyond.x, beyond.y], "moved": true})).unwrap();
            assert_eq!(range(&s), 0..6);
            assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &original));
            assert_eq!(s.doc().unwrap().history.undo.len(), undo);
        }
    }
}

fn blank_direction_fixture(rtl: bool, inherited: bool) -> (Session, ItemId, StoryId) {
    let (mut s, fid, sid) = fixture("First\n\nLast", 400.0);
    let neighbors = {
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        [serde_json::to_value(story.paras.first().unwrap()).unwrap(), serde_json::to_value(story.paras.get(2).unwrap()).unwrap()]
    };
    // A collapsed selection formats only the empty middle paragraph.
    s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 6})).unwrap();
    if inherited {
        const PARENT: &str = "R7a Blank Direction Parent";
        const CHILD: &str = "R7a Blank Direction Child";
        s.execute(
            "style.paragraph.create",
            &json!({
                "name": PARENT, "para": {"direction": if rtl { "rightToLeft" } else { "leftToRight" }, "align": "right"}
            }),
        )
        .unwrap();
        s.execute("style.paragraph.create", &json!({"name": CHILD, "basedOn": PARENT})).unwrap();
        s.execute("style.paragraph.apply", &json!({"name": CHILD, "clearOverrides": true})).unwrap();
        let doc = &s.doc().unwrap().doc;
        let pf = doc.story(sid).unwrap().paras.get(1).unwrap();
        assert_eq!(pf.style, CHILD);
        assert_eq!(pf.para.direction, None, "fixture must have no local direction override");
        assert_eq!(pf.para.align, None);
        let child = doc.styles.para(CHILD).unwrap();
        assert_eq!(child.based_on.as_deref(), Some(PARENT));
        assert_eq!(child.para.direction, None, "named child must inherit its direction");
        assert_eq!(child.para.align, None);
    } else {
        s.execute(
            "type.para",
            &json!({"attrs": {
                "direction": if rtl { "rightToLeft" } else { "leftToRight" }, "align": "right"
            }}),
        )
        .unwrap();
    }
    let doc = &s.doc().unwrap().doc;
    let story = doc.story(sid).unwrap();
    let pf = story.paras.get(1).unwrap();
    let props = doc.styles.resolve_para(pf).0;
    assert_eq!(props.direction, if rtl { designcraft_doc::TextDirection::RightToLeft } else { designcraft_doc::TextDirection::LeftToRight });
    assert_eq!(props.align, designcraft_doc::Align::Right);
    assert_eq!(
        [serde_json::to_value(story.paras.first().unwrap()).unwrap(), serde_json::to_value(story.paras.get(2).unwrap()).unwrap(),],
        neighbors,
        "only the middle paragraph is formatted"
    );
    (s, fid, sid)
}

fn check_blank_separator_direction(rtl: bool, inherited: bool) {
    let (mut s, fid, sid) = blank_direction_fixture(rtl, inherited);
    let cs = composed(&s, sid);
    let l = line(&s, sid, 1);
    assert!(l.glyphs.is_empty(), "fixture must be a glyphless middle paragraph");
    assert_eq!(l.range, 6..6);
    assert!(l.first_in_para && l.last_in_para);
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text.as_bytes().get(6), Some(&b'\n'));
    let (fi, cx, _, _, _) = designcraft_compose::caret(&cs, 6).unwrap();
    assert_eq!(fi, 0);
    assert_eq!(cx, l.end_x);
    assert_eq!(cx, l.x1, "right-aligned blank caret must be at the right edge");
    let advance = if rtl { -24.0 } else { 24.0 };
    let exact = point(&s, fid, &l, cx);
    let beyond = point(&s, fid, &l, cx + advance);
    let interior = point(&s, fid, &l, cx - advance);
    let doc = &s.doc().unwrap().doc;
    let loc = doc.find(fid).unwrap();
    let xf = doc.parent_xf(&loc) * doc.text_xf(doc.item(fid).unwrap());
    let original = doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let redo = s.doc().unwrap().history.redo.len();

    for p in [exact, beyond, interior] {
        assert_eq!(designcraft_compose::hit(&cs, fi, xf.inverse() * p), Some(6), "actual public hit must be the blank paragraph's LF");
        s.execute("edit.deselectAll", &json!({})).unwrap();
        s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
        assert_eq!(range(&s), 6..6, "ordinary placement excludes the LF on either side");
    }
    s.execute("text.extendTo", &json!({"frame": fid.0, "point": [exact.x, exact.y]})).unwrap();
    assert_eq!(range(&s), 6..6, "exact blank endpoint excludes the LF");

    s.execute("text.extendTo", &json!({"frame": fid.0, "point": [beyond.x, beyond.y]})).unwrap();
    assert_eq!(range(&s), 6..7, "extension beyond the effective paragraph end must select its LF");
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().slice(range(&s)), "\n");
    for (p, moved) in [(beyond, true), (exact, false), (interior, true), (beyond, false)] {
        s.execute("text.release", &json!({"frame": fid.0, "point": [p.x, p.y], "moved": moved})).unwrap();
        assert_eq!(range(&s), 6..7, "repeated release retains the selected LF");
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &original));
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
        assert_eq!(s.doc().unwrap().history.redo.len(), redo);
    }
    s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": 6})).unwrap();
    s.execute("text.extendTo", &json!({"frame": fid.0, "point": [interior.x, interior.y]})).unwrap();
    assert_eq!(range(&s), 6..6, "extension on the interior side excludes the LF");
    s.execute("text.release", &json!({"frame": fid.0, "point": [interior.x, interior.y], "moved": false})).unwrap();
    assert_eq!(range(&s), 6..6);
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &original));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert_eq!(s.doc().unwrap().history.redo.len(), redo);
}

#[test]
fn r7a_rtl_blank_explicit_direction_selects_separator() {
    check_blank_separator_direction(true, false);
}

#[test]
fn r7a_rtl_blank_inherited_style_direction_selects_separator() {
    check_blank_separator_direction(true, true);
}

#[test]
fn r7a_rtl_blank_ltr_right_aligned_control() {
    check_blank_separator_direction(false, false);
}

fn check_selection_endpoint_transform_matrix(rtl: bool, blank: bool) {
    let profiles = ["rotation/shear", "translated", "scaled", "horizontal reflection", "vertical reflection", "translated/scaled/reflected"];
    for (rotation, shear) in [(37.0, 13.0), (37.0, -13.0), (-37.0, 13.0), (-37.0, -13.0)] {
        for (profile, name) in profiles.iter().enumerate() {
            let (mut s, fid, sid) =
                if blank { blank_direction_fixture(rtl, false) } else { sidecar_bidi_fixture(if rtl { "\u{05d0}\nTail" } else { "a\nTail" }, rtl) };
            let len = s.doc().unwrap().doc.story(sid).unwrap().len();
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": len})).unwrap();
            s.execute("type.char", &json!({"attrs": {"size": 12.0, "leading": {"kind": "points", "value": 14.4}}})).unwrap();
            s.execute("edit.deselectAll", &json!({})).unwrap();
            // A public fixed first baseline makes the scalar geometry reproducible.
            s.execute("object.textFrameOptions", &json!({"ids": [fid.0], "firstBaseline": "fixed"})).unwrap();
            s.execute("transform.set", &json!({"ids": [fid.0], "ref": 4, "rotation": rotation, "shear": shear})).unwrap();
            if matches!(profile, 1 | 5) {
                s.execute("transform.move", &json!({"ids": [fid.0], "dx": 224.5, "dy": -113.75})).unwrap();
            }
            if matches!(profile, 2 | 5) {
                s.execute("transform.scale", &json!({"ids": [fid.0], "sx": 1.5, "sy": 0.75})).unwrap();
            }
            if matches!(profile, 3 | 5) {
                s.execute("transform.flip", &json!({"ids": [fid.0], "axis": "horizontal"})).unwrap();
            }
            if profile == 4 {
                s.execute("transform.flip", &json!({"ids": [fid.0], "axis": "vertical"})).unwrap();
            }

            let cs = composed(&s, sid);
            let l = line(&s, sid, usize::from(blank));
            let b = l.range.end;
            assert!(l.first_in_para && l.last_in_para);
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text.as_bytes().get(b), Some(&b'\n'));
            if blank {
                assert!(l.glyphs.is_empty(), "fixture must remain a glyphless paragraph");
                assert_eq!(l.range, 6..6);
                let doc = &s.doc().unwrap().doc;
                let para = doc.story(sid).unwrap().paras.get(l.para).unwrap();
                assert_eq!(
                    doc.styles.resolve_para(para).0.direction,
                    if rtl { designcraft_doc::TextDirection::RightToLeft } else { designcraft_doc::TextDirection::LeftToRight }
                );
            } else {
                let glyphs: Vec<_> = l.glyphs.iter().filter(|g| g.len > 0).collect();
                assert_eq!(glyphs.len(), 1, "single source cluster gives an unambiguous logical endpoint");
                let g = glyphs[0];
                assert_eq!(g.rtl, rtl);
                assert!(g.gid != 0 && g.adv > 1.0 && g.face.family == "DC R7a Selection Sidecar Bidi");
                assert_eq!(b, if rtl { 2 } else { 1 });
            }
            let (fi, cx, baseline, _, _) = designcraft_compose::caret(&cs, b).unwrap();
            assert_eq!(fi, 0);
            assert_eq!(baseline, l.baseline);
            if blank {
                assert_eq!(cx, l.end_x);
                assert_eq!(cx, l.x1);
            }
            let doc = &s.doc().unwrap().doc;
            let loc = doc.find(fid).unwrap();
            let xf = doc.parent_xf(&loc) * doc.text_xf(doc.item(fid).unwrap());
            // Use the actual public caret, without offsetting or snapping the exact point.
            let exact = xf * Point::new(cx, baseline);
            let advance = if rtl { -0.5 } else { 0.5 };
            let beyond = xf * Point::new(cx + advance, baseline);
            let interior = xf * Point::new(cx - advance, baseline);
            let inverse_exact = xf.inverse() * exact;
            let context =
                format!("rtl={rtl} blank={blank} rotation={rotation} shear={shear} profile={name} endpoint={cx:?} inverse_x={:?}", inverse_exact.x);
            assert!((inverse_exact.x - cx).abs() < 1e-6, "fixture must have only tiny round-trip error: {context}");
            let original = doc.clone();
            let undo = s.doc().unwrap().history.undo.len();
            let redo = s.doc().unwrap().history.redo.len();

            for p in [exact, beyond, interior] {
                assert_eq!(designcraft_compose::hit(&cs, fi, xf.inverse() * p), Some(b), "public hit must identify the real LF: {context}");
                s.execute("edit.deselectAll", &json!({})).unwrap();
                s.execute("text.placeCaret", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
                assert_eq!(range(&s), b..b, "ordinary placement must stay collapsed: {context}");
                s.execute("text.release", &json!({"frame": fid.0, "point": [p.x, p.y], "moved": false})).unwrap();
                assert_eq!(range(&s), b..b, "ordinary release retains the caret: {context}");
                assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &original));
                assert_eq!(s.doc().unwrap().history.undo.len(), undo);
                assert_eq!(s.doc().unwrap().history.redo.len(), redo);
            }

            for (label, p, end) in [("exact", exact, b), ("interior", interior, b), ("beyond", beyond, b + 1)] {
                s.execute("text.select", &json!({"story": sid.0, "anchor": b, "focus": b})).unwrap();
                s.execute("text.extendTo", &json!({"frame": fid.0, "point": [p.x, p.y]})).unwrap();
                assert_eq!(range(&s), b..end, "{label} extension must distinguish the mapped endpoint from half a point beyond it: {context}");
                let sel = s.doc().unwrap().selection.text.unwrap();
                assert_eq!((sel.anchor, sel.focus), (b, end), "extension preserves the anchor and advances in story order: {context}");
                assert!(sel.cell.is_none(), "outer-story regression");
                assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().slice(range(&s)), if end == b { "" } else { "\n" });
                for (released, moved) in [(p, true), (exact, false), (interior, true), (beyond, false)] {
                    s.execute("text.release", &json!({"frame": fid.0, "point": [released.x, released.y], "moved": moved})).unwrap();
                    assert_eq!(range(&s), b..end, "repeated release retains the {label} range: {context}");
                    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &original));
                    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
                    assert_eq!(s.doc().unwrap().history.redo.len(), redo);
                }
            }
        }
    }
}

#[test]
fn r7a_selection_endpoint_ltr_transform_matrix() {
    check_selection_endpoint_transform_matrix(false, false);
}

#[test]
fn r7a_selection_endpoint_rtl_transform_matrix() {
    check_selection_endpoint_transform_matrix(true, false);
}

#[test]
fn r7a_selection_endpoint_blank_ltr_transform_matrix() {
    check_selection_endpoint_transform_matrix(false, true);
}

#[test]
fn r7a_selection_endpoint_blank_rtl_transform_matrix() {
    check_selection_endpoint_transform_matrix(true, true);
}

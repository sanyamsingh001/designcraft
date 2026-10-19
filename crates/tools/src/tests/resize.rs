use super::*;

fn guide(orientation: Orientation, position: f64) -> Guide {
    Guide { orientation, position, spread: true, locked: false, layer: None, liquid: false }
}

fn add_guides(doc: &mut Document, rect: Rect) {
    Arc::make_mut(&mut doc.spreads[0]).pages[0].guides.extend([
        guide(Orientation::Vertical, rect.x0),
        guide(Orientation::Vertical, rect.x1),
        guide(Orientation::Horizontal, rect.y0),
        guide(Orientation::Horizontal, rect.y1),
    ]);
    doc.settings.baseline_grid.increment = 0.0;
}

fn guide_view() -> SnapView {
    SnapView { snap_to_guides: true, ..SnapView::OFF }
}

fn event(cx: &ToolContext, kind: PointerKind, at: Point, mods: Mods) -> PointerEvent {
    let p = cx.layout.to_canvas(SpreadRef::Doc(0), at);
    PointerEvent::new(kind, p.x, p.y).with_mods(mods)
}

fn begin_resize(tool: &mut dyn Tool, cx: &ToolContext, from: Rect, handle: usize, mods: Mods) {
    let at = select::handles(from)[handle];
    assert_eq!(tool.pointer(cx, &event(cx, PointerKind::Down, at, mods)), vec![Action::Begin("Resize".into())]);
}

fn drag_resize(tool: &mut dyn Tool, cx: &ToolContext, at: Point, mods: Mods, from: Rect, distribute: bool) -> Rect {
    let actions = tool.pointer(cx, &event(cx, PointerKind::Drag, at, mods));
    match &actions[..] {
        [Action::Preview(cmd, p)] => {
            assert_eq!(cmd, "transform.resize");
            let f = p["from"].as_array().unwrap();
            assert_rect(Rect::new(f[0].as_f64().unwrap(), f[1].as_f64().unwrap(), f[2].as_f64().unwrap(), f[3].as_f64().unwrap()), from);
            assert_eq!(p["content"], mods.cmd);
            assert_eq!(p["distribute"], distribute);
            let r = p["to"].as_array().unwrap();
            Rect::new(r[0].as_f64().unwrap(), r[1].as_f64().unwrap(), r[2].as_f64().unwrap(), r[3].as_f64().unwrap())
        }
        other => panic!("expected one resize preview, got {other:?}"),
    }
}

fn assert_rect(actual: Rect, expected: Rect) {
    assert!(near(Point::new(actual.x0, actual.y0), Point::new(expected.x0, expected.y0)), "{actual:?}, want {expected:?}");
    assert!(near(Point::new(actual.x1, actual.y1), Point::new(expected.x1, expected.y1)), "{actual:?}, want {expected:?}");
}

#[test]
fn resize_alt_handles_shrink_and_snap_with_aspect() {
    let from = Rect::new(100.0, 80.0, 220.0, 160.0);
    let targets = Rect::new(130.0, 100.0, 190.0, 140.0);
    for turn in 0..4 {
        let mut doc = Document::new(&NewDocument::default());
        let moving = add_rect(&mut doc, from);
        Arc::make_mut(&mut doc.spreads[0]).pages[0].view_rotation = turn;
        add_guides(&mut doc, targets);
        let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
        let mut cx = ctx(&doc, &sel, &cache, &layout);
        cx.snap = guide_view();
        for (shift, y_offset) in [(false, 2.0), (true, 2.0), (true, 0.5)] {
            for handle in 0..8 {
                let expected = if shift {
                    targets
                } else {
                    match handle {
                        1 | 5 => Rect::new(from.x0, targets.y0, from.x1, targets.y1),
                        3 | 7 => Rect::new(targets.x0, from.y0, targets.x1, from.y1),
                        _ => targets,
                    }
                };
                let mods = Mods { alt: true, shift, ..Mods::default() };
                let mut at = select::handles(expected)[handle];
                at.x += if matches!(handle, 0 | 6 | 7) { 2.0 } else { -2.0 };
                at.y += if matches!(handle, 0..=2) { y_offset } else { -y_offset };
                let mut tool = create("selection");
                begin_resize(tool.as_mut(), &cx, from, handle, mods);
                let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
                assert_rect(to, expected);
                assert!(near(to.center(), from.center()));
                if shift {
                    assert!((to.width() / to.height() - 1.5).abs() < 1e-4);
                }
            }
        }
    }
}

#[test]
fn resize_shift_sides_snap_and_keep_the_opposite_midpoint() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let mut doc = Document::new(&NewDocument::default());
    let moving = add_rect(&mut doc, from);
    add_guides(&mut doc, Rect::new(70.0, 80.0, 230.0, 170.0));
    let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
    let mut cx = ctx(&doc, &sel, &cache, &layout);
    cx.snap = guide_view();
    let mods = Mods { shift: true, ..Mods::default() };
    for (handle, at, expected) in [
        (1, Point::new(150.0, 82.0), Rect::new(80.0, 80.0, 220.0, 150.0)),
        (3, Point::new(228.0, 125.0), Rect::new(100.0, 92.5, 230.0, 157.5)),
        (5, Point::new(150.0, 168.0), Rect::new(80.0, 100.0, 220.0, 170.0)),
        (7, Point::new(72.0, 125.0), Rect::new(70.0, 92.5, 200.0, 157.5)),
    ] {
        let mut tool = create("selection");
        begin_resize(tool.as_mut(), &cx, from, handle, mods);
        let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
        assert_rect(to, expected);
        assert!(near(select::handles(to)[(handle + 4) % 8], select::handles(from)[(handle + 4) % 8]));
        assert!((to.width() / to.height() - 2.0).abs() < 1e-4);
    }
}

#[test]
fn resize_alt_crosses_the_center_without_losing_signed_spans() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let expected = Rect::new(170.0, 135.0, 130.0, 115.0);
    for shift in [false, true] {
        for (handle, at) in
            [(0, Point::new(170.0, 135.0)), (2, Point::new(130.0, 135.0)), (4, Point::new(130.0, 115.0)), (6, Point::new(170.0, 115.0))]
        {
            let to = select::resize_rect(from, handle, at, Mods { alt: true, shift, ..Mods::default() });
            assert_rect(to, expected);
            assert!(near(to.center(), from.center()));
        }
    }
}

#[test]
fn resize_shift_leaves_the_dependent_axis_unsnapped() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let mut doc = Document::new(&NewDocument::default());
    let moving = add_rect(&mut doc, from);
    Arc::make_mut(&mut doc.spreads[0]).pages[0].guides.push(guide(Orientation::Horizontal, 170.0));
    doc.settings.baseline_grid.increment = 0.0;
    let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
    let mut cx = ctx(&doc, &sel, &cache, &layout);
    cx.snap = guide_view();
    let mods = Mods { shift: true, ..Mods::default() };
    let mut tool = create("selection");
    begin_resize(tool.as_mut(), &cx, from, 4, mods);
    let to = drag_resize(tool.as_mut(), &cx, Point::new(238.0, 160.0), mods, from, false);
    assert_rect(to, Rect::new(100.0, 100.0, 238.0, 169.0));
}

#[test]
fn resize_repeated_previews_keep_the_initial_anchor_and_clear_feedback() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let mut doc = Document::new(&NewDocument { width: 320.0, ..NewDocument::default() });
    let moving = add_rect(&mut doc, from);
    let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
    let mut cx = ctx(&doc, &sel, &cache, &layout);
    cx.snap = SnapView { smart_guides: true, align_edges: true, ..SnapView::OFF };
    let mut tool = create("selection");
    begin_resize(tool.as_mut(), &cx, from, 3, Mods::default());
    let to = drag_resize(tool.as_mut(), &cx, Point::new(318.0, 125.0), Mods::default(), from, false);
    assert_rect(to, Rect::new(100.0, 100.0, 320.0, 150.0));
    assert!(!tool.overlays(&cx).is_empty());
    // The next sample sees the previous preview in the live document.
    doc.item_mut(moving).unwrap().path = shapes::rectangle(to);
    let mut cx = ctx(&doc, &sel, &cache, &layout);
    cx.snap = SnapView { smart_guides: true, align_edges: true, ..SnapView::OFF };
    let to = drag_resize(tool.as_mut(), &cx, Point::new(278.0, 125.0), Mods::default(), from, false);
    assert_rect(to, Rect::new(100.0, 100.0, 278.0, 150.0));
    assert!(tool.overlays(&cx).is_empty());
    assert_eq!(tool.pointer(&cx, &event(&cx, PointerKind::Up, Point::new(278.0, 125.0), Mods::default())), vec![Action::Commit]);
}

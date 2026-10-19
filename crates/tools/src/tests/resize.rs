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
fn resize_handles_snap_to_guides_page_edges_and_grid() {
    for category in 0..3 {
        for turn in 0..4 {
            for rotated_object in [false, true] {
                let mut doc = Document::new(&NewDocument { width: 320.0, height: 240.0, ..NewDocument::default() });
                let moving = add_rect(&mut doc, Rect::new(100.0, 80.0, 220.0, 160.0));
                if rotated_object {
                    doc.item_mut(moving).unwrap().xf = Affine::rotate_about(30.0_f64.to_radians(), Point::new(160.0, 120.0));
                }
                Arc::make_mut(&mut doc.spreads[0]).pages[0].view_rotation = turn;
                let targets = match category {
                    0 => Rect::new(40.0, 30.0, 280.0, 210.0),
                    1 => doc.spreads[0].pages[0].bounds(),
                    _ => Rect::new(40.0, 40.0, 280.0, 200.0),
                };
                add_guides(&mut doc, targets);
                doc.settings.grid.horizontal = 40.0;
                doc.settings.grid.vertical = 40.0;
                doc.settings.grid.subdivisions = 2;
                let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
                let from = doc.item(moving).unwrap().bounds();
                let mut cx = ctx(&doc, &sel, &cache, &layout);
                cx.snap = match category {
                    0 => guide_view(),
                    1 => SnapView { smart_guides: true, align_edges: true, ..SnapView::OFF },
                    _ => SnapView { snap_to_document_grid: true, ..SnapView::OFF },
                };
                for cmd in [false, true] {
                    let mods = Mods { cmd, ..Mods::default() };
                    for handle in 0..8 {
                        let left = matches!(handle, 0 | 6 | 7);
                        let right = matches!(handle, 2..=4);
                        let top = matches!(handle, 0..=2);
                        let bottom = matches!(handle, 4..=6);
                        let expected = Rect::new(
                            if left { targets.x0 } else { from.x0 },
                            if top { targets.y0 } else { from.y0 },
                            if right { targets.x1 } else { from.x1 },
                            if bottom { targets.y1 } else { from.y1 },
                        );
                        let mut at = select::handles(expected)[handle];
                        if left || right {
                            at.x += 2.0;
                        }
                        if top || bottom {
                            at.y += 2.0;
                        }
                        let mut tool = create("selection");
                        begin_resize(tool.as_mut(), &cx, from, handle, mods);
                        let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
                        assert_rect(to, expected);
                        let overlays = tool.overlays(&cx);
                        if category == 1 {
                            // Page alignment feedback must lie on the snapped edge in canvas coordinates.
                            let inverse = layout.xf(SpreadRef::Doc(0)).inverse();
                            let guides: Vec<_> = overlays
                                .iter()
                                .filter_map(|o| match o {
                                    Overlay::Guide { a, b } => Some((inverse * *a, inverse * *b)),
                                    _ => None,
                                })
                                .collect();
                            assert_eq!(guides.len(), usize::from(left || right) + usize::from(top || bottom));
                            if left || right {
                                let x = if left { targets.x0 } else { targets.x1 };
                                assert!(guides.iter().any(|(a, b)| (a.x - x).abs() < 1e-4 && (b.x - x).abs() < 1e-4));
                            }
                            if top || bottom {
                                let y = if top { targets.y0 } else { targets.y1 };
                                assert!(guides.iter().any(|(a, b)| (a.y - y).abs() < 1e-4 && (b.y - y).abs() < 1e-4));
                            }
                        } else {
                            assert!(overlays.is_empty());
                        }
                        assert_eq!(tool.pointer(&cx, &event(&cx, PointerKind::Up, at, mods)), vec![Action::Commit]);
                        assert!(tool.overlays(&cx).is_empty());
                    }
                }
            }
        }
    }
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

fn assert_alt_crossed_corner_dimension_snap(horizontal: bool) {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let center = from.center();
    let cases = if horizontal {
        [(0, Point::new(180.0, 115.0)), (2, Point::new(120.0, 115.0)), (4, Point::new(120.0, 135.0)), (6, Point::new(180.0, 135.0))]
    } else {
        [(0, Point::new(130.0, 140.0)), (2, Point::new(170.0, 140.0)), (4, Point::new(170.0, 110.0)), (6, Point::new(130.0, 110.0))]
    };
    // Each target is three points above or below the crossed driving size (60 wide or 30 high).
    // The other reference dimension cannot attract the dependent axis.
    for target in if horizontal { [57.0, 63.0] } else { [27.0, 33.0] } {
        let mut doc = Document::new(&NewDocument::default());
        let moving = add_rect(&mut doc, from);
        let reference = if horizontal { Rect::new(300.0, 300.0, 300.0 + target, 340.0) } else { Rect::new(300.0, 300.0, 380.0, 300.0 + target) };
        add_rect(&mut doc, reference);
        let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
        let mut cx = ctx(&doc, &sel, &cache, &layout);
        cx.snap = SnapView { smart_guides: true, smart_dimensions: true, ..SnapView::OFF };
        for shift in [false, true] {
            let mods = Mods { alt: true, shift, ..Mods::default() };
            let (w, h) =
                if horizontal { (-target, if shift { -target / 2.0 } else { 20.0 }) } else { (if shift { -target * 2.0 } else { 40.0 }, -target) };
            let expected = Rect::new(center.x - w / 2.0, center.y - h / 2.0, center.x + w / 2.0, center.y + h / 2.0);
            for (handle, at) in cases {
                let mut tool = create("selection");
                begin_resize(tool.as_mut(), &cx, from, handle, mods);
                let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
                assert_rect(to, expected);
                assert!(near(to.center(), center));
                assert!((if horizontal { to.width().abs() } else { to.height().abs() } - target).abs() < 1e-4);
                if shift {
                    assert!((to.width() / to.height() - 2.0).abs() < 1e-4);
                }
            }
        }
    }
}

#[test]
fn resize_alt_crossed_corners_snap_horizontal_dimensions_with_and_without_shift() {
    assert_alt_crossed_corner_dimension_snap(true);
}

#[test]
fn resize_alt_crossed_corners_snap_vertical_dimensions_with_and_without_shift() {
    assert_alt_crossed_corner_dimension_snap(false);
}

fn add_dimension_reference(doc: &mut Document, horizontal: bool, length: f64) {
    let rect = if horizontal { Rect::new(300.0, 300.0, 300.0 + length, 380.0) } else { Rect::new(300.0, 300.0, 380.0, 300.0 + length) };
    let reference = add_rect(doc, rect);
    doc.item_mut(reference).unwrap().stroke = Stroke::none();
}

fn assert_dimension_feedback(tool: &dyn Tool, cx: &ToolContext, to: Rect, horizontal: bool, length: f64) {
    let overlays = tool.overlays(cx);
    let labels: Vec<_> = overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Measure { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, vec![designcraft_geom::format_measure(length, cx.unit)]);
    let inverse = cx.layout.xf(SpreadRef::Doc(0)).inverse();
    let guides: Vec<_> = overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Guide { a, b } => Some((inverse * *a, inverse * *b)),
            _ => None,
        })
        .collect();
    assert_eq!(guides.len(), 1);
    let (a, b) = if horizontal {
        (Point::new(to.x0, to.y0.max(to.y1)), Point::new(to.x1, to.y0.max(to.y1)))
    } else {
        (Point::new(to.x0.max(to.x1), to.y0), Point::new(to.x0.max(to.x1), to.y1))
    };
    assert!(
        (near(guides[0].0, a) && near(guides[0].1, b)) || (near(guides[0].0, b) && near(guides[0].1, a)),
        "{guides:?}, want the segment {a:?}..{b:?}"
    );
}

fn assert_reflected_non_alt_dimension_snap(horizontal: bool) {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let handles: &[usize] = if horizontal { &[3, 7, 0, 2, 4, 6] } else { &[5, 1, 0, 2, 4, 6] };
    // Grow and shrink the reflected driving span. Horizontal handle 3 with a
    // 60 pt target is the reported [42,125] pointer: signed width -58 must become -60.
    for target in if horizontal { [60.0, 56.0] } else { [30.0, 26.0] } {
        let mut doc = Document::new(&NewDocument::default());
        let moving = add_rect(&mut doc, from);
        doc.item_mut(moving).unwrap().stroke = Stroke::none();
        add_dimension_reference(&mut doc, horizontal, target);
        let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
        let mut cx = ctx(&doc, &sel, &cache, &layout);
        cx.snap = SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF };
        for shift in [false, true] {
            for cmd in [false, true] {
                let mods = Mods { shift, cmd, ..Mods::default() };
                for &handle in handles {
                    let anchor = select::handles(from)[(handle + 4) % 8];
                    let left = matches!(handle, 0 | 6 | 7);
                    let right = matches!(handle, 2..=4);
                    let top = matches!(handle, 0..=2);
                    let bottom = matches!(handle, 4..=6);
                    // The other corner dimension stays far from either reference
                    // length and cannot become Shift's driving axis.
                    let (raw_w, raw_h) = if horizontal { (-58.0, 20.0) } else { (40.0, -28.0) };
                    let at = Point::new(
                        if left {
                            anchor.x - raw_w
                        } else if right {
                            anchor.x + raw_w
                        } else {
                            anchor.x
                        },
                        if top {
                            anchor.y - raw_h
                        } else if bottom {
                            anchor.y + raw_h
                        } else {
                            anchor.y
                        },
                    );
                    let (w, h) = if horizontal {
                        (
                            -target,
                            if shift {
                                -target / 2.0
                            } else if top || bottom {
                                raw_h
                            } else {
                                from.height()
                            },
                        )
                    } else {
                        (
                            if shift {
                                -target * 2.0
                            } else if left || right {
                                raw_w
                            } else {
                                from.width()
                            },
                            -target,
                        )
                    };
                    let expected = Rect::new(
                        if left {
                            anchor.x - w
                        } else if right {
                            anchor.x
                        } else {
                            anchor.x - w / 2.0
                        },
                        if top {
                            anchor.y - h
                        } else if bottom {
                            anchor.y
                        } else {
                            anchor.y - h / 2.0
                        },
                        if left {
                            anchor.x
                        } else if right {
                            anchor.x + w
                        } else {
                            anchor.x + w / 2.0
                        },
                        if top {
                            anchor.y
                        } else if bottom {
                            anchor.y + h
                        } else {
                            anchor.y + h / 2.0
                        },
                    );
                    let mut tool = create("selection");
                    begin_resize(tool.as_mut(), &cx, from, handle, mods);
                    let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
                    assert_rect(to, expected);
                    assert!(near(select::handles(to)[(handle + 4) % 8], anchor));
                    let signed_length = if horizontal { to.width() } else { to.height() };
                    assert!(signed_length < 0.0 && (signed_length.abs() - target).abs() < 1e-4);
                    if shift {
                        assert!((to.width() / to.height() - 2.0).abs() < 1e-4);
                    }
                    assert_dimension_feedback(tool.as_ref(), &cx, to, horizontal, target);
                    assert_eq!(tool.pointer(&cx, &event(&cx, PointerKind::Up, at, mods)), vec![Action::Commit]);
                    assert!(tool.overlays(&cx).is_empty());
                }
            }
        }
    }
}

#[test]
fn resize_reflected_non_alt_horizontal_dimensions_keep_opposite_anchors() {
    assert_reflected_non_alt_dimension_snap(true);
}

#[test]
fn resize_reflected_non_alt_vertical_dimensions_keep_opposite_anchors() {
    assert_reflected_non_alt_dimension_snap(false);
}

#[test]
fn resize_reflected_non_alt_position_snaps_keep_spread_deltas() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let mut doc = Document::new(&NewDocument::default());
    let moving = add_rect(&mut doc, from);
    doc.item_mut(moving).unwrap().stroke = Stroke::none();
    add_guides(&mut doc, Rect::new(260.0, 180.0, 40.0, 70.0));
    let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
    let mut cx = ctx(&doc, &sel, &cache, &layout);
    cx.snap = guide_view();
    for shift in [false, true] {
        let mods = Mods { shift, ..Mods::default() };
        for (handle, at, plain, constrained) in [
            (3, Point::new(42.0, 125.0), Rect::new(100.0, 100.0, 40.0, 150.0), Rect::new(100.0, 140.0, 40.0, 110.0)),
            (7, Point::new(258.0, 125.0), Rect::new(260.0, 100.0, 200.0, 150.0), Rect::new(260.0, 140.0, 200.0, 110.0)),
            (1, Point::new(150.0, 178.0), Rect::new(100.0, 180.0, 200.0, 150.0), Rect::new(180.0, 180.0, 120.0, 150.0)),
            (5, Point::new(150.0, 72.0), Rect::new(100.0, 100.0, 200.0, 70.0), Rect::new(180.0, 100.0, 120.0, 70.0)),
        ] {
            let mut tool = create("selection");
            begin_resize(tool.as_mut(), &cx, from, handle, mods);
            let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
            assert_rect(to, if shift { constrained } else { plain });
            assert!(near(select::handles(to)[(handle + 4) % 8], select::handles(from)[(handle + 4) % 8]));
            assert!(tool.overlays(&cx).is_empty(), "ruler-guide snapping does not emit dimension feedback");
            assert_eq!(tool.pointer(&cx, &event(&cx, PointerKind::Up, at, mods)), vec![Action::Commit]);
        }
    }
}

fn assert_stroked_corner_dimension_filter(horizontal: bool, alternative: Option<f64>, alt: bool) {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let center = from.center();
    let mut doc = Document::new(&NewDocument::default());
    let moving = add_rect(&mut doc, from);
    doc.item_mut(moving).unwrap().stroke = Stroke { weight: 4.0, ..Stroke::default() };
    add_dimension_reference(&mut doc, horizontal, 3.0);
    if let Some(length) = alternative {
        add_dimension_reference(&mut doc, horizontal, length);
    }
    let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
    let mut cx = ctx(&doc, &sel, &cache, &layout);
    cx.snap = SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF };
    let signs: &[f64] = if alt { &[-1.0, 1.0] } else { &[1.0] };
    for shift in [false, true] {
        let mods = Mods { alt, shift, ..Mods::default() };
        for &sign in signs {
            // The visible proposal is 5 pt, so the impossible 3 pt target is closer
            // than the reachable 8 pt target. Filter before selecting the nearest.
            let (raw_w, raw_h) = if horizontal { (sign, if shift { 0.25 } else { 20.0 }) } else { (if shift { 1.0 } else { 20.0 }, sign) };
            let size = alternative.map_or(1.0, |length| length - 4.0) * sign;
            let (w, h) = if horizontal { (size, if shift { size / 2.0 } else { 20.0 }) } else { (if shift { size * 2.0 } else { 20.0 }, size) };
            for handle in [0, 2, 4, 6] {
                let left = matches!(handle, 0 | 6);
                let top = matches!(handle, 0 | 2);
                let anchor = select::handles(from)[(handle + 4) % 8];
                let at = if alt {
                    Point::new(center.x + if left { -raw_w / 2.0 } else { raw_w / 2.0 }, center.y + if top { -raw_h / 2.0 } else { raw_h / 2.0 })
                } else {
                    Point::new(anchor.x + if left { -raw_w } else { raw_w }, anchor.y + if top { -raw_h } else { raw_h })
                };
                let expected = if alt {
                    Rect::new(center.x - w / 2.0, center.y - h / 2.0, center.x + w / 2.0, center.y + h / 2.0)
                } else {
                    Rect::new(
                        if left { anchor.x - w } else { anchor.x },
                        if top { anchor.y - h } else { anchor.y },
                        if left { anchor.x } else { anchor.x + w },
                        if top { anchor.y } else { anchor.y + h },
                    )
                };
                let mut tool = create("selection");
                begin_resize(tool.as_mut(), &cx, from, handle, mods);
                let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
                assert_rect(to, expected);
                if alt {
                    assert!(near(to.center(), center));
                } else {
                    assert!(near(select::handles(to)[(handle + 4) % 8], anchor));
                }
                if let Some(length) = alternative {
                    assert!((if horizontal { to.width().abs() } else { to.height().abs() } + 4.0 - length).abs() < 1e-4);
                    assert_dimension_feedback(tool.as_ref(), &cx, to, horizontal, length);
                } else {
                    assert!(tool.overlays(&cx).is_empty(), "an unreachable dimension must not draw feedback");
                }
            }
        }
    }
}

#[test]
fn resize_alt_corners_ignore_dimensions_below_fixed_stroke_outset() {
    for horizontal in [false, true] {
        assert_stroked_corner_dimension_filter(horizontal, None, true);
    }
}

#[test]
fn resize_alt_corners_choose_a_reachable_dimension_over_a_closer_unreachable_one() {
    for horizontal in [false, true] {
        assert_stroked_corner_dimension_filter(horizontal, Some(8.0), true);
    }
}

#[test]
fn resize_stroked_dimension_filter_preserves_normal_opposite_anchors() {
    for horizontal in [false, true] {
        for alternative in [None, Some(8.0)] {
            assert_stroked_corner_dimension_filter(horizontal, alternative, false);
        }
    }
}

#[test]
fn resize_command_content_keeps_dimensions_below_the_original_stroke_outset() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let center = from.center();
    for horizontal in [false, true] {
        let mut doc = Document::new(&NewDocument::default());
        let moving = add_rect(&mut doc, from);
        doc.item_mut(moving).unwrap().stroke = Stroke { weight: 4.0, ..Stroke::default() };
        add_dimension_reference(&mut doc, horizontal, 3.0);
        add_dimension_reference(&mut doc, horizontal, 8.0);
        let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
        let mut cx = ctx(&doc, &sel, &cache, &layout);
        cx.snap = SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF };
        for alt in [false, true] {
            for shift in [false, true] {
                let (w, h) = if horizontal { (1.0, if shift { 0.25 } else { 20.0 }) } else { (if shift { 1.0 } else { 20.0 }, 1.0) };
                let at = if alt { Point::new(center.x + w / 2.0, center.y + h / 2.0) } else { Point::new(from.x0 + w, from.y0 + h) };
                for cmd in [false, true] {
                    let mods = Mods { cmd, alt, shift, ..Mods::default() };
                    let mut tool = create("selection");
                    begin_resize(tool.as_mut(), &cx, from, 4, mods);
                    let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
                    // A 5 pt proposal is closer to 3 than 8. Ordinary frame resizing
                    // rejects 3 because its 4 pt stroke is fixed; content resizing
                    // must retain the old candidate choice because the stroke can scale.
                    // This tests selection/feedback, not the final scaled-stroke bounds.
                    assert_dimension_feedback(tool.as_ref(), &cx, to, horizontal, if cmd { 3.0 } else { 8.0 });
                }
            }
        }
    }
}

#[test]
fn resize_collapsed_sides_can_match_a_reachable_visible_dimension() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let center = from.center();
    for horizontal in [false, true] {
        let mut doc = Document::new(&NewDocument::default());
        let moving = add_rect(&mut doc, from);
        doc.item_mut(moving).unwrap().stroke = Stroke { weight: 4.0, ..Stroke::default() };
        add_dimension_reference(&mut doc, horizontal, 3.0);
        add_dimension_reference(&mut doc, horizontal, 8.0);
        let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
        let mut cx = ctx(&doc, &sel, &cache, &layout);
        cx.snap = SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF };
        for alt in [false, true] {
            for shift in [false, true] {
                let mods = Mods { alt, shift, ..Mods::default() };
                let (handle, at) = if horizontal {
                    (3, Point::new(if alt { center.x } else { from.x0 }, center.y))
                } else {
                    (5, Point::new(center.x, if alt { center.y } else { from.y0 }))
                };
                let (w, h) = if horizontal { (4.0, if shift { 2.0 } else { from.height() }) } else { (if shift { 8.0 } else { from.width() }, 4.0) };
                let expected = if alt {
                    Rect::new(center.x - w / 2.0, center.y - h / 2.0, center.x + w / 2.0, center.y + h / 2.0)
                } else if horizontal {
                    Rect::new(from.x0, center.y - h / 2.0, from.x0 + w, center.y + h / 2.0)
                } else {
                    Rect::new(center.x - w / 2.0, from.y0, center.x + w / 2.0, from.y0 + h)
                };
                let mut tool = create("selection");
                begin_resize(tool.as_mut(), &cx, from, handle, mods);
                let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
                assert_rect(to, expected);
                assert_dimension_feedback(tool.as_ref(), &cx, to, horizontal, 8.0);
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
fn resize_respects_snap_switches_zoom_and_inactive_edges() {
    let from = Rect::new(100.0, 100.0, 200.0, 150.0);
    let mut doc = Document::new(&NewDocument::default());
    let moving = add_rect(&mut doc, from);
    add_guides(&mut doc, Rect::new(80.0, 102.0, 203.0, 148.0));
    let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
    for (view, zoom, pointer_x, expected_x) in [
        (guide_view(), 1.0, 202.0, 203.0),
        (guide_view(), 4.0, 202.0, 203.0),
        (guide_view(), 4.0, 201.5, 201.5),
        (SnapView::OFF, 1.0, 202.0, 202.0),
        (SnapView { show_guides: false, ..guide_view() }, 1.0, 202.0, 202.0),
        (SnapView { zone_px: 0.0, ..guide_view() }, 1.0, 202.0, 202.0),
        // The selection's own old edge at 200 is not an alignment target.
        (SnapView { smart_guides: true, align_edges: true, ..SnapView::OFF }, 1.0, 202.0, 202.0),
    ] {
        let mut cx = ctx(&doc, &sel, &cache, &layout);
        cx.snap = view;
        cx.zoom = zoom;
        let mut tool = create("selection");
        begin_resize(tool.as_mut(), &cx, from, 3, Mods::default());
        let to = drag_resize(tool.as_mut(), &cx, Point::new(pointer_x, 125.0), Mods::default(), from, false);
        assert_rect(to, Rect::new(100.0, 100.0, expected_x, 150.0));
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

#[test]
fn resize_live_distribute_skips_snapping() {
    let mut doc = Document::new(&NewDocument::default());
    let a = add_rect(&mut doc, Rect::new(100.0, 100.0, 150.0, 150.0));
    let b = add_rect(&mut doc, Rect::new(200.0, 100.0, 250.0, 150.0));
    add_guides(&mut doc, Rect::new(80.0, 80.0, 300.0, 170.0));
    let (sel, cache, layout) = (Selection::items(vec![a, b]), Cache::new(), CanvasLayout::new(&doc, false));
    let mut cx = ctx(&doc, &sel, &cache, &layout);
    cx.snap = guide_view();
    let from = Rect::new(100.0, 100.0, 250.0, 150.0);
    let mods = Mods { space: true, ..Mods::default() };
    let mut tool = create("selection");
    begin_resize(tool.as_mut(), &cx, from, 3, mods);
    let to = drag_resize(tool.as_mut(), &cx, Point::new(298.0, 125.0), mods, from, true);
    assert_rect(to, Rect::new(100.0, 100.0, 298.0, 150.0));
    assert!(tool.overlays(&cx).is_empty());
}

#[test]
fn r35_scaled_affine_strokes_skip_dimensions_but_zero_outsets_still_match() {
    let transforms = [
        Affine::scale(2.0),
        Affine::scale_non_uniform(2.0, 0.5),         // determinant 1 does not preserve stroke outsets
        Affine::new([1.0, 0.0, 1.0, 1.0, 0.0, 0.0]), // shear with determinant 1
        Affine::rotate(30.0_f64.to_radians()),
        Affine::scale_non_uniform(-2.0, 2.0),
        Affine::rotate(30.0_f64.to_radians()) * Affine::scale_non_uniform(-1.0, 1.0),
    ];
    for xf in transforms {
        for stroke in [true, false] {
            let mut doc = Document::new(&NewDocument::default());
            let moving = add_rect(&mut doc, Rect::new(0.0, 0.0, 100.0, 50.0));
            let item = doc.item_mut(moving).unwrap();
            item.xf = xf;
            item.stroke = if stroke { Stroke { weight: 4.0, ..Stroke::default() } } else { Stroke::none() };
            let bounds = item.bounds();
            item.xf = Affine::translate((100.0 - bounds.x0, 100.0 - bounds.y0)) * xf;
            let from = item.bounds();
            let width = from.width() / 4.0;
            let target = width + 4.0; // 54 pt in the reported 2x-frame case
            add_dimension_reference(&mut doc, true, target);
            let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
            let mut cx = ctx(&doc, &sel, &cache, &layout);
            cx.snap = SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF };
            let mods = Mods { cmd: true, ..Mods::default() };
            let mut tool = create("selection");
            begin_resize(tool.as_mut(), &cx, from, 3, mods);
            let at = Point::new(from.x0 + width, from.center().y);
            let to = drag_resize(tool.as_mut(), &cx, at, mods, from, false);
            let expected = Rect::new(from.x0, from.y0, from.x0 + if stroke { width } else { target }, from.y1);
            assert_rect(to, expected);
            if stroke {
                assert!(tool.overlays(&cx).is_empty(), "unmodeled affine {xf:?} must not advertise a dimension");
            } else {
                assert_dimension_feedback(tool.as_ref(), &cx, to, true, target);
            }
        }
    }
}

#[test]
fn r35_scaled_stroked_unit_axis_transforms_keep_dimension_matching() {
    // Cover both determinant signs, including computed quarter-turn coefficients.
    for turn in 0..4 {
        for reflect in [false, true] {
            let mut doc = Document::new(&NewDocument::default());
            let moving = add_rect(&mut doc, Rect::new(0.0, 0.0, 100.0, 50.0));
            let item = doc.item_mut(moving).unwrap();
            let xf = Affine::rotate(f64::from(turn) * std::f64::consts::FRAC_PI_2) * Affine::scale_non_uniform(if reflect { -1.0 } else { 1.0 }, 1.0);
            item.xf = xf;
            item.stroke = Stroke { weight: 4.0, ..Stroke::default() };
            let bounds = item.bounds();
            item.xf = Affine::translate((100.0 - bounds.x0, 100.0 - bounds.y0)) * xf;
            let from = item.bounds();
            let width = from.width() / 4.0;
            let target = width + 2.0; // quarter width, unchanged height: stroke scale is 1/2
            add_dimension_reference(&mut doc, true, target);
            let (sel, cache, layout) = (Selection::items(vec![moving]), Cache::new(), CanvasLayout::new(&doc, false));
            let mut cx = ctx(&doc, &sel, &cache, &layout);
            cx.snap = SnapView { smart_guides: true, smart_dimensions: true, zone_px: 6.0, ..SnapView::OFF };
            let mods = Mods { cmd: true, ..Mods::default() };
            let mut tool = create("selection");
            begin_resize(tool.as_mut(), &cx, from, 3, mods);
            let to = drag_resize(tool.as_mut(), &cx, Point::new(from.x0 + width + 1.0, from.center().y), mods, from, false);
            assert_rect(to, Rect::new(from.x0, from.y0, from.x0 + width, from.y1));
            assert_dimension_feedback(tool.as_ref(), &cx, to, true, target);
        }
    }
}

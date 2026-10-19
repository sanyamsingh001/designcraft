//! Selection tool (V) and Direct Selection tool (A).
//!
//! Click selects the frontmost item (Shift toggles), drag moves (Alt duplicates, Shift constrains),
//! the 8 bounding-box handles resize (Shift keeps proportions, Alt from centre, Cmd scales content),
//! empty-canvas drags make a marquee, double-clicking a text frame switches to the Type tool.

use designcraft_doc::{Arrowhead, Content, Item, ItemId, SpreadRef, StrokeAlign};
use designcraft_geom::{Affine, Point, Rect, Vec2};
use serde_json::{Value, json};

use crate::{Action, Cursor, Gesture, Mods, Overlay, PointerEvent, PointerKind, SnapRequest, Tool, ToolContext, ToolKey, rect_json, spread_json};

#[derive(Clone, Debug)]
enum Drag {
    None,
    Pending {
        start: Point,
        hit: bool,
    },
    Move {
        start: Point,
        origin_spread: SpreadRef,
        bounds0: Option<Rect>,
        exclude: Vec<ItemId>,
    },
    Resize {
        handle: usize,
        start: Point,
        from: Rect,
        spread: SpreadRef,
        stroke_pad: [f64; 2],
        scaled_stroke: bool,
        fixed_stroke: bool,
    },
    Rotate {
        center: Point,
        start_angle: f64,
        start_rotation: Option<f64>,
    },
    Marquee {
        start: Point,
        cur: Point,
    },
    Anchor {
        id: u64,
        si: usize,
        ai: usize,
        handle: Option<&'static str>,
        start: Point,
        /// Spread and the anchor's spread position at pointer down.
        /// None when that point could not be read. The live document during a drag is the
        /// previous preview, so the snap must not read the anchor from it again.
        at: Option<(SpreadRef, Point)>,
    },
}

/// Anchor or handle of a selected item under `p` (canvas): (item, subpath, anchor, handle).
pub fn anchor_at(cx: &ToolContext, p: Point) -> Option<(u64, usize, usize, Option<&'static str>)> {
    anchor_at_in(cx, &cx.selection.items, p)
}

/// [`anchor_at`] over the given items.
pub fn anchor_at_in(cx: &ToolContext, ids: &[designcraft_doc::ItemId], p: Point) -> Option<(u64, usize, usize, Option<&'static str>)> {
    let tol = cx.tol(5.0);
    for id in ids {
        let (Some(it), Some(xf)) = (cx.doc.item(*id), cx.item_canvas_xf(*id)) else { continue };
        let m = xf * it.xf;
        for (si, sp) in it.path.subpaths.iter().enumerate() {
            for (ai, a) in sp.anchors.iter().enumerate() {
                if ((m * a.p) - p).hypot() <= tol {
                    return Some((id.0, si, ai, None));
                }
                if a.has_in() && ((m * a.h_in) - p).hypot() <= tol {
                    return Some((id.0, si, ai, Some("in")));
                }
                if a.has_out() && ((m * a.h_out) - p).hypot() <= tol {
                    return Some((id.0, si, ai, Some("out")));
                }
            }
        }
    }
    None
}

/// Spread holding the anchor, and the anchor in that spread's coordinates.
fn anchor_spread_point(cx: &ToolContext, id: u64, si: usize, ai: usize) -> Option<(SpreadRef, Point)> {
    let item_id = ItemId(id);
    let loc = cx.doc.find(item_id)?;
    let it = cx.doc.item(item_id)?;
    let local = it.path.subpaths.get(si)?.anchors.get(ai)?.p;
    Some((loc.spread, (cx.doc.parent_xf(&loc) * it.xf) * local))
}

/// Spread-space correction as a canvas delta. Translation cancels, leaving the linear part of `xf`.
fn spread_delta_to_canvas(xf: Affine, d: Vec2) -> Vec2 {
    (xf * Point::new(d.x, d.y)) - (xf * Point::ORIGIN)
}

pub struct SelectionTool {
    direct: bool,
    drag: Drag,
    hover_handle: Option<usize>,
    guides: Vec<Overlay>,
    /// Selected item to toggle if this Shift press never becomes a drag.
    shift_release: Option<u64>,
}

impl SelectionTool {
    pub fn new(direct: bool) -> Self {
        Self { direct, drag: Drag::None, hover_handle: None, guides: vec![], shift_release: None }
    }
}

/// Handle positions (canvas) of a rect: 0 TL, 1 T, 2 TR, 3 R, 4 BR, 5 B, 6 BL, 7 L.
pub fn handles(r: Rect) -> [Point; 8] {
    let c = r.center();
    [
        Point::new(r.x0, r.y0),
        Point::new(c.x, r.y0),
        Point::new(r.x1, r.y0),
        Point::new(r.x1, c.y),
        Point::new(r.x1, r.y1),
        Point::new(c.x, r.y1),
        Point::new(r.x0, r.y1),
        Point::new(r.x0, c.y),
    ]
}

fn handle_at(cx: &ToolContext, p: Point) -> Option<usize> {
    let b = cx.selection_bounds()?;
    let tol = cx.tol(5.0);
    handles(b).iter().position(|h| (h.x - p.x).abs() <= tol && (h.y - p.y).abs() <= tol)
}

/// Just outside a corner handle of the selection → rotate.
fn rotate_zone(cx: &ToolContext, p: Point) -> Option<Point> {
    let b = cx.selection_bounds()?;
    let inner = cx.tol(5.0);
    let outer = cx.tol(18.0);
    for c in [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)] {
        let d = (c - p).hypot();
        let outside = p.x < b.x0 || p.x > b.x1 || p.y < b.y0 || p.y > b.y1;
        if d > inner && d <= outer && outside {
            return Some(b.center());
        }
    }
    None
}

/// New rect when dragging `handle` of `from` to `p`.
pub fn resize_rect(from: Rect, handle: usize, p: Point, m: Mods) -> Rect {
    finish_resize(from, handle, resize_edges_rect(from, handle, p), m)
}

/// Edges the pointer sets, before Shift or Alt.
fn resize_edges_rect(from: Rect, handle: usize, p: Point) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (from.x0, from.y0, from.x1, from.y1);
    match handle {
        0 => {
            x0 = p.x;
            y0 = p.y;
        }
        1 => y0 = p.y,
        2 => {
            x1 = p.x;
            y0 = p.y;
        }
        3 => x1 = p.x,
        4 => {
            x1 = p.x;
            y1 = p.y;
        }
        5 => y1 = p.y,
        6 => {
            x0 = p.x;
            y1 = p.y;
        }
        _ => x0 = p.x,
    }
    Rect::new(x0, y0, x1, y1)
}

fn finish_resize(from: Rect, handle: usize, rect: Rect, m: Mods) -> Rect {
    // Measure a centered drag from the center before choosing the aspect's driving axis.
    let rect = if m.alt { apply_alt(from, handle, rect) } else { rect };
    if m.shift && from.width() > 1e-9 && from.height() > 1e-9 {
        let s = aspect_scale(from, handle, rect);
        if m.alt { centered_rect(from, from.width() * s, from.height() * s) } else { apply_aspect(from, handle, rect, s) }
    } else {
        rect
    }
}

fn aspect_scale(from: Rect, handle: usize, rect: Rect) -> f64 {
    let sx = (rect.x1 - rect.x0) / from.width();
    let sy = (rect.y1 - rect.y0) / from.height();
    if matches!(handle, 1 | 5) {
        sy
    } else if matches!(handle, 3 | 7) || sx.abs() > sy.abs() {
        sx
    } else {
        sy
    }
}

/// True when the width, not the height, drives a Shift constraint.
fn drives_x(handle: usize, from: Rect, rect: Rect) -> bool {
    if matches!(handle, 1 | 5) {
        return false;
    }
    if matches!(handle, 3 | 7) {
        return true;
    }
    if from.width() <= 1e-9 || from.height() <= 1e-9 {
        return true;
    }
    let sx = ((rect.x1 - rect.x0) / from.width()).abs();
    let sy = ((rect.y1 - rect.y0) / from.height()).abs();
    sx > sy
}

fn apply_aspect(from: Rect, handle: usize, rect: Rect, s: f64) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    let (w, h) = (from.width() * s, from.height() * s);
    match handle {
        0 => {
            x0 = x1 - w;
            y0 = y1 - h;
        }
        1 => {
            x0 = from.center().x - w / 2.0;
            x1 = from.center().x + w / 2.0;
            y0 = y1 - h;
        }
        2 => {
            x1 = x0 + w;
            y0 = y1 - h;
        }
        3 => {
            x1 = x0 + w;
            y0 = from.center().y - h / 2.0;
            y1 = from.center().y + h / 2.0;
        }
        4 => {
            x1 = x0 + w;
            y1 = y0 + h;
        }
        5 => {
            x0 = from.center().x - w / 2.0;
            x1 = from.center().x + w / 2.0;
            y1 = y0 + h;
        }
        6 => {
            x0 = x1 - w;
            y1 = y0 + h;
        }
        7 => {
            x0 = x1 - w;
            y0 = from.center().y - h / 2.0;
            y1 = from.center().y + h / 2.0;
        }
        _ => {
            x0 = x1 - w;
            y0 = y1 - h;
        }
    }
    Rect::new(x0, y0, x1, y1)
}

/// Snap changes the size. Shift then restores the aspect, and Alt places that size on the original center.
///
/// Nudging one edge of an already centered rect walks the center. Mirroring that nudge doubles a length match.
fn finish_snapped_resize(
    from: Rect,
    handle: usize,
    proposed: Rect,
    mods: Mods,
    driver_x: bool,
    x_edges: [bool; 3],
    y_edges: [bool; 3],
    hit: &crate::Snap,
) -> Rect {
    if !mods.alt {
        // Dimension deltas grow the span's magnitude; reflected spans grow in the
        // opposite direction. Position deltas already use spread coordinates.
        let mut delta = hit.delta;
        if hit.length_delta[0].is_some() {
            delta.x *= proposed.width().signum();
        }
        if hit.length_delta[1].is_some() {
            delta.y *= proposed.height().signum();
        }
        let mut to = crate::snap::nudge_edges(proposed, x_edges, y_edges, delta);
        if mods.shift && from.width() > 1e-9 && from.height() > 1e-9 {
            let s = if driver_x { (to.x1 - to.x0) / from.width() } else { (to.y1 - to.y0) / from.height() };
            if s.is_finite() {
                to = apply_aspect(from, handle, to, s);
            }
        }
        return to;
    }
    let mut w = proposed.x1 - proposed.x0;
    let mut h = proposed.y1 - proposed.y0;
    w += centered_size_delta(w, x_edges, hit.delta.x, hit.length_delta[0]);
    h += centered_size_delta(h, y_edges, hit.delta.y, hit.length_delta[1]);
    if mods.shift && from.width() > 1e-9 && from.height() > 1e-9 {
        if driver_x {
            let s = w / from.width();
            if s.is_finite() {
                h = from.height() * s;
            }
        } else {
            let s = h / from.height();
            if s.is_finite() {
                w = from.width() * s;
            }
        }
    }
    if !(w.is_finite() && h.is_finite()) {
        return proposed;
    }
    centered_rect(from, w, h)
}

/// Length match: apply the difference once in the size's direction, preserving a crossed-center reflection.
/// Position match: the flagged edge moves and the other side mirrors.
fn centered_size_delta(size: f64, edges: [bool; 3], delta: f64, length_delta: Option<f64>) -> f64 {
    if let Some(diff) = length_delta.filter(|d| d.is_finite()) {
        return diff * size.signum();
    }
    let edge = axis_size_delta(edges, delta);
    if edge.is_finite() { edge * 2.0 } else { 0.0 }
}

/// Signed change in `hi - lo` when one flagged edge moves by `delta`.
fn axis_size_delta(edges: [bool; 3], delta: f64) -> f64 {
    if !delta.is_finite() {
        return 0.0;
    }
    let [low_edge, _, high_edge] = edges;
    if high_edge && !low_edge {
        delta
    } else if low_edge && !high_edge {
        -delta
    } else {
        0.0
    }
}

fn centered_rect(from: Rect, w: f64, h: f64) -> Rect {
    let c = from.center();
    Rect::new(c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0)
}

fn apply_alt(from: Rect, handle: usize, rect: Rect) -> Rect {
    let c = from.center();
    // Only the dragged side sets the half-size; the opposite side must not prevent shrinking.
    let (x_edges, y_edges) = resize_edge_flags(handle);
    let w = match x_edges {
        [true, _, false] => 2.0 * (c.x - rect.x0),
        [false, _, true] => 2.0 * (rect.x1 - c.x),
        _ => from.width(),
    };
    let h = match y_edges {
        [true, _, false] => 2.0 * (c.y - rect.y0),
        [false, _, true] => 2.0 * (rect.y1 - c.y),
        _ => from.height(),
    };
    centered_rect(from, w, h)
}

/// Left/center/right and top/center/bottom for a handle. Centers stay off.
fn resize_edge_flags(handle: usize) -> ([bool; 3], [bool; 3]) {
    let x = match handle {
        0 | 6 | 7 => [true, false, false],
        2..=4 => [false, false, true],
        _ => [false, false, false],
    };
    let y = match handle {
        0..=2 => [true, false, false],
        4..=6 => [false, false, true],
        _ => [false, false, false],
    };
    (x, y)
}

fn axis_length(edges: [bool; 3], size: f64, pad: f64) -> Option<f64> {
    if edges == [false, false, false] {
        return None;
    }
    let len = size.abs() + pad;
    len.is_finite().then_some(len)
}

/// Visible size minus path size, for one selected item. Centered stroke then matches the outer edge.
fn single_stroke_pad(cx: &ToolContext) -> (f64, f64) {
    if cx.selection.items.len() != 1 {
        return (0.0, 0.0);
    }
    let Some(id) = cx.selection.items.first() else { return (0.0, 0.0) };
    let Some(it) = cx.doc.item(*id) else { return (0.0, 0.0) };
    let path = it.bounds();
    let vis = it.visible_bounds();
    (positive_pad(vis.width() - path.width()), positive_pad(vis.height() - path.height()))
}

/// Ordinary resizing bakes a closed frame's path without changing its stroke.
/// Text frames share this fixed-outset policy, although their Cmd content scale does not.
fn single_fixed_stroke(cx: &ToolContext) -> bool {
    if cx.selection.items.len() != 1 {
        return false;
    }
    let Some(id) = cx.selection.items.first() else { return false };
    let Some(it) = cx.doc.item(*id) else { return false };
    cx.doc.top_level_of(*id) == Some(*id)
        && it.path.is_closed()
        && matches!(it.content, Content::Unassigned | Content::Graphic(_) | Content::Text(_))
        && it.stroke.weight.is_finite()
        && it.stroke.weight >= 0.0
        && it.xf.determinant().is_finite()
        && it.xf.determinant().abs() > 1e-9
}

/// Closed paths and graphic frames bake their Cmd content scale and can predict its stroke.
/// Text and groups transform their stroke differently, including with Scale Strokes off.
fn single_scaled_stroke(cx: &ToolContext) -> bool {
    single_fixed_stroke(cx)
        && cx.selection.items.first().and_then(|id| cx.doc.item(*id)).is_some_and(|it| {
            // The captured pad omits the item's stroke transform. Only unit axis
            // reflections and quarter-turns preserve that pad on both spread axes.
            // A unit determinant (or even an oblique rotation) is not enough.
            let [a, b, c, d, _, _] = it.xf.as_coeffs();
            let unit = |v: f64| (v.abs() - 1.0).abs() <= 1e-9;
            let zero = |v: f64| v.abs() <= 1e-9;
            let unit_axes = (unit(a) && zero(b) && zero(c) && unit(d)) || (zero(a) && unit(b) && unit(c) && zero(d));
            matches!(it.content, Content::Unassigned | Content::Graphic(_)) && unit_axes
        })
}

/// Zero outward stroke extent needs no stroke-scaling model: every selected
/// item's committed bounds undergo the same diagonal resize. Include text,
/// groups and multiple items only after checking all their original subtrees.
/// A lone nested item is safe here only when its parent transform is identity.
fn zero_outset_selection(cx: &ToolContext) -> bool {
    if cx.selection.items.is_empty() || cx.selection.items.len() > 4096 {
        return false;
    }
    let mut visited = 0;
    cx.selection.items.iter().all(|id| {
        let Some(loc) = cx.doc.find(*id) else { return false };
        if cx.doc.parent_xf(&loc) != Affine::IDENTITY || (cx.selection.items.len() > 1 && cx.doc.top_level_of(*id) != Some(*id)) {
            return false;
        }
        cx.doc.item(*id).is_some_and(|it| zero_outset_item(it, 0, &mut visited))
    })
}

fn zero_outset_item(it: &Item, depth: usize, visited: &mut usize) -> bool {
    *visited = visited.saturating_add(1);
    if depth > 32
        || *visited > 4096
        || !it.stroke.weight.is_finite()
        || it.stroke.weight < 0.0
        || !it.xf.determinant().is_finite()
        || it.xf.determinant().abs() <= 1e-9
    {
        return false;
    }
    let no_outset = it.stroke.is_none()
        || (it.path.is_closed() && it.stroke.align == StrokeAlign::Inside)
        || (it.stroke.weight == 0.0 && it.stroke.start == Arrowhead::None && it.stroke.end == Arrowhead::None);
    no_outset && it.children().iter().all(|child| zero_outset_item(child, depth + 1, visited))
}

// Keep aligned with transform.resize's minimum absolute per-axis scale.
const MIN_RESIZE_SCALE: f64 = 1e-3;

fn fixed_min_lengths(from: Rect, pad: [f64; 2]) -> [f64; 2] {
    if !(from.width().is_finite() && from.height().is_finite()) || from.width() <= 1e-9 || from.height() <= 1e-9 {
        return pad;
    }
    [from.width() * MIN_RESIZE_SCALE + pad[0], from.height() * MIN_RESIZE_SCALE + pad[1]]
        .map(|length| (length - 1e-12 * (1.0 + length.abs())).max(0.0))
}

fn scaled_visible_sizes(from: Rect, to: Rect, pad: [f64; 2]) -> Option<[f64; 2]> {
    if !(from.width().is_finite() && from.height().is_finite() && to.width().is_finite() && to.height().is_finite())
        || from.width() <= 1e-9
        || from.height() <= 1e-9
    {
        return None;
    }
    let sx = (to.width().abs() / from.width()).max(MIN_RESIZE_SCALE);
    let sy = (to.height().abs() / from.height()).max(MIN_RESIZE_SCALE);
    let k = (sx * sy).sqrt();
    let size = [from.width() * sx + pad[0] * k, from.height() * sy + pad[1] * k];
    size.iter().all(|v| v.is_finite()).then_some(size)
}

fn scaled_min_lengths(from: Rect, to: Rect, pad: [f64; 2], shift: bool, x_edges: [bool; 3], y_edges: [bool; 3]) -> [f64; 2] {
    let both = shift || (x_edges != [false; 3] && y_edges != [false; 3]);
    let kx = if both { MIN_RESIZE_SCALE } else { (MIN_RESIZE_SCALE * (to.height().abs() / from.height()).max(MIN_RESIZE_SCALE)).sqrt() };
    let ky = if both { MIN_RESIZE_SCALE } else { (MIN_RESIZE_SCALE * (to.width().abs() / from.width()).max(MIN_RESIZE_SCALE)).sqrt() };
    // Candidate bounds can lose a few ulps when subtracting distant coordinates.
    // The final solve still checks both actual scale and visible-size agreement.
    [from.width() * MIN_RESIZE_SCALE + pad[0] * kx, from.height() * MIN_RESIZE_SCALE + pad[1] * ky]
        .map(|length| (length - 1e-12 * (1.0 + length.abs())).max(0.0))
}

/// Solve visible target = path magnitude + original stroke pad * final scale.
/// Shift gives a linear equation; one free axis gives a quadratic in sqrt(size).
/// Two matched axes share a scale: a bounded monotone solve handles their coupling.
fn solve_scaled_sizes(base: [f64; 2], other: [f64; 2], pad: [f64; 2], targets: [Option<f64>; 2], shift: bool, driver_x: bool) -> Option<[f64; 2]> {
    let mut size = other;
    if shift {
        let axis = usize::from(!driver_x);
        if let Some(target) = targets[axis] {
            let k = target / (base[axis] + pad[axis]);
            size = [base[0] * k, base[1] * k];
        }
    } else if let [Some(w), Some(h)] = targets {
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let mut lo = 0.0;
        let mut hi = ((w / base[0]) * (h / base[1])).sqrt();
        if !hi.is_finite() {
            return None;
        }
        for _ in 0..64 {
            let k = (lo + hi) / 2.0;
            let needed = (((w - pad[0] * k).max(0.0) / base[0]) * ((h - pad[1] * k).max(0.0) / base[1])).sqrt();
            if k < needed {
                lo = k;
            } else {
                hi = k;
            }
        }
        let k = (lo + hi) / 2.0;
        size = [w - pad[0] * k, h - pad[1] * k];
    } else {
        for axis in 0..2 {
            if let Some(target) = targets[axis] {
                if target <= 0.0 {
                    return None;
                }
                let a = pad[axis] * (other[1 - axis] / base[1 - axis] / base[axis]).sqrt();
                let root = 2.0 * target / ((a * a + 4.0 * target).sqrt() + a);
                size[axis] = root * root;
            }
        }
    }
    (0..2).all(|axis| size[axis].is_finite() && size[axis] / base[axis] >= MIN_RESIZE_SCALE - 1e-12).then_some(size)
}

fn finish_scaled_snap(
    from: Rect,
    handle: usize,
    proposed: Rect,
    mods: Mods,
    driver_x: bool,
    x_edges: [bool; 3],
    y_edges: [bool; 3],
    hit: &crate::Snap,
    pad: [f64; 2],
    visible: [f64; 2],
) -> Option<Rect> {
    if hit.length_delta == [None; 2] {
        return Some(finish_snapped_resize(from, handle, proposed, mods, driver_x, x_edges, y_edges, hit));
    }
    let targets = [hit.length_delta[0].map(|d| visible[0] + d), hit.length_delta[1].map(|d| visible[1] + d)];
    let mut positions = hit.clone();
    if targets[0].is_some() {
        positions.delta.x = 0.0;
    }
    if targets[1].is_some() {
        positions.delta.y = 0.0;
    }
    positions.length_delta = [None; 2];
    let position_rect = finish_snapped_resize(from, handle, proposed, mods, driver_x, x_edges, y_edges, &positions);
    let sizes = solve_scaled_sizes(
        [from.width(), from.height()],
        [position_rect.width().abs(), position_rect.height().abs()],
        pad,
        targets,
        mods.shift,
        driver_x,
    )?;
    let mut paths = hit.clone();
    for (axis, span, flags) in [(0, proposed.width(), x_edges), (1, proposed.height(), y_edges)] {
        if targets[axis].is_some() {
            let diff = sizes[axis] - span.abs();
            paths.length_delta[axis] = Some(diff);
            let delta = if flags[0] { -diff } else { diff };
            if axis == 0 {
                paths.delta.x = delta;
            } else {
                paths.delta.y = delta;
            }
        }
    }
    let to = finish_snapped_resize(from, handle, proposed, mods, driver_x, x_edges, y_edges, &paths);
    let actual = scaled_visible_sizes(from, to, pad)?;
    (0..2).all(|axis| targets[axis].is_none_or(|target| (actual[axis] - target).abs() <= 1e-8 * (1.0 + target.abs()))).then_some(to)
}

fn positive_pad(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

impl Tool for SelectionTool {
    fn id(&self) -> &'static str {
        if self.direct { "directSelection" } else { "selection" }
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover_handle = handle_at(cx, p);
                vec![]
            }
            PointerKind::Down => {
                self.shift_release = None;
                if self.direct
                    && let Some((id, si, ai, handle)) = anchor_at(cx, p)
                {
                    let at = anchor_spread_point(cx, id, si, ai);
                    self.drag = Drag::Anchor { id, si, ai, handle, start: p, at };
                    return vec![Action::Begin(if handle.is_some() { "Move Direction Handle".into() } else { "Move Anchor".into() })];
                }
                if !self.direct
                    && handle_at(cx, p).is_none()
                    && let Some(center) = rotate_zone(cx, p)
                {
                    self.drag = Drag::Rotate { center, start_angle: (p - center).atan2(), start_rotation: crate::snap::reference_rotation(cx) };
                    return vec![Action::Begin("Rotate".into())];
                }
                if let Some(h) = handle_at(cx, p).filter(|_| !self.direct)
                    && let (Some(b), Some(first)) = (cx.selection_bounds(), cx.selection.items.first())
                    && let Some(loc) = cx.doc.find(*first)
                {
                    // On a turned spread the canvas handle is another one of the spread's.
                    let turns = cx.layout.slot(loc.spread).map_or(0, |s| s.rotation as usize);
                    let handle = (h + 8 - 2 * turns) % 8;
                    // Preview contexts contain the previous preview, while the engine resizes
                    // the pointer-down document each time. Keep its stroke inputs here too.
                    let (pad_x, pad_y) = single_stroke_pad(cx);
                    let zero_outset = zero_outset_selection(cx);
                    self.drag = Drag::Resize {
                        handle,
                        start: p,
                        from: cx.layout.xf(loc.spread).inverse().transform_rect_bbox(b),
                        spread: loc.spread,
                        stroke_pad: [pad_x, pad_y],
                        scaled_stroke: single_scaled_stroke(cx) || zero_outset,
                        fixed_stroke: single_fixed_stroke(cx) || zero_outset,
                    };
                    return vec![Action::Begin("Resize".into())];
                }
                // Cmd+Shift-click a parent item on a page: override it there and select the copy.
                if ev.mods.cmd
                    && ev.mods.shift
                    && cx.hit(p).is_none()
                    && let Some((page, id)) = cx.hit_parent_on_page(p)
                {
                    self.drag = Drag::Pending { start: p, hit: true };
                    return vec![Action::Exec("layout.overrideParentItems".into(), json!({"page": page, "ids": [id.0]}))];
                }
                match cx.hit(p) {
                    Some((sr, id)) => {
                        let id = if self.direct { id } else { cx.doc.top_level_of(id).unwrap_or(id) };
                        let mut out = vec![];
                        if ev.mods.shift {
                            if cx.selection.contains(id) {
                                // Keep it selected so a Shift-drag can constrain. A click still toggles, on release.
                                self.shift_release = Some(id.0);
                            } else {
                                out.push(Action::Exec("selection.toggle".into(), json!({"id": id.0})));
                            }
                        } else if !cx.selection.contains(id) {
                            out.push(Action::Exec("selection.set".into(), json!({"ids": [id.0], "content": self.direct})));
                        }
                        self.drag = Drag::Pending { start: p, hit: true };
                        let _ = sr;
                        out
                    }
                    None => {
                        self.drag = Drag::Pending { start: p, hit: false };
                        if ev.mods.shift { vec![] } else { vec![Action::Exec("selection.set".into(), json!({"ids": []}))] }
                    }
                }
            }
            PointerKind::Drag => match self.drag.clone() {
                Drag::Pending { start, hit } => {
                    if (p - start).hypot() < cx.tol(3.0) {
                        return vec![];
                    }
                    self.shift_release = None;
                    if hit && !cx.selection.items.is_empty() {
                        let sr = cx.selection.items.first().and_then(|i| cx.doc.find(*i)).map(|l| l.spread).unwrap_or(SpreadRef::Doc(0));
                        let exclude = cx.selection.items.clone();
                        self.drag = Drag::Move { start, origin_spread: sr, bounds0: cx.selection_visible_bounds(), exclude };
                        let label = if ev.mods.alt { "Duplicate" } else { "Move" };
                        let mut v = vec![Action::Begin(label.into())];
                        v.extend(self.pointer(cx, ev));
                        v
                    } else {
                        self.drag = Drag::Marquee { start, cur: p };
                        vec![]
                    }
                }
                Drag::Move { start, origin_spread, bounds0, exclude } => {
                    let mut d: Vec2 = p - start;
                    let mut screen_x_locked = false;
                    let mut screen_y_locked = false;
                    if ev.mods.shift {
                        if d.x.abs() > d.y.abs() {
                            d.y = 0.0;
                            screen_y_locked = true;
                        } else {
                            d.x = 0.0;
                            screen_x_locked = true;
                        }
                    }
                    // Snap on the spread under the pointer. Shift uses that spread's view rotation.
                    let target = cx.layout.spread_at(p).map(|(s, _)| s).unwrap_or(origin_spread);
                    // 180 degrees keeps the screen axis. 90 and 270 swap it into the other spread axis.
                    let odd_turn = cx.layout.slot(target).is_some_and(|slot| slot.rotation % 2 == 1);
                    let spread_x_locked = if odd_turn { screen_y_locked } else { screen_x_locked };
                    let spread_y_locked = if odd_turn { screen_x_locked } else { screen_y_locked };
                    let x_edges = if spread_x_locked { [false; 3] } else { [true; 3] };
                    let y_edges = if spread_y_locked { [false; 3] } else { [true; 3] };
                    self.guides.clear();
                    // The movement in the origin spread's coordinates (its view may be turned).
                    let mut ds = cx.layout.delta_to_spread(origin_spread, d);
                    let b0s = bounds0.map(|b| cx.layout.xf(origin_spread).inverse().transform_rect_bbox(b));
                    let q0 = b0s.map_or(Point::ORIGIN, |b| Point::new(b.x0, b.y0));
                    // Same corner conversion transform.move already uses when the spread changes.
                    let mut landed = cx.layout.to_spread(target, cx.layout.to_canvas(origin_spread, q0 + ds));
                    // Command suspends snapping. A Shift-locked spread axis is not offered.
                    if !ev.mods.cmd
                        && cx.snap.any()
                        && let Some(b0) = b0s
                    {
                        let proposed = if target == origin_spread {
                            b0 + ds
                        } else {
                            let shift = landed - q0;
                            Rect::new(b0.x0 + shift.x, b0.y0 + shift.y, b0.x1 + shift.x, b0.y1 + shift.y)
                        };
                        let hit = crate::snap::snap(
                            cx,
                            SnapRequest {
                                spread: target,
                                gesture: Gesture::Move,
                                rect: proposed,
                                x_edges,
                                y_edges,
                                exclude: &exclude,
                                copying: ev.mods.alt,
                                lengths: [None, None],
                                angle: None,
                                radius: 0.0,
                                pointer: cx.layout.to_spread(target, p),
                            },
                        );
                        if target == origin_spread {
                            if x_edges != [false, false, false] && hit.delta.x.is_finite() {
                                ds.x += hit.delta.x;
                            }
                            if y_edges != [false, false, false] && hit.delta.y.is_finite() {
                                ds.y += hit.delta.y;
                            }
                        } else {
                            if x_edges != [false, false, false] && hit.delta.x.is_finite() {
                                landed.x += hit.delta.x;
                            }
                            if y_edges != [false, false, false] && hit.delta.y.is_finite() {
                                landed.y += hit.delta.y;
                            }
                        }
                        self.guides = hit.guides;
                    }
                    // Same spread: dx, dy are the origin spread translation. Crossing: the corner
                    // conversion, so the reparented item lands on the snapped target point.
                    let (dx, dy) = if target == origin_spread {
                        (ds.x, ds.y)
                    } else {
                        let dt = landed - q0;
                        (dt.x, dt.y)
                    };
                    let mut params = json!({"dx": dx, "dy": dy, "copy": ev.mods.alt});
                    if target != origin_spread {
                        params = json!({"dx": dx, "dy": dy, "copy": ev.mods.alt, "toSpread": spread_json(target)});
                    }
                    vec![Action::Preview("transform.move".into(), params)]
                }
                Drag::Resize { handle, start, from, spread, stroke_pad, scaled_stroke, fixed_stroke } => {
                    let _ = start;
                    let pointer = cx.layout.to_spread(spread, p);
                    let raw = resize_edges_rect(from, handle, pointer);
                    let mut to = resize_rect(from, handle, pointer, ev.mods);
                    // Space while dragging: Live Distribute (several objects keep their size).
                    let distribute = ev.mods.space && cx.selection.items.len() > 1;
                    self.guides.clear();
                    // Command still scales content. It does not skip the snap.
                    if !distribute && cx.snap.any() {
                        let (mut x_edges, mut y_edges) = resize_edge_flags(handle);
                        let constraint = if ev.mods.alt { apply_alt(from, handle, raw) } else { raw };
                        let driver_x = drives_x(handle, from, constraint);
                        if ev.mods.shift {
                            if driver_x {
                                y_edges = [false, false, false];
                            } else {
                                x_edges = [false, false, false];
                            }
                        }
                        let exclude = cx.selection.items.clone();
                        let [pad_x, pad_y] = stroke_pad;
                        let proposed = to;
                        let scales_stroke = ev.mods.cmd && cx.scale_strokes;
                        let predicted = if scales_stroke && scaled_stroke { scaled_visible_sizes(from, proposed, stroke_pad) } else { None };
                        let dimensions_supported = if ev.mods.cmd { scaled_stroke } else { fixed_stroke };
                        let lengths = if !dimensions_supported {
                            // Only verified stroke policies or zero-outset selections can
                            // advertise dimensions. Other policies still snap to positions.
                            [None; 2]
                        } else if scales_stroke {
                            predicted.map_or([None; 2], |[w, h]| [axis_length(x_edges, w, 0.0), axis_length(y_edges, h, 0.0)])
                        } else {
                            [axis_length(x_edges, proposed.width(), pad_x), axis_length(y_edges, proposed.height(), pad_y)]
                        };
                        let minimum = if predicted.is_some() {
                            scaled_min_lengths(from, proposed, stroke_pad, ev.mods.shift, x_edges, y_edges)
                        } else if fixed_stroke {
                            // Fixed outsets still require a path span at the command's
                            // .001 scale floor, including ordinary text-frame resizing.
                            fixed_min_lengths(from, stroke_pad)
                        } else {
                            stroke_pad
                        };
                        let snap = |lengths| {
                            crate::snap::snap_with_min_lengths(
                                cx,
                                SnapRequest {
                                    spread,
                                    gesture: Gesture::Resize,
                                    rect: proposed,
                                    x_edges,
                                    y_edges,
                                    exclude: &exclude,
                                    copying: false,
                                    lengths,
                                    angle: None,
                                    radius: 0.0,
                                    pointer,
                                },
                                minimum,
                            )
                        };
                        let mut hit = snap(lengths);
                        if let Some(visible) = predicted {
                            if let Some(solved) =
                                finish_scaled_snap(from, handle, proposed, ev.mods, driver_x, x_edges, y_edges, &hit, stroke_pad, visible)
                            {
                                to = solved;
                            } else {
                                // A coupled match or a position correction can make the final
                                // scale unreachable. Retry position passes without dimensions.
                                hit = snap([None; 2]);
                                to = finish_snapped_resize(from, handle, proposed, ev.mods, driver_x, x_edges, y_edges, &hit);
                            }
                        } else {
                            to = finish_snapped_resize(from, handle, proposed, ev.mods, driver_x, x_edges, y_edges, &hit);
                        }
                        crate::snap::lay_dimension_guides(&mut hit.guides, cx.layout.xf(spread), proposed, to, x_edges, y_edges, hit.length_delta);
                        self.guides = hit.guides;
                    }
                    vec![Action::Preview(
                        "transform.resize".into(),
                        json!({"from": rect_json(from), "to": rect_json(to), "content": ev.mods.cmd, "distribute": distribute}),
                    )]
                }
                Drag::Marquee { start, .. } => {
                    self.drag = Drag::Marquee { start, cur: p };
                    vec![]
                }
                Drag::Rotate { center, start_angle, start_rotation } => {
                    let mut a = ((p - center).atan2() - start_angle).to_degrees();
                    self.guides.clear();
                    if ev.mods.shift {
                        // 45 degree steps. The rotation pass does not run while Shift is held.
                        a = (a / 45.0).round() * 45.0;
                    } else if let Some(start) = start_rotation {
                        let raw = -a;
                        if let Some(resulting) = crate::snap::folded_rotation(start + raw) {
                            let spread =
                                cx.selection.items.first().and_then(|id| cx.doc.find(*id)).map(|loc| loc.spread).unwrap_or(SpreadRef::Doc(0));
                            let exclude = cx.selection.items.clone();
                            // The rect is unused by the angle pass. It only has to be finite.
                            let hit = crate::snap::snap(
                                cx,
                                SnapRequest {
                                    spread,
                                    gesture: Gesture::Rotate,
                                    rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                                    x_edges: [false, false, false],
                                    y_edges: [false, false, false],
                                    exclude: &exclude,
                                    copying: false,
                                    lengths: [None, None],
                                    angle: Some(resulting),
                                    radius: (p - center).hypot(),
                                    pointer: cx.layout.to_spread(spread, p),
                                },
                            );
                            if let Some(snapped) = hit.angle
                                && let Some(delta) = crate::snap::rotation_command_delta(start, snapped, raw)
                            {
                                a = -delta;
                            }
                            self.guides = hit.guides;
                        }
                    }
                    // Screen y points down: a positive screen angle is clockwise.
                    vec![Action::Preview("transform.rotate".into(), json!({"angle": -a}))]
                }
                Drag::Anchor { id, si, ai, handle, start, at } => {
                    let d = p - start;
                    let mut dx = d.x;
                    let mut dy = d.y;
                    self.guides.clear();
                    // Direction handles do not snap. An anchor excludes the path it is on.
                    if handle.is_none()
                        && let Some((spread, origin)) = at
                    {
                        let proposed = origin + cx.layout.delta_to_spread(spread, d);
                        let exclude = [ItemId(id)];
                        let hit = crate::snap::snap(
                            cx,
                            SnapRequest {
                                spread,
                                gesture: Gesture::Point,
                                rect: Rect::from_points(proposed, proposed),
                                x_edges: [true, true, true],
                                y_edges: [true, true, true],
                                exclude: &exclude,
                                copying: false,
                                lengths: [None, None],
                                angle: None,
                                radius: 0.0,
                                pointer: proposed,
                            },
                        );
                        // dx and dy stay in canvas space. Snap.delta is spread space.
                        let corr = spread_delta_to_canvas(cx.layout.xf(spread), hit.delta);
                        if corr.x.is_finite() {
                            dx += corr.x;
                        }
                        if corr.y.is_finite() {
                            dy += corr.y;
                        }
                        self.guides = hit.guides;
                    }
                    let mut params = json!({"id": id, "anchors": [[si, ai]], "dx": dx, "dy": dy});
                    if let Some(h) = handle {
                        params["handle"] = json!(h);
                    }
                    vec![Action::Preview("path.moveAnchors".into(), params)]
                }
                Drag::None => vec![],
            },
            PointerKind::Up => {
                self.guides.clear();
                let release = self.shift_release.take();
                let d = std::mem::replace(&mut self.drag, Drag::None);
                match d {
                    Drag::Move { .. } | Drag::Resize { .. } | Drag::Anchor { .. } | Drag::Rotate { .. } => vec![Action::Commit],
                    Drag::Pending { .. } => match release {
                        Some(id) => vec![Action::Exec("selection.toggle".into(), json!({"id": id}))],
                        None => vec![],
                    },
                    Drag::Marquee { start, cur } => {
                        let r = Rect::from_points(start, cur);
                        // Items whose bounds intersect the marquee.
                        let mut ids: Vec<Value> = Vec::new();
                        for slot in &cx.layout.slots {
                            let Some(sp) = cx.doc.spread(slot.spread) else { continue };
                            for it in &sp.items {
                                let b = slot.xf.transform_rect_bbox(it.bounds());
                                let locked = it.locked || cx.doc.layer(it.layer).is_some_and(|l| l.locked || !l.visible);
                                if !locked && !it.hidden && b.x0 < r.x1 && b.x1 > r.x0 && b.y0 < r.y1 && b.y1 > r.y0 {
                                    ids.push(Value::from(it.id.0));
                                }
                            }
                        }
                        vec![Action::Exec("selection.set".into(), json!({"ids": ids, "add": ev.mods.shift}))]
                    }
                    Drag::None => vec![],
                }
            }
            PointerKind::DoubleClick => {
                if let Some((_, id)) = cx.hit(p)
                    && cx.doc.item(id).is_some_and(|i| i.is_text_frame())
                {
                    let (sr, sp) = cx.layout.spread_at(p).unwrap_or((SpreadRef::Doc(0), p));
                    let _ = sr;
                    return vec![
                        Action::SwitchTool("type".into()),
                        Action::Exec("text.placeCaret".into(), json!({"frame": id.0, "point": [sp.x, sp.y]})),
                    ];
                }
                vec![]
            }
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        let inc = cx.doc.settings.keyboard_increment * if mods.shift { 10.0 } else { 1.0 };
        let mv = |dx: f64, dy: f64| vec![Action::Exec("transform.move".into(), json!({"dx": dx, "dy": dy, "copy": mods.alt}))];
        if cx.selection.items.is_empty() {
            return vec![];
        }
        match key {
            ToolKey::Left => mv(-inc, 0.0),
            ToolKey::Right => mv(inc, 0.0),
            ToolKey::Up => mv(0.0, -inc),
            ToolKey::Down => mv(0.0, inc),
            ToolKey::Delete | ToolKey::Backspace => vec![Action::Exec("edit.clear".into(), json!({}))],
            ToolKey::Escape => vec![Action::Exec("selection.set".into(), json!({"ids": []}))],
            _ => vec![],
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        match &self.drag {
            Drag::Marquee { start, cur } => vec![Overlay::Marquee(Rect::from_points(*start, *cur))],
            Drag::Move { .. } | Drag::Resize { .. } | Drag::Rotate { .. } | Drag::Anchor { .. } => self.guides.clone(),
            _ => vec![],
        }
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if self.direct {
            return Cursor::ArrowHollow;
        }
        match self.drag {
            Drag::Move { .. } => return Cursor::Move,
            Drag::Resize { handle, .. } => return handle_cursor(handle),
            Drag::Rotate { .. } => return Cursor::Rotate,
            _ => {}
        }
        match handle_at(cx, p) {
            Some(h) => handle_cursor(h),
            None if rotate_zone(cx, p).is_some() => Cursor::Rotate,
            None => Cursor::Arrow,
        }
    }

    fn busy(&self) -> bool {
        !matches!(self.drag, Drag::None)
    }
}

fn handle_cursor(h: usize) -> Cursor {
    match h {
        0 | 4 => Cursor::ResizeNwSe,
        2 | 6 => Cursor::ResizeNeSw,
        1 | 5 => Cursor::ResizeV,
        _ => Cursor::ResizeH,
    }
}

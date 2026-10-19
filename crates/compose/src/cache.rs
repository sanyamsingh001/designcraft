//! Composition cache shared by the engine, the renderer and exporters.
//!
//! An entry is valid while the inputs it was composed from are the *same allocations*: the story
//! `Arc` (and its revision), the styles `Arc`, the frame items' `Arc`s and the `Arc`s of every
//! wrapping item on the frames' spreads. Entries hold clones of those `Arc`s, so the pointers can't
//! be reused by other allocations while the entry lives.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use designcraft_doc::{Document, Item, Story, StoryId, Styles, WrapMode};

use crate::vars::RunningIndex;
use crate::{ComposeOptions, ComposedStory};

type Key = (StoryId, Option<String>);

struct Entry {
    sig: Vec<usize>,
    _keep: (Arc<Story>, Arc<Styles>, Vec<Arc<Item>>),
    out: Arc<ComposedStory>,
    stamp: u64,
}

#[derive(Default)]
pub struct Cache {
    map: Mutex<(HashMap<Key, Entry>, u64)>,
    /// Running-header index with the document signature it was built for.
    running: Mutex<Option<(u64, Arc<RunningIndex>)>>,
    /// Cross-reference anchor pages with the document signature they were built for.
    xrefs: Mutex<Option<(u64, Arc<crate::xref::XrefIndex>)>>,
    /// The composition each story (page-independent key) had before its latest recomposition,
    /// with the story revision it was composed from (repaint only the frames an edit changed).
    previous: Mutex<HashMap<StoryId, (Vec<usize>, Arc<ComposedStory>)>>,
}

fn section_signature(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", doc.sections).hash(&mut h);
    h.finish()
}

/// Identity of the source layout and labels consumed by running headers and cross-references.
fn doc_signature(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (id, st) in &doc.stories {
        (id.0, Arc::as_ptr(st) as usize, st.rev).hash(&mut h);
        // Script settings can move this source's heading/anchor without replacing its Arc.
        advanced_type_inputs(doc, st).map(|value| value.map(f64::to_bits)).hash(&mut h);
    }
    // Footnote layout can move a source heading/anchor without replacing its Arc.
    if doc.stories.values().any(|story| !story.notes.is_empty()) {
        format!("{:?}", doc.footnote_options).hash(&mut h);
    }
    for sp in &doc.spreads {
        (Arc::as_ptr(sp) as usize).hash(&mut h);
    }
    (Arc::as_ptr(&doc.styles) as usize).hash(&mut h);
    format!("{:?}", doc.text_variables).hash(&mut h);
    section_signature(doc).hash(&mut h);
    // These inputs can move source headings/anchors while stories, styles and spreads stay shared.
    doc.hyphenation_exceptions.hash(&mut h);
    for condition in doc.conditions.iter().filter(|condition| !condition.visible) {
        condition.name.hash(&mut h);
    }
    (doc.settings.glyph_fallback, doc.font_scope).hash(&mut h);
    // A source's grid can change where a running heading or cross-reference anchor lands.
    (doc.settings.baseline_grid.start.to_bits(), doc.settings.baseline_grid.increment.to_bits()).hash(&mut h);
    h.finish()
}

/// Keep every f64 bit on both native and wasm32 signatures, in low/high 32-bit words.
fn float_signature_words(value: f64) -> [usize; 2] {
    let bits = value.to_bits();
    [(bits as u32) as usize, ((bits >> 32) as u32) as usize]
}

/// Advanced Type values used by this story's synthesized scripts, including its overset tail.
/// OpenType scripts use font features instead; unused named styles are not dependencies.
pub fn advanced_type_inputs(doc: &Document, story: &Story) -> [Option<f64>; 4] {
    let mut used = [false; 2];
    script_usage(doc, story, &mut used, 0);
    let a = doc.settings.advanced_type;
    [
        used[0].then_some(a.superscript_size),
        used[0].then_some(a.superscript_position),
        used[1].then_some(a.subscript_size),
        used[1].then_some(a.subscript_position),
    ]
}

fn script_position(position: designcraft_doc::Position, used: &mut [bool; 2]) {
    match position {
        designcraft_doc::Position::Superscript => used[0] = true,
        designcraft_doc::Position::Subscript => used[1] = true,
        _ => {}
    }
}

fn script_segment(doc: &Document, text: &str, position: designcraft_doc::Position, used: &mut [bool; 2]) {
    // Reference markers replace the run's position, exactly as shape_para does.
    if text.chars().any(|c| c != designcraft_doc::FOOTNOTE_REF && c != designcraft_doc::ENDNOTE_REF) {
        script_position(position, used);
    }
    if text.contains(designcraft_doc::FOOTNOTE_REF) {
        script_position(doc.footnote_options.ref_position, used);
    }
    if text.contains(designcraft_doc::ENDNOTE_REF) {
        script_position(designcraft_doc::Position::Superscript, used);
    }
}

fn script_usage(doc: &Document, story: &Story, used: &mut [bool; 2], depth: usize) {
    if depth >= 64 {
        // Do not silently drop a dependency at the same nesting bound as reflow comparison.
        *used = [true; 2];
        return;
    }
    if used.iter().all(|value| *value) {
        return;
    }
    let mut runs = story.runs().peekable();
    for (pi, (range, format)) in story.para_ranges().into_iter().zip(&story.paras).enumerate() {
        if story.para_table(pi).is_some() {
            continue;
        }
        let (para, base) = doc.styles.resolve_para(format);
        let overlays = crate::overlay::overlays(&story.text, range.clone(), &para.nested_styles, &para.grep_styles);
        while let Some((run, chars)) = runs.peek() {
            if run.start >= range.end {
                break;
            }
            if run.end <= range.start {
                runs.next();
                continue;
            }
            let start = run.start.max(range.start);
            let end = run.end.min(range.end);
            let mut cuts: Vec<_> = overlays.iter().flat_map(|(r, _)| [r.start, r.end]).filter(|p| *p > start && *p < end).collect();
            cuts.extend([start, end]);
            cuts.sort_unstable();
            cuts.dedup();
            for points in cuts.windows(2) {
                let Some(text) = story.text.get(points[0]..points[1]) else { continue };
                let overlay = overlays.iter().rev().find(|(r, _)| r.start <= points[0] && points[0] < r.end);
                let under = overlay.map_or_else(
                    || base.clone(),
                    |(_, name)| doc.styles.resolve_char(&base, &designcraft_doc::CharFormat { style: name.clone(), ..Default::default() }),
                );
                script_segment(doc, text, doc.styles.resolve_char(&under, chars).position, used);
                // Line-style ranges depend on the resulting breaks. Keep their possible inputs
                // in the invisible tail too; nested/GREP overlays and direct formatting win.
                if overlay.is_none() {
                    for line in para.nested_line_styles.iter().take_while(|line| line.lines != 0) {
                        let under = doc.styles.resolve_char(&base, &designcraft_doc::CharFormat { style: line.style.clone(), ..Default::default() });
                        script_segment(doc, text, doc.styles.resolve_char(&under, chars).position, used);
                    }
                }
            }
            if run.end > range.end {
                break;
            }
            runs.next();
        }
    }
    for note in &story.notes {
        script_usage(doc, &note.text, used, depth + 1);
    }
    for table in story.tables.values() {
        let owners = table.owners();
        let columns = table.ncols();
        for (i, cell) in table.cells.iter().enumerate() {
            if columns != 0 && owners.get(i) == Some(&(i / columns, i % columns)) {
                script_usage(doc, &cell.text, used, depth + 1);
            }
        }
    }
}

/// Page coordinates consumed by custom anchored objects in this frame's inner space.
/// A margin edit is irrelevant to PageEdge/Anchor/TextFrame coordinates and to unused axes.
pub fn page_anchor_inputs(story: &Story, frame: &crate::FrameSpec) -> Vec<f64> {
    use designcraft_doc::anchored::AnchorRelative as R;
    let mut inputs = Vec::new();
    for object in &story.objects {
        if let designcraft_doc::AnchorPosition::Custom { x_relative, y_relative, ref_point, .. } = &object.position {
            let area = |relative| match relative {
                R::PageMargin => Some(frame.page_rect.map_or(frame.area, |rects| rects.1)),
                R::PageEdge => Some(frame.page_rect.map_or(frame.area, |rects| rects.0)),
                _ => None,
            };
            if let Some(rect) = area(*x_relative) {
                inputs.push(rect.x0 + rect.width() * f64::from(ref_point % 3) * 0.5);
            }
            if let Some(rect) = area(*y_relative) {
                inputs.push(rect.y0 + rect.height() * f64::from((ref_point / 3).min(2)) * 0.5);
            }
        }
    }
    inputs
}

impl Cache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The composed story, from cache or freshly composed.
    pub fn get(&self, doc: &Document, sid: StoryId, page_name: Option<&str>) -> Arc<ComposedStory> {
        let Some(story) = doc.stories.get(&sid) else { return Arc::new(ComposedStory { story: sid, ..Default::default() }) };
        let (mut sig, keep_items) = signature(doc, story);
        let key = (sid, page_name.map(str::to_string));
        let with_vars = crate::vars::has_vars(story);
        let running = with_vars && crate::vars::has_running(doc, story);
        if with_vars {
            // Variables depend on the page count, sections and definitions (and running headers on
            // the whole document).
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", doc.text_variables).hash(&mut h);
            sig.extend([doc.page_count(), h.finish() as usize]);
            if running {
                sig.push(doc_signature(doc) as usize);
            }
        }
        let with_xrefs = crate::xref::has_xrefs(story);
        if with_xrefs {
            // Cross-references show other stories' text and pages.
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", doc.xref_formats).hash(&mut h);
            sig.extend([doc_signature(doc) as usize, h.finish() as usize]);
        }
        if !story.notes.is_empty() {
            // Footnote numbers continue from earlier stories; layout follows the options.
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", doc.footnote_options).hash(&mut h);
            sig.extend([doc.footnote_start(sid) as usize, h.finish() as usize]);
        }
        // Endnote references depend on their number and label formatting, even in overset text.
        if !story.endnotes.is_empty() {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (doc.endnote_options.style as usize, &doc.endnote_options.prefix, &doc.endnote_options.suffix).hash(&mut h);
            sig.extend([doc.endnote_start(sid) as usize, h.finish() as usize]);
        }
        // User hyphenation exceptions.
        if !doc.hyphenation_exceptions.is_empty() {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            doc.hyphenation_exceptions.hash(&mut h);
            sig.push(h.finish() as usize);
        }
        // Hidden conditions take their text out of the layout.
        for (i, c) in doc.conditions.iter().enumerate() {
            if !c.visible {
                sig.push(usize::MAX - i);
            }
        }
        // Only synthesized scripts consume these settings. Preserve every consumed f64 bit.
        sig.extend(advanced_type_inputs(doc, story).into_iter().flatten().flat_map(float_signature_words));
        // Missing glyphs: fallback fonts or the font's box.
        sig.push(usize::from(doc.settings.glyph_fallback));
        // Named lists continue from earlier stories.
        for l in &doc.settings.lists {
            sig.push(doc.list_start(sid, &l.name) as usize);
        }
        {
            let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
            g.1 += 1;
            let stamp = g.1;
            if let Some(e) = g.0.get_mut(&key)
                && e.sig == sig
            {
                e.stamp = stamp;
                return e.out.clone();
            }
        }
        // Parent-page items are composed per page; variables need that page's index.
        let page = match page_name {
            Some(n) if with_vars => (0..doc.page_count()).find(|&i| doc.page_name(i) == n),
            _ => None,
        };
        let running = running.then(|| self.running_index(doc));
        let out = Arc::new(crate::compose_story(
            doc,
            sid,
            &ComposeOptions { page_name: page_name.map(str::to_string), page, running, label: None, xrefs: with_xrefs.then(|| self.xref_index(doc)) },
        ));
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let stamp = g.1;
        if g.0.len() > 4096 {
            g.0.retain(|_, e| stamp - e.stamp < 64);
        }
        let replaced = g.0.insert(key.clone(), Entry { sig, _keep: (story.clone(), doc.styles.clone(), keep_items), out: out.clone(), stamp });
        drop(g);
        if key.1.is_none()
            && let Some(old) = replaced
        {
            self.previous.lock().unwrap_or_else(|e| e.into_inner()).insert(sid, (old.sig, old.out));
        }
        out
    }

    /// The cached composition of `sid` made from exactly this version of `doc` (the current
    /// entry or the one it replaced), without composing anything.
    pub fn composed_for(&self, doc: &Document, sid: StoryId) -> Option<Arc<ComposedStory>> {
        let story = doc.stories.get(&sid)?;
        let (sig, _) = signature(doc, story);
        // Only the geometry/story part of the signature (variables etc. add to it).
        let matches = |s: &[usize]| s.len() >= sig.len() && s[..sig.len()] == sig[..];
        {
            let g = self.map.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(e) = g.0.get(&(sid, None))
                && matches(&e.sig)
            {
                return Some(e.out.clone());
            }
        }
        let g = self.previous.lock().unwrap_or_else(|e| e.into_inner());
        g.get(&sid).filter(|(s, _)| matches(s)).map(|(_, c)| c.clone())
    }

    /// Frames whose composed content differs between two compositions of a story.
    pub fn changed_frames(a: &ComposedStory, b: &ComposedStory) -> Vec<designcraft_doc::ItemId> {
        let same_line = |x: &crate::Line, y: &crate::Line| {
            x.range == y.range
                && (x.baseline - y.baseline).abs() < 1e-9
                && (x.x0 - y.x0).abs() < 1e-9
                && (x.end_x - y.end_x).abs() < 1e-9
                && x.glyphs.len() == y.glyphs.len()
                && x.glyphs
                    .iter()
                    .zip(&y.glyphs)
                    .all(|(g, h)| g.gid == h.gid && g.style == h.style && (g.x - h.x).abs() < 1e-9 && (g.y - h.y).abs() < 1e-9)
        };
        let mut out = Vec::new();
        for fb in &b.frames {
            let same = a.frame(fb.frame).is_some_and(|fa| {
                fa.lines.len() == fb.lines.len()
                    && fa.lines.iter().zip(&fb.lines).all(|(x, y)| same_line(x, y))
                    && fa.decos == fb.decos
                    && fa.tables.is_empty()
                    && fb.tables.is_empty()
                    && fa.notes.is_empty()
                    && fb.notes.is_empty()
                    && fa.objects.is_empty()
                    && fb.objects.is_empty()
            }) && a.styles == b.styles;
            if !same {
                out.push(fb.frame);
            }
        }
        for fa in &a.frames {
            if b.frame(fa.frame).is_none() {
                out.push(fa.frame);
            }
        }
        out
    }

    pub fn clear(&self) {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).0.clear();
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.xrefs.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Anchor pages for cross-references (rebuilt when the document changes).
    pub fn xref_index(&self, doc: &Document) -> Arc<crate::xref::XrefIndex> {
        let sig = doc_signature(doc);
        if let Some((s, r)) = &*self.xrefs.lock().unwrap_or_else(|e| e.into_inner())
            && *s == sig
        {
            return r.clone();
        }
        let r = Arc::new(crate::xref::XrefIndex::build(doc, &|sid| self.get(doc, sid, None)));
        *self.xrefs.lock().unwrap_or_else(|e| e.into_inner()) = Some((sig, r.clone()));
        r
    }

    /// The running-header index for `doc` (rebuilt when the document changes).
    pub fn running_index(&self, doc: &Document) -> Arc<RunningIndex> {
        let sig = doc_signature(doc);
        if let Some((s, r)) = &*self.running.lock().unwrap_or_else(|e| e.into_inner())
            && *s == sig
        {
            return r.clone();
        }
        let r = Arc::new(RunningIndex::build(doc, &|sid| self.get(doc, sid, None)));
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = Some((sig, r.clone()));
        r
    }
}

fn signature(doc: &Document, story: &Arc<Story>) -> (Vec<usize>, Vec<Arc<Item>>) {
    // The document's font scope: another document's fonts of the same name set it differently.
    let mut sig = vec![
        Arc::as_ptr(story) as usize,
        story.rev as usize,
        Arc::as_ptr(&doc.styles) as usize,
        section_signature(doc) as usize,
        doc.font_scope as usize,
    ];
    let mut keep = Vec::new();
    let mut spreads = Vec::new();
    let mut document_grid = false;
    for f in &story.frames {
        let Some(loc) = doc.find(*f) else {
            sig.push(0);
            continue;
        };
        if let Some(item) = doc.item_at(&loc)
            && let Some(tf) = item.text_frame()
            && tf.options.baseline_grid.is_none()
            && !doc.frame_vertical(item)
        {
            document_grid = true;
        }
        if let Some(sp) = doc.spread(loc.spread) {
            let top = &sp.items[loc.top()];
            sig.push(Arc::as_ptr(top) as usize);
            keep.push(top.clone());
            // Page position matters for page-number markers.
            sig.push(loc.top());
            if !spreads.contains(&loc.spread) {
                spreads.push(loc.spread);
                for it in &sp.items {
                    if it.wrap.mode != WrapMode::None {
                        sig.push(Arc::as_ptr(it) as usize);
                        keep.push(it.clone());
                    }
                }
                sig.push(usize::MAX);
            }
        }
    }
    if story.objects.iter().any(|object| {
        matches!(&object.position,
        designcraft_doc::AnchorPosition::Custom { x_relative, y_relative, .. }
            if matches!(x_relative, designcraft_doc::anchored::AnchorRelative::PageMargin | designcraft_doc::anchored::AnchorRelative::PageEdge)
                || matches!(y_relative, designcraft_doc::anchored::AnchorRelative::PageMargin | designcraft_doc::anchored::AnchorRelative::PageEdge))
    }) {
        for frame in crate::frame_specs(doc, story.id) {
            sig.extend(page_anchor_inputs(story, &frame).into_iter().flat_map(float_signature_words));
        }
    }
    if document_grid {
        // Only the grid coordinates used by composition, not colour or display preferences.
        sig.extend([doc.settings.baseline_grid.start, doc.settings.baseline_grid.increment].into_iter().flat_map(float_signature_words));
    }
    (sig, keep)
}

#[cfg(test)]
mod review8_grid32_projection_tests {
    use super::*;
    use designcraft_doc::build::NewDocument;
    use designcraft_doc::{Composer, GridAlign, ItemId, Leading, ParaAttrs, ParaFormat, Position, SpreadRef};
    use designcraft_geom::Rect;

    fn fixture(text: &str, grid: GridAlign, position: Position) -> (Document, StoryId, ItemId) {
        let face = designcraft_fonts::FontDb::global().face("Source Serif 4", "Regular");
        assert_eq!((face.family.as_str(), face.style.as_str()), ("Source Serif 4", "Regular"), "fixture: named bundled face");
        assert_ne!(face.glyph_for('M'), 0, "fixture: M must exist");
        assert_ne!(face.glyph_for('2'), 0, "fixture: 2 must exist");
        let mut doc = Document::new(&NewDocument { facing_pages: false, ..Default::default() });
        doc.settings.baseline_grid.start = 0.0;
        doc.settings.baseline_grid.increment = 12.0;
        doc.settings.advanced_type.superscript_size = 50.0;
        doc.settings.advanced_type.superscript_position = 20.0;
        doc.settings.advanced_type.subscript_size = 50.0;
        doc.settings.advanced_type.subscript_position = 20.0;
        let para = ParaFormat {
            para: ParaAttrs { composer: Some(Composer::SingleLine), hyphenate: Some(false), grid_align: Some(grid), ..Default::default() },
            ..Default::default()
        };
        let layer = doc.default_layer();
        let (frame, sid) = doc.add_text_frame(SpreadRef::Doc(0), Rect::new(36.0, 36.0, 576.0, 156.0), layer, text, para).unwrap();
        let story = doc.story_mut(sid).unwrap();
        story.format_chars(0..text.len(), |format| {
            format.over.font_family = Some("Source Serif 4".into());
            format.over.font_style = Some("Regular".into());
            format.over.size = Some(12.0);
            format.over.leading = Some(Leading::Points(14.0));
        });
        if position != Position::Normal {
            assert_eq!(text, "M2", "fixture: isolated synthesized script glyph");
            story.format_chars(1..2, |format| format.over.position = Some(position));
        }
        doc.check().unwrap();
        let story = doc.story(sid).unwrap();
        assert!(!crate::vars::has_vars(story) && !crate::xref::has_xrefs(story), "fixture: no reference-bank signature suffix");
        assert!(story.notes.is_empty() && story.endnotes.is_empty() && story.tables.is_empty() && story.objects.is_empty());
        let specs = crate::frame_specs(&doc, sid);
        assert_eq!(specs.len(), 1);
        assert_eq!((specs[0].area.width(), specs[0].area.height()), (540.0, 120.0), "fixture: fixed plain frame");
        assert!(!specs[0].vertical && specs[0].opts.baseline_grid.is_none());
        assert_eq!(doc.page_count(), 1);
        for para in &story.paras {
            assert_eq!(doc.styles.resolve_para(para).0.grid_align, grid);
        }
        (doc, sid, frame)
    }

    fn grid_fixture() -> (Document, StoryId, ItemId) {
        fixture(&("MMMMMMMM\n".repeat(11) + "MMMMMMMM"), GridAlign::AllLines, Position::Normal)
    }

    fn shared_allocations(a: &Document, b: &Document) {
        assert!(Arc::ptr_eq(&a.styles, &b.styles), "fixture: styles Arc retained");
        assert_eq!(a.stories.len(), b.stories.len());
        for (sid, story) in &a.stories {
            assert!(Arc::ptr_eq(story, b.stories.get(sid).unwrap()), "fixture: every story Arc retained");
        }
        assert_eq!(a.spreads.len(), b.spreads.len());
        for (old, new) in a.spreads.iter().zip(&b.spreads) {
            assert!(Arc::ptr_eq(old, new), "fixture: spread and frame allocations retained");
            for (item, other) in old.items.iter().zip(&new.items) {
                assert!(Arc::ptr_eq(item, other), "fixture: item Arc retained");
            }
        }
    }

    fn direct(doc: &Document, sid: StoryId) -> ComposedStory {
        // Plain fixtures have no variables/xrefs/notes; Default is their actual consumed input.
        crate::compose_story(doc, sid, &ComposeOptions::default())
    }

    fn same_composition(a: &ComposedStory, b: &ComposedStory) {
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "every consumed composition field must agree");
    }

    fn stored32(cache: &Cache, sid: StoryId) -> Vec<u32> {
        // Read the actual Entry populated by production Cache::get, not a signature helper replica.
        let entries = cache.map.lock().unwrap();
        entries.0.get(&(sid, None)).unwrap().sig.iter().map(|word| *word as u32).collect()
    }

    fn changed_entry(a: &Document, b: &Document, sid: StoryId, old: &ComposedStory, new: &ComposedStory) {
        shared_allocations(a, b);
        a.check().unwrap();
        b.check().unwrap();
        let cache = Cache::new();
        let warm = cache.get(a, sid, None);
        same_composition(old, &warm);
        assert!(Arc::ptr_eq(&warm, &cache.get(a, sid, None)), "fixture: repeated query is a warm cache hit");
        let original = stored32(&cache, sid);
        let changed = cache.get(b, sid, None);
        let updated = stored32(&cache, sid);
        assert_ne!(original, updated, "defect oracle: actual stored signature must distinguish consumed controls on wasm32");
        same_composition(new, &changed);
        assert!(Arc::ptr_eq(&changed, &cache.get(b, sid, None)));
        same_composition(old, &cache.get(a, sid, None));
        assert_eq!(stored32(&cache, sid), original, "restoring exact settings restores the actual signature words");
    }

    fn baselines(cs: &ComposedStory) -> Vec<f64> {
        cs.frames.iter().flat_map(|frame| frame.lines.iter().map(|line| line.baseline)).collect()
    }

    fn grid_change(increment: bool) {
        let (before, sid, _) = grid_fixture();
        let mut after = before.clone();
        if increment {
            after.settings.baseline_grid.increment = 36.0;
        } else {
            after.settings.baseline_grid.start = 5.0;
        }
        let old = direct(&before, sid);
        let new = direct(&after, sid);
        assert!(old.is_overset() && new.is_overset(), "fixture: both actual compositions leave overset text");
        assert!(old.line_count() >= 2 && new.line_count() >= 2, "fixture: enough visible baselines to compare");
        assert_ne!(baselines(&old), baselines(&new), "fixture: the exact numeric control changes real baseline output");
        changed_entry(&before, &after, sid, &old, &new);
    }

    #[test]
    fn r8_grid32_used_start_changes_actual_stored_signature() {
        grid_change(false);
    }

    #[test]
    fn r8_grid32_used_increment_changes_actual_stored_signature() {
        grid_change(true);
    }

    fn script_metrics(cs: &ComposedStory) -> (f64, f64) {
        assert!(!cs.is_overset() && cs.line_count() == 1, "fixture: the two actual glyphs must fit one line");
        let glyphs = &cs.frames[0].lines[0].glyphs;
        let base = glyphs.iter().find(|glyph| glyph.byte == 0 && glyph.visible).unwrap();
        let script = glyphs.iter().find(|glyph| glyph.byte == 1 && glyph.visible).unwrap();
        assert!(base.sy > 0.0 && script.sy > 0.0);
        (script.sy / base.sy, base.y - script.y)
    }

    fn script_change(position: Position, size: bool) {
        let (before, sid, _) = fixture("M2", GridAlign::None, position);
        let mut after = before.clone();
        match (position, size) {
            (Position::Superscript, true) => after.settings.advanced_type.superscript_size = 100.0,
            (Position::Superscript, false) => after.settings.advanced_type.superscript_position = 40.0,
            (Position::Subscript, true) => after.settings.advanced_type.subscript_size = 100.0,
            (Position::Subscript, false) => after.settings.advanced_type.subscript_position = 40.0,
            _ => panic!("test fixture requires synthesized superscript or subscript"),
        }
        let old = direct(&before, sid);
        let new = direct(&after, sid);
        let (old_scale, old_shift) = script_metrics(&old);
        let (new_scale, new_shift) = script_metrics(&new);
        let direction = if position == Position::Superscript { 1.0 } else { -1.0 };
        assert!((old_scale - 0.5).abs() < 1e-9, "fixture: 50 percent synthesized script");
        assert!((old_shift - direction * 2.4).abs() < 1e-9, "fixture: 20 percent of explicit 12pt size");
        if size {
            assert!((new_scale - 1.0).abs() < 1e-9, "fixture: 100 percent script must change actual glyph scale");
            assert!((new_shift - old_shift).abs() < 1e-9, "fixture: only size changes");
        } else {
            assert!((new_scale - old_scale).abs() < 1e-9, "fixture: only position changes");
            assert!((new_shift - direction * 4.8).abs() < 1e-9, "fixture: 40 percent must change actual glyph offset");
        }
        changed_entry(&before, &after, sid, &old, &new);
    }

    #[test]
    fn r8_grid32_superscript_size_changes_actual_stored_signature() {
        script_change(Position::Superscript, true);
    }

    #[test]
    fn r8_grid32_superscript_position_changes_actual_stored_signature() {
        script_change(Position::Superscript, false);
    }

    #[test]
    fn r8_grid32_subscript_size_changes_actual_stored_signature() {
        script_change(Position::Subscript, true);
    }

    #[test]
    fn r8_grid32_subscript_position_changes_actual_stored_signature() {
        script_change(Position::Subscript, false);
    }

    fn unchanged_entry(before: &Document, after: &Document, sid: StoryId) {
        shared_allocations(before, after);
        let old = direct(before, sid);
        same_composition(&old, &direct(after, sid));
        let cache = Cache::new();
        let warm = cache.get(before, sid, None);
        let signature = stored32(&cache, sid);
        same_composition(&old, &warm);
        let current = cache.get(after, sid, None);
        assert_eq!(signature, stored32(&cache, sid), "control: actual non-consumed settings preserve signature");
        assert!(Arc::ptr_eq(&warm, &current), "control: unchanged inputs retain the warm composition");
    }

    #[test]
    fn r8_grid32_unchanged_display_and_frame_override_controls() {
        let (before, sid, frame) = grid_fixture();
        unchanged_entry(&before, &before.clone(), sid);
        let mut display = before.clone();
        display.settings.baseline_grid.color = [1, 2, 3];
        display.settings.baseline_grid.view_threshold = 0.5;
        unchanged_entry(&before, &display, sid);
        let mut overridden = before.clone();
        overridden.item_mut(frame).unwrap().text_frame_mut().unwrap().options.baseline_grid = Some((12.0, 0.0));
        let mut changed_document_grid = overridden.clone();
        changed_document_grid.settings.baseline_grid.start = 5.0;
        changed_document_grid.settings.baseline_grid.increment = 36.0;
        unchanged_entry(&overridden, &changed_document_grid, sid);
    }

    #[test]
    fn r8_grid32_unused_controls_preserve_actual_composition() {
        let (before, sid, _) = fixture("M2", GridAlign::None, Position::Normal);
        let mut after = before.clone();
        after.settings.baseline_grid.start = 5.0;
        after.settings.baseline_grid.increment = 36.0;
        after.settings.advanced_type.superscript_size = 100.0;
        after.settings.advanced_type.superscript_position = 40.0;
        after.settings.advanced_type.subscript_size = 100.0;
        after.settings.advanced_type.subscript_position = 40.0;
        shared_allocations(&before, &after);
        let old = direct(&before, sid);
        same_composition(&old, &direct(&after, sid));
        let cache = Cache::new();
        same_composition(&old, &cache.get(&before, sid, None));
        same_composition(&old, &cache.get(&after, sid, None));
    }
}

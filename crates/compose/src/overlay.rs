//! Character styles that paragraphs apply by themselves: nested styles (from the paragraph start
//! through or up to a delimiter) and GREP styles (every match of a pattern).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use designcraft_doc::{GrepStyle, NestedStyle, NestedUntil};

thread_local! {
    static PATTERNS: RefCell<HashMap<String, Option<regex::Regex>>> = RefCell::new(HashMap::new());
}

fn is_event(u: &NestedUntil, c: char, prev: Option<char>, next: Option<char>) -> bool {
    match u {
        NestedUntil::Sentences => matches!(c, '.' | '!' | '?') && next.is_none_or(char::is_whitespace),
        NestedUntil::Words => c.is_whitespace() && prev.is_some_and(|p| !p.is_whitespace()),
        NestedUntil::Characters => !matches!(c, '\n'),
        NestedUntil::Letters => c.is_alphabetic(),
        NestedUntil::Digits => c.is_numeric(),
        NestedUntil::Tab => c == '\t',
        NestedUntil::ForcedLineBreak => c == designcraft_doc::FORCED_LINE_BREAK,
        NestedUntil::EmSpace => c == '\u{2003}',
        NestedUntil::EnSpace => c == '\u{2002}',
        NestedUntil::Chars(s) => s.contains(c),
    }
}

/// Character-style ranges for the paragraph `text[range]`, in application order (later wins).
pub(crate) fn overlays(text: &str, range: Range<usize>, nested: &[NestedStyle], grep: &[GrepStyle]) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let para = &text[range.clone()];
    // The paragraph's own end mark isn't styled.
    let end = range.start + para.trim_end_matches('\n').len();
    let mut pos = range.start;
    for ns in nested {
        if pos >= end {
            break;
        }
        let mut count = 0;
        let mut stop = end;
        let chars: Vec<(usize, char)> = text[pos..end].char_indices().map(|(i, c)| (pos + i, c)).collect();
        for (k, &(i, c)) in chars.iter().enumerate() {
            let prev = if k > 0 { Some(chars[k - 1].1) } else { None };
            let next = chars.get(k + 1).map(|x| x.1);
            if is_event(&ns.until, c, prev, next) {
                count += 1;
                if count >= ns.count.max(1) {
                    stop = if ns.through { i + c.len_utf8() } else { i };
                    break;
                }
            }
        }
        if stop > pos && !ns.style.is_empty() {
            out.push((pos..stop, ns.style.clone()));
        }
        pos = stop;
    }
    for g in grep {
        if g.style.is_empty() || g.pattern.is_empty() {
            continue;
        }
        PATTERNS.with(|p| {
            let mut p = p.borrow_mut();
            let re = p.entry(g.pattern.clone()).or_insert_with(|| regex::Regex::new(&g.pattern).ok());
            if let Some(re) = re {
                for m in re.find_iter(&text[range.start..end]) {
                    if !m.is_empty() {
                        out.push((range.start + m.start()..range.start + m.end(), g.style.clone()));
                    }
                }
            }
        });
    }
    out
}

/// Whether visible text definitely uses Optical, does not use it, or needs final line ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpticalKerningUse {
    Used,
    NotUsed,
    Unresolved,
}

impl OpticalKerningUse {
    fn from_used(used: bool) -> Self {
        if used { Self::Used } else { Self::NotUsed }
    }

    // Alternatives describe the same text under possible line styles, not independent uses.
    fn alternative(self, other: Self) -> Self {
        if self == other { self } else { Self::Unresolved }
    }

    /// A definite use is sufficient for the document-wide approximation warning.
    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Used, _) | (_, Self::Used) => Self::Used,
            (Self::Unresolved, _) | (_, Self::Unresolved) => Self::Unresolved,
            (Self::NotUsed, Self::NotUsed) => Self::NotUsed,
        }
    }
}

/// Effective Optical formatting for preflight, including source-only overset and children.
/// Composed paragraphs retain their final nested-line result. The fallback resolves text
/// substitutions and generated labels; it does not shape glyphs or run a second layout.
/// Differing Optical outcomes across possible nested line styles remain unresolved.
pub fn source_optical_kerning(
    doc: &designcraft_doc::Document,
    story: &designcraft_doc::Story,
    composed: Option<&crate::ComposedStory>,
    opts: &crate::ComposeOptions,
) -> OpticalKerningUse {
    let page = composed.and_then(|cs| cs.frames.last()).map(|f| frame_page(doc, f.frame)).unwrap_or_default();
    optical_story(doc, story, composed, opts, &page, 0)
}

// Only kerning and pre-reference visibility affect this diagnostic. Resolve full styles
// through the existing resolver, then compare these outcomes without choosing line ranges.
struct SourceOptical {
    visible: Option<bool>,
    text: OpticalKerningUse,
    footnote: OpticalKerningUse,
    endnote: OpticalKerningUse,
}

impl SourceOptical {
    fn resolve(
        doc: &designcraft_doc::Document,
        inherited: &designcraft_doc::CharProps,
        format: &designcraft_doc::CharFormat,
        sub: &crate::shape::SubstCtx,
    ) -> Self {
        let props = doc.styles.resolve_char(inherited, format);
        let visible = props.change != designcraft_doc::ChangeMark::Deleted
            && (props.conditions.is_empty() || !props.conditions.iter().all(|name| sub.hidden_conditions.contains(name)));
        let used = |kerning| OpticalKerningUse::from_used(visible && kerning == designcraft_doc::Kerning::Optical);
        Self {
            visible: Some(visible),
            text: used(props.kerning),
            footnote: used(crate::shape::reference_props(&doc.styles, inherited, format, sub, false).kerning),
            endnote: used(crate::shape::reference_props(&doc.styles, inherited, format, sub, true).kerning),
        }
    }

    fn include(&mut self, other: Self) {
        if self.visible != other.visible {
            self.visible = None;
        }
        self.text = self.text.alternative(other.text);
        self.footnote = self.footnote.alternative(other.footnote);
        self.endnote = self.endnote.alternative(other.endnote);
    }
}

type SourcePage = (Option<String>, Option<usize>);

fn frame_page(doc: &designcraft_doc::Document, frame: designcraft_doc::ItemId) -> SourcePage {
    let Some(loc) = doc.find(frame) else { return Default::default() };
    let Some(item) = doc.item_at(&loc) else { return Default::default() };
    let (name, page, _) = crate::frame_page(doc, loc.spread, doc.spread(loc.spread), item);
    (name, page)
}

// Footnote numbering uses the first real frame containing a paragraph, even when
// that frame has no document page. Missing paragraphs use the caller's fallback.
#[derive(Default)]
struct ParagraphPages {
    first: HashMap<usize, Option<usize>>,
}

impl ParagraphPages {
    fn new(doc: &designcraft_doc::Document, composed: Option<&crate::ComposedStory>) -> Self {
        let mut pages = Self::default();
        for frame in composed.into_iter().flat_map(|cs| &cs.frames).filter(|f| f.frame.0 != 0 && !f.lines.is_empty()) {
            #[cfg(test)]
            tests_optical_pages::count_frame();
            let page = frame_page(doc, frame.frame).1;
            for line in &frame.lines {
                #[cfg(test)]
                tests_optical_pages::count_line();
                pages.first.entry(line.para).or_insert(page);
            }
        }
        pages
    }

    fn get(&self, pi: usize, fallback: Option<usize>) -> Option<usize> {
        #[cfg(test)]
        tests_optical_pages::count_lookup();
        self.first.get(&pi).copied().unwrap_or(fallback)
    }
}

// prepend_label shapes with an empty substitution context and the resolved paragraph base.
// List expressions and note labels can be empty or contain only controls. Inspect
// the expanded label text using the same substitution rules as the compositor.
fn generated_uses_optical(base: &designcraft_doc::CharProps, label: &str) -> bool {
    if base.kerning != designcraft_doc::Kerning::Optical || base.change == designcraft_doc::ChangeMark::Deleted {
        return false;
    }
    let sub = crate::shape::SubstCtx::default();
    label.char_indices().any(|(at, c)| crate::shape::replacement_text(&sub, at, c).is_some_and(|t| !t.is_empty()) || used_character(c))
}

// Local to one parent story; repeated placements retain their first layout-order match.
#[derive(Default)]
struct ComposedChildren<'a> {
    cells: std::collections::HashMap<(u64, usize, usize), &'a crate::ComposedStory>,
    notes: std::collections::HashMap<u64, (&'a crate::FrameText, &'a crate::PlacedNote)>,
}

impl<'a> ComposedChildren<'a> {
    fn from_frames(frames: impl IntoIterator<Item = &'a crate::FrameText>) -> Self {
        let mut children = Self::default();
        for frame in frames {
            for table in &frame.tables {
                for cell in &table.cells {
                    children.cells.entry((table.table, cell.row, cell.col)).or_insert(cell.text.as_ref());
                }
            }
            for note in &frame.notes {
                children.notes.entry(note.id).or_insert((frame, note));
            }
        }
        children
    }
}

fn optical_story(
    doc: &designcraft_doc::Document,
    story: &designcraft_doc::Story,
    composed: Option<&crate::ComposedStory>,
    opts: &crate::ComposeOptions,
    page: &SourcePage,
    depth: usize,
) -> OpticalKerningUse {
    if depth > 16 {
        return OpticalKerningUse::NotUsed;
    }
    if composed.is_some_and(|cs| cs.optical_kerning.iter().take(story.paras.len()).any(|used| *used == Some(true))) {
        return OpticalKerningUse::Used;
    }
    let mut result = OpticalKerningUse::NotUsed;
    let needs_source = (0..story.paras.len()).any(|pi| composed.and_then(|cs| cs.optical_kerning.get(pi)).copied().flatten().is_none());
    let mut notes = crate::notes::Notes::new(doc, story);
    // Preserve source order and the original body-label index, including suppressed references.
    let note_refs: Vec<usize> =
        if notes.active() { story.text.match_indices(designcraft_doc::FOOTNOTE_REF).map(|(at, _)| at).collect() } else { Vec::new() };
    // None means the reference's visibility requires uncomposed nested line ranges.
    let mut eligible_notes = vec![Some(true); note_refs.len()];
    let mut next_note = 0;
    let paragraph_pages = if notes.active() { ParagraphPages::new(doc, composed) } else { ParagraphPages::default() };
    let mut sub = crate::shape::SubstCtx {
        page_name: opts.page_name.clone().or_else(|| page.0.clone()),
        vars: if needs_source && crate::vars::has_vars(story) {
            std::sync::Arc::new(crate::vars::values(doc, opts.page.or(page.1), opts.running.as_deref()))
        } else {
            Default::default()
        },
        xrefs: if needs_source { crate::xref::texts_in(doc, story, 0..story.len(), opts.xrefs.as_deref()) } else { Default::default() },
        hidden_conditions: doc.conditions.iter().filter(|c| !c.visible).map(|c| c.name.clone()).collect(),
        ..Default::default()
    };
    if needs_source && !story.endnotes.is_empty() {
        let first = doc.endnote_start(story.id);
        for (k, (at, _)) in story.text.match_indices(designcraft_doc::ENDNOTE_REF).enumerate() {
            sub.notes.insert(at, doc.endnote_options.label(first.saturating_add(k as u32)));
        }
    }
    let context = if depth == 0 { crate::StoryContext::Outer } else { crate::StoryContext::Child };
    let ranges = story.para_ranges();
    let mut list_counter: u32 = 0;
    let mut named_counters = std::collections::HashMap::<String, u32>::new();
    for (pi, pf) in story.paras.iter().enumerate() {
        let Some(range) = ranges.get(pi) else { continue };
        let (para, base) = crate::source_paragraph_properties(doc, story, pf, range, context);
        let first_note = next_note;
        while next_note < note_refs.len() && note_refs[next_note] < range.end {
            next_note += 1;
        }
        let final_refs = composed.and_then(|cs| cs.footnote_references.get(pi)).and_then(Option::as_ref);
        if let Some(refs) = final_refs {
            for k in first_note..next_note {
                eligible_notes[k] = Some(refs.contains(&note_refs[k]));
            }
        }
        let source_footnotes = first_note < next_note && final_refs.is_none();
        // Advance through composed paragraphs too, matching the compositor's named
        // list prepass and its local counter. Empty list paragraphs do not count.
        let label = match para.list_type {
            designcraft_doc::ListType::Numbers | designcraft_doc::ListType::Bullets if range.is_empty() => String::new(),
            designcraft_doc::ListType::Numbers if !para.list_name.is_empty() => {
                let n = named_counters.entry(para.list_name.clone()).or_insert_with(|| doc.list_start(story.id, &para.list_name));
                *n = para.start_at.map_or(n.saturating_add(1), |start| start.max(1));
                para.number_label(*n)
            }
            designcraft_doc::ListType::Numbers => {
                list_counter = para.start_at.map_or(list_counter.saturating_add(1), |start| start.max(1));
                para.number_label(list_counter)
            }
            designcraft_doc::ListType::Bullets => para.bullet_label(),
            designcraft_doc::ListType::None => {
                list_counter = 0;
                String::new()
            }
        };
        // Table paragraphs bypass shape_para and prepend_label in the compositor.
        if story.para_table(pi).is_some() {
            list_counter = 0;
            continue;
        }
        if notes.active() {
            notes.number_refs(doc, range.clone(), paragraph_pages.get(pi, page.1), &mut sub);
        }
        let final_optical = composed.and_then(|cs| cs.optical_kerning.get(pi)).copied().flatten();
        if let Some(used) = final_optical {
            if used {
                return OpticalKerningUse::Used;
            }
            if !source_footnotes {
                continue;
            }
        }
        let check_optical = final_optical.is_none();
        if check_optical
            && (generated_uses_optical(&base, &label) || (pi == 0 && opts.label.as_deref().is_some_and(|label| generated_uses_optical(&base, label))))
        {
            return OpticalKerningUse::Used;
        }
        // Include the unstyled base and each possible line style. This deliberately
        // over-approximates placement, but equal Optical/visibility outcomes are definite.
        let mut seen = std::collections::HashSet::new();
        let line_styles: Vec<_> = para
            .nested_line_styles
            .iter()
            .take_while(|line| line.lines > 0)
            .filter(|line| !line.style.is_empty() && line.style != designcraft_doc::NO_CHAR_STYLE && seen.insert(line.style.as_str()))
            .map(|line| designcraft_doc::CharFormat { style: line.style.clone(), over: Default::default() })
            .collect();
        let automatic = overlays(&story.text, range.clone(), &para.nested_styles, &para.grep_styles);
        let mut source_note = first_note;
        for (run, format) in story.runs() {
            let (a, b) = (run.start.max(range.start), run.end.min(range.end));
            if a >= b {
                continue;
            }
            let mut cuts = vec![a, b];
            cuts.extend(automatic.iter().flat_map(|(r, _)| [r.start, r.end]).filter(|at| *at > a && *at < b));
            cuts.sort_unstable();
            cuts.dedup();
            for part in cuts.windows(2) {
                let Some((&start, &end)) = part.first().zip(part.get(1)) else { continue };
                let top = automatic.iter().rev().find(|(r, _)| r.start <= start && start < r.end);
                let inherited = match top {
                    Some((_, style)) => {
                        doc.styles.resolve_char(&base, &designcraft_doc::CharFormat { style: style.clone(), over: Default::default() })
                    }
                    None => base.clone(),
                };
                let mut source = SourceOptical::resolve(doc, &inherited, format, &sub);
                // shape_para selects one top overlay: nested/GREP ranges replace line styles.
                // Run styles and direct overrides are resolved above every candidate.
                if top.is_none() {
                    for style in &line_styles {
                        let inherited = doc.styles.resolve_char(&base, style);
                        source.include(SourceOptical::resolve(doc, &inherited, format, &sub));
                    }
                }
                if source_footnotes {
                    while source_note < next_note && note_refs[source_note] < end {
                        if note_refs[source_note] >= start {
                            eligible_notes[source_note] = source.visible;
                        }
                        source_note += 1;
                    }
                }
                if !check_optical
                    || (source.text == OpticalKerningUse::NotUsed
                        && source.footnote == OpticalKerningUse::NotUsed
                        && source.endnote == OpticalKerningUse::NotUsed)
                {
                    continue;
                }
                for (offset, c) in story.text.get(start..end).unwrap_or("").char_indices() {
                    let used = crate::shape::replacement_text(&sub, start + offset, c).is_some_and(|text| !text.is_empty()) || used_character(c);
                    if !used {
                        continue;
                    }
                    let used = match c {
                        designcraft_doc::FOOTNOTE_REF => source.footnote,
                        designcraft_doc::ENDNOTE_REF => source.endnote,
                        _ => source.text,
                    };
                    result = result.combine(used);
                    if result == OpticalKerningUse::Used {
                        return result;
                    }
                }
            }
        }
    }
    let children = ComposedChildren::from_frames(composed.into_iter().flat_map(|cs| &cs.frames));
    for table in story.tables.values() {
        let columns = table.ncols();
        if columns == 0 {
            continue;
        }
        let owners = table.owners();
        for (index, cell) in table.cells.iter().enumerate() {
            let (row, col) = (index / columns, index % columns);
            if owners.get(index) != Some(&(row, col)) || cell.graphic.is_some() {
                continue;
            }
            let child = children.cells.get(&(table.id, row, col)).copied();
            // compose_cell receives the explicit options, without the parent's frame page.
            let cell_page = (opts.page_name.clone(), opts.page);
            result = result.combine(optical_story(doc, &cell.text, child, opts, &cell_page, depth + 1));
            if result == OpticalKerningUse::Used {
                return result;
            }
        }
    }
    for (k, note) in story.notes.iter().enumerate() {
        let placed = children.notes.get(&note.id).copied();
        if placed.is_none() && eligible_notes.get(k) == Some(&Some(false)) {
            continue;
        }
        let child = placed.map(|(_, n)| n.text.as_ref());
        let note_page = placed.filter(|(f, _)| f.frame.0 != 0).map_or_else(|| page.clone(), |(f, _)| frame_page(doc, f.frame));
        let mut note_opts = opts.clone();
        note_opts.label = notes.active().then(|| notes.body_label(doc, k));
        let mut used = optical_story(doc, &note.text, child, &note_opts, &note_page, depth + 1);
        if placed.is_none() && eligible_notes.get(k) == Some(&None) && used != OpticalKerningUse::NotUsed {
            used = OpticalKerningUse::Unresolved;
        }
        result = result.combine(used);
        if result == OpticalKerningUse::Used {
            return result;
        }
    }
    // Endnote source is retained separately; it has no prepend_label composition input.
    let end_opts = crate::ComposeOptions { label: None, ..opts.clone() };
    for note in &story.endnotes {
        result = result.combine(optical_story(doc, &note.text, None, &end_opts, page, depth + 1));
        if result == OpticalKerningUse::Used {
            return result;
        }
    }
    result
}

fn used_character(c: char) -> bool {
    !c.is_control() && c != designcraft_doc::FORCED_LINE_BREAK && !('\u{E000}'..='\u{E1FF}').contains(&c)
}

pub(crate) fn used_text(text: &str) -> bool {
    text.chars().any(used_character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_and_grep_ranges() {
        let text = "Q: what is 42? A drop of 7 words.\n".to_string();
        let n = |style: &str, through: bool, count: u32, until: NestedUntil| NestedStyle { style: style.into(), through, count, until };
        // Through the first ':' then up to the second word end.
        let o = overlays(&text, 0..text.len(), &[n("Lead", true, 1, NestedUntil::Chars(":".into())), n("Next", false, 2, NestedUntil::Words)], &[]);
        assert_eq!(&text[o[0].0.clone()], "Q:");
        assert_eq!(&text[o[1].0.clone()], " what is", "up to the end of the second word");
        // Digits by GREP.
        let o = overlays(&text, 0..text.len(), &[], &[GrepStyle { style: "Num".into(), pattern: r"\d+".into() }]);
        let got: Vec<&str> = o.iter().map(|(r, _)| &text[r.clone()]).collect();
        assert_eq!(got, ["42", "7"]);
        // A bad pattern is ignored.
        assert!(overlays(&text, 0..text.len(), &[], &[GrepStyle { style: "X".into(), pattern: "(".into() }]).is_empty());
    }
}

#[cfg(test)]
mod tests_optical_children;

#[cfg(test)]
mod tests_optical_pages;

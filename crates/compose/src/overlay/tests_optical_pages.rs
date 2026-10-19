use super::{OpticalKerningUse, ParagraphPages, frame_page, source_optical_kerning};
use crate::{ComposeOptions, ComposedStory, FrameText, Line};
use designcraft_doc::build::NewDocument;
use designcraft_doc::{CharAttrs, Document, ItemId, Kerning, ParaFormat, SpreadRef, StoryId};
use designcraft_geom::Rect;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PageWork {
    frames: usize,
    lines: usize,
    lookups: usize,
}

thread_local! {
    static WORK: Cell<PageWork> = const { Cell::new(PageWork { frames: 0, lines: 0, lookups: 0 }) };
}

// Count the operations in the production diagnostic path, not a copy of its algorithm.
// Thread-local counters keep unrelated parallel tests from changing the measurements.
pub(super) fn count_frame() {
    WORK.with(|work| {
        let mut next = work.get();
        next.frames += 1;
        work.set(next);
    });
}

pub(super) fn count_line() {
    WORK.with(|work| {
        let mut next = work.get();
        next.lines += 1;
        work.set(next);
    });
}

pub(super) fn count_lookup() {
    WORK.with(|work| {
        let mut next = work.get();
        next.lookups += 1;
        work.set(next);
    });
}

fn metrics_story(paragraphs: usize) -> (Document, StoryId, ItemId, ComposedStory) {
    let mut doc = Document::new(&NewDocument { pages: 2, facing_pages: false, ..Default::default() });
    let text = vec!["Metrics text."; paragraphs].join("\n");
    let format = ParaFormat { chars: CharAttrs { kerning: Some(Kerning::Metrics), ..Default::default() }, ..Default::default() };
    let layer = doc.default_layer();
    let (frame, sid) =
        doc.add_text_frame(SpreadRef::Doc(0), Rect::new(0.0, 0.0, 400.0, paragraphs as f64 * 32.0 + 128.0), layer, &text, format.clone()).unwrap();
    let story = doc.story_mut(sid).unwrap();
    let at = story.len();
    story.insert_note(at, "Metrics footnote.", format);
    let composed = crate::compose_story(&doc, sid, &ComposeOptions::default());
    let story = doc.story(sid).unwrap();
    assert_eq!(story.paras.len(), paragraphs);
    assert_eq!(story.notes.len(), 1);
    assert_eq!(story.text.matches(designcraft_doc::FOOTNOTE_REF).count(), 1);
    assert!(crate::notes::Notes::new(&doc, story).active());
    assert!(!composed.is_overset());
    assert_eq!(composed.optical_kerning, vec![Some(false); paragraphs]);
    assert_eq!(composed.frames.iter().map(|f| f.notes.len()).sum::<usize>(), 1, "the footnote must actually be placed");
    (doc, sid, frame, composed)
}

#[test]
fn metrics_footnote_diagnostic_page_work_scales_linearly() {
    let mut previous: Option<PageWork> = None;
    for paragraphs in [128, 256, 512] {
        let (doc, sid, _, composed) = metrics_story(paragraphs);
        let story = doc.story(sid).unwrap();
        let lines = composed.frames.iter().map(|f| f.lines.len()).sum::<usize>();
        assert_eq!(lines, paragraphs, "the short paragraphs must each occupy one line");
        for _ in 0..2 {
            // Repeated preflight checks reuse the same composition, as the UI does.
            WORK.with(|work| work.set(PageWork::default()));
            assert_eq!(source_optical_kerning(&doc, story, Some(&composed), &ComposeOptions::default()), OpticalKerningUse::NotUsed);
            let work = WORK.with(Cell::get);
            assert_eq!(work.frames, 1, "resolve the real frame once, not once per paragraph");
            assert_eq!(work.lines, lines, "visit each composed line once per diagnostic call");
            assert_eq!(work.lookups, paragraphs, "the valid footnote must keep the paragraph lookup path active");
            if let Some(previous) = previous {
                assert_eq!(work.lines, 2 * previous.lines);
                assert_eq!(work.lookups, 2 * previous.lookups);
                assert_eq!(work.frames, previous.frames);
            }
        }
        previous = Some(WORK.with(Cell::get));
    }
}

fn frame_with_paragraphs(frame: ItemId, template: &Line, paragraphs: &[usize]) -> FrameText {
    let lines = paragraphs.iter().map(|&para| Line { para, ..template.clone() }).collect();
    FrameText { frame, lines, ..Default::default() }
}

#[test]
fn paragraph_pages_preserve_first_real_occurrence_and_fallback() {
    let (mut doc, _, first, composed) = metrics_story(3);
    let layer = doc.default_layer();
    let (second, _) = doc.add_text_frame(SpreadRef::Doc(1), Rect::new(0.0, 0.0, 400.0, 200.0), layer, "", ParaFormat::default()).unwrap();
    assert_eq!(frame_page(&doc, first).1, Some(0));
    assert_eq!(frame_page(&doc, second).1, Some(1));
    let template = &composed.frames[0].lines[0];
    let unknown = ItemId(u64::MAX);
    let layout = ComposedStory {
        frames: vec![
            frame_with_paragraphs(ItemId(0), template, &[0, 4]),
            frame_with_paragraphs(first, template, &[0, 1, 1]),
            frame_with_paragraphs(second, template, &[1, 2]),
            frame_with_paragraphs(unknown, template, &[3]),
            frame_with_paragraphs(second, template, &[3]),
            FrameText { frame: first, ..Default::default() },
        ],
        ..Default::default()
    };
    let pages = ParagraphPages::new(&doc, Some(&layout));
    let before = WORK.with(Cell::get);
    for _ in 0..3 {
        for (pi, expected) in [(0, Some(0)), (1, Some(0)), (2, Some(1)), (3, None), (4, Some(9)), (5, Some(9))] {
            assert_eq!(pages.get(pi, Some(9)), expected);
            // The previous lookup's behavior is the oracle, including an unresolved
            // nonzero frame: its None must not become either the fallback or a later page.
            let legacy = layout
                .frames
                .iter()
                .find(|f| f.frame.0 != 0 && f.lines.iter().any(|line| line.para == pi))
                .map_or(Some(9), |f| frame_page(&doc, f.frame).1);
            assert_eq!(pages.get(pi, Some(9)), legacy);
        }
    }
    let after = WORK.with(Cell::get);
    assert_eq!((after.frames, after.lines), (before.frames, before.lines), "hits and misses do not revisit the layout");
    assert_eq!(pages.get(4, None), None);
    assert_eq!(ParagraphPages::new(&doc, None).get(0, Some(9)), Some(9));
    assert_eq!(ParagraphPages::new(&doc, Some(&ComposedStory::default())).get(0, None), None);
}

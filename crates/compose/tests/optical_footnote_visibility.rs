use designcraft_compose::{ComposeOptions, OpticalKerningUse, compose_story, source_optical_kerning};
use designcraft_doc::build::NewDocument;
use designcraft_doc::{
    ChangeMark, CharAttrs, CharacterStyle, Composer, Condition, Document, FOOTNOTE_REF, GrepStyle, NestedLineStyle, ParaAttrs, ParaFormat, SpreadRef,
    StoryId,
};
use designcraft_geom::Rect;

const NOTE_TEXT: &str = "Optical footnote body.";
const HIDDEN: &str = "Hidden reference";
const VISIBLE: &str = "Visible reference";
const HIDDEN_CHILD: &str = "Inherited hidden reference";

struct Fixture {
    doc: Document,
    story: StoryId,
    note: u64,
    anchor: usize,
    paragraph: usize,
    overset: bool,
}

fn fixture(overset: bool, composer: Composer) -> Fixture {
    let prefix = if overset { "Filler paragraph.\n".repeat(20) } else { "Main text. ".into() };
    fixture_with_prefix(&prefix, overset, composer)
}

fn fixture_with_prefix(prefix: &str, overset: bool, composer: Composer) -> Fixture {
    let mut doc = Document::new(&NewDocument::default());
    doc.conditions.push(Condition { name: HIDDEN.into(), color: [255, 0, 0], visible: false });
    doc.conditions.push(Condition { name: VISIBLE.into(), color: [0, 128, 0], visible: true });
    let anchor = prefix.len();
    let paragraph = prefix.matches('\n').count();
    let format = ParaFormat {
        para: ParaAttrs { composer: Some(composer), ..Default::default() },
        chars: CharAttrs { kerning: Some(designcraft_doc::Kerning::Metrics), ..Default::default() },
        ..Default::default()
    };
    let layer = doc.default_layer();
    let height = if overset { 64.0 } else { 500.0 };
    let (_, story) = doc.add_text_frame(SpreadRef::Doc(0), Rect::new(0.0, 0.0, 400.0, height), layer, prefix, format.clone()).unwrap();
    let parent = doc.story_mut(story).unwrap();
    let note = parent.insert_note(anchor, NOTE_TEXT, format);
    let body = &mut parent.note_mut(note).unwrap().text;
    body.format_chars(0..NOTE_TEXT.len(), |f| f.over.kerning = Some(designcraft_doc::Kerning::Optical));
    parent.check().unwrap();
    Fixture { doc, story, note, anchor, paragraph, overset }
}

fn reference_attrs(fixture: &mut Fixture, attrs: CharAttrs) {
    fixture.doc.story_mut(fixture.story).unwrap().format_chars(fixture.anchor..fixture.anchor + FOOTNOTE_REF.len_utf8(), |f| {
        f.over = attrs.clone();
    });
}

fn hidden_style(fixture: &mut Fixture) {
    let styles = fixture.doc.styles_mut();
    styles.character.push(CharacterStyle {
        name: "Hidden reference base".into(),
        based_on: None,
        chars: CharAttrs { conditions: Some(vec![HIDDEN.into()]), ..Default::default() },
        shortcut: String::new(),
    });
    styles.character.push(CharacterStyle {
        name: HIDDEN_CHILD.into(),
        based_on: Some("Hidden reference base".into()),
        chars: CharAttrs::default(),
        shortcut: String::new(),
    });
}

// Check observable composition before checking the diagnostic. Short-frame cases
// exercise both a shaped-but-unplaced reference and an entirely unreached paragraph.
fn assert_note_visibility(fixture: &Fixture, reference_visible: bool, check_source_only: bool) {
    let opts = ComposeOptions::default();
    let composed = compose_story(&fixture.doc, fixture.story, &opts);
    let story = fixture.doc.story(fixture.story).unwrap();
    assert_eq!(story.notes.len(), 1, "suppression must retain the note in the source");
    assert_eq!(story.note(fixture.note).unwrap().text.text, NOTE_TEXT);
    assert_eq!(story.note_anchor(fixture.note), Some(fixture.anchor));
    assert!(composed.optical_kerning.iter().all(|used| *used != Some(true)), "only the child body uses Optical");
    let placed: Vec<_> = composed.frames.iter().flat_map(|frame| &frame.notes).collect();
    let visible_reference = composed
        .frames
        .iter()
        .flat_map(|frame| &frame.lines)
        .flat_map(|line| &line.glyphs)
        .any(|glyph| glyph.byte == fixture.anchor && glyph.len > 0 && glyph.visible);
    if fixture.overset {
        assert!(composed.is_overset());
        assert!(composed.line_count() > 0, "the frame must compose real text before it runs out of room");
        assert!(composed.overset_at.is_some_and(|at| at < fixture.anchor));
        if fixture.paragraph == 0 {
            assert_eq!(composed.optical_kerning.first(), Some(&Some(false)), "the parent paragraph was actually shaped");
        } else {
            assert!(composed.frames.iter().flat_map(|frame| &frame.lines).all(|line| line.para < fixture.paragraph));
            assert_eq!(composed.optical_kerning.get(fixture.paragraph).copied().flatten(), None, "the reference paragraph was not shaped");
        }
        assert!(placed.is_empty(), "the overset reference must have no placed note");
        assert!(!visible_reference);
    } else {
        assert!(!composed.is_overset());
        assert_eq!(visible_reference, reference_visible, "the rendered reference must agree with effective visibility");
        assert_eq!(placed.len(), usize::from(reference_visible), "a suppressed reference must not place its note");
        if reference_visible {
            let note = placed[0];
            assert_eq!(note.id, fixture.note);
            assert!(note.text.line_count() > 0);
            assert!(note.text.frames.iter().flat_map(|frame| &frame.lines).flat_map(|line| &line.glyphs).any(|glyph| glyph.visible));
            assert!(note.text.optical_kerning.contains(&Some(true)), "the placed body must actually be shaped with Optical");
        }
    }
    assert_eq!(
        source_optical_kerning(&fixture.doc, story, Some(&composed), &opts),
        if reference_visible { OpticalKerningUse::Used } else { OpticalKerningUse::NotUsed },
        "the diagnostic must follow the effective reference, including unplaced overset notes"
    );
    if check_source_only {
        assert_eq!(
            source_optical_kerning(&fixture.doc, story, None, &opts),
            if reference_visible { OpticalKerningUse::Used } else { OpticalKerningUse::NotUsed },
            "source fallback must distinguish a suppressed reference from a visible unplaced reference"
        );
    }
}

#[test]
fn visible_placed_optical_footnote_warns() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        assert_note_visibility(&fixture(false, composer), true, true);
    }
}

#[test]
fn visible_overset_optical_footnote_warns_from_source() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        assert_note_visibility(&fixture(true, composer), true, true);
    }
}

#[test]
fn hidden_condition_on_reference_suppresses_optical_body() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        for overset in [false, true] {
            let mut fixture = fixture(overset, composer);
            reference_attrs(&mut fixture, CharAttrs { conditions: Some(vec![HIDDEN.into()]), ..Default::default() });
            assert_note_visibility(&fixture, false, true);
        }
    }
}

#[test]
fn deleted_reference_suppresses_retained_optical_body() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        for overset in [false, true] {
            let mut fixture = fixture(overset, composer);
            reference_attrs(&mut fixture, CharAttrs { change: Some(ChangeMark::Deleted), ..Default::default() });
            assert_note_visibility(&fixture, false, true);
        }
    }
}

#[test]
fn inherited_character_style_hides_reference_and_optical_body() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        for overset in [false, true] {
            let mut fixture = fixture(overset, composer);
            hidden_style(&mut fixture);
            fixture.doc.story_mut(fixture.story).unwrap().format_chars(fixture.anchor..fixture.anchor + FOOTNOTE_REF.len_utf8(), |f| {
                f.style = HIDDEN_CHILD.into();
            });
            assert_note_visibility(&fixture, false, true);
        }
    }
}

#[test]
fn grep_style_hides_reference_and_optical_body() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        for overset in [false, true] {
            let mut fixture = fixture(overset, composer);
            hidden_style(&mut fixture);
            fixture.doc.story_mut(fixture.story).unwrap().paras[fixture.paragraph].para.grep_styles =
                Some(vec![GrepStyle { style: HIDDEN_CHILD.into(), pattern: FOOTNOTE_REF.to_string() }]);
            assert_note_visibility(&fixture, false, true);
        }
    }
}

#[test]
fn composed_nested_line_style_hides_reference_and_optical_body() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        let mut fixture = fixture(false, composer);
        hidden_style(&mut fixture);
        fixture.doc.story_mut(fixture.story).unwrap().paras[fixture.paragraph].para.nested_line_styles =
            Some(vec![NestedLineStyle { style: HIDDEN_CHILD.into(), lines: 1 }]);
        // Nested line ranges belong to actual composition; the source-only API
        // deliberately does not evaluate them for unreached paragraphs.
        assert_note_visibility(&fixture, false, false);
    }
}

#[test]
fn any_visible_condition_keeps_optical_footnote_visible() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        for overset in [false, true] {
            let mut fixture = fixture(overset, composer);
            hidden_style(&mut fixture);
            fixture.doc.story_mut(fixture.story).unwrap().format_chars(fixture.anchor..fixture.anchor + FOOTNOTE_REF.len_utf8(), |f| {
                f.style = HIDDEN_CHILD.into();
            });
            reference_attrs(&mut fixture, CharAttrs { conditions: Some(vec![HIDDEN.into(), VISIBLE.into()]), ..Default::default() });
            assert_note_visibility(&fixture, true, true);
        }
    }
}

#[test]
fn visible_overset_reference_in_shaped_paragraph_still_warns() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        let fixture = fixture_with_prefix(&"Filler paragraph. ".repeat(80), true, composer);
        assert_note_visibility(&fixture, true, true);
    }
}

#[test]
fn hidden_and_visible_references_in_one_paragraph_keep_their_own_bodies() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        for optical_in_hidden_body in [false, true] {
            let mut fixture = fixture(false, composer);
            let parent = fixture.doc.story_mut(fixture.story).unwrap();
            let visible_anchor = parent.len();
            let format = ParaFormat {
                para: ParaAttrs { composer: Some(composer), ..Default::default() },
                chars: CharAttrs { kerning: Some(designcraft_doc::Kerning::Metrics), ..Default::default() },
                ..Default::default()
            };
            let visible_note = parent.insert_note(visible_anchor, NOTE_TEXT, format);
            for (id, optical) in [(fixture.note, optical_in_hidden_body), (visible_note, !optical_in_hidden_body)] {
                parent.note_mut(id).unwrap().text.format_chars(0..NOTE_TEXT.len(), |f| {
                    f.over.kerning = Some(if optical { designcraft_doc::Kerning::Optical } else { designcraft_doc::Kerning::Metrics });
                });
            }
            reference_attrs(&mut fixture, CharAttrs { conditions: Some(vec![HIDDEN.into()]), ..Default::default() });
            let opts = ComposeOptions::default();
            let composed = compose_story(&fixture.doc, fixture.story, &opts);
            let story = fixture.doc.story(fixture.story).unwrap();
            assert_eq!(story.notes.len(), 2, "the hidden note remains in the source before the visible note");
            assert!(!composed.is_overset());
            let placed: Vec<_> = composed.frames.iter().flat_map(|frame| &frame.notes).collect();
            assert_eq!(placed.len(), 1);
            assert_eq!(placed[0].id, visible_note, "eligibility must be tracked per reference, not per paragraph or placed-note ordinal");
            assert!(placed[0].text.line_count() > 0);
            assert_eq!(placed[0].text.optical_kerning.contains(&Some(true)), !optical_in_hidden_body);
            for actual in [Some(&composed), None] {
                assert_eq!(
                    source_optical_kerning(&fixture.doc, story, actual, &opts),
                    if optical_in_hidden_body { OpticalKerningUse::NotUsed } else { OpticalKerningUse::Used },
                    "only the body belonging to the visible reference may produce the warning"
                );
            }
        }
    }
}

#[test]
fn composed_nested_line_style_suppresses_shaped_but_unplaced_reference_tail() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        let prefix = "First normal line.\u{2028}Second normal line.\u{2028}Hidden tail. ";
        let second_line_start = "First normal line.\u{2028}".len();
        let mut fixture = fixture_with_prefix(prefix, true, composer);
        hidden_style(&mut fixture);
        fixture.doc.story_mut(fixture.story).unwrap().paras[0].para.nested_line_styles = Some(vec![
            NestedLineStyle { style: designcraft_doc::NO_CHAR_STYLE.into(), lines: 1 },
            NestedLineStyle { style: designcraft_doc::NO_CHAR_STYLE.into(), lines: 1 },
            NestedLineStyle { style: HIDDEN_CHILD.into(), lines: 8 },
        ]);
        // Forced breaks keep two normal lines ahead of the hidden tail during
        // every nested-line shaping pass. Only the first normal line fits.
        let mut frames = designcraft_compose::frame_specs(&fixture.doc, fixture.story);
        assert_eq!(frames.len(), 1);
        let frame = frames.first_mut().unwrap();
        frame.area = Rect::new(0.0, 0.0, 400.0, 18.0);
        frame.opts.first_baseline = designcraft_doc::FirstBaseline::Fixed;
        frame.opts.first_baseline_min = 12.0;
        let opts = ComposeOptions::default();
        let story = fixture.doc.story(fixture.story).unwrap();
        let composed = designcraft_compose::compose(&fixture.doc, story, &frames, &opts);
        assert_eq!(story.paras.len(), 1, "forced line breaks must leave the reference in the shaped paragraph");
        assert_eq!(story.notes.len(), 1);
        assert_eq!(story.note_anchor(fixture.note), Some(fixture.anchor));
        assert!(composed.is_overset());
        assert_eq!(composed.optical_kerning, vec![Some(false)], "the whole parent paragraph was actually shaped");
        assert_eq!(composed.line_count(), 1, "the second normal line and hidden reference tail must remain unplaced");
        let line = composed.frames.iter().flat_map(|frame| &frame.lines).next().unwrap();
        assert_eq!(line.para, 0);
        assert_eq!(line.range.start, 0);
        assert!(line.range.end <= second_line_start);
        assert!(line.glyphs.iter().any(|glyph| glyph.visible), "the placed prefix must contain real visible text");
        assert!(line.glyphs.iter().filter(|glyph| glyph.visible).all(|glyph| glyph.byte < second_line_start));
        assert!(line.glyphs.iter().all(|glyph| glyph.byte < fixture.anchor), "no reference glyph was placed");
        assert!(composed.overset_at.is_some_and(|at| at > 0 && at < fixture.anchor));
        assert!(composed.frames.iter().all(|frame| frame.notes.is_empty()), "the unplaced reference must have no composed child");
        assert_eq!(
            source_optical_kerning(&fixture.doc, story, Some(&composed), &opts),
            OpticalKerningUse::NotUsed,
            "final nested-line suppression must survive even though the reference has no placed glyph or child"
        );
        assert_eq!(
            source_optical_kerning(&fixture.doc, story, None, &opts),
            OpticalKerningUse::Unresolved,
            "an Optical body cannot prove use when the reference needs uncomposed line ranges"
        );
    }
}

#[test]
fn r47_optical_uncomposed_reference_visibility_requires_an_optical_child() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        let mut fixture = fixture(true, composer);
        hidden_style(&mut fixture);
        fixture.doc.story_mut(fixture.story).unwrap().paras[fixture.paragraph].para.nested_line_styles =
            Some(vec![NestedLineStyle { style: HIDDEN_CHILD.into(), lines: 1 }]);
        let opts = ComposeOptions::default();
        let composed = compose_story(&fixture.doc, fixture.story, &opts);
        assert!(composed.is_overset());
        assert_eq!(composed.optical_kerning.get(fixture.paragraph), None);
        assert!(composed.frames.iter().all(|frame| frame.notes.is_empty()));
        let before = fixture.doc.clone();
        let story = fixture.doc.story(fixture.story).unwrap();
        for actual in [Some(&composed), None] {
            assert_eq!(source_optical_kerning(&fixture.doc, story, actual, &opts), OpticalKerningUse::Unresolved);
        }
        assert_eq!(fixture.doc, before);
        // Visibility ambiguity alone is not an Optical problem for all-Metrics content.
        fixture.doc.story_mut(fixture.story).unwrap().note_mut(fixture.note).unwrap().text.format_chars(0..NOTE_TEXT.len(), |f| {
            f.over.kerning = Some(designcraft_doc::Kerning::Metrics);
        });
        let story = fixture.doc.story(fixture.story).unwrap();
        assert_eq!(source_optical_kerning(&fixture.doc, story, None, &opts), OpticalKerningUse::NotUsed);
        fixture.doc.story_mut(fixture.story).unwrap().note_mut(fixture.note).unwrap().text.format_chars(0..NOTE_TEXT.len(), |f| {
            f.over.kerning = Some(designcraft_doc::Kerning::Optical);
        });
        // A higher-priority automatic style decides the reference without line ranges.
        for (style, expected) in [(HIDDEN_CHILD, OpticalKerningUse::NotUsed), (designcraft_doc::NO_CHAR_STYLE, OpticalKerningUse::Used)] {
            fixture.doc.story_mut(fixture.story).unwrap().paras[fixture.paragraph].para.grep_styles =
                Some(vec![GrepStyle { style: style.into(), pattern: FOOTNOTE_REF.to_string() }]);
            let story = fixture.doc.story(fixture.story).unwrap();
            assert_eq!(source_optical_kerning(&fixture.doc, story, None, &opts), expected);
        }
    }
}

#[test]
fn r47_optical_retained_endnote_source_preserves_uncertainty() {
    for composer in [Composer::Paragraph, Composer::SingleLine] {
        let mut fixture = fixture(false, composer);
        fixture.doc.styles_mut().character.push(CharacterStyle {
            name: "Endnote Optical line".into(),
            based_on: None,
            chars: CharAttrs { kerning: Some(designcraft_doc::Kerning::Optical), ..Default::default() },
            shortcut: String::new(),
        });
        let parent = fixture.doc.story_mut(fixture.story).unwrap();
        parent.note_mut(fixture.note).unwrap().text.format_chars(0..NOTE_TEXT.len(), |f| {
            f.over.kerning = Some(designcraft_doc::Kerning::Metrics);
        });
        let format = ParaFormat {
            para: ParaAttrs {
                composer: Some(composer),
                nested_line_styles: Some(vec![NestedLineStyle { style: "Endnote Optical line".into(), lines: 1 }]),
                ..Default::default()
            },
            chars: CharAttrs { kerning: Some(designcraft_doc::Kerning::Metrics), ..Default::default() },
            ..Default::default()
        };
        parent.insert_endnote(parent.len(), "Endnote body", format);
        let opts = ComposeOptions::default();
        let before = fixture.doc.clone();
        let story = fixture.doc.story(fixture.story).unwrap();
        assert_eq!(story.endnotes.len(), 1);
        assert_eq!(story.endnotes[0].text.text, "Endnote body");
        assert_eq!(source_optical_kerning(&fixture.doc, story, None, &opts), OpticalKerningUse::Unresolved);
        assert_eq!(fixture.doc, before);
        let parent = fixture.doc.story_mut(fixture.story).unwrap();
        let body = &mut std::sync::Arc::make_mut(parent.endnotes.first_mut().unwrap()).text;
        body.format_chars(0..body.len(), |f| f.over.kerning = Some(designcraft_doc::Kerning::Metrics));
        let story = fixture.doc.story(fixture.story).unwrap();
        assert_eq!(source_optical_kerning(&fixture.doc, story, None, &opts), OpticalKerningUse::NotUsed);
    }
}

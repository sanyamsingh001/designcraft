//! Deterministic work and dependency oracles for the private script-usage memo.
use std::sync::atomic::Ordering;

use designcraft_doc::build::NewDocument;
use designcraft_doc::{CharAttrs, CharacterStyle, Composer, GrepStyle, ParaAttrs, ParaFormat, Position, SpreadRef};
use designcraft_geom::Rect;

use super::*;

fn work(cache: &Cache) -> usize {
    cache.script_mask_resolutions.load(Ordering::Relaxed)
}

fn fixture(text: &str) -> (Document, StoryId) {
    let mut doc = Document::new(&NewDocument { facing_pages: false, ..Default::default() });
    let para = ParaFormat {
        para: ParaAttrs { composer: Some(Composer::SingleLine), hyphenate: Some(false), ..Default::default() },
        chars: CharAttrs { font_family: Some("Source Serif 4".into()), font_style: Some("Regular".into()), ..Default::default() },
        ..Default::default()
    };
    let layer = doc.default_layer();
    let (_, sid) = doc.add_text_frame(SpreadRef::Doc(0), Rect::new(36.0, 36.0, 576.0, 500.0), layer, text, para).unwrap();
    (doc, sid)
}

fn grep_style(doc: &mut Document, sid: StoryId) {
    doc.styles_mut().character.push(CharacterStyle {
        name: "R34 Ink".into(),
        based_on: None,
        chars: CharAttrs { fill: Some("Black".into()), ..Default::default() },
        shortcut: String::new(),
    });
    let story = doc.story_mut(sid).unwrap();
    story.format_paras(0..story.len(), |para| {
        para.para.grep_styles = Some(vec![GrepStyle { style: "R34 Ink".into(), pattern: ".".into() }]);
    });
}

fn script_metrics(composed: &ComposedStory) -> (f64, f64) {
    let glyphs: Vec<_> = composed.frames.iter().flat_map(|frame| &frame.lines).flat_map(|line| &line.glyphs).collect();
    let base = glyphs.iter().find(|glyph| glyph.byte == 0 && glyph.visible).unwrap();
    let script = glyphs.iter().find(|glyph| glyph.byte == 1 && glyph.visible).unwrap();
    assert!(base.sy > 0.0 && script.sy > 0.0);
    (script.sy / base.sy, base.y - script.y)
}

fn used(cache: &Cache, doc: &Document, sid: StoryId) -> [bool; 2] {
    let story = doc.stories.get(&sid).unwrap();
    let inputs = cache.advanced_type_inputs(doc, story);
    assert_eq!(inputs, advanced_type_inputs(doc, story), "memo preserves the public uncached helper's values");
    [inputs[0].is_some(), inputs[2].is_some()]
}

#[test]
fn unchanged_dense_grep_cache_hits_and_document_signatures_resolve_once() {
    let (mut doc, sid) = fixture(&"M".repeat(256));
    grep_style(&mut doc, sid);
    assert_eq!(doc.story(sid).unwrap().chars.len(), 1, "one source run split by dense GREP matches");
    let cache = Cache::new();
    let composed = cache.get(&doc, sid, None);
    assert_eq!(work(&cache), 1, "the first lookup must resolve the source mask");
    assert_eq!(used(&cache, &doc, sid), [false, false]);
    let signature = doc_signature(&cache, &doc);
    let running = cache.running_index(&doc);
    let xrefs = cache.xref_index(&doc);
    for _ in 0..4 {
        assert!(Arc::ptr_eq(&composed, &cache.get(&doc, sid, None)));
        assert_eq!(signature, doc_signature(&cache, &doc));
        assert!(Arc::ptr_eq(&running, &cache.running_index(&doc)));
        assert!(Arc::ptr_eq(&xrefs, &cache.xref_index(&doc)));
        assert_eq!(work(&cache), 1, "warm navigation and reference signatures must not resolve overlays again");
    }
    let mut unused = doc.clone();
    unused.settings.advanced_type.superscript_size = 91.0;
    unused.settings.advanced_type.superscript_position = 31.0;
    unused.settings.advanced_type.subscript_size = 87.0;
    unused.settings.advanced_type.subscript_position = 27.0;
    assert!(Arc::ptr_eq(&composed, &cache.get(&unused, sid, None)));
    assert_eq!(signature, doc_signature(&cache, &unused));
    assert_eq!(work(&cache), 1, "unused numeric settings neither invalidate nor recompute the mask");
}

#[test]
fn signature_first_running_and_xref_builders_reenter_get_without_resolving_again() {
    use designcraft_doc::vars::{TextVariable, Use, VarKind, var_char};
    let (mut doc, source) = fixture("M2");
    let target = doc.story_mut(source).unwrap().insert_anchor(2, "R34 destination");
    let index = doc.text_variables.len();
    let style = doc.story(source).unwrap().paras[0].style.clone();
    doc.text_variables
        .push(TextVariable::new("R34 header", VarKind::RunningHeader { style: style.clone(), use_: Use::FirstOnPage, character: false }));
    let layer = doc.default_layer();
    let (_, consumer) = doc
        .add_text_frame(SpreadRef::Doc(0), Rect::new(36.0, 520.0, 576.0, 700.0), layer, &var_char(index).unwrap().to_string(), ParaFormat::default())
        .unwrap();
    let story = doc.story_mut(consumer).unwrap();
    story.insert_xref(story.len(), designcraft_doc::xref::CrossRef { target, format: "Page Number".into() });
    doc.check().unwrap();
    let cache = Cache::new();
    let signature = doc_signature(&cache, &doc);
    assert_eq!(work(&cache), 2, "the cold document signature resolves both actual story allocations");
    let composed = cache.get(&doc, consumer, None);
    assert!(!composed.is_overset());
    let running = cache.running_index(&doc);
    assert_eq!(running.value(&style, false, Use::FirstOnPage, 0), "M2");
    let xrefs = cache.xref_index(&doc);
    assert_eq!(xrefs.page(target), Some(doc.page_name(0).as_str()));
    for _ in 0..3 {
        assert!(Arc::ptr_eq(&composed, &cache.get(&doc, consumer, None)));
        assert!(Arc::ptr_eq(&running, &cache.running_index(&doc)));
        assert!(Arc::ptr_eq(&xrefs, &cache.xref_index(&doc)));
        assert_eq!(signature, doc_signature(&cache, &doc));
        assert_eq!(work(&cache), 2, "both index builders and their composition callbacks share the masks");
    }
}

#[test]
fn story_and_style_changes_refresh_masks_even_for_the_same_story_id() {
    let (mut doc, sid) = fixture("M2");
    grep_style(&mut doc, sid);
    let cache = Cache::new();
    let original = cache.get(&doc, sid, None);
    assert_eq!(used(&cache, &doc, sid), [false, false]);
    assert_eq!(work(&cache), 1);

    let mut edited = doc.clone();
    edited.story_mut(sid).unwrap().format_chars(1..2, |format| format.over.position = Some(Position::Superscript));
    assert_ne!(edited.story(sid).unwrap().rev, doc.story(sid).unwrap().rev);
    assert!(!Arc::ptr_eq(&original, &cache.get(&edited, sid, None)));
    assert_eq!(used(&cache, &edited, sid), [true, false]);
    assert_eq!(work(&cache), 2);

    let mut styled = doc.clone();
    styled.styles_mut().char_style_mut("R34 Ink").unwrap().chars.position = Some(Position::Subscript);
    assert!(Arc::ptr_eq(&doc.stories[&sid], &styled.stories[&sid]), "only the style allocation changes");
    assert!(!Arc::ptr_eq(&original, &cache.get(&styled, sid, None)));
    assert_eq!(used(&cache, &styled, sid), [false, true]);
    assert_eq!(work(&cache), 3);
    let signature = doc_signature(&cache, &styled);
    assert_eq!(signature, doc_signature(&cache, &styled));
    assert_eq!(work(&cache), 3);
    assert_eq!(used(&cache, &doc, sid), [false, false], "the original snapshot retains its own mask");
    assert_eq!(work(&cache), 3);
}

#[test]
fn child_edits_without_outer_revision_changes_and_reference_position_refresh_masks() {
    let (mut doc, sid) = fixture("M");
    doc.footnote_options.ref_position = Position::Normal;
    let story = doc.story_mut(sid).unwrap();
    story.insert_note(1, "N2", ParaFormat::default());
    let mut table = designcraft_doc::table::Table::new(77, 1, 1, 0, 0, 120.0);
    table.cell_mut(0, 0).unwrap().text = Story::with_text(StoryId(0), "C2", ParaFormat::default());
    story.insert_table(story.len(), table);
    let revision = story.rev;
    doc.check().unwrap();
    let cache = Cache::new();
    cache.get(&doc, sid, None);
    assert_eq!(used(&cache, &doc, sid), [false, false]);
    assert_eq!(work(&cache), 1);

    let mut note = doc.clone();
    let outer = note.story_mut(sid).unwrap();
    Arc::make_mut(&mut outer.notes[0]).text.format_chars(1..2, |format| format.over.position = Some(Position::Superscript));
    assert_eq!(outer.rev, revision, "child edits need not advance the outer revision");
    assert!(!Arc::ptr_eq(&doc.stories[&sid], &note.stories[&sid]));
    cache.get(&note, sid, None);
    assert_eq!(used(&cache, &note, sid), [true, false]);
    assert_eq!(work(&cache), 2);

    let mut cell = doc.clone();
    let outer = cell.story_mut(sid).unwrap();
    Arc::make_mut(outer.tables.get_mut(&77).unwrap()).cell_mut(0, 0).unwrap().text.format_chars(1..2, |format| {
        format.over.position = Some(Position::Subscript);
    });
    assert_eq!(outer.rev, revision);
    assert!(!Arc::ptr_eq(&doc.stories[&sid], &cell.stories[&sid]));
    cache.get(&cell, sid, None);
    assert_eq!(used(&cache, &cell, sid), [false, true]);
    assert_eq!(work(&cache), 3);

    let mut reference = doc.clone();
    reference.footnote_options.ref_position = Position::Superscript;
    assert!(Arc::ptr_eq(&doc.stories[&sid], &reference.stories[&sid]));
    assert!(Arc::ptr_eq(&doc.styles, &reference.styles));
    cache.get(&reference, sid, None);
    assert_eq!(used(&cache, &reference, sid), [true, false]);
    assert_eq!(work(&cache), 4);
    doc_signature(&cache, &reference);
    assert_eq!(work(&cache), 4);
    assert_eq!(used(&cache, &doc, sid), [false, false]);
    assert_eq!(work(&cache), 4, "restoring the reference position reuses the retained original mask");
}

#[test]
fn consumed_numeric_changes_invalidate_composition_and_reference_signatures_without_resolving_masks() {
    for position in [Position::Superscript, Position::Subscript] {
        let (mut doc, sid) = fixture("M2");
        doc.story_mut(sid).unwrap().format_chars(1..2, |format| format.over.position = Some(position));
        doc.settings.advanced_type.superscript_size = 50.0;
        doc.settings.advanced_type.superscript_position = 20.0;
        doc.settings.advanced_type.subscript_size = 50.0;
        doc.settings.advanced_type.subscript_position = 20.0;
        let cache = Cache::new();
        let original = cache.get(&doc, sid, None);
        let signature = doc_signature(&cache, &doc);
        let running = cache.running_index(&doc);
        let xrefs = cache.xref_index(&doc);
        let old_metrics = script_metrics(&original);
        assert_eq!(work(&cache), 1);
        for size in [true, false] {
            let mut changed = doc.clone();
            match (position, size) {
                (Position::Superscript, true) => changed.settings.advanced_type.superscript_size = 100.0,
                (Position::Superscript, false) => changed.settings.advanced_type.superscript_position = 40.0,
                (Position::Subscript, true) => changed.settings.advanced_type.subscript_size = 100.0,
                (Position::Subscript, false) => changed.settings.advanced_type.subscript_position = 40.0,
                _ => panic!("fixture requires synthesized superscript or subscript"),
            }
            assert!(Arc::ptr_eq(&doc.stories[&sid], &changed.stories[&sid]));
            assert!(Arc::ptr_eq(&doc.styles, &changed.styles));
            let current = cache.get(&changed, sid, None);
            assert!(!Arc::ptr_eq(&original, &current));
            assert_ne!(signature, doc_signature(&cache, &changed));
            let new_metrics = script_metrics(&current);
            if size {
                assert_ne!(old_metrics.0, new_metrics.0, "fresh numeric size reaches actual glyph metrics");
            } else {
                assert_ne!(old_metrics.1, new_metrics.1, "fresh numeric shift reaches actual relative glyph placement");
            }
            let fresh = crate::compose_story(&changed, sid, &ComposeOptions::default());
            assert_eq!(format!("{current:?}"), format!("{fresh:?}"), "warm and fresh compositions agree");
            assert!(Arc::ptr_eq(&current, &cache.get(&changed, sid, None)));
            assert!(!Arc::ptr_eq(&running, &cache.running_index(&changed)));
            assert!(!Arc::ptr_eq(&xrefs, &cache.xref_index(&changed)));
            assert_eq!(work(&cache), 1, "numeric controls project through the retained mask, including signature paths");
        }
    }
}

#[test]
fn weak_memo_pins_detach_unique_inputs_before_mutation() {
    let (mut doc, sid) = fixture("M2");
    grep_style(&mut doc, sid);
    doc.footnote_options.ref_position = Position::Normal;
    doc.story_mut(sid).unwrap().insert_note(2, "N2", ParaFormat::default());
    let revision = doc.story(sid).unwrap().rev;
    let story_address = Arc::as_ptr(&doc.stories[&sid]) as usize;
    let styles_address = Arc::as_ptr(&doc.styles) as usize;
    let cache = Cache::new();
    assert_eq!(used(&cache, &doc, sid), [false, false]);
    assert_eq!(Arc::strong_count(&doc.stories[&sid]), 1, "the memo adds no strong source owner");
    assert_eq!(Arc::strong_count(&doc.styles), 1, "the memo adds no strong styles owner");
    assert!(Arc::get_mut(doc.stories.get_mut(&sid).unwrap()).is_none(), "the memo alone prevents in-place source mutation");
    assert!(Arc::get_mut(&mut doc.styles).is_none(), "the memo alone prevents in-place style mutation");

    let outer = doc.story_mut(sid).unwrap();
    Arc::make_mut(&mut outer.notes[0]).text.format_chars(1..2, |format| format.over.position = Some(Position::Superscript));
    assert_eq!(outer.rev, revision, "only the nested note's revision changes");
    assert_ne!(Arc::as_ptr(&doc.stories[&sid]) as usize, story_address, "make_mut detaches the weak-pinned outer allocation");
    assert_eq!(used(&cache, &doc, sid), [true, false]);
    assert_eq!(work(&cache), 2);

    doc.styles_mut().char_style_mut("R34 Ink").unwrap().chars.position = Some(Position::Subscript);
    assert_ne!(Arc::as_ptr(&doc.styles) as usize, styles_address, "make_mut also detaches weak-pinned styles");
    assert_eq!(used(&cache, &doc, sid), [true, true]);
    assert_eq!(work(&cache), 3);
}

#[test]
fn obsolete_source_and_style_data_drop_while_memo_pins_remain() {
    let (mut doc, sid) = fixture("M2");
    grep_style(&mut doc, sid);
    doc.footnote_options.ref_position = Position::Normal;
    doc.story_mut(sid).unwrap().insert_note(2, "obsolete note data", ParaFormat::default());
    let story_data = Arc::downgrade(&doc.stories[&sid]);
    let style_data = Arc::downgrade(&doc.styles);
    let note_data = Arc::downgrade(&doc.stories[&sid].notes[0]);
    let cache = Cache::new();
    doc_signature(&cache, &doc);
    assert_eq!(work(&cache), 1);
    drop(doc);

    // Dropping the last strong owner runs Story/Styles destructors, including their Vec/String data.
    assert_eq!(story_data.strong_count(), 0);
    assert_eq!(style_data.strong_count(), 0);
    assert!(story_data.upgrade().is_none() && style_data.upgrade().is_none());
    assert!(note_data.upgrade().is_none(), "the source's owned child data is released without evicting the memo");
    let masks = cache.script_masks.lock().unwrap();
    assert_eq!(masks.entries.len(), 1, "the discarded version is still indexed");
    assert_eq!(masks.order.len(), 1);
    let entry = masks.entries.values().next().unwrap();
    assert!(Weak::ptr_eq(&entry._keep.0, &story_data), "the memo still pins the original source allocation");
    assert!(Weak::ptr_eq(&entry._keep.1, &style_data), "the memo still pins the original styles allocation");
}

#[test]
fn memo_bounds_distinct_allocations_and_releases_weak_pins_on_eviction_and_clear() {
    let mut doc = Document::new(&NewDocument::default());
    let cache = Cache::new();
    let first_style = doc.styles.clone();
    let first_story = Arc::new(Story::with_text(StoryId(1), "M", ParaFormat::default()));
    cache.advanced_type_inputs(&doc, &first_story);
    cache.advanced_type_inputs(&doc, &first_story);
    assert_eq!(work(&cache), 1, "a live source/style snapshot reuses its mask");
    assert_eq!(Arc::weak_count(&first_story), 1);
    assert_eq!(Arc::weak_count(&first_style), 1);
    doc.styles = Arc::new(Styles::default());
    for _ in 1..SCRIPT_MASK_LIMIT {
        let story = Arc::new(Story::with_text(StoryId(1), "M", ParaFormat::default()));
        cache.advanced_type_inputs(&doc, &story);
    }
    assert_eq!(work(&cache), SCRIPT_MASK_LIMIT, "equal IDs and revisions do not alias distinct weak-pinned allocations");
    assert_eq!(Arc::weak_count(&first_story), 1, "the first entry is still within the FIFO bound");
    assert_eq!(Arc::weak_count(&first_style), 1);
    let last_story = Arc::new(Story::with_text(StoryId(1), "M", ParaFormat::default()));
    cache.advanced_type_inputs(&doc, &last_story);
    assert_eq!(Arc::weak_count(&first_story), 0, "FIFO eviction releases the source allocation pin");
    assert_eq!(Arc::weak_count(&first_style), 0, "FIFO eviction releases the styles allocation pin");
    assert_eq!(Arc::weak_count(&last_story), 1);
    assert_eq!(Arc::weak_count(&doc.styles), SCRIPT_MASK_LIMIT);
    {
        let masks = cache.script_masks.lock().unwrap();
        assert_eq!(masks.entries.len(), SCRIPT_MASK_LIMIT);
        assert_eq!(masks.order.len(), SCRIPT_MASK_LIMIT);
    }
    cache.clear();
    assert_eq!(Arc::weak_count(&last_story), 0, "clear releases the remaining source pins");
    assert_eq!(Arc::weak_count(&doc.styles), 0, "clear releases the remaining styles pins");
    {
        let masks = cache.script_masks.lock().unwrap();
        assert!(masks.entries.is_empty() && masks.order.is_empty());
    }
    cache.advanced_type_inputs(&doc, &last_story);
    assert_eq!(work(&cache), SCRIPT_MASK_LIMIT + 2, "the lifetime work counter exposes the next cold resolution after clear");
}

#[test]
fn r35_projection_reuses_render_memo_across_unrelated_unit_edits() {
    let (mut doc, sid) = fixture(&"M2 body text.\n".repeat(16));
    grep_style(&mut doc, sid);
    doc.styles_mut().char_style_mut("R34 Ink").unwrap().chars.position = Some(Position::Superscript);
    let cache = Cache::new();
    let composed = cache.get(&doc, sid, None);
    let inputs = cache.advanced_type_inputs(&doc, &doc.stories[&sid]).map(|value| value.map(f64::to_bits));
    assert!(inputs[0].is_some() && inputs[1].is_some());
    assert_eq!(&inputs[2..], &[None, None]);
    assert_eq!(work(&cache), 1);

    for units in [designcraft_geom::Unit::Millimeters, designcraft_geom::Unit::Inches, designcraft_geom::Unit::Points] {
        let mut edited = doc.clone();
        edited.settings.horizontal_units = units;
        assert!(Arc::ptr_eq(&doc.stories[&sid], &edited.stories[&sid]));
        assert!(Arc::ptr_eq(&doc.styles, &edited.styles));
        for _ in 0..4 {
            for snapshot in [&doc, &edited] {
                assert_eq!(cache.advanced_type_inputs(snapshot, &snapshot.stories[&sid]).map(|value| value.map(f64::to_bits)), inputs);
                assert!(Arc::ptr_eq(&composed, &cache.get(snapshot, sid, None)));
            }
            assert_eq!(work(&cache), 1, "old/new projections share the render memo without another GREP/run scan");
        }
    }

    for position in [f64::from_bits(doc.settings.advanced_type.superscript_position.to_bits() ^ 1), -0.0] {
        let mut edited = doc.clone();
        edited.settings.advanced_type.superscript_position = position;
        edited.settings.advanced_type.subscript_position = 99.0;
        let projected = cache.advanced_type_inputs(&edited, &edited.stories[&sid]);
        assert_eq!(projected[1].unwrap().to_bits(), position.to_bits(), "the memo projects every current numeric bit");
        assert_eq!(&projected[2..], &[None, None], "unused subscript controls remain absent");
        assert_eq!(work(&cache), 1, "numeric projection does not rescan script usage");
    }
    cache.clear();
    cache.advanced_type_inputs(&doc, &doc.stories[&sid]);
    assert_eq!(work(&cache), 2, "clearing the shared memo must require a new scan");
}

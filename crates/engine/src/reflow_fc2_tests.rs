//! fc2 compatibility: warm reference banks, section edits and parity-break allocation.
use super::*;

const RUNNING: &str = "FC2 Running";
const CONDITION: &str = "FC2 Prefix";

fn running_values(d: &Document, bank: &Cache) -> Vec<String> {
    let vi = d.variable(RUNNING).unwrap();
    let index = bank.running_index(d);
    (0..d.page_count()).map(|page| compose::vars::values(d, Some(page), Some(&index))[vi].clone()).collect()
}

fn source_change(hyphenation: bool) -> (&'static str, Value) {
    if hyphenation {
        ("hyphenation.addException", json!({"word": "rep~re~sen~ta~tion"}))
    } else {
        ("condition.options", json!({"name": CONDITION, "visible": false}))
    }
}

/// Derive a page crossing from independent composition, not cache invalidation or assumed metrics.
fn crossing_source(hyphenation: bool) -> (Session, StoryId, StoryId, u64) {
    let face = designcraft_fonts::FontDb::global().face("Source Serif 4", "Regular");
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Source Serif 4", "Regular"));
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 2, "width": 612, "height": 792, "margins": 36, "facingPages": false})).unwrap();
    s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
    let prefix = "representation ".repeat(12);
    let start = prefix.len() + 1;
    let made =
        s.execute("frame.create", &json!({"spread": 0, "rect": [36,36,576,636], "content": "text", "text": format!("{prefix}\nHead")})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    let first = ItemId(made["id"].as_u64().unwrap());
    select_all(&mut s, sid);
    s.execute(
        "type.char",
        &json!({"attrs": {
            "fontFamily": "Source Serif 4", "fontStyle": "Regular", "size": 12,
            "leading": {"kind": "points", "value": 14}, "position": "normal"
        }}),
    )
    .unwrap();
    s.execute(
        "type.para",
        &json!({"attrs": {
            "composer": "singleLine", "hyphenate": hyphenation, "hyphLastWord": true,
            "keepLinesTogether": false, "keepFirst": 1, "keepLast": 1,
            "spaceBefore": 0, "spaceAfter": 0, "gridAlign": "none"
        }}),
    )
    .unwrap();
    if hyphenation {
        s.execute("hyphenation.addException", &json!({"word": "representation"})).unwrap();
    } else {
        s.execute("condition.new", &json!({"name": CONDITION})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": prefix.len()})).unwrap();
        s.execute("condition.apply", &json!({"name": CONDITION})).unwrap();
    }
    let (command, params) = source_change(hyphenation);
    let mut measure = None;
    for width in [100.0, 120.0, 140.0, 160.0, 180.0] {
        r9_resize(&mut s, first, width, 600.0);
        let old = fresh(&doc(&s), sid);
        let changed = probe(&mut s, command, params.clone());
        let new = fresh(&changed, sid);
        assert!(!old.is_overset() && !new.is_overset());
        if new.line_count() < old.line_count() {
            let bottom = |cs: &ComposedStory| {
                let line = cs.frames[0].lines.last().unwrap();
                line.baseline + line.descent - cs.frames[0].columns[0].y0
            };
            measure = Some((width, (bottom(&old) + bottom(&new)) / 2.0));
            break;
        }
    }
    let (width, height) = measure.expect("fixture requires an actual source line-count transition");
    r9_resize(&mut s, first, width, height);
    let spread = doc(&s).page_loc(1).unwrap().0;
    let second = s
        .execute(
            "frame.create",
            &json!({
                "spread": spread, "rect": [36,36,36.0+width,636], "content": "text", "text": "", "caret": false
            }),
        )
        .unwrap();
    // Public fixture construction: there is no text.thread command.
    s.edit(|d, _| {
        d.thread(first, ItemId(second["id"].as_u64().unwrap()))?;
        Ok(())
    })
    .unwrap();
    s.execute("style.paragraph.create", &json!({"name": "FC2 Heading"})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": start, "focus": start+4})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "FC2 Heading"})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": start, "focus": start})).unwrap();
    let anchor = s.execute("anchor.create", &json!({"name": "FC2 Anchor"})).unwrap()["id"].as_u64().unwrap();
    s.execute(
        "variables.define",
        &json!({
            "name": RUNNING, "type": "runningHeader", "style": "FC2 Heading", "use": "firstOnPage"
        }),
    )
    .unwrap();
    s.execute("xref.defineFormat", &json!({"name": "FC2 Page", "definition": "<pageNum />"})).unwrap();
    let made = s.execute("frame.create", &json!({"spread": 0, "rect": [300,100,570,200], "content": "text", "text": ""})).unwrap();
    let consumer = StoryId(made["story"].as_u64().unwrap());
    s.execute("variables.insert", &json!({"name": RUNNING})).unwrap();
    s.execute("text.insert", &json!({"text": " "})).unwrap();
    s.execute("xref.insert", &json!({"anchor": anchor, "format": "FC2 Page"})).unwrap();
    let before = doc(&s);
    let after = probe(&mut s, command, params);
    r9_shared_sources(&before, &after, true);
    assert_eq!(r9_source_page(&before, sid, anchor), 1);
    assert_eq!(r9_source_page(&after, sid, anchor), 0);
    assert_eq!(running_values(&before, &Cache::new()), vec!["", "Head"]);
    assert_eq!(running_values(&after, &Cache::new()), vec!["Head", "Head"]);
    (s, sid, consumer, anchor)
}

#[test]
fn fc2_warm_reference_banks_follow_visibility_and_hyphenation_source_crossings() {
    for hyphenation in [false, true] {
        for commit in [false, true] {
            let (mut s, sid, consumer, anchor) = crossing_source(hyphenation);
            let before = doc(&s);
            let xrefs = s.cache.xref_index(&before);
            let running = s.cache.running_index(&before);
            same_composition(&fresh(&before, consumer), &s.cache.get(&before, consumer, None));
            same_composition(&fresh(&before, sid), &s.cache.get(&before, sid, None));
            let count = s.doc().unwrap().history.undo.len();
            let journal = s.journal.len();
            let (command, params) = source_change(hyphenation);
            apply(&mut s, command, params, false, commit);
            let after = doc(&s);
            r9_shared_sources(&before, &after, true);
            assert_eq!(r9_source_page(&after, sid, anchor), 0);
            assert!(!Arc::ptr_eq(&xrefs, &s.cache.xref_index(&after)));
            assert!(!Arc::ptr_eq(&running, &s.cache.running_index(&after)));
            assert_eq!(s.cache.xref_index(&after).page_index(anchor), Some(0));
            assert_eq!(running_values(&after, &s.cache), running_values(&after, &Cache::new()));
            same_composition(&fresh(&after, consumer), &s.cache.get(&after, consumer, None));
            same_composition(&fresh(&after, sid), &s.cache.get(&after, sid, None));
            assert_eq!(s.execute("xref.list", &json!({})).unwrap()[0]["text"], after.page_name(0));
            assert_eq!(s.execute("anchor.list", &json!({})).unwrap()[0]["page"], after.page_name(0));
            assert_eq!(s.journal.len(), journal + 1);
            history(&mut s, sid, &before, &after, count);
            public_native_controls(&mut s, consumer);
        }
    }
}

#[test]
fn fc2_same_count_section_edit_updates_warm_labels_and_page_markers() {
    for commit in [false, true] {
        let (mut s, sid, consumer, anchor) = crossing_source(false);
        s.execute("layout.section", &json!({"page": 1, "startNumber": 1})).unwrap();
        let made = s
            .execute(
                "frame.create",
                &json!({
                    "spread": 0, "rect": [300,250,570,300], "content": "text",
                    "text": format!("Page {}", designcraft_doc::story::PAGE_NUMBER)
                }),
            )
            .unwrap();
        let marker = StoryId(made["story"].as_u64().unwrap());
        let before = doc(&s);
        let xrefs = s.cache.xref_index(&before);
        let running = s.cache.running_index(&before);
        let old_marker = s.cache.get(&before, marker, None);
        let old_consumer = s.cache.get(&before, consumer, None);
        let count = s.doc().unwrap().history.undo.len();
        apply(&mut s, "layout.section", json!({"page": 1, "startNumber": 10}), false, commit);
        let after = doc(&s);
        assert_eq!(before.sections.len(), after.sections.len());
        r9_shared_sources(&before, &after, true);
        assert_eq!(r9_source_page(&after, sid, anchor), 1);
        assert!(!Arc::ptr_eq(&xrefs, &s.cache.xref_index(&after)));
        assert!(!Arc::ptr_eq(&running, &s.cache.running_index(&after)));
        assert_eq!(s.execute("xref.list", &json!({})).unwrap()[0]["text"], "11");
        assert_eq!(s.execute("anchor.list", &json!({})).unwrap()[0]["page"], "11");
        assert_ne!(format!("{old_marker:?}"), format!("{:?}", fresh(&after, marker)));
        assert_ne!(format!("{old_consumer:?}"), format!("{:?}", fresh(&after, consumer)));
        same_composition(&fresh(&after, marker), &s.cache.get(&after, marker, None));
        same_composition(&fresh(&after, consumer), &s.cache.get(&after, consumer, None));
        history(&mut s, marker, &before, &after, count);
        public_native_controls(&mut s, consumer);
    }
}

#[test]
fn fc2_display_and_unrelated_text_edits_preserve_explicit_primary_removal() {
    for commit in [false, true] {
        let (mut s, sid, removed) = header_case(true, false);
        s.execute("condition.new", &json!({"name": CONDITION})).unwrap();
        let made = s
            .execute(
                "frame.create",
                &json!({
                    "spread": 0, "rect": [350,250,570,300], "content": "text", "text": "Unrelated"
                }),
            )
            .unwrap();
        let other = StoryId(made["story"].as_u64().unwrap());
        for (command, params) in [
            ("condition.options", json!({"name": CONDITION, "visible": true, "color": [10,20,30]})),
            ("document.preferences", json!({"baselineGrid": {"color": [10,20,30], "viewThreshold": 0.5}})),
            ("text.insert", json!({"text": "Unrelated replacement"})),
        ] {
            select_all(&mut s, other);
            let before = doc(&s);
            let expected = probe(&mut s, command, params.clone());
            let xrefs = s.cache.xref_index(&before);
            let running = s.cache.running_index(&before);
            same_composition(&fresh(&before, sid), &s.cache.get(&before, sid, None));
            same_composition(&fresh(&before, sid), &fresh(&expected, sid));
            let count = s.doc().unwrap().history.undo.len();
            apply(&mut s, command, params, false, commit);
            if command != "text.insert" {
                assert!(Arc::ptr_eq(&xrefs, &s.cache.xref_index(&doc(&s))));
                assert!(Arc::ptr_eq(&running, &s.cache.running_index(&doc(&s))));
            }
            preserved(&mut s, sid, removed, &before, &expected, count);
        }
    }
}

#[test]
fn fc2_odd_even_breaks_bound_primary_page_allocation() {
    use designcraft_doc::story::{EVEN_PAGE_BREAK, ODD_PAGE_BREAK};
    for (br, pages) in [(EVEN_PAGE_BREAK, 2), (ODD_PAGE_BREAK, 3)] {
        let mut s = Session::new();
        s.execute(
            "file.new",
            &json!({
                "pages": 1, "width": 612, "height": 792, "margins": 36,
                "primaryTextFrame": true, "facingPages": false
            }),
        )
        .unwrap();
        s.execute("prefs.set", &json!({"smartTextReflow": true})).unwrap();
        let sid = doc(&s).settings.primary_story.unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        let before = doc(&s);
        let count = s.doc().unwrap().history.undo.len();
        let text = format!("a{br}\n{}", WORDS.repeat(4));
        s.execute("text.insert", &json!({"text": text})).unwrap();
        let after = doc(&s);
        assert_eq!(after.story(sid).unwrap().text, text);
        assert_eq!(after.page_count(), pages, "an early parity break is not a full-frame capacity sample");
        assert_eq!(after.story(sid).unwrap().frames.len(), pages);
        let cs = fresh(&after, sid);
        assert!(!cs.is_overset());
        let (fi, ..) = compose::caret(&cs, 1 + br.len_utf8() + 1).unwrap();
        assert_eq!(after.page_of_item(cs.frames[fi].frame), Some(pages - 1));
        history(&mut s, sid, &before, &after, count);
    }
}

// An unused condition definition changes no consumed text, even when it starts hidden.
fn unused_condition_preserves_explicit_primary_page_removal(visible: bool) {
    let (mut s, sid) = primary_chain(false);
    assert!(s.prefs.smart_text_reflow);
    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    assert!(before.conditions.is_empty(), "fixture: the primary story has no applied condition");
    let name = "Unused";
    let color = [10, 20, 30];
    let mut expected = (*before).clone();
    expected.conditions.push(designcraft_doc::Condition { name: name.to_string(), color, visible });
    let expected = Arc::new(expected);
    same_composition(&fresh(&before, sid), &fresh(&expected, sid));
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("condition.new", &json!({"name": name, "color": color, "visible": visible})).unwrap();
    assert_eq!(
        doc(&s).page_count(),
        before.page_count(),
        "an unused condition must not recreate intentionally removed primary pages (visible={visible})"
    );
    assert_eq!(s.journal.len(), journal + 1, "one condition edit produces one journal entry");
    preserved(&mut s, sid, removed, &before, &expected, count);
}

#[test]
fn unused_hidden_condition_preserves_explicit_primary_page_removal() {
    unused_condition_preserves_explicit_primary_page_removal(false);
}

#[test]
fn unused_visible_condition_preserves_explicit_primary_page_removal() {
    unused_condition_preserves_explicit_primary_page_removal(true);
}

// A used condition in the overset tail must still resume primary growth when its visibility changes.
#[test]
fn used_overset_condition_visibility_resumes_primary_growth() {
    let (mut s, sid) = primary_chain(false);
    assert!(s.prefs.smart_text_reflow);
    let name = "Used overset tail";
    let color = [10, 20, 30];
    s.execute("condition.new", &json!({"name": name, "color": color, "visible": true})).unwrap();
    let fitted = doc(&s);
    let composed = fresh(&fitted, sid);
    let last = *fitted.story(sid).unwrap().frames.last().unwrap();
    let range = &composed.frame(last).unwrap().range;
    assert!(range.end - range.start > 4 * WORDS.len(), "fixture: the last page has room for unconditioned text and a short tail");
    let end = range.start + 2 * WORDS.len();
    let tail = "CONDITIONAL TAIL";
    let start = end + 1;
    // Shorten the occupied last-page suffix rather than depending on spare capacity for an appended paragraph.
    s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": fitted.story(sid).unwrap().len()})).unwrap();
    s.execute("text.insert", &json!({"text": format!("\n{tail}")})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": start, "focus": start + tail.len()})).unwrap();
    s.execute("condition.apply", &json!({"name": name, "only": true})).unwrap();
    let visible_tail = |cs: &ComposedStory| {
        cs.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).any(|g| g.visible && (start..start + tail.len()).contains(&g.byte))
    };
    let shown = fresh(&doc(&s), sid);
    assert!(!shown.is_overset(), "fixture: the used tail fits before removing capacity");
    assert!(visible_tail(&shown), "fixture: the applied visible condition really draws text");
    s.execute("condition.options", &json!({"name": name, "visible": false})).unwrap();
    let hidden = fresh(&doc(&s), sid);
    assert!(!hidden.is_overset());
    assert!(!visible_tail(&hidden), "fixture: the same applied condition really hides text");

    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    let old_story = before.stories.get(&sid).unwrap();
    let reduced = fresh(&before, sid);
    assert!(reduced.overset_at.is_some_and(|at| at < end), "fixture: unconditioned text oversets before the separate tail paragraph");
    let (_, base_chars) = before.styles.resolve_para(&old_story.paras[old_story.para_at(start)]);
    let props = before.styles.resolve_char(&base_chars, old_story.format_after(start));
    assert_eq!(props.conditions, vec![name.to_string()], "fixture: exactly this condition controls the tail");
    assert_eq!(before.conditions, vec![designcraft_doc::Condition { name: name.to_string(), color, visible: false }]);
    same_composition(&reduced, &s.cache.get(&before, sid, None));

    // Independently specify the definition-only delta; preview is a public-command probe, not the oracle.
    let mut expected = (*before).clone();
    expected.conditions[0].visible = true;
    let params = json!({"name": name, "visible": true});
    let toggled = probe(&mut s, "condition.options", params.clone());
    assert_eq!(*toggled, expected, "the toggle itself changes only the condition definition");
    assert_eq!(full_doc(&toggled), full_doc(&expected));
    assert!(Arc::ptr_eq(old_story, toggled.stories.get(&sid).unwrap()));
    assert!(Arc::ptr_eq(&before.styles, &toggled.styles));
    let still_overset = fresh(&toggled, sid);
    assert!(still_overset.is_overset(), "revealing the tail cannot create capacity in the reduced chain");
    same_composition(&reduced, &still_overset);

    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("condition.options", &params).unwrap();
    let after = doc(&s);
    assert!(after.page_count() > before.page_count(), "a used condition toggle must resume primary-page growth");
    let added = after.page_count() - before.page_count();
    let old_frames = old_story.frames.len();
    let new_story = after.story(sid).unwrap();
    assert_eq!(new_story.frames.len(), old_frames + added);
    assert_eq!(new_story.rev, old_story.rev + added as u64, "each newly threaded empty frame advances the story revision once");
    assert_eq!(&new_story.frames[..old_frames], &old_story.frames);
    assert_eq!(after.spreads.len(), before.spreads.len() + added);
    assert!(!after.settings.facing_pages);
    for (sp, frame) in after.spreads[before.spreads.len()..].iter().zip(&new_story.frames[old_frames..]) {
        assert_eq!(sp.pages.len(), 1);
        assert_eq!(sp.items.len(), 1);
        assert_eq!(sp.items[0].id, *frame);
        assert_eq!(sp.items[0].text_frame().unwrap().story, sid);
        assert_eq!(sp.items[0].bounds(), sp.pages[0].margin_rect());
    }
    assert!(after.next_id > before.next_id);
    assert_eq!(s.journal.len(), journal + 1);
    assert!(visible_tail(&fresh(&after, sid)), "the revealed tail must really be composed after growth");

    // Ignore only allocated capacity/ids and the asserted thread revisions; repagination regenerates spread ids.
    let mut without_growth = (*after).clone();
    without_growth.spreads.truncate(before.spreads.len());
    for (sp, old) in without_growth.spreads.iter_mut().zip(&before.spreads) {
        Arc::make_mut(sp).id = old.id;
    }
    let restored_story = Arc::make_mut(without_growth.stories.get_mut(&sid).unwrap());
    restored_story.frames.truncate(old_frames);
    restored_story.rev = old_story.rev;
    without_growth.next_id = before.next_id;
    assert_eq!(without_growth, expected, "apart from appended capacity/ids/thread revisions, the full Doc changes only visibility");
    assert_eq!(full_doc(&without_growth), full_doc(&expected));
    growth(&mut s, sid, Some(removed), &before, count);
}

#[test]
fn r52_nonmatching_grep_condition_preserves_explicit_primary_page_removal() {
    let (mut s, sid) = primary_chain(false);
    assert!(s.prefs.smart_text_reflow);
    let name = "Dormant";
    let style = "Dormant GREP";
    let pattern = "ZZZ_NEVER_PRESENT";
    let color = [10, 20, 30];
    s.execute("condition.new", &json!({"name": name, "color": color, "visible": true})).unwrap();
    s.execute("style.character.create", &json!({"name": style, "chars": {"conditions": [name]}})).unwrap();
    select_all(&mut s, sid);
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": style, "pattern": pattern}]}})).unwrap();

    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    let old_story = before.stories.get(&sid).unwrap();
    assert_eq!(before.conditions, vec![designcraft_doc::Condition { name: name.to_string(), color, visible: true }]);
    assert!(
        !regex::Regex::new(pattern).unwrap().is_match(&old_story.text),
        "fixture: the GREP rule matches nowhere in the full story, including its overset tail"
    );
    for para in &old_story.paras {
        let (props, base) = before.styles.resolve_para(para);
        assert_eq!(props.grep_styles.len(), 1);
        assert_eq!((props.grep_styles[0].style.as_str(), props.grep_styles[0].pattern.as_str()), (style, pattern));
        assert!(base.conditions.is_empty(), "fixture: the paragraph does not apply the condition");
        for (_, format) in old_story.runs() {
            assert!(before.styles.resolve_char(&base, format).conditions.is_empty(), "fixture: no text run directly applies the condition");
        }
        let format = designcraft_doc::CharFormat { style: style.to_string(), ..Default::default() };
        let referenced = before.styles.resolve_char(&base, &format);
        assert_eq!(referenced.conditions, vec![name.to_string()], "fixture: only the nonmatching GREP style references this condition");
    }
    let reduced = fresh(&before, sid);
    assert!(reduced.is_overset(), "fixture: an occupied tail was removed, leaving real overset");
    same_composition(&reduced, &s.cache.get(&before, sid, None));

    // The independent oracle changes one visibility bit; preview only verifies the public command's delta.
    let mut expected = (*before).clone();
    expected.conditions[0].visible = false;
    let expected = Arc::new(expected);
    assert!(expected.conditions_hide(&[name.to_string()]));
    same_composition(&reduced, &fresh(&expected, sid));
    let params = json!({"name": name, "visible": false});
    let toggled = probe(&mut s, "condition.options", params.clone());
    assert_eq!(*toggled, *expected, "the toggle itself must change only condition visibility");
    assert_eq!(full_doc(&toggled), full_doc(&expected));
    assert!(Arc::ptr_eq(old_story, toggled.stories.get(&sid).unwrap()));
    assert!(Arc::ptr_eq(&before.styles, &toggled.styles));
    same_composition(&reduced, &fresh(&toggled, sid));

    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("condition.options", &params).unwrap();
    assert_eq!(s.journal.len(), journal + 1, "one condition edit produces one journal entry");
    assert_eq!(s.journal.last().unwrap(), &("condition.options".into(), params));
    assert_eq!(
        doc(&s).page_count(),
        before.page_count(),
        "defect oracle: a nonmatching GREP condition must not recreate intentionally removed primary pages"
    );
    preserved(&mut s, sid, removed, &before, &expected, count);
}

#[test]
fn r52_matched_overset_grep_condition_resumes_primary_growth() {
    let (mut s, sid) = primary_chain(false);
    assert!(s.prefs.smart_text_reflow);
    let name = "Used GREP tail";
    let style = "R52 GREP";
    let pattern = "R52_CONDITIONAL_GREP_TAIL";
    let color = [10, 20, 30];
    s.execute("condition.new", &json!({"name": name, "color": color, "visible": true})).unwrap();
    s.execute("style.character.create", &json!({"name": style, "chars": {"conditions": [name]}})).unwrap();

    let fitted = doc(&s);
    let composed = fresh(&fitted, sid);
    let last = *fitted.story(sid).unwrap().frames.last().unwrap();
    let range = &composed.frame(last).unwrap().range;
    assert!(range.end - range.start > 4 * WORDS.len(), "fixture: occupied last page has room for unconditioned text and the match");
    let end = range.start + 2 * WORDS.len();
    let start = end + 1;
    s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": fitted.story(sid).unwrap().len()})).unwrap();
    s.execute("text.insert", &json!({"text": format!("\n{pattern}")})).unwrap();
    select_all(&mut s, sid);
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": style, "pattern": pattern}]}})).unwrap();
    let match_range = start..start + pattern.len();
    let visible_match =
        |cs: &ComposedStory| cs.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).any(|g| g.visible && match_range.contains(&g.byte));
    let shown = fresh(&doc(&s), sid);
    assert!(!shown.is_overset(), "fixture: the actual matched text fits before capacity removal");
    assert!(visible_match(&shown), "fixture: the visible GREP condition really draws matched text");
    s.execute("condition.options", &json!({"name": name, "visible": false})).unwrap();
    let hidden = fresh(&doc(&s), sid);
    assert!(!hidden.is_overset());
    assert!(!visible_match(&hidden), "fixture: the same GREP condition really hides the match");

    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    let old_story = before.stories.get(&sid).unwrap();
    let matches: Vec<_> = regex::Regex::new(pattern).unwrap().find_iter(&old_story.text).map(|m| m.range()).collect();
    assert_eq!(matches, vec![match_range.clone()], "fixture: exactly one nonempty GREP match at the asserted tail range");
    for para in &old_story.paras {
        let (props, base) = before.styles.resolve_para(para);
        assert_eq!(props.grep_styles.len(), 1);
        assert_eq!((props.grep_styles[0].style.as_str(), props.grep_styles[0].pattern.as_str()), (style, pattern));
        assert!(base.conditions.is_empty(), "fixture: no paragraph applies the condition directly");
        for (_, format) in old_story.runs() {
            assert!(before.styles.resolve_char(&base, format).conditions.is_empty(), "fixture: no run applies the condition directly");
        }
        let format = designcraft_doc::CharFormat { style: style.to_string(), ..Default::default() };
        assert_eq!(before.styles.resolve_char(&base, &format).conditions, vec![name.to_string()]);
    }
    assert_eq!(before.conditions, vec![designcraft_doc::Condition { name: name.to_string(), color, visible: false }]);
    let reduced = fresh(&before, sid);
    assert!(
        reduced.overset_at.is_some_and(|at| at < end && at < match_range.start),
        "fixture: unconditioned text oversets before the entire actual GREP match"
    );
    same_composition(&reduced, &s.cache.get(&before, sid, None));

    // The independent expected Doc changes only visibility; the matching tail is not yet composed.
    let mut expected = (*before).clone();
    expected.conditions[0].visible = true;
    let params = json!({"name": name, "visible": true});
    let toggled = probe(&mut s, "condition.options", params.clone());
    assert_eq!(*toggled, expected, "the public toggle itself changes only condition visibility");
    assert_eq!(full_doc(&toggled), full_doc(&expected));
    assert!(Arc::ptr_eq(old_story, toggled.stories.get(&sid).unwrap()));
    assert!(Arc::ptr_eq(&before.styles, &toggled.styles));
    let still_overset = fresh(&expected, sid);
    assert!(still_overset.is_overset());
    same_composition(&reduced, &still_overset);

    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("condition.options", &params).unwrap();
    let after = doc(&s);
    assert!(after.page_count() > before.page_count(), "a condition on an actual overset GREP match must resume primary growth");
    let added = after.page_count() - before.page_count();
    let old_frames = old_story.frames.len();
    let new_story = after.story(sid).unwrap();
    assert_eq!(new_story.frames.len(), old_frames + added);
    assert_eq!(new_story.rev, old_story.rev + added as u64, "each appended primary frame advances the thread revision once");
    assert_eq!(&new_story.frames[..old_frames], &old_story.frames);
    assert_eq!(after.spreads.len(), before.spreads.len() + added);
    assert!(!after.settings.facing_pages);
    for (sp, frame) in after.spreads[before.spreads.len()..].iter().zip(&new_story.frames[old_frames..]) {
        assert_eq!(sp.pages.len(), 1);
        assert_eq!(sp.items.len(), 1);
        assert_eq!(sp.items[0].id, *frame);
        assert_eq!(sp.items[0].text_frame().unwrap().story, sid);
        assert_eq!(sp.items[0].bounds(), sp.pages[0].margin_rect());
    }
    assert!(after.next_id > before.next_id);
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last().unwrap(), &("condition.options".into(), params));
    assert!(visible_match(&fresh(&after, sid)), "the revealed GREP match must actually compose after growth");

    // Account only for the structurally asserted new capacity, IDs and thread revisions.
    let mut without_growth = (*after).clone();
    without_growth.spreads.truncate(before.spreads.len());
    for (sp, old) in without_growth.spreads.iter_mut().zip(&before.spreads) {
        Arc::make_mut(sp).id = old.id;
    }
    let restored_story = Arc::make_mut(without_growth.stories.get_mut(&sid).unwrap());
    restored_story.frames.truncate(old_frames);
    restored_story.rev = old_story.rev;
    without_growth.next_id = before.next_id;
    assert_eq!(without_growth, expected, "apart from asserted capacity/IDs/revisions, only condition visibility may change");
    assert_eq!(full_doc(&without_growth), full_doc(&expected));
    growth(&mut s, sid, Some(removed), &before, count);
}

#[test]
fn r53_fully_shadowed_grep_condition_preserves_explicit_primary_page_removal() {
    let (mut s, sid) = primary_chain(false);
    assert!(s.prefs.smart_text_reflow);
    let name = "Dormant";
    let style = "Dormant GREP";
    let cover = "Cover";
    let pattern = "Words";
    let color = [10, 20, 30];
    s.execute("condition.new", &json!({"name": name, "color": color, "visible": true})).unwrap();
    s.execute("style.character.create", &json!({"name": style, "chars": {"conditions": [name]}})).unwrap();
    s.execute("style.character.create", &json!({"name": cover, "chars": {"conditions": []}})).unwrap();
    select_all(&mut s, sid);
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": style, "pattern": pattern}, {"style": cover, "pattern": pattern}]}})).unwrap();

    let fitted = doc(&s);
    let shown = fresh(&fitted, sid);
    assert!(!shown.is_overset());
    let mut fitted_hidden = (*fitted).clone();
    fitted_hidden.conditions[0].visible = false;
    same_composition(&shown, &fresh(&fitted_hidden, sid));

    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    let old_story = before.stories.get(&sid).unwrap();
    assert_eq!(old_story.paras.len(), 1, "fixture: both rules cover the same single paragraph");
    assert_eq!(old_story.text, WORDS.repeat(250));
    assert_eq!(before.conditions, vec![designcraft_doc::Condition { name: name.to_string(), color, visible: true }]);
    let matches: Vec<_> = regex::Regex::new(pattern).unwrap().find_iter(&old_story.text).map(|m| m.range()).collect();
    assert_eq!(matches.len(), 250, "fixture: this is a matching, not an unused-pattern control");
    assert!(matches.iter().all(|range| !range.is_empty() && &old_story.text[range.clone()] == pattern));
    let (props, base) = before.styles.resolve_para(&old_story.paras[0]);
    assert_eq!(props.grep_styles.len(), 2);
    assert_eq!((props.grep_styles[0].style.as_str(), props.grep_styles[0].pattern.as_str()), (style, pattern));
    assert_eq!((props.grep_styles[1].style.as_str(), props.grep_styles[1].pattern.as_str()), (cover, pattern));
    assert!(props.nested_styles.is_empty() && props.nested_line_styles.is_empty());
    assert!(base.conditions.is_empty());
    for (_, format) in old_story.runs() {
        assert!(before.styles.resolve_char(&base, format).conditions.is_empty(), "fixture: no run directly applies the condition");
    }
    let conditional = designcraft_doc::CharFormat { style: style.to_string(), ..Default::default() };
    let unconditional = designcraft_doc::CharFormat { style: cover.to_string(), ..Default::default() };
    assert_eq!(before.styles.resolve_char(&base, &conditional).conditions, vec![name.to_string()]);
    assert!(before.styles.resolve_char(&base, &unconditional).conditions.is_empty());
    let overlays = compose::paragraph_style_overlays(&old_story.text, &props.nested_styles, &props.grep_styles);
    let expected_overlays: Vec<_> = matches
        .iter()
        .cloned()
        .map(|range| (range, style.to_string()))
        .chain(matches.iter().cloned().map(|range| (range, cover.to_string())))
        .collect();
    assert_eq!(overlays, expected_overlays, "fixture: both nonempty range sets are identical, with unconditional Cover last");
    for range in &matches {
        for at in range.clone() {
            let last = overlays.iter().rev().find(|(range, _)| range.contains(&at)).unwrap();
            assert_eq!(last.1, cover, "fixture: the conditional rule wins no matched character");
        }
    }
    let reduced = fresh(&before, sid);
    let overset = reduced.overset_at.unwrap();
    assert!(matches.iter().any(|range| range.end <= overset));
    assert!(matches.iter().any(|range| range.start > overset), "fixture: the shadowed rules also match the uncomposed tail");
    same_composition(&reduced, &s.cache.get(&before, sid, None));

    // Specify the complete Doc independently; preview only checks the command's own delta.
    let mut expected = (*before).clone();
    expected.conditions[0].visible = false;
    let expected = Arc::new(expected);
    assert!(expected.conditions_hide(&[name.to_string()]));
    same_composition(&reduced, &fresh(&expected, sid));
    let params = json!({"name": name, "visible": false});
    let toggled = probe(&mut s, "condition.options", params.clone());
    assert_eq!(*toggled, *expected, "the toggle itself must change only condition visibility");
    assert_eq!(full_doc(&toggled), full_doc(&expected));
    assert!(Arc::ptr_eq(old_story, toggled.stories.get(&sid).unwrap()));
    assert!(Arc::ptr_eq(&before.styles, &toggled.styles));
    same_composition(&reduced, &fresh(&toggled, sid));

    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("condition.options", &params).unwrap();
    assert_eq!(s.journal.len(), journal + 1, "one condition edit produces one journal entry");
    assert_eq!(s.journal.last().unwrap(), &("condition.options".into(), params));
    assert_eq!(
        doc(&s).page_count(),
        before.page_count(),
        "defect oracle: a fully shadowed GREP condition must not recreate intentionally removed primary pages"
    );
    preserved(&mut s, sid, removed, &before, &expected, count);
}

#[test]
fn r53_partially_shadowed_overset_grep_condition_resumes_primary_growth() {
    let (mut s, sid) = primary_chain(false);
    assert!(s.prefs.smart_text_reflow);
    let name = "Partly covered GREP tail";
    let style = "R53 GREP";
    let cover = "R53 Cover";
    let prefix = "R53_CONDITIONAL_";
    let pattern = "R53_CONDITIONAL_GREP_TAIL";
    let color = [10, 20, 30];
    s.execute("condition.new", &json!({"name": name, "color": color, "visible": true})).unwrap();
    s.execute("style.character.create", &json!({"name": style, "chars": {"conditions": [name]}})).unwrap();
    s.execute("style.character.create", &json!({"name": cover, "chars": {"conditions": []}})).unwrap();

    let fitted = doc(&s);
    let composed = fresh(&fitted, sid);
    let last = *fitted.story(sid).unwrap().frames.last().unwrap();
    let range = &composed.frame(last).unwrap().range;
    assert!(range.end - range.start > 4 * WORDS.len(), "fixture: occupied last page has room for unconditioned text and the match");
    let end = range.start + 2 * WORDS.len();
    let start = end + 1;
    s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": fitted.story(sid).unwrap().len()})).unwrap();
    s.execute("text.insert", &json!({"text": format!("\n{pattern}")})).unwrap();
    select_all(&mut s, sid);
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": style, "pattern": pattern}, {"style": cover, "pattern": prefix}]}})).unwrap();
    let match_range = start..start + pattern.len();
    let cover_range = start..start + prefix.len();
    let used_range = cover_range.end..match_range.end;
    assert!(match_range.start == cover_range.start && !used_range.is_empty());
    let visible_used =
        |cs: &ComposedStory| cs.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).any(|g| g.visible && used_range.contains(&g.byte));
    let visible_cover =
        |cs: &ComposedStory| cs.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).any(|g| g.visible && cover_range.contains(&g.byte));
    let shown = fresh(&doc(&s), sid);
    assert!(!shown.is_overset(), "fixture: the actual matched text fits before capacity removal");
    assert!(visible_used(&shown), "fixture: the unshadowed suffix really draws when its condition is visible");
    assert!(visible_cover(&shown), "fixture: the later unconditional prefix really draws");
    s.execute("condition.options", &json!({"name": name, "visible": false})).unwrap();
    let hidden = fresh(&doc(&s), sid);
    assert!(!hidden.is_overset());
    assert!(!visible_used(&hidden), "fixture: the still-winning conditional suffix really hides");
    assert!(visible_cover(&hidden), "fixture: the shadowing prefix remains visible");

    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    let old_story = before.stories.get(&sid).unwrap();
    let matches: Vec<_> = regex::Regex::new(pattern).unwrap().find_iter(&old_story.text).map(|m| m.range()).collect();
    assert_eq!(matches, vec![match_range.clone()], "fixture: exactly one nonempty GREP match at the asserted tail range");
    let covers: Vec<_> = regex::Regex::new(prefix).unwrap().find_iter(&old_story.text).map(|m| m.range()).collect();
    assert_eq!(covers, vec![cover_range.clone()], "fixture: the later rule covers only a proper prefix of the one conditional match");
    for para in &old_story.paras {
        let (props, base) = before.styles.resolve_para(para);
        assert_eq!(props.grep_styles.len(), 2);
        assert_eq!((props.grep_styles[0].style.as_str(), props.grep_styles[0].pattern.as_str()), (style, pattern));
        assert_eq!((props.grep_styles[1].style.as_str(), props.grep_styles[1].pattern.as_str()), (cover, prefix));
        assert!(props.nested_styles.is_empty() && props.nested_line_styles.is_empty());
        assert!(base.conditions.is_empty(), "fixture: no paragraph applies the condition directly");
        for (_, format) in old_story.runs() {
            assert!(before.styles.resolve_char(&base, format).conditions.is_empty(), "fixture: no run applies the condition directly");
        }
        let format = designcraft_doc::CharFormat { style: style.to_string(), ..Default::default() };
        assert_eq!(before.styles.resolve_char(&base, &format).conditions, vec![name.to_string()]);
        let cover_format = designcraft_doc::CharFormat { style: cover.to_string(), ..Default::default() };
        assert!(before.styles.resolve_char(&base, &cover_format).conditions.is_empty());
    }
    let para_range = &old_story.para_ranges()[old_story.para_at(start)];
    assert_eq!(para_range.start, start);
    let (props, _) = before.styles.resolve_para(&old_story.paras[old_story.para_at(start)]);
    let overlays = compose::paragraph_style_overlays(&old_story.text[para_range.clone()], &props.nested_styles, &props.grep_styles);
    assert_eq!(overlays, vec![(0..pattern.len(), style.to_string()), (0..prefix.len(), cover.to_string())]);
    for at in 0..pattern.len() {
        let last = overlays.iter().rev().find(|(range, _)| range.contains(&at)).unwrap();
        assert_eq!(last.1, if at < prefix.len() { cover } else { style }, "fixture: only the prefix is shadowed");
    }
    assert_eq!(before.conditions, vec![designcraft_doc::Condition { name: name.to_string(), color, visible: false }]);
    let reduced = fresh(&before, sid);
    assert!(
        reduced.overset_at.is_some_and(|at| at < end && at < match_range.start),
        "fixture: unconditioned text oversets before the entire actual GREP match"
    );
    same_composition(&reduced, &s.cache.get(&before, sid, None));

    // The independent expected Doc changes only visibility; the matching tail is not yet composed.
    let mut expected = (*before).clone();
    expected.conditions[0].visible = true;
    let params = json!({"name": name, "visible": true});
    let toggled = probe(&mut s, "condition.options", params.clone());
    assert_eq!(*toggled, expected, "the public toggle itself changes only condition visibility");
    assert_eq!(full_doc(&toggled), full_doc(&expected));
    assert!(Arc::ptr_eq(old_story, toggled.stories.get(&sid).unwrap()));
    assert!(Arc::ptr_eq(&before.styles, &toggled.styles));
    let still_overset = fresh(&expected, sid);
    assert!(still_overset.is_overset());
    same_composition(&reduced, &still_overset);

    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("condition.options", &params).unwrap();
    let after = doc(&s);
    assert!(after.page_count() > before.page_count(), "a condition on an actual overset GREP match must resume primary growth");
    let added = after.page_count() - before.page_count();
    let old_frames = old_story.frames.len();
    let new_story = after.story(sid).unwrap();
    assert_eq!(new_story.frames.len(), old_frames + added);
    assert_eq!(new_story.rev, old_story.rev + added as u64, "each appended primary frame advances the thread revision once");
    assert_eq!(&new_story.frames[..old_frames], &old_story.frames);
    assert_eq!(after.spreads.len(), before.spreads.len() + added);
    assert!(!after.settings.facing_pages);
    for (sp, frame) in after.spreads[before.spreads.len()..].iter().zip(&new_story.frames[old_frames..]) {
        assert_eq!(sp.pages.len(), 1);
        assert_eq!(sp.items.len(), 1);
        assert_eq!(sp.items[0].id, *frame);
        assert_eq!(sp.items[0].text_frame().unwrap().story, sid);
        assert_eq!(sp.items[0].bounds(), sp.pages[0].margin_rect());
    }
    assert!(after.next_id > before.next_id);
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last().unwrap(), &("condition.options".into(), params));
    let expanded = fresh(&after, sid);
    assert!(visible_used(&expanded), "the unshadowed conditional suffix must actually compose after growth");
    assert!(visible_cover(&expanded), "the shadowing prefix remains visible after growth");

    // Account only for the structurally asserted new capacity, IDs and thread revisions.
    let mut without_growth = (*after).clone();
    without_growth.spreads.truncate(before.spreads.len());
    for (sp, old) in without_growth.spreads.iter_mut().zip(&before.spreads) {
        Arc::make_mut(sp).id = old.id;
    }
    let restored_story = Arc::make_mut(without_growth.stories.get_mut(&sid).unwrap());
    restored_story.frames.truncate(old_frames);
    restored_story.rev = old_story.rev;
    without_growth.next_id = before.next_id;
    assert_eq!(without_growth, expected, "apart from asserted capacity/IDs/revisions, only condition visibility may change");
    assert_eq!(full_doc(&without_growth), full_doc(&expected));
    growth(&mut s, sid, Some(removed), &before, count);
}

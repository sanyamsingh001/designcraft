//! REVIEW8 reflow regressions. Only public Session/compose/format APIs, usable on the pinned BEFORE tree.
use std::sync::Arc;

use designcraft_compose::{self as compose, Cache, ComposeOptions, ComposedStory};
use designcraft_doc::{Document, ItemId, StoryId};
use serde_json::{Value, json};

use crate::Session;

const WORDS: &str = "Words keep coming and the page fills up quickly with them. ";
const HEADING: &str = "Review8 Heading";

fn doc(s: &Session) -> Arc<Document> {
    s.doc().unwrap().doc.clone()
}

fn full_doc(d: &Document) -> Value {
    serde_json::to_value(d).unwrap()
}

/// Independent direct composition with the same consumed indices as the renderer.
/// Default ComposeOptions are intentionally NOT a reference oracle for running headers.
fn fresh(d: &Document, sid: StoryId) -> ComposedStory {
    let bank = Cache::new();
    let story = d.story(sid).unwrap();
    let options = ComposeOptions {
        running: compose::vars::has_running(d, story).then(|| bank.running_index(d)),
        xrefs: compose::xref::has_xrefs(story).then(|| bank.xref_index(d)),
        ..Default::default()
    };
    compose::compose_story(d, sid, &options)
}

fn same_composition(a: &ComposedStory, b: &ComposedStory) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"), "all composition fields must agree");
}

fn public_native_controls(s: &mut Session, sid: StoryId) {
    let expected = doc(s);
    expected.check().unwrap();
    let reference = fresh(&expected, sid);
    same_composition(&reference, &s.cache.get(&expected, sid, None));
    let story = s.execute("story.get", &json!({"story": sid.0})).unwrap();
    assert_eq!(story["overset"], json!(reference.overset_at));
    let inspect = s.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(inspect["pageCount"], json!(expected.page_count()));
    let summary = inspect["stories"].as_array().unwrap().iter().find(|v| v["id"] == json!(sid.0)).unwrap();
    assert_eq!(summary["overset"], json!(reference.is_overset()));
    let serialized = s.execute("file.serialize", &json!({})).unwrap();
    let bytes = crate::cmd::base64_decode(serialized["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"), "fixture must exercise native ZIP, not an empty fallback");
    let loaded = designcraft_format::load(&bytes).unwrap();
    assert_eq!(loaded, *expected, "the full native document must survive");
    assert_eq!(full_doc(&loaded), full_doc(&expected));
    let mut reopened = Session::new();
    reopened.execute("file.openBytes", &json!({"base64": serialized["base64"]})).unwrap();
    assert_eq!(*doc(&reopened), *expected);
    same_composition(&reference, &reopened.cache.get(&doc(&reopened), sid, None));
    assert!(Arc::ptr_eq(&doc(s), &expected), "queries/serialization must be read-only");
}

fn history(s: &mut Session, sid: StoryId, before: &Arc<Document>, after: &Arc<Document>, count: usize) {
    assert_eq!(s.doc().unwrap().history.undo.len(), count + 1, "one user edit owns one undo entry");
    assert!(Arc::ptr_eq(&s.doc().unwrap().history.undo.last().unwrap().doc, before));
    public_native_controls(s, sid);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(s), before));
    assert_eq!(full_doc(&doc(s)), full_doc(before));
    public_native_controls(s, sid);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(s), after));
    assert_eq!(full_doc(&doc(s)), full_doc(after));
    public_native_controls(s, sid);
}

fn apply(s: &mut Session, command: &str, params: Value, cold: bool, commit: bool) {
    if cold {
        s.cache.clear();
    }
    if commit {
        s.begin_interaction("Review8 reflow edit").unwrap();
        s.preview(command, params).unwrap();
        if cold {
            s.cache.clear();
        }
        s.commit_interaction().unwrap();
    } else {
        s.execute(command, &params).unwrap();
    }
}

fn probe(s: &mut Session, command: &str, params: Value) -> Arc<Document> {
    let before = doc(s);
    let undo = s.doc().unwrap().history.undo.len();
    s.begin_interaction("Review8 independent fixture probe").unwrap();
    s.preview(command, params).unwrap();
    let result = doc(s);
    assert_eq!(result.page_count(), before.page_count(), "preview defers reflow");
    result.check().unwrap();
    s.cancel_interaction();
    assert!(Arc::ptr_eq(&doc(s), &before));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    result
}

fn select_all(s: &mut Session, sid: StoryId) {
    let end = doc(s).story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": end})).unwrap();
}

fn primary_chain(grid: bool) -> (Session, StoryId) {
    let mut s = Session::new();
    s.execute(
        "file.new",
        &json!({"pages": 1, "width": 612, "height": 792, "margins": 36, "columns": 1,
        "primaryTextFrame": true, "facingPages": false}),
    )
    .unwrap();
    let sid = doc(&s).settings.primary_story.unwrap();
    s.execute("document.preferences", &json!({"baselineGrid": {"start": 0, "increment": 12}})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    s.execute("type.para", &json!({"attrs": {"gridAlign": if grid { "allLines" } else { "none" }}})).unwrap();
    s.execute("text.insert", &json!({"text": WORDS.repeat(250)})).unwrap();
    let d = doc(&s);
    let specs = compose::frame_specs(&d, sid);
    assert_eq!((d.settings.page_width, d.settings.page_height), (612.0, 792.0), "fixture: Letter page geometry");
    assert_eq!((specs[0].area.width(), specs[0].area.height()), (540.0, 720.0), "fixture: 36pt margins");
    assert_eq!((d.settings.baseline_grid.start, d.settings.baseline_grid.increment), (0.0, 12.0));
    let story = d.story(sid).unwrap();
    assert_eq!(story.text, WORDS.repeat(250));
    let (para, chars) = d.styles.resolve_para(&story.paras[0]);
    assert_eq!(para.grid_align, if grid { designcraft_doc::GridAlign::AllLines } else { designcraft_doc::GridAlign::None });
    assert_eq!((chars.font_family.as_str(), chars.font_style.as_str(), chars.size), ("Source Serif 4", "Regular", 12.0));
    let face = designcraft_fonts::FontDb::global().face(&chars.font_family, &chars.font_style);
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Source Serif 4", "Regular"), "fixture: require the named font");
    let cs = fresh(&d, sid);
    assert!(d.page_count() >= 3, "fixture: occupied primary chain must span multiple pages");
    assert!(!cs.is_overset(), "fixture: source chain must fit before removal");
    same_composition(&cs, &s.cache.get(&d, sid, None));
    (s, sid)
}

fn remove_tail(s: &mut Session, sid: StoryId) -> ItemId {
    let original = doc(s);
    let last = *original.story(sid).unwrap().frames.last().unwrap();
    assert!(fresh(&original, sid).frame(last).is_some_and(|f| !f.range.is_empty()), "fixture: remove an occupied frame");
    let page = original.page_of_item(last).unwrap();
    s.execute("layout.pages.delete", &json!({"pages": [page]})).unwrap();
    let reduced = doc(s);
    assert_eq!(reduced.page_count(), original.page_count() - 1);
    assert_eq!(reduced.story(sid).unwrap().text, original.story(sid).unwrap().text);
    assert!(reduced.item(last).is_none());
    assert!(fresh(&reduced, sid).is_overset(), "fixture: removal must reduce real consumed capacity");
    same_composition(&fresh(&reduced, sid), &s.cache.get(&reduced, sid, None));
    last
}

fn source(s: &mut Session, style: &str, text: &str, y: f64, character: bool) -> (ItemId, StoryId) {
    let r = s
        .execute(
            "frame.create",
            &json!({"spread": 0, "rect": [10, y, 310, y + 36.0],
        "content": "text", "text": text, "caret": false}),
        )
        .unwrap();
    let sid = StoryId(r["story"].as_u64().unwrap());
    select_all(s, sid);
    s.execute(if character { "style.character.apply" } else { "style.paragraph.apply" }, &json!({"name": style})).unwrap();
    (ItemId(r["id"].as_u64().unwrap()), sid)
}

fn header_definition(use_: &str) -> Value {
    json!({"name": "Running Header", "type": "runningHeader", "style": HEADING,
        "character": false, "use": use_, "before": "Head[", "after": "]"})
}

fn header_values(d: &Document) -> Vec<String> {
    let bank = Cache::new();
    let index = bank.running_index(d);
    let vi = d.variable("Running Header").unwrap();
    std::iter::once(None).chain((0..=d.page_count()).map(Some)).map(|page| compose::vars::values(d, page, Some(&index))[vi].clone()).collect()
}

fn header_case(tail: bool, second: bool) -> (Session, StoryId, ItemId) {
    let (mut s, sid) = primary_chain(false);
    s.execute("style.paragraph.create", &json!({"name": HEADING})).unwrap();
    s.execute("style.character.create", &json!({"name": HEADING})).unwrap();
    let (_, heading) = source(&mut s, HEADING, "Stable heading", 100.0, false);
    select_all(&mut s, heading);
    s.execute("style.character.apply", &json!({"name": HEADING})).unwrap();
    if second {
        source(&mut s, HEADING, "Second heading", 160.0, false);
    }
    s.execute("variables.define", &header_definition("firstOnPage")).unwrap();
    let at = if tail { doc(&s).story(sid).unwrap().len() } else { 0 };
    s.execute("text.select", &json!({"story": sid.0, "anchor": at, "focus": at})).unwrap();
    if tail {
        s.execute("text.insert", &json!({"text": "\n"})).unwrap();
    }
    s.execute("variables.insert", &json!({"name": "Running Header"})).unwrap();
    s.execute("text.insert", &json!({"text": "\n"})).unwrap();
    let removed = remove_tail(&mut s, sid);
    let d = doc(&s);
    let cs = fresh(&d, sid);
    let mark = d.story(sid).unwrap().text.find(designcraft_doc::vars::var_char(d.variable("Running Header").unwrap()).unwrap()).unwrap();
    assert!(
        cs.overset_at.is_some_and(|at| if tail { at < mark } else { mark < at }),
        "fixture: header position must be genuinely visible or overset"
    );
    assert!(compose::compose_story(&d, sid, &Default::default()).is_overset(), "fixture: these definition controls must have literal overset too");
    (s, sid, removed)
}

fn growth(s: &mut Session, sid: StoryId, removed: Option<ItemId>, before: &Arc<Document>, count: usize) {
    let after = doc(s);
    assert!(after.page_count() > before.page_count(), "defect oracle: consumed edit must resume growth");
    assert!(after.story(sid).unwrap().frames.len() > before.story(sid).unwrap().frames.len());
    assert_eq!(after.story(sid).unwrap().text, before.story(sid).unwrap().text);
    if let Some(removed) = removed {
        assert!(after.item(removed).is_none());
    }
    assert!(!fresh(&after, sid).is_overset(), "defect oracle: actual expanded text must fit");
    history(s, sid, before, &after, count);
}

fn preserved(s: &mut Session, sid: StoryId, removed: ItemId, before: &Arc<Document>, expected: &Arc<Document>, count: usize) {
    let after = doc(s);
    assert_eq!(*after, **expected, "defect oracle: only the requested edit may change the full document");
    assert_eq!(full_doc(&after), full_doc(expected));
    assert_eq!(after.page_count(), before.page_count());
    assert_eq!(after.story(sid), before.story(sid));
    assert!(after.item(removed).is_none());
    assert!(fresh(&after, sid).is_overset(), "explicit removal must remain");
    history(s, sid, before, &after, count);
}

fn narrow_header() -> (Session, StoryId, StoryId, ItemId) {
    let face = designcraft_fonts::FontDb::global().face("Source Serif 4", "Regular");
    assert_eq!(face.family, "Source Serif 4", "fixture: require the named bundled font");
    assert_eq!(face.style, "Regular", "fixture: require the named face");
    assert_ne!(face.glyph_for('M'), 0, "fixture: M must exist");
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1, "width": 612, "height": 792, "margins": 36, "columns": 1, "facingPages": false})).unwrap();
    s.execute("style.paragraph.create", &json!({"name": HEADING})).unwrap();
    let (_, heading) = source(&mut s, HEADING, "A", 20.0, false);
    let mut definition = header_definition("firstOnPage");
    definition["before"] = json!("");
    definition["after"] = json!("");
    s.execute("variables.define", &definition).unwrap();
    let made = s
        .execute(
            "frame.create",
            &json!({"spread": 0, "rect": [36, 100, 136, 118],
        "content": "text", "text": "", "caret": false}),
        )
        .unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    let frame = ItemId(made["id"].as_u64().unwrap());
    s.execute("object.primaryTextFrame", &json!({"ids": [frame.0], "on": true})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    s.execute("variables.insert", &json!({"name": "Running Header"})).unwrap();
    s.execute("text.insert", &json!({"text": " MMMMMM"})).unwrap();
    select_all(&mut s, sid);
    s.execute(
        "type.char",
        &json!({"attrs": {"fontFamily": "Source Serif 4", "size": 12,
        "leading": {"kind": "points", "value": 14}}}),
    )
    .unwrap();
    s.execute("type.para", &json!({"attrs": {"composer": "singleLine", "hyphenate": false}})).unwrap();
    let d = doc(&s);
    assert_eq!(d.page_count(), 1, "fixture: short expanded text needs only the initial page");
    assert_eq!(d.story(sid).unwrap().frames, vec![frame]);
    let spec = compose::frame_specs(&d, sid);
    assert_eq!((spec[0].area.width(), spec[0].area.height()), (100.0, 18.0));
    let story = d.story(sid).unwrap();
    let (para, base) = d.styles.resolve_para(&story.paras[0]);
    assert_eq!(para.composer, designcraft_doc::Composer::SingleLine, "fixture: deterministic line breaker");
    assert!(!para.hyphenate);
    assert_eq!(para.grid_align, designcraft_doc::GridAlign::None);
    for (_, chars) in story.runs() {
        let resolved = d.styles.resolve_char(&base, chars);
        assert_eq!((resolved.font_family.as_str(), resolved.font_style.as_str(), resolved.size), ("Source Serif 4", "Regular", 12.0));
        assert_eq!(resolved.leading, designcraft_doc::Leading::Points(14.0));
    }
    let cs = fresh(&d, sid);
    assert!(!cs.is_overset() && cs.line_count() == 1, "fixture: named font/metrics must fit the short header on one line");
    assert!(!compose::compose_story(&d, sid, &Default::default()).is_overset());
    same_composition(&cs, &s.cache.get(&d, sid, None));
    (s, sid, heading, frame)
}

#[test]
fn r8_reflow_header_only_growth() {
    for cold in [false, true] {
        for commit in [false, true] {
            let (mut s, sid, heading, _) = narrow_header();
            let params = json!({"story": heading.0, "start": 0, "end": 1, "text": "MMMMMMMM"});
            let expected = probe(&mut s, "story.replaceRange", params.clone());
            assert!(fresh(&expected, sid).is_overset(), "fixture: long header must overflow in the actual consumed composition");
            assert!(
                !compose::compose_story(&expected, sid, &Default::default()).is_overset(),
                "fixture: overflow must come only from the header body"
            );
            assert_eq!(fresh(&expected, heading).line_count(), 1, "fixture: heading source itself must fit");
            let before = doc(&s);
            let count = s.doc().unwrap().history.undo.len();
            apply(&mut s, "story.replaceRange", params, cold, commit);
            assert_eq!(doc(&s).story(heading).unwrap().text, "MMMMMMMM");
            growth(&mut s, sid, None, &before, count);
        }
    }
}

#[test]
fn r8_reflow_header_pruning_keeps_consumed_second_frame() {
    for cold in [false, true] {
        let (mut s, sid, heading, first) = narrow_header();
        s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
        s.execute("layout.pages.insert", &json!({"count": 2})).unwrap();
        let mut frames = vec![first];
        for page in [1, 2] {
            let spread = doc(&s).page_loc(page).unwrap().0;
            let made = s
                .execute(
                    "frame.create",
                    &json!({"spread": spread, "rect": [36, 100, 136, 118],
                "content": "text", "text": "", "caret": false}),
                )
                .unwrap();
            frames.push(ItemId(made["id"].as_u64().unwrap()));
        }
        // Threading has no command id; public Session::edit is only fixture construction.
        s.edit(|d, _| {
            d.thread(frames[0], frames[1])?;
            d.thread(frames[1], frames[2])?;
            Ok(())
        })
        .unwrap();
        s.execute("story.replaceRange", &json!({"story": heading.0, "start": 0, "end": 1, "text": "MMMMMMMM"})).unwrap();
        let before = doc(&s);
        let cs = fresh(&before, sid);
        assert!(!cs.is_overset(), "fixture: three explicit frames must fit");
        assert!(cs.frame(frames[1]).is_some_and(|f| !f.range.is_empty()), "fixture: second frame must hold consumed text");
        assert!(cs.frame(frames[2]).is_some_and(|f| f.range.is_empty()), "fixture: only third frame may be pruned");
        s.execute("prefs.set", &json!({"smartTextReflow": true})).unwrap();
        let params = json!({"story": heading.0, "start": 0, "end": 8, "text": "MMMMMMM"});
        let expected = probe(&mut s, "story.replaceRange", params.clone());
        assert!(fresh(&expected, sid).frame(frames[1]).is_some_and(|f| !f.range.is_empty()), "fixture: seven-M header still needs the second frame");
        let count = s.doc().unwrap().history.undo.len();
        apply(&mut s, "story.replaceRange", params, cold, false);
        let after = doc(&s);
        assert_eq!(after.page_count(), 2, "defect oracle: prune only the unused tail");
        assert_eq!(after.story(sid).unwrap().frames, frames[..2]);
        assert!(after.item(frames[2]).is_none());
        assert!(!fresh(&after, sid).is_overset());
        history(&mut s, sid, &before, &after, count);
    }
}

#[test]
fn r8_reflow_grid_controls_resume_removed_capacity() {
    for field in ["start", "increment"] {
        for cold in [false, true] {
            for commit in [false, true] {
                let (mut s, sid) = primary_chain(true);
                let original = doc(&s);
                let frames = original.story(sid).unwrap().frames.clone();
                assert!((3..=32).contains(&frames.len()), "fixture: bounded occupied multi-page chain");
                assert_eq!(original.page_count(), frames.len(), "fixture: exactly one primary frame per page");
                // A grid phase change can recover a few lines across several remaining pages.
                // Retain just the original first frame, proving each deleted continuation occupied.
                let removed = remove_tail(&mut s, sid);
                for _ in 2..frames.len() {
                    remove_tail(&mut s, sid);
                }
                let before = doc(&s);
                assert_eq!(before.page_count(), 1, "fixture: one original page remains");
                assert_eq!(before.story(sid).unwrap().frames, vec![frames[0]], "fixture: one original primary frame remains");
                assert_eq!(before.story(sid).unwrap().text, original.story(sid).unwrap().text, "fixture: all source text retained");
                assert!(frames[1..].iter().all(|id| before.item(*id).is_none()), "fixture: every explicit continuation removal remains");
                assert!(fresh(&before, sid).is_overset(), "fixture: occupied capacity removal leaves actual overset");
                let params = if field == "start" { json!({"baselineGrid": {"start": 5}}) } else { json!({"baselineGrid": {"increment": 36}}) };
                let expected = probe(&mut s, "document.preferences", params.clone());
                let old = fresh(&before, sid);
                let changed = fresh(&expected, sid);
                assert_eq!(before.story(sid), expected.story(sid), "fixture: only document grid changes");
                assert_ne!(compose::frame_specs(&before, sid)[0].grid, compose::frame_specs(&expected, sid)[0].grid);
                let baselines = |cs: &ComposedStory| cs.frames.iter().flat_map(|f| f.lines.iter().map(|l| l.baseline)).collect::<Vec<_>>();
                assert_ne!(baselines(&old), baselines(&changed), "fixture: the chosen numeric control must change real baselines");
                assert!(changed.is_overset(), "fixture: changed remaining capacity still needs growth");
                let count = s.doc().unwrap().history.undo.len();
                apply(&mut s, "document.preferences", params, cold, commit);
                growth(&mut s, sid, Some(removed), &before, count);
            }
        }
    }
}

#[test]
fn r8_reflow_grid_unused_and_display_settings_preserve_removal() {
    for control in ["unusedGeometry", "unchanged", "color", "viewThreshold", "frameOverride"] {
        for cold in [false, true] {
            for commit in [false, true] {
                let (mut s, sid) = primary_chain(control != "unusedGeometry");
                if control == "frameOverride" {
                    let frames = doc(&s).story(sid).unwrap().frames.clone();
                    s.edit(|d, _| {
                        for id in &frames {
                            d.item_mut(*id).unwrap().text_frame_mut().unwrap().options.baseline_grid = Some((12.0, 0.0));
                        }
                        Ok(())
                    })
                    .unwrap();
                }
                let removed = remove_tail(&mut s, sid);
                let before = doc(&s);
                let params = match control {
                    "color" => json!({"baselineGrid": {"color": [1, 2, 3]}}),
                    "viewThreshold" => json!({"baselineGrid": {"viewThreshold": 0.5}}),
                    "unchanged" => json!({"baselineGrid": {"start": 0, "increment": 12}}),
                    _ => json!({"baselineGrid": {"start": 5, "increment": 36}}),
                };
                let expected = probe(&mut s, "document.preferences", params.clone());
                same_composition(&fresh(&before, sid), &fresh(&expected, sid));
                let count = s.doc().unwrap().history.undo.len();
                apply(&mut s, "document.preferences", params, cold, commit);
                preserved(&mut s, sid, removed, &before, &expected, count);
            }
        }
    }
}

#[test]
fn r8_reflow_header_equal_definition_preserves_removal() {
    for tail in [false, true] {
        for cold in [false, true] {
            for commit in [false, true] {
                let (mut s, sid, removed) = header_case(tail, false);
                let before = doc(&s);
                let params = header_definition("lastOnPage");
                let expected = probe(&mut s, "variables.define", params.clone());
                assert_ne!(before.text_variables, expected.text_variables, "fixture: used definition must actually change");
                assert_eq!(header_values(&before), header_values(&expected), "fixture: all effective pages, carry and no-page wrappers must agree");
                same_composition(&fresh(&before, sid), &fresh(&expected, sid));
                let count = s.doc().unwrap().history.undo.len();
                apply(&mut s, "variables.define", params, cold, commit);
                preserved(&mut s, sid, removed, &before, &expected, count);
            }
        }
    }
}

#[test]
fn r8_reflow_header_equivalent_style_character_definitions() {
    for binding in ["style", "character"] {
        for tail in [false, true] {
            for cold in [false, true] {
                let (mut s, sid, removed) = header_case(tail, false);
                if binding == "style" {
                    // Add the equal alternative while retaining the currently reduced primary chain.
                    s.execute("style.paragraph.create", &json!({"name": "Review8 Equivalent"})).unwrap();
                    source(&mut s, "Review8 Equivalent", "Stable heading", 160.0, false);
                }
                let before = doc(&s);
                let mut params = header_definition("firstOnPage");
                if binding == "style" {
                    params["style"] = json!("Review8 Equivalent");
                } else {
                    params["character"] = json!(true);
                }
                let expected = probe(&mut s, "variables.define", params.clone());
                assert_ne!(before.text_variables, expected.text_variables);
                assert_eq!(header_values(&before), header_values(&expected));
                same_composition(&fresh(&before, sid), &fresh(&expected, sid));
                let count = s.doc().unwrap().history.undo.len();
                apply(&mut s, "variables.define", params, cold, false);
                preserved(&mut s, sid, removed, &before, &expected, count);
            }
        }
    }
}

#[test]
fn r8_reflow_header_changed_used_output_resumes() {
    for tail in [false, true] {
        for cold in [false, true] {
            for commit in [false, true] {
                let (mut s, sid, removed) = header_case(tail, true);
                let before = doc(&s);
                let params = header_definition("lastOnPage");
                let expected = probe(&mut s, "variables.define", params.clone());
                assert_ne!(header_values(&before), header_values(&expected), "fixture: consumed output must change");
                if tail {
                    same_composition(&fresh(&before, sid), &fresh(&expected, sid));
                }
                let count = s.doc().unwrap().history.undo.len();
                apply(&mut s, "variables.define", params, cold, commit);
                growth(&mut s, sid, Some(removed), &before, count);
            }
        }
    }
}

#[test]
fn r8_reflow_unused_grid_with_header_preserves_removal() {
    for cold in [false, true] {
        for commit in [false, true] {
            let (mut s, sid, removed) = header_case(false, false);
            let before = doc(&s);
            let params = json!({"baselineGrid": {"start": 5, "increment": 36}});
            let expected = probe(&mut s, "document.preferences", params.clone());
            assert_eq!(header_values(&before), header_values(&expected));
            same_composition(&fresh(&before, sid), &fresh(&expected, sid));
            let count = s.doc().unwrap().history.undo.len();
            apply(&mut s, "document.preferences", params, cold, commit);
            preserved(&mut s, sid, removed, &before, &expected, count);
        }
    }
}

fn object_removal(command: &str, cold: bool, commit: bool) {
    let (mut s, sid) = primary_chain(false);
    let preceding = doc(&s);
    let removed = *preceding.story(sid).unwrap().frames.last().unwrap();
    assert!(fresh(&preceding, sid).frame(removed).is_some_and(|f| !f.range.is_empty()));
    s.execute("selection.set", &json!({"ids": [removed.0]})).unwrap();
    s.execute("object.label", &json!({"label": "Review8 preceding edit", "ids": [removed.0]})).unwrap();
    let before = doc(&s);
    assert_eq!(before.item(removed).unwrap().label, "Review8 preceding edit");
    assert_eq!(before.page_count(), preceding.page_count(), "fixture: label edit preserves reduced capacity inputs");
    same_composition(&fresh(&preceding, sid), &fresh(&before, sid));
    let preceding_label = s.doc().unwrap().history.undo.last().unwrap().label.clone();
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    apply(&mut s, command, json!({}), cold, commit);
    let after = doc(&s);
    assert!(after.item(removed).is_none());
    assert_eq!(after.page_count(), before.page_count());
    assert_eq!(after.story(sid).unwrap().text, before.story(sid).unwrap().text);
    assert_eq!(after.story(sid).unwrap().frames, before.story(sid).unwrap().frames.iter().copied().filter(|id| *id != removed).collect::<Vec<_>>());
    assert!(fresh(&after, sid).is_overset());
    assert_eq!(s.doc().unwrap().history.undo.len(), count + 1, "defect oracle: object removal owns exactly one Undo entry");
    assert_eq!(s.journal.len(), journal + 1, "only the user command is journalled");
    assert_eq!(s.journal.last().unwrap().0, command);
    history(&mut s, sid, &before, &after, count);
    // A second undo must undo the preceding label, not an identical Clear snapshot.
    s.execute("edit.undo", &json!({})).unwrap();
    let second = s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(second["undone"], json!(preceding_label));
    assert!(Arc::ptr_eq(&doc(&s), &preceding), "defect oracle: second Undo reaches the exact preceding document");
    assert_eq!(full_doc(&doc(&s)), full_doc(&preceding));
    assert!(doc(&s).item(removed).unwrap().label.is_empty());
    public_native_controls(&mut s, sid);
}

#[test]
fn r8_reflow_object_cut_one_history_step() {
    for cold in [false, true] {
        object_removal("edit.cut", cold, false);
    }
}

#[test]
fn r8_reflow_clear_primary_frame_one_history_step() {
    for cold in [false, true] {
        object_removal("edit.clear", cold, false);
    }
}

#[test]
fn r8_reflow_object_cut_in_interaction_one_history_step() {
    for cold in [false, true] {
        object_removal("edit.cut", cold, true);
    }
}

#[test]
fn r8_reflow_text_cut_and_clear_one_history_step() {
    for command in ["edit.cut", "edit.clear"] {
        let (mut s, sid) = primary_chain(false);
        let before = doc(&s);
        select_all(&mut s, sid);
        let count = s.doc().unwrap().history.undo.len();
        let result = s.execute(command, &json!({})).unwrap();
        let after = doc(&s);
        assert_eq!(after.page_count(), 1);
        assert!(after.story(sid).unwrap().text.is_empty());
        if command == "edit.cut" {
            assert_eq!(result["text"], json!(before.story(sid).unwrap().text));
        }
        history(&mut s, sid, &before, &after, count);
    }
}

// REVIEW9 fixes 01..04. Fixture gates use actual public composition before defect assertions.
fn r9_shared_sources(a: &Document, b: &Document, spread_shared: bool) {
    assert!(Arc::ptr_eq(&a.styles, &b.styles));
    assert_eq!(a.stories.len(), b.stories.len());
    for (sid, story) in &a.stories {
        assert!(Arc::ptr_eq(story, b.stories.get(sid).unwrap()), "fixture: story allocation retained");
    }
    assert_eq!(a.spreads.len(), b.spreads.len());
    for (old, new) in a.spreads.iter().zip(&b.spreads) {
        if spread_shared {
            assert!(Arc::ptr_eq(old, new), "fixture: spread allocation retained");
        }
        assert_eq!(old.items.len(), new.items.len());
        for (item, other) in old.items.iter().zip(&new.items) {
            assert!(Arc::ptr_eq(item, other), "fixture: frame/wrap allocation retained");
        }
    }
}

fn r9_preference_preserves_removal(field: &str) {
    for cold in [false, true] {
        for commit in [false, true] {
            let (mut s, sid) = primary_chain(false);
            let removed = remove_tail(&mut s, sid);
            let before = doc(&s);
            let mut advanced = serde_json::Map::new();
            advanced.insert(field.into(), json!(80));
            let params = json!({"advancedType": advanced});
            let expected = probe(&mut s, "document.preferences", params.clone());
            assert_ne!(before.settings.advanced_type, expected.settings.advanced_type);
            r9_shared_sources(&before, &expected, true);
            let old = fresh(&before, sid);
            let new = fresh(&expected, sid);
            assert!(old.is_overset() && new.is_overset(), "fixture: genuinely removed occupied capacity");
            same_composition(&old, &new);
            let count = s.doc().unwrap().history.undo.len();
            let journal = s.journal.len();
            apply(&mut s, "document.preferences", params, cold, commit);
            assert_eq!(s.journal.len(), journal + 1);
            preserved(&mut s, sid, removed, &before, &expected, count);
        }
    }
}

#[test]
fn r9_reflow_unused_superscript_size_preserves_real_removed_capacity() {
    r9_preference_preserves_removal("superscriptSize");
}

#[test]
fn r9_reflow_unused_superscript_position_preserves_real_removed_capacity() {
    r9_preference_preserves_removal("superscriptPosition");
}

#[test]
fn r9_reflow_unused_subscript_size_preserves_real_removed_capacity() {
    r9_preference_preserves_removal("subscriptSize");
}

#[test]
fn r9_reflow_unused_subscript_position_preserves_real_removed_capacity() {
    r9_preference_preserves_removal("subscriptPosition");
}

#[test]
fn r9_reflow_same_preference_unused_opposite_script_and_used_script_controls() {
    for position in ["superscript", "subscript"] {
        for used in [false, true] {
            let (mut s, sid) = primary_chain(false);
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 8})).unwrap();
            s.execute("type.char", &json!({"attrs": {"position": position}})).unwrap();
            let removed = remove_tail(&mut s, sid);
            let before = doc(&s);
            let field = if (position == "superscript") == used { "superscriptSize" } else { "subscriptSize" };
            let mut advanced = serde_json::Map::new();
            advanced.insert(field.into(), json!(100));
            let params = json!({"advancedType": advanced});
            let expected = probe(&mut s, "document.preferences", params.clone());
            r9_shared_sources(&before, &expected, true);
            let old = fresh(&before, sid);
            let changed = fresh(&expected, sid);
            assert!(old.is_overset() && changed.is_overset());
            if used {
                assert_ne!(format!("{old:?}"), format!("{changed:?}"), "fixture: used script metrics really change");
            } else {
                same_composition(&old, &changed);
            }
            let count = s.doc().unwrap().history.undo.len();
            apply(&mut s, "document.preferences", params, true, true);
            if used {
                growth(&mut s, sid, Some(removed), &before, count);
            } else {
                preserved(&mut s, sid, removed, &before, &expected, count);
                let same_before = doc(&s);
                let same_params = json!({"advancedType": serde_json::to_value(same_before.settings.advanced_type).unwrap()});
                let same_expected = probe(&mut s, "document.preferences", same_params.clone());
                let same_count = s.doc().unwrap().history.undo.len();
                apply(&mut s, "document.preferences", same_params, false, false);
                preserved(&mut s, sid, removed, &same_before, &same_expected, same_count);
            }
        }
    }
}

fn r9_object_clipboard(command: &str, earlier_text: bool) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let made = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    if earlier_text {
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 3})).unwrap();
        assert_eq!(s.execute("edit.copy", &json!({})).unwrap()["text"], "OLD");
    }
    let shape = s.execute("frame.create", &json!({"rect": [100,100,140,140], "content": "unassigned", "shape": "ellipse"})).unwrap();
    let id = ItemId(shape["id"].as_u64().unwrap());
    s.execute("selection.set", &json!({"ids": [id.0]})).unwrap();
    assert!(s.doc().unwrap().selection.text.is_none());
    let before = doc(&s);
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute(command, &json!({})).unwrap();
    let after = doc(&s);
    if command == "edit.cut" {
        assert!(after.item(id).is_none());
        assert_eq!(s.journal.len(), journal + 1, "one public Cut, no nested Clear journal");
        assert_eq!(s.journal.last().unwrap().0, "edit.cut");
        history(&mut s, sid, &before, &after, count);
    } else {
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(s.doc().unwrap().history.undo.len(), count);
    }
    s.execute("text.select", &json!({"story": sid.0, "anchor": 3, "focus": 3})).unwrap();
    let paste_before = doc(&s);
    let paste_count = s.doc().unwrap().history.undo.len();
    let result = s.execute("edit.paste", &json!({})).unwrap();
    let pasted = doc(&s);
    assert_eq!(result["anchored"], 1, "defect oracle: latest object clipboard must win over old formatted text");
    assert_eq!(pasted.story(sid).unwrap().text, format!("OLD{}", designcraft_doc::OBJECT_MARK));
    assert_eq!(pasted.story(sid).unwrap().objects.len(), 1);
    assert_eq!(pasted.story(sid).unwrap().objects[0].item.shape, before.item(id).unwrap().shape);
    history(&mut s, sid, &paste_before, &pasted, paste_count);
}

#[test]
fn r9_reflow_object_cut_replaces_old_formatted_clipboard_public_paste_single_journal() {
    r9_object_clipboard("edit.cut", true);
}

#[test]
fn r9_reflow_object_copy_and_cut_without_text_clipboard_controls() {
    r9_object_clipboard("edit.copy", true);
    r9_object_clipboard("edit.cut", false);
}

#[test]
fn r9_reflow_formatted_text_cut_and_plain_paste_controls() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let made = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "OLD"})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    select_all(&mut s, sid);
    s.execute("type.char", &json!({"attrs": {"tracking": 75}})).unwrap();
    let before = doc(&s);
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    assert_eq!(s.execute("edit.cut", &json!({})).unwrap()["text"], "OLD");
    let cut = doc(&s);
    assert!(cut.story(sid).unwrap().is_empty());
    assert_eq!(s.journal.len(), journal + 1);
    history(&mut s, sid, &before, &cut, count);
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    assert_eq!(s.execute("edit.paste", &json!({})).unwrap()["formatted"], true);
    assert_eq!(doc(&s).story(sid).unwrap().text, "OLD");
    assert_eq!(doc(&s).story(sid).unwrap().format_after(0).over.tracking, Some(75.0));
    s.execute("text.select", &json!({"story": sid.0, "anchor": 3, "focus": 3})).unwrap();
    s.execute("edit.pasteWithoutFormatting", &json!({"text": " plain"})).unwrap();
    assert_eq!(doc(&s).story(sid).unwrap().text, "OLD plain");
    public_native_controls(&mut s, sid);
}

fn r9_resize(s: &mut Session, id: ItemId, width: f64, height: f64) {
    let rect = doc(s).item(id).unwrap().bounds();
    s.execute(
        "transform.resize",
        &json!({"ids": [id.0], "from": [rect.x0,rect.y0,rect.x1,rect.y1],
        "to": [rect.x0,rect.y0,rect.x0+width,rect.y0+height], "content": false}),
    )
    .unwrap();
}

fn r9_script_preference(position: &str, size: f64) -> Value {
    if position == "superscript" {
        json!({"advancedType": {"superscriptSize": size, "superscriptPosition": 20}})
    } else {
        json!({"advancedType": {"subscriptSize": size, "subscriptPosition": 20}})
    }
}

fn r9_source_page(d: &Document, sid: StoryId, anchor: u64) -> usize {
    let source = fresh(d, sid);
    assert!(!source.is_overset(), "fixture: actual source must fully fit the retained chain");
    let pos = d.story(sid).unwrap().anchor_pos(anchor).unwrap();
    let (fi, ..) = compose::caret(&source, pos).unwrap();
    d.page_of_item(source.frames[fi].frame).unwrap()
}

fn r9_running_values(d: &Document, bank: &Cache) -> Vec<String> {
    let vi = d.variable("Review9 Running").unwrap();
    let index = bank.running_index(d);
    (0..d.page_count()).map(|page| compose::vars::values(d, Some(page), Some(&index))[vi].clone()).collect()
}

fn r9_crossing_source(position: &str) -> (Session, StoryId, StoryId, u64) {
    let face = designcraft_fonts::FontDb::global().face("Source Serif 4", "Regular");
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Source Serif 4", "Regular"));
    assert_ne!(face.glyph_for('M'), 0);
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 2, "width": 612, "height": 792, "margins": 36, "facingPages": false})).unwrap();
    s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
    s.execute("document.preferences", &r9_script_preference(position, 50.0)).unwrap();
    let made = s.execute("frame.create", &json!({"spread": 0, "rect": [36,36,576,636], "content": "text", "text": "MMMM MMMM\nHead"})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    let first = ItemId(made["id"].as_u64().unwrap());
    select_all(&mut s, sid);
    s.execute(
        "type.char",
        &json!({"attrs": {"fontFamily": "Source Serif 4", "fontStyle": "Regular", "size": 12,
        "leading": {"kind": "points", "value": 14}, "position": "normal"}}),
    )
    .unwrap();
    s.execute(
        "type.para",
        &json!({"attrs": {"composer": "singleLine", "hyphenate": false,
        "keepLinesTogether": false, "keepFirst": 1, "keepLast": 1, "gridAlign": "none"}}),
    )
    .unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 9})).unwrap();
    s.execute("type.char", &json!({"attrs": {"position": position}})).unwrap();
    let old_wide = fresh(&doc(&s), sid);
    let changed = probe(&mut s, "document.preferences", r9_script_preference(position, 100.0));
    let new_wide = fresh(&changed, sid);
    assert_eq!(old_wide.line_count(), 2);
    assert_eq!(new_wide.line_count(), 2);
    let old_line = &old_wide.frames[0].lines[0];
    let new_line = &new_wide.frames[0].lines[0];
    let width = ((old_line.end_x - old_line.x0) + (new_line.end_x - new_line.x0)) / 2.0;
    assert!(width > 12.0 && width < 540.0, "fixture: finite measure derived from real shaped advances");
    r9_resize(&mut s, first, width, 600.0);
    let narrow_old = fresh(&doc(&s), sid);
    let narrow_changed = probe(&mut s, "document.preferences", r9_script_preference(position, 100.0));
    let narrow_new = fresh(&narrow_changed, sid);
    assert_eq!(narrow_old.line_count(), 2, "fixture: small script prefix stays on one actual line");
    assert_eq!(narrow_new.line_count(), 3, "fixture: large script prefix uses two actual lines");
    let bottom = |cs: &ComposedStory| {
        let line = cs.frames[0].lines.last().unwrap();
        line.baseline + line.descent - cs.frames[0].columns[0].y0
    };
    let height = (bottom(&narrow_old) + bottom(&narrow_new)) / 2.0;
    assert!(height > 12.0 && height < 600.0);
    r9_resize(&mut s, first, width, height);
    let second_spread = doc(&s).page_loc(1).unwrap().0;
    let second = s
        .execute(
            "frame.create",
            &json!({"spread": second_spread, "rect": [36,36,36.0+width,36.0+height],
        "content": "text", "text": "", "caret": false}),
        )
        .unwrap();
    // No text.thread command exists. Public Session::edit is fixture construction only.
    s.edit(|d, _| {
        d.thread(first, ItemId(second["id"].as_u64().unwrap()))?;
        Ok(())
    })
    .unwrap();
    s.execute("style.paragraph.create", &json!({"name": "Review9 Source Heading"})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 10, "focus": 14})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "Review9 Source Heading"})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 10, "focus": 10})).unwrap();
    let anchor = s.execute("anchor.create", &json!({"name": "Review9 Anchor"})).unwrap()["id"].as_u64().unwrap();
    s.execute(
        "variables.define",
        &json!({"name": "Review9 Running", "type": "runningHeader",
        "style": "Review9 Source Heading", "use": "firstOnPage"}),
    )
    .unwrap();
    s.execute("xref.defineFormat", &json!({"name": "Review9 Page", "definition": "<pageNum />"})).unwrap();
    let consumer = s.execute("frame.create", &json!({"spread": 0, "rect": [300,100,570,200], "content": "text", "text": ""})).unwrap();
    let consumer = StoryId(consumer["story"].as_u64().unwrap());
    s.execute("variables.insert", &json!({"name": "Review9 Running"})).unwrap();
    s.execute("text.insert", &json!({"text": " "})).unwrap();
    s.execute("xref.insert", &json!({"anchor": anchor, "format": "Review9 Page"})).unwrap();
    let before = doc(&s);
    let after = probe(&mut s, "document.preferences", r9_script_preference(position, 100.0));
    r9_shared_sources(&before, &after, true);
    assert_eq!(r9_source_page(&before, sid, anchor), 0, "fixture: actual source anchor starts on page 0");
    assert_eq!(r9_source_page(&after, sid, anchor), 1, "fixture: reject if actual source has no page crossing");
    let old_bank = Cache::new();
    let new_bank = Cache::new();
    assert_eq!(old_bank.xref_index(&before).page_index(anchor), Some(0));
    assert_eq!(new_bank.xref_index(&after).page_index(anchor), Some(1));
    assert_eq!(r9_running_values(&before, &old_bank), vec!["Head", "Head"]);
    assert_eq!(r9_running_values(&after, &new_bank), vec!["", "Head"]);
    assert_ne!(
        format!("{:?}", fresh(&before, consumer)),
        format!("{:?}", fresh(&after, consumer)),
        "fixture: real running and page-xref consumer output changes"
    );
    (s, sid, consumer, anchor)
}

fn r9_source_bank_change(position: &str) {
    let (mut s, sid, consumer, anchor) = r9_crossing_source(position);
    let before = doc(&s);
    let old_xrefs = s.cache.xref_index(&before);
    let old_running = s.cache.running_index(&before);
    same_composition(&fresh(&before, consumer), &s.cache.get(&before, consumer, None));
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("document.preferences", &r9_script_preference(position, 100.0)).unwrap();
    let after = doc(&s);
    r9_shared_sources(&before, &after, true);
    assert_eq!(r9_source_page(&after, sid, anchor), 1, "prerequisite: actual source crosses independently of warm indices");
    let fresh_bank = Cache::new();
    assert_eq!(fresh_bank.xref_index(&after).page_index(anchor), Some(1));
    assert_eq!(r9_running_values(&after, &fresh_bank), vec!["", "Head"]);
    let changed_xrefs = s.cache.xref_index(&after);
    assert_eq!(changed_xrefs.page_index(anchor), Some(1), "defect oracle: warm xref bank follows actual source");
    assert!(!Arc::ptr_eq(&old_xrefs, &changed_xrefs));
    let changed_running = s.cache.running_index(&after);
    assert!(!Arc::ptr_eq(&old_running, &changed_running));
    assert_eq!(r9_running_values(&after, &s.cache), r9_running_values(&after, &fresh_bank));
    same_composition(&fresh(&after, consumer), &s.cache.get(&after, consumer, None));
    let refs = s.execute("xref.list", &json!({})).unwrap();
    assert_eq!(refs[0]["text"], after.page_name(1));
    let anchors = s.execute("anchor.list", &json!({})).unwrap();
    assert_eq!(anchors[0]["page"], after.page_name(1));
    assert_eq!(s.journal.len(), journal + 1);
    history(&mut s, sid, &before, &after, count);
    public_native_controls(&mut s, consumer);
    s.cache.clear();
    same_composition(&fresh(&after, sid), &s.cache.get(&after, sid, None));
    same_composition(&fresh(&after, consumer), &s.cache.get(&after, consumer, None));
}

#[test]
fn r9_reflow_superscript_source_crossing_updates_real_warm_running_and_xref_banks() {
    r9_source_bank_change("superscript");
}

#[test]
fn r9_reflow_subscript_source_crossing_updates_real_warm_running_and_xref_banks() {
    r9_source_bank_change("subscript");
}

#[test]
fn r9_reflow_source_banks_preserve_unused_opposite_script_same_and_display_controls() {
    let (mut s, sid, consumer, anchor) = r9_crossing_source("superscript");
    for params in [
        json!({"advancedType": {"subscriptSize": 100, "subscriptPosition": 80}}),
        r9_script_preference("superscript", 50.0),
        json!({"baselineGrid": {"color": [1,2,3], "viewThreshold": 0.5}}),
    ] {
        let before = doc(&s);
        let xrefs = s.cache.xref_index(&before);
        let running = s.cache.running_index(&before);
        let source = s.cache.get(&before, sid, None);
        let old_consumer = s.cache.get(&before, consumer, None);
        s.execute("document.preferences", &params).unwrap();
        let after = doc(&s);
        r9_shared_sources(&before, &after, true);
        assert_eq!(r9_source_page(&after, sid, anchor), 0);
        same_composition(&fresh(&before, sid), &fresh(&after, sid));
        assert!(Arc::ptr_eq(&xrefs, &s.cache.xref_index(&after)), "unused source preferences preserve the actual bank");
        assert!(Arc::ptr_eq(&running, &s.cache.running_index(&after)));
        assert!(Arc::ptr_eq(&source, &s.cache.get(&after, sid, None)));
        assert!(Arc::ptr_eq(&old_consumer, &s.cache.get(&after, consumer, None)));
        public_native_controls(&mut s, consumer);
    }
}

fn r9_anchor(s: &mut Session, sid: StoryId, relative: &str) {
    let shape = s.execute("frame.create", &json!({"spread": 0, "rect": [100,100,110,110], "content": "unassigned"})).unwrap();
    s.execute(
        "anchored.insert",
        &json!({"ids": [shape["id"]], "story": sid.0, "pos": 0,
        "position": "custom", "xRelative": relative, "yRelative": relative, "refPoint": 0, "objectPoint": 0,
        "xOffset": 0, "yOffset": 0, "keepWithinColumn": false}),
    )
    .unwrap();
}

fn r9_origin(cs: &ComposedStory) -> designcraft_geom::Point {
    let objects: Vec<_> = cs.frames.iter().flat_map(|f| &f.objects).collect();
    assert_eq!(objects.len(), 1, "fixture: one actually composed custom object");
    objects[0].origin
}

fn r9_margin_fixture(relative: &str) -> (Session, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1, "width": 612, "height": 792, "margins": 36, "facingPages": false})).unwrap();
    let made = s.execute("frame.create", &json!({"rect": [100,100,300,300], "content": "text", "text": "A"})).unwrap();
    let sid = StoryId(made["story"].as_u64().unwrap());
    r9_anchor(&mut s, sid, relative);
    (s, sid)
}

#[test]
fn r9_reflow_page_margin_coordinates_update_warm_cache_with_shared_source_allocations() {
    for cold in [false, true] {
        let (mut s, sid) = r9_margin_fixture("pageMargin");
        let before = doc(&s);
        let old = fresh(&before, sid);
        same_composition(&old, &s.cache.get(&before, sid, None));
        let expected = probe(&mut s, "layout.marginsAndColumns", json!({"pages": [0], "margins": 72, "adjustLayout": false}));
        r9_shared_sources(&before, &expected, false);
        let changed = fresh(&expected, sid);
        let delta = r9_origin(&changed) - r9_origin(&old);
        assert!((delta.x - 36.0).abs() < 1e-9 && (delta.y - 36.0).abs() < 1e-9, "fixture: consumed PageMargin input actually moves the object");
        if cold {
            s.cache.clear();
        }
        s.cache.get(&before, sid, None);
        let count = s.doc().unwrap().history.undo.len();
        let journal = s.journal.len();
        s.execute("layout.marginsAndColumns", &json!({"pages": [0], "margins": 72, "adjustLayout": false})).unwrap();
        let after = doc(&s);
        assert_eq!(full_doc(&after), full_doc(&expected));
        r9_shared_sources(&before, &after, false);
        assert_eq!(r9_origin(&s.cache.get(&after, sid, None)), r9_origin(&changed), "defect oracle: warm object follows consumed margin");
        same_composition(&changed, &s.cache.get(&after, sid, None));
        assert_eq!(s.journal.len(), journal + 1);
        history(&mut s, sid, &before, &after, count);
    }
}

#[test]
fn r9_reflow_page_edge_anchor_text_frame_and_same_margin_controls() {
    for relative in ["pageEdge", "anchor", "textFrame", "pageMargin"] {
        let (mut s, sid) = r9_margin_fixture(relative);
        let before = doc(&s);
        let old = s.cache.get(&before, sid, None);
        let margins = if relative == "pageMargin" { 36 } else { 72 };
        s.execute("layout.marginsAndColumns", &json!({"pages": [0], "margins": margins, "adjustLayout": false})).unwrap();
        let after = doc(&s);
        r9_shared_sources(&before, &after, false);
        same_composition(&old, &fresh(&after, sid));
        assert!(Arc::ptr_eq(&old, &s.cache.get(&after, sid, None)), "unused margin input preserves actual composition cache");
        public_native_controls(&mut s, sid);
    }
}

fn r9_margin_removal(relative: &str, cold: bool, commit: bool) {
    let (mut s, sid) = primary_chain(false);
    r9_anchor(&mut s, sid, relative);
    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    let params = json!({"pages": [0], "margins": 72, "adjustLayout": false});
    let expected = probe(&mut s, "layout.marginsAndColumns", params.clone());
    r9_shared_sources(&before, &expected, false);
    let old = fresh(&before, sid);
    let changed = fresh(&expected, sid);
    assert!(old.is_overset() && changed.is_overset(), "fixture: real removed primary capacity still oversets");
    if relative == "pageMargin" {
        assert_ne!(r9_origin(&old), r9_origin(&changed), "fixture: this coordinate is consumed");
    } else {
        same_composition(&old, &changed);
    }
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    apply(&mut s, "layout.marginsAndColumns", params, cold, commit);
    assert_eq!(s.journal.len(), journal + 1);
    if relative == "pageMargin" {
        growth(&mut s, sid, Some(removed), &before, count);
    } else {
        preserved(&mut s, sid, removed, &before, &expected, count);
    }
}

#[test]
fn r9_reflow_consumed_page_margin_resumes_removed_primary_capacity() {
    for cold in [false, true] {
        for commit in [false, true] {
            r9_margin_removal("pageMargin", cold, commit);
        }
    }
}

#[test]
fn r9_reflow_unused_page_margin_preserves_removed_primary_capacity_controls() {
    for relative in ["pageEdge", "anchor", "textFrame"] {
        for cold in [false, true] {
            for commit in [false, true] {
                r9_margin_removal(relative, cold, commit);
            }
        }
    }
}

#[test]
fn r9_reflow_vertical_and_frame_grid_override_unused_setting_controls() {
    for vertical in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let made = s.execute("frame.create", &json!({"rect": [36,36,300,300], "content": "text", "text": "M2", "vertical": vertical})).unwrap();
        let sid = StoryId(made["story"].as_u64().unwrap());
        let id = ItemId(made["id"].as_u64().unwrap());
        select_all(&mut s, sid);
        s.execute("type.para", &json!({"attrs": {"gridAlign": "allLines"}})).unwrap();
        if !vertical {
            s.edit(|d, _| {
                d.item_mut(id).unwrap().text_frame_mut().unwrap().options.baseline_grid = Some((12.0, 0.0));
                Ok(())
            })
            .unwrap();
        }
        let before = doc(&s);
        let old = s.cache.get(&before, sid, None);
        s.execute(
            "document.preferences",
            &json!({"baselineGrid": {"start": 5, "increment": 36},
            "advancedType": {"superscriptSize": 100, "subscriptSize": 100}}),
        )
        .unwrap();
        let after = doc(&s);
        r9_shared_sources(&before, &after, true);
        same_composition(&old, &fresh(&after, sid));
        assert!(Arc::ptr_eq(&old, &s.cache.get(&after, sid, None)), "grid-free/override and normal-script controls preserve cache");
        public_native_controls(&mut s, sid);
    }
}

// REVIEW11: resizing the primary chain must not prune artwork inside a trailing group.
#[derive(Clone, Copy, Debug)]
enum R11Tail {
    Singleton,
    Ungrouped,
    Grouped,
    Nested,
}

struct R11Prune {
    session: Session,
    story: StoryId,
    first: ItemId,
    last: ItemId,
    protected: Vec<ItemId>,
}

fn r11_prune_fixture(tail: R11Tail, reflow: bool) -> R11Prune {
    let face = designcraft_fonts::FontDb::global().face("Source Serif 4", "Regular");
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Source Serif 4", "Regular"));
    assert_ne!(face.glyph_for('A'), 0);
    assert_ne!(face.glyph_for('B'), 0);
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
    s.execute(
        "file.new",
        &json!({"pages": 2, "width": 612, "height": 792, "margins": 36,
        "columns": 1, "primaryTextFrame": true, "facingPages": false}),
    )
    .unwrap();
    let sid = doc(&s).settings.primary_story.unwrap();
    let frames = doc(&s).story(sid).unwrap().frames.clone();
    assert_eq!(frames.len(), 2);
    for id in &frames {
        let bounds = doc(&s).item(*id).unwrap().bounds();
        s.execute(
            "transform.resize",
            &json!({"ids": [id.0], "from": [bounds.x0,bounds.y0,bounds.x1,bounds.y1],
            "to": [36,36,576,54], "content": false}),
        )
        .unwrap();
    }
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    s.execute("text.insert", &json!({"text": "A\nB"})).unwrap();
    select_all(&mut s, sid);
    s.execute(
        "type.char",
        &json!({"attrs": {"fontFamily": "Source Serif 4", "fontStyle": "Regular",
        "size": 12, "leading": {"kind": "points", "value": 14}}}),
    )
    .unwrap();
    s.execute(
        "type.para",
        &json!({"attrs": {"composer": "singleLine", "hyphenate": false,
        "keepLinesTogether": false, "keepFirst": 1, "keepLast": 1,
        "spaceBefore": 0, "spaceAfter": 0, "gridAlign": "none"}}),
    )
    .unwrap();
    let spread = doc(&s).page_loc(1).unwrap().0;
    let mut protected = Vec::new();
    let mut root = frames[1];
    if !matches!(tail, R11Tail::Singleton) {
        let artwork = ItemId(
            s.execute(
                "frame.create",
                &json!({"spread": spread, "rect": [100,100,140,140],
                "content": "unassigned", "shape": "ellipse"}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        protected.push(artwork);
        if matches!(tail, R11Tail::Grouped | R11Tail::Nested) {
            root = ItemId(s.execute("object.group", &json!({"ids": [frames[1].0, artwork.0]})).unwrap()["id"].as_u64().unwrap());
            protected.push(root);
        }
        if matches!(tail, R11Tail::Nested) {
            let other = ItemId(
                s.execute("frame.create", &json!({"spread": spread, "rect": [160,100,200,140], "content": "unassigned"})).unwrap()["id"]
                    .as_u64()
                    .unwrap(),
            );
            protected.push(other);
            root = ItemId(s.execute("object.group", &json!({"ids": [root.0, other.0]})).unwrap()["id"].as_u64().unwrap());
            protected.push(root);
        }
    }
    s.execute("prefs.set", &json!({"smartTextReflow": reflow})).unwrap();
    s.execute("selection.set", &json!({"ids": [frames[0].0]})).unwrap();
    let before = doc(&s);
    before.check().unwrap();
    assert_eq!(before.page_count(), 2);
    assert_eq!(before.page_of_item(frames[0]), Some(0));
    assert_eq!(before.page_of_item(frames[1]), Some(1));
    assert_eq!(before.story(sid).unwrap().text, "A\nB");
    assert_eq!(before.story(sid).unwrap().frames, frames);
    let roots = &before.spreads[spread].items;
    if matches!(tail, R11Tail::Ungrouped) {
        assert_eq!(roots.len(), 2, "control: separate artwork prevents page pruning");
    } else {
        assert_eq!(roots.len(), 1, "fixture: the tail page has exactly one root");
        assert_eq!(roots[0].id, root);
    }
    assert_eq!(before.top_level_of(frames[1]), Some(root));
    assert_eq!(
        before.find(frames[1]).unwrap().path.len(),
        match tail {
            R11Tail::Grouped => 2,
            R11Tail::Nested => 3,
            _ => 1,
        }
    );
    let specs = compose::frame_specs(&before, sid);
    assert_eq!(specs.len(), 2);
    for spec in &specs {
        assert_eq!((spec.area.width(), spec.area.height()), (540.0, 18.0));
    }
    let flow = fresh(&before, sid);
    assert!(!flow.is_overset(), "fixture: the original primary story fits the chain");
    assert_eq!(flow.line_count(), 2);
    for id in &frames {
        assert!(!flow.frame(*id).unwrap().range.is_empty(), "fixture: both primary frames hold real text");
        assert_eq!(flow.frame(*id).unwrap().lines.len(), 1);
    }
    same_composition(&flow, &s.cache.get(&before, sid, None));
    R11Prune { session: s, story: sid, first: frames[0], last: frames[1], protected }
}

fn r11_resize_preview(case: &mut R11Prune) -> (Arc<Document>, Value) {
    let params = json!({"ids": [case.first.0], "from": [36,36,576,54], "to": [36,36,576,96], "content": false});
    case.session.begin_interaction("Review11 primary resize").unwrap();
    case.session.preview("transform.resize", params.clone()).unwrap();
    let preview = doc(&case.session);
    preview.check().unwrap();
    assert_eq!(preview.page_count(), 2, "preview must defer pruning");
    assert_eq!(preview.story(case.story).unwrap().frames, vec![case.first, case.last]);
    assert_eq!(preview.story(case.story).unwrap().text, "A\nB");
    assert_eq!(preview.item(case.first).unwrap().bounds(), designcraft_geom::Rect::new(36.0, 36.0, 576.0, 96.0));
    let flow = fresh(&preview, case.story);
    assert!(!flow.is_overset());
    assert_eq!(flow.frame(case.first).unwrap().lines.len(), 2, "fixture: resizing really moves both lines into the first frame");
    assert_eq!(flow.frame(case.first).unwrap().range.end, preview.story(case.story).unwrap().len());
    assert!(flow.frame(case.last).unwrap().range.is_empty(), "fixture: the previously occupied tail is now empty");
    for id in &case.protected {
        assert!(preview.item(*id).is_some(), "fixture: artwork and groups still exist before Commit");
    }
    same_composition(&flow, &case.session.cache.get(&preview, case.story, None));
    (preview, params)
}

fn r11_commit_case(tail: R11Tail, reflow: bool, cold: bool) {
    let mut case = r11_prune_fixture(tail, reflow);
    let before = doc(&case.session);
    let selection = case.session.doc().unwrap().selection.clone();
    let count = case.session.doc().unwrap().history.undo.len();
    let journal = case.session.journal.clone();
    let (preview, params) = r11_resize_preview(&mut case);
    assert_eq!(case.session.doc().unwrap().history.undo.len(), count);
    assert_eq!(case.session.journal, journal);
    if cold {
        case.session.cache.clear();
    }
    case.session.commit_interaction().unwrap();
    let after = doc(&case.session);
    let prunes = reflow && matches!(tail, R11Tail::Singleton);
    for id in &case.protected {
        assert_eq!(after.item(*id), before.item(*id), "defect oracle: {tail:?} artwork/group {id:?} survives the resize");
    }
    if prunes {
        assert_eq!(after.page_count(), 1, "control: a bare empty primary frame still allows pruning");
        assert_eq!(after.story(case.story).unwrap().frames, vec![case.first]);
        assert!(after.item(case.last).is_none());
        assert_eq!(after.item(case.first), preview.item(case.first));
    } else {
        assert_eq!(*after, *preview, "only the requested resize may change this document");
        assert_eq!(after.page_count(), 2);
        assert_eq!(after.story(case.story).unwrap().frames, vec![case.first, case.last]);
        assert_eq!(after.item(case.last), before.item(case.last));
    }
    assert_eq!(after.story(case.story).unwrap().text, before.story(case.story).unwrap().text);
    assert!(!fresh(&after, case.story).is_overset());
    assert!(case.session.doc().unwrap().interaction.is_none());
    assert_eq!(case.session.doc().unwrap().selection, selection);
    assert_eq!(case.session.doc().unwrap().history.undo.len(), count + 1);
    assert_eq!(case.session.doc().unwrap().history.undo.last().unwrap().label, "Review11 primary resize");
    assert_eq!(case.session.journal.len(), journal.len() + 1);
    assert_eq!(case.session.journal.last(), Some(&("transform.resize".to_string(), params)));
    // Exact snapshots, public native ZIP save/load/open and composition at all three history states.
    history(&mut case.session, case.story, &before, &after, count);
    assert_eq!(case.session.doc().unwrap().selection, selection);
}

#[test]
fn r11_reflow_grouped_tail_artwork_survives_committed_resize() {
    for cold in [false, true] {
        r11_commit_case(R11Tail::Grouped, true, cold);
    }
}

#[test]
fn r11_reflow_nested_tail_artwork_survives_committed_resize() {
    for cold in [false, true] {
        r11_commit_case(R11Tail::Nested, true, cold);
    }
}

#[test]
fn r11_reflow_bare_empty_tail_still_prunes_on_committed_resize() {
    for cold in [false, true] {
        r11_commit_case(R11Tail::Singleton, true, cold);
    }
}

#[test]
fn r11_reflow_ungrouped_artwork_preserves_tail_page_on_committed_resize() {
    for cold in [false, true] {
        r11_commit_case(R11Tail::Ungrouped, true, cold);
    }
}

#[test]
fn r11_reflow_off_keeps_empty_tail_and_artwork_on_committed_resize() {
    for tail in [R11Tail::Singleton, R11Tail::Ungrouped, R11Tail::Grouped, R11Tail::Nested] {
        r11_commit_case(tail, false, true);
    }
}

#[test]
fn r11_reflow_cancel_restores_occupied_tail_and_artwork() {
    for tail in [R11Tail::Singleton, R11Tail::Ungrouped, R11Tail::Grouped, R11Tail::Nested] {
        let mut case = r11_prune_fixture(tail, true);
        let before = doc(&case.session);
        let selection = case.session.doc().unwrap().selection.clone();
        let count = case.session.doc().unwrap().history.undo.len();
        let journal = case.session.journal.clone();
        r11_resize_preview(&mut case);
        case.session.cache.clear();
        case.session.cancel_interaction();
        assert!(Arc::ptr_eq(&doc(&case.session), &before));
        assert_eq!(case.session.doc().unwrap().selection, selection);
        assert!(case.session.doc().unwrap().interaction.is_none());
        assert_eq!(case.session.doc().unwrap().history.undo.len(), count);
        assert_eq!(case.session.journal, journal);
        assert!(!fresh(&doc(&case.session), case.story).frame(case.last).unwrap().range.is_empty());
        public_native_controls(&mut case.session, case.story);
    }
}

// REVIEW14: a cache hit must not bypass a consumed variable used only by a printed child story.
const R14_NESTED_VALUE: &str = "Review14 Nested Value";
const R14_UNUSED_VALUE: &str = "Review14 Unused Value";

#[derive(Clone, Copy, Debug)]
enum R14VariableEdit {
    Consumed,
    Unused,
    Unchanged,
}

fn r14_child_flow(cs: &ComposedStory, child: designcraft_doc::CellAddr) -> &ComposedStory {
    if let Some(id) = child.footnote_id() {
        cs.frames.iter().flat_map(|frame| &frame.notes).find(|note| note.id == id).unwrap().text.as_ref()
    } else {
        let table = cs.frames.iter().flat_map(|frame| &frame.tables).find(|table| table.table == child.table).unwrap();
        table.cell(child.row, child.col).unwrap().text.as_ref()
    }
}

fn r14_child_glyphs(cs: &ComposedStory, child: designcraft_doc::CellAddr) -> Vec<Value> {
    let flow = r14_child_flow(cs, child);
    assert!(!flow.is_overset(), "fixture: the printed child itself fits");
    let glyphs: Vec<_> = flow
        .frames
        .iter()
        .flat_map(|frame| &frame.lines)
        .flat_map(|line| &line.glyphs)
        .filter(|glyph| glyph.visible)
        .map(|g| json!([g.gid, g.byte, g.len, g.x, g.y, g.adv, g.sx, g.sy]))
        .collect();
    assert!(!glyphs.is_empty(), "fixture: inspect actual child glyphs, not only the stored variable marker");
    glyphs
}

fn r14_nested_variable_fixture(note: bool) -> (Session, StoryId, designcraft_doc::CellAddr, Vec<ItemId>) {
    let (mut s, sid) = primary_chain(false);
    for (name, text) in [(R14_NESTED_VALUE, "A"), (R14_UNUSED_VALUE, "Unused")] {
        s.execute("variables.define", &json!({"name": name, "type": "custom", "text": text})).unwrap();
    }
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    if note {
        s.execute("footnote.insert", &json!({"text": ""})).unwrap();
    } else {
        s.execute("table.insert", &json!({"rows": 1, "cols": 1, "width": 300})).unwrap();
    }
    let child = s.doc().unwrap().selection.text.unwrap().cell.unwrap();
    assert_eq!(child.footnote_id().is_some(), note);
    s.execute("variables.insert", &json!({"name": R14_NESTED_VALUE})).unwrap();
    let at = doc(&s).story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": at, "focus": at})).unwrap();

    let original = doc(&s);
    original.check().unwrap();
    let outer = original.story(sid).unwrap();
    let frames = outer.frames.clone();
    let marker = designcraft_doc::vars::var_char(original.variable(R14_NESTED_VALUE).unwrap()).unwrap().to_string();
    assert!(!compose::vars::has_vars(outer) && !compose::xref::has_xrefs(outer), "fixture: no outer reference-bank signature");
    assert_eq!(original.text_story(sid, Some(child)).unwrap().text, marker);
    assert!(original.endnote_story.is_none() && original.data_merge.placeholders.is_empty());
    assert!(original.all_items().into_iter().all(|id| original.item(id).unwrap().live_caption.is_none()));
    assert_eq!(original.page_count(), frames.len(), "fixture: one primary frame per page");
    assert!((3..=32).contains(&frames.len()), "fixture: bounded occupied chain");
    let full = fresh(&original, sid);
    assert!(!full.is_overset(), "fixture: the original primary chain fits");
    assert_eq!(r14_child_flow(&full, child).text_len, marker.len());
    r14_child_glyphs(&full, child);
    let removed = frames[1..].to_vec();
    for frame in &removed {
        assert!(!full.frame(*frame).unwrap().range.is_empty(), "fixture: every removed continuation was occupied");
    }

    // Public setup shared with the original base: it must not rely on the new deletion guard.
    s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
    for frame in removed.iter().rev() {
        let before = doc(&s);
        let page = before.page_of_item(*frame).unwrap();
        assert_eq!(page + 1, before.page_count());
        s.execute("layout.pages.delete", &json!({"pages": [page]})).unwrap();
        assert_eq!(doc(&s).page_count() + 1, before.page_count());
    }
    s.execute("prefs.set", &json!({"smartTextReflow": true})).unwrap();
    let reduced = doc(&s);
    assert_eq!(reduced.page_count(), 1);
    assert_eq!(reduced.story(sid).unwrap().frames, vec![frames[0]]);
    assert_eq!(reduced.story(sid).unwrap().text, outer.text);
    assert_eq!(reduced.text_story(sid, Some(child)), original.text_story(sid, Some(child)));
    assert!(removed.iter().all(|id| reduced.item(*id).is_none()));
    let flow = fresh(&reduced, sid);
    assert!(flow.is_overset(), "fixture: explicit removal leaves actual primary overset");
    assert_eq!(r14_child_glyphs(&flow, child), r14_child_glyphs(&full, child), "the child is still printed on the retained page");
    public_native_controls(&mut s, sid);
    (s, sid, child, removed)
}

fn r14_nested_variable_case(note: bool, edit: R14VariableEdit, cold: bool, commit: bool) {
    let (mut s, sid, child, removed) = r14_nested_variable_fixture(note);
    let before = doc(&s);
    let selection = s.doc().unwrap().selection.clone();
    let old = fresh(&before, sid);
    let old_glyphs = r14_child_glyphs(&old, child);
    let (name, value) = match edit {
        R14VariableEdit::Consumed => (R14_NESTED_VALUE, "MMMMMMMM"),
        R14VariableEdit::Unused => (R14_UNUSED_VALUE, "MMMMMMMM"),
        R14VariableEdit::Unchanged => (R14_NESTED_VALUE, "A"),
    };
    let params = json!({"name": name, "type": "custom", "text": value});
    let expected = probe(&mut s, "variables.define", params.clone());
    r9_shared_sources(&before, &expected, true);
    assert_eq!(before.story(sid), expected.story(sid), "the edit changes no outer or child source bytes");
    assert_eq!(before.text_variables == expected.text_variables, matches!(edit, R14VariableEdit::Unchanged));
    let changed = fresh(&expected, sid);
    assert!(changed.is_overset(), "fixture: the independent edited layout still needs primary capacity");
    let expected_glyphs = r14_child_glyphs(&changed, child);
    if matches!(edit, R14VariableEdit::Consumed) {
        assert_ne!(old_glyphs, expected_glyphs, "fixture: the edited variable changes actual printed child glyphs");
    } else {
        same_composition(&old, &changed);
    }
    same_composition(&old, &s.cache.get(&before, sid, None));
    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    // Ordinary execute warms its pre-command composition even after clear; Commit also starts
    // from the gesture's original document after its preview cache has optionally been cleared.
    apply(&mut s, "variables.define", params.clone(), cold, commit);
    let after = doc(&s);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert!(s.prefs.smart_text_reflow);
    assert_eq!(after.settings.primary_story, Some(sid));
    assert_eq!(after.text_story(sid, Some(child)), before.text_story(sid, Some(child)));
    assert!(removed.iter().all(|id| after.item(*id).is_none()), "explicitly deleted frame ids stay deleted");
    assert_eq!(s.journal.len(), journal + 1, "one variable command owns the journal entry");
    assert_eq!(s.journal.last(), Some(&("variables.define".to_string(), params)));
    assert_eq!(r14_child_glyphs(&fresh(&after, sid), child), expected_glyphs);
    if matches!(edit, R14VariableEdit::Consumed) {
        // This is the introduced regression oracle. The helpers also require actual fit and
        // full-document Undo/Redo, inspect/story queries, native ZIP load and public reopen.
        growth(&mut s, sid, Some(removed[0]), &before, count);
    } else {
        preserved(&mut s, sid, removed[0], &before, &expected, count);
    }
    assert_eq!(s.doc().unwrap().selection, selection);
}

fn r14_nested_variable_matrix(note: bool, edit: R14VariableEdit) {
    for cold in [false, true] {
        for commit in [false, true] {
            r14_nested_variable_case(note, edit, cold, commit);
        }
    }
}

fn r14_consumed_variable_matrix(note: bool, commit: bool) {
    for cold in [false, true] {
        r14_nested_variable_case(note, R14VariableEdit::Consumed, cold, commit);
    }
}

#[test]
fn r14_reflow_nested_note_consumed_variable_execute_resumes_removed_capacity() {
    r14_consumed_variable_matrix(true, false);
}

#[test]
fn r14_reflow_nested_cell_consumed_variable_execute_resumes_removed_capacity() {
    r14_consumed_variable_matrix(false, false);
}

#[test]
fn r14_reflow_nested_note_consumed_variable_commit_resumes_removed_capacity() {
    r14_consumed_variable_matrix(true, true);
}

#[test]
fn r14_reflow_nested_cell_consumed_variable_commit_resumes_removed_capacity() {
    r14_consumed_variable_matrix(false, true);
}

#[test]
fn r14_reflow_nested_note_unused_variable_preserves_removed_capacity() {
    r14_nested_variable_matrix(true, R14VariableEdit::Unused);
}

#[test]
fn r14_reflow_nested_cell_unused_variable_preserves_removed_capacity() {
    r14_nested_variable_matrix(false, R14VariableEdit::Unused);
}

#[test]
fn r14_reflow_nested_note_same_variable_preserves_removed_capacity() {
    r14_nested_variable_matrix(true, R14VariableEdit::Unchanged);
}

#[test]
fn r14_reflow_nested_cell_same_variable_preserves_removed_capacity() {
    r14_nested_variable_matrix(false, R14VariableEdit::Unchanged);
}

// REVIEW17: fitting primary text must refresh a changed child-only variable, without needing growth.
fn r17_fitting_child_variable(note: bool, commit: bool, cold: bool, smart_text_reflow: bool) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1, "width": 612, "height": 792, "margins": 36, "primaryTextFrame": true, "facingPages": false})).unwrap();
    let sid = doc(&s).settings.primary_story.unwrap();
    for (name, text) in [(R14_NESTED_VALUE, "A"), (R14_UNUSED_VALUE, "Unused")] {
        s.execute("variables.define", &json!({"name": name, "type": "custom", "text": text})).unwrap();
    }
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    s.execute("text.insert", &json!({"text": "Body"})).unwrap();
    if note {
        s.execute("footnote.insert", &json!({"text": ""})).unwrap();
    } else {
        s.execute("table.insert", &json!({"rows": 1, "cols": 1, "width": 300})).unwrap();
    }
    let child = s.doc().unwrap().selection.text.unwrap().cell.unwrap();
    assert_eq!(child.footnote_id().is_some(), note);
    s.execute("variables.insert", &json!({"name": R14_NESTED_VALUE})).unwrap();
    let at = doc(&s).story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": at, "focus": at})).unwrap();
    s.execute("prefs.set", &json!({"smartTextReflow": smart_text_reflow})).unwrap();
    let original = doc(&s);
    original.check().unwrap();
    let outer = original.story(sid).unwrap();
    assert!(!compose::vars::has_vars(outer) && !compose::xref::has_xrefs(outer));
    assert!(original.endnote_story.is_none() && original.data_merge.placeholders.is_empty());
    assert_eq!((original.page_count(), outer.frames.len()), (1, 1));
    let reference = fresh(&original, sid);
    assert!(!reference.is_overset(), "fixture: primary and printed child fit before the edit");
    let old_glyphs = r14_child_glyphs(&reference, child);
    let warm = s.cache.get(&original, sid, None);
    same_composition(&reference, &warm);

    // A real but unused definition edit must keep the actual cache entry and all capacity/geometry.
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let unused = json!({"name": R14_UNUSED_VALUE, "type": "custom", "text": "Unused changed"});
    apply(&mut s, "variables.define", unused.clone(), false, commit);
    let before = doc(&s);
    let mut expected_doc = full_doc(&original);
    expected_doc["textVariables"][original.variable(R14_UNUSED_VALUE).unwrap()]["text"] = json!("Unused changed");
    assert_eq!(full_doc(&before), expected_doc, "only the unused definition changes");
    r9_shared_sources(&original, &before, true);
    assert_eq!(before.spreads, original.spreads);
    assert_eq!(before.story(sid), original.story(sid));
    assert_eq!(before.page_count(), 1);
    assert!(Arc::ptr_eq(&warm, &s.cache.get(&before, sid, None)), "unused definitions retain the warm primary composition");
    same_composition(&reference, &fresh(&before, sid));
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert!(Arc::ptr_eq(&s.doc().unwrap().history.undo.last().unwrap().doc, &original));
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last(), Some(&("variables.define".to_string(), unused)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(&s), &original));
    assert_eq!(full_doc(&doc(&s)), full_doc(&original));
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert_eq!(s.doc().unwrap().history.redo.len(), 1);
    assert!(Arc::ptr_eq(&warm, &s.cache.get(&doc(&s), sid, None)));
    same_composition(&reference, &s.cache.get(&doc(&s), sid, None));
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(&s), &before));
    assert_eq!(full_doc(&doc(&s)), full_doc(&before));
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert!(s.doc().unwrap().history.redo.is_empty());
    assert!(Arc::ptr_eq(&warm, &s.cache.get(&doc(&s), sid, None)));
    same_composition(&reference, &s.cache.get(&doc(&s), sid, None));

    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let params = json!({"name": R14_NESTED_VALUE, "type": "custom", "text": "MMMMMMMM"});
    // The legacy cold cases exercise old-snapshot capture. Warm cases must retain the actual entry
    // through preview: clearing it would hide the disabled-reflow commit bug.
    assert!(Arc::ptr_eq(&warm, &s.cache.get(&before, sid, None)));
    apply(&mut s, "variables.define", params.clone(), cold, commit);
    let after = doc(&s);
    after.check().unwrap();
    let mut expected_doc = full_doc(&before);
    expected_doc["textVariables"][before.variable(R14_NESTED_VALUE).unwrap()]["text"] = json!("MMMMMMMM");
    assert_eq!(full_doc(&after), expected_doc, "only the consumed definition changes");
    r9_shared_sources(&before, &after, true);
    assert_eq!(after.spreads, before.spreads);
    assert_eq!(after.story(sid), before.story(sid));
    assert_eq!(after.page_count(), 1, "refresh must not generate unnecessary capacity");
    let expected = fresh(&after, sid);
    assert!(!expected.is_overset(), "fixture: changed primary and printed child still fit");
    let expected_glyphs = r14_child_glyphs(&expected, child);
    assert_ne!(old_glyphs, expected_glyphs, "fixture: definition edit changes actual printed child glyphs");
    let cached = s.cache.get(&after, sid, None);
    assert_eq!(r14_child_glyphs(&cached, child), expected_glyphs, "fitting primary cache must show the changed child value");
    same_composition(&expected, &cached);
    assert!(!Arc::ptr_eq(&warm, &cached), "consumed child references replace the old composition");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert!(Arc::ptr_eq(&s.doc().unwrap().history.undo.last().unwrap().doc, &before));
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last(), Some(&("variables.define".to_string(), params)));

    // History restores definitions on the same source allocations, so cached output must follow too.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(&s), &before));
    assert_eq!(full_doc(&doc(&s)), full_doc(&before));
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert_eq!(s.doc().unwrap().history.redo.len(), 1);
    let restored = s.cache.get(&doc(&s), sid, None);
    assert_eq!(r14_child_glyphs(&restored, child), old_glyphs);
    same_composition(&fresh(&before, sid), &restored);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&doc(&s), &after));
    assert_eq!(full_doc(&doc(&s)), full_doc(&after));
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert!(s.doc().unwrap().history.redo.is_empty());
    let restored = s.cache.get(&doc(&s), sid, None);
    assert_eq!(r14_child_glyphs(&restored, child), expected_glyphs);
    same_composition(&expected, &restored);
}

#[test]
fn r17_fitting_note_variable_execute_refreshes_child_cache() {
    r17_fitting_child_variable(true, false, true, true);
}

#[test]
fn r17_fitting_cell_variable_commit_refreshes_child_cache() {
    r17_fitting_child_variable(false, true, true, true);
}

// REVIEW28: a warm committed child reference refreshes with either reflow preference; unused
// definitions retain the same composition. Each case checks both edits and their full Undo/Redo.
#[test]
fn r28_fitting_note_variable_warm_commit_with_reflow_off() {
    r17_fitting_child_variable(true, true, false, false);
}

#[test]
fn r28_fitting_cell_variable_warm_commit_with_reflow_off() {
    r17_fitting_child_variable(false, true, false, false);
}

#[test]
fn r28_fitting_note_variable_warm_commit_with_reflow_on() {
    r17_fitting_child_variable(true, true, false, true);
}

#[test]
fn r28_fitting_cell_variable_warm_commit_with_reflow_on() {
    r17_fitting_child_variable(false, true, false, true);
}

// External footnote layout changes must refresh consumed references without restoring unused capacity.
mod r30_rf_footnote_probe {
    use super::*;

    const CROSSING_SPACE: f64 = 36.0;
    const NON_CROSSING_SPACE: f64 = 1.0;

    #[derive(Clone, Copy)]
    struct Ids {
        primary: StoryId,
        source: StoryId,
        first: ItemId,
        second: ItemId,
        anchor: u64,
    }
    struct Case {
        session: Session,
        ids: Ids,
        removed: ItemId,
        height: f64,
    }
    #[derive(Debug)]
    struct Oracle {
        page: Option<usize>,
        text: String,
        source_fits: bool,
        note_stays_in_first: bool,
    }

    // Every call owns a separate bank. Session.cache is never the oracle.
    fn independent(d: &Document, ids: Ids) -> Oracle {
        let bank = Cache::new();
        let index = bank.xref_index(d);
        let flow = bank.get(d, ids.source, None);
        let refs = &d.story(ids.primary).unwrap().xrefs;
        assert_eq!(refs.len(), 1, "PRECONDITION: exactly one consumed page reference");
        Oracle {
            page: index.page_index(ids.anchor),
            text: compose::xref::xref_text(d, &refs[0], Some(&index)),
            source_fits: !flow.is_overset(),
            note_stays_in_first: flow.frame(ids.first).is_some_and(|f| f.notes.len() == 1),
        }
    }

    fn same_allocations(a: &Document, b: &Document) -> bool {
        a.stories.len() == b.stories.len()
            && a.stories.iter().all(|(id, story)| b.stories.get(id).is_some_and(|other| Arc::ptr_eq(story, other)))
            && a.spreads.len() == b.spreads.len()
            && a.spreads.iter().zip(&b.spreads).all(|(old, new)| Arc::ptr_eq(old, new))
            && Arc::ptr_eq(&a.styles, &b.styles)
    }

    fn option_only(a: &Document, b: &Document, value: f64) {
        assert_eq!(b.footnote_options.space_before, value);
        assert!(same_allocations(a, b), "PRECONDITION: all story/spread/style Arcs must stay shared");
        let mut normalized: Document = b.clone();
        normalized.footnote_options.space_before = a.footnote_options.space_before;
        assert_eq!(&normalized, a, "PRECONDITION: spaceBefore is the only document change");
    }

    fn calibrate(s: &mut Session, ids: Ids) -> f64 {
        // 81 bounded candidates. All three layouts are real fresh compositions.
        for height in (40_u32..=200).step_by(2) {
            r9_resize(s, ids.first, 240.0, f64::from(height));
            let low = doc(s);
            let high = probe(s, "footnote.options", json!({"spaceBefore": CROSSING_SPACE}));
            let near = probe(s, "footnote.options", json!({"spaceBefore": NON_CROSSING_SPACE}));
            option_only(&low, &high, CROSSING_SPACE);
            option_only(&low, &near, NON_CROSSING_SPACE);
            let old = independent(&low, ids);
            let changed = independent(&high, ids);
            let control = independent(&near, ids);
            let sound = [&old, &changed, &control].iter().all(|o| o.source_fits && o.note_stays_in_first);
            if sound
                && old.page == Some(0)
                && changed.page == Some(1)
                && control.page == old.page
                && old.text == low.page_name(0)
                && changed.text == high.page_name(1)
                && old.text != changed.text
                && control.text == old.text
            {
                assert_eq!(r9_source_page(&low, ids.source, ids.anchor), 0);
                assert_eq!(r9_source_page(&high, ids.source, ids.anchor), 1);
                assert_eq!(r9_source_page(&near, ids.source, ids.anchor), 0);
                eprintln!(
                    "RF30_CALIBRATION {}",
                    json!({
                        "height": height, "width": 240, "candidate_limit": 81,
                        "spaces": [0, NON_CROSSING_SPACE, CROSSING_SPACE],
                        "fresh_pages": [old.page, control.page, changed.page],
                        "fresh_texts": [old.text, control.text, changed.text],
                        "all_story_spread_style_arcs_shared": true,
                        "source_fits_and_note_stays_first": true
                    })
                );
                return f64::from(height);
            }
        }
        panic!("PRECONDITION FAILED: no fresh-index/caret-verified footnote crossing in 81 heights; this does not confirm R29-RF-1");
    }

    fn fixture() -> Case {
        let (mut s, primary) = primary_chain(false);
        s.execute("prefs.set", &json!({"smartTextReflow": false})).unwrap();
        s.execute(
            "footnote.options",
            &json!({"spaceBefore": 0, "spaceBetween": 0, "refPosition": "normal", "separator": " ", "rule": {"on": false}}),
        )
        .unwrap();
        let made = s
            .execute("frame.create", &json!({"spread": 0, "rect": [36,36,276,236], "content": "text", "text": "Lead\nHead", "caret": false}))
            .unwrap();
        let source = StoryId(made["story"].as_u64().unwrap());
        let first = ItemId(made["id"].as_u64().unwrap());
        let second_spread = doc(&s).page_loc(1).unwrap().0;
        let made = s
            .execute("frame.create", &json!({"spread": second_spread, "rect": [36,36,276,636], "content": "text", "text": "", "caret": false}))
            .unwrap();
        let second = ItemId(made["id"].as_u64().unwrap());
        // There is no text.thread command. This is the existing fixture's
        // public setup API, not the measured mutation.
        s.edit(|d, _| {
            d.thread(first, second)?;
            Ok(())
        })
        .unwrap();
        s.execute("text.select", &json!({"story": source.0, "anchor": 4, "focus": 4})).unwrap();
        s.execute("footnote.insert", &json!({"text": "A short note."})).unwrap();
        select_all(&mut s, source);
        s.execute(
            "type.char",
            &json!({"attrs": {"fontFamily": "Source Serif 4", "fontStyle": "Regular", "size": 12,
                "leading": {"kind": "points", "value": 14}, "position": "normal"}}),
        )
        .unwrap();
        s.execute(
            "type.para",
            &json!({"attrs": {"composer": "singleLine", "hyphenate": false, "keepLinesTogether": false,
                "keepFirst": 1, "keepLast": 1, "spaceBefore": 0, "spaceAfter": 0, "gridAlign": "none"}}),
        )
        .unwrap();
        let at = doc(&s).story(source).unwrap().text.find("Head").unwrap();
        s.execute("text.select", &json!({"story": source.0, "anchor": at, "focus": at})).unwrap();
        let anchor = s.execute("anchor.create", &json!({"name": "R30 footnote source anchor"})).unwrap()["id"].as_u64().unwrap();
        s.execute("xref.defineFormat", &json!({"name": "R30 Page Number", "definition": "<pageNum />"})).unwrap();
        s.execute("text.select", &json!({"story": primary.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("xref.insert", &json!({"anchor": anchor, "format": "R30 Page Number"})).unwrap();
        let ids = Ids { primary, source, first, second, anchor };
        let height = calibrate(&mut s, ids);
        s.execute("prefs.set", &json!({"smartTextReflow": true})).unwrap();
        let full = doc(&s);
        full.check().unwrap();
        assert!(s.prefs.smart_text_reflow, "PRECONDITION: enabled-reflow path");
        assert_eq!(full.story(source).unwrap().frames, vec![first, second]);
        assert_eq!(full.story(source).unwrap().notes.len(), 1);
        assert!(full.story(source).unwrap().xrefs.is_empty(), "PRECONDITION: no self-reference approximation");
        let primary_story = full.story(primary).unwrap();
        assert!(primary_story.notes.is_empty() && primary_story.endnotes.is_empty() && primary_story.tables.is_empty());
        let removed = *primary_story.frames.last().unwrap();
        assert!(full.page_of_item(removed).unwrap() >= 2, "PRECONDITION: retain both source pages");
        let full_flow = fresh(&full, primary);
        assert!(!full_flow.is_overset(), "PRECONDITION: primary fits before removal");
        assert!(full_flow.frame(removed).is_some_and(|f| !f.range.is_empty()), "PRECONDITION: occupied capacity");
        s.execute("edit.clear", &json!({"ids": [removed.0]})).unwrap();
        let reduced = doc(&s);
        assert!(reduced.item(removed).is_none());
        assert_eq!(reduced.page_count(), full.page_count(), "PRECONDITION: remove frame, retain pages");
        assert_eq!(reduced.story(primary).unwrap().frames.len() + 1, primary_story.frames.len());
        assert_eq!(reduced.story(primary).unwrap().text, primary_story.text);
        assert!(fresh(&reduced, primary).is_overset(), "PRECONDITION: removed primary capacity must overset");
        assert_eq!(reduced.page_of_item(first), Some(0));
        assert_eq!(reduced.page_of_item(second), Some(1));
        reduced.check().unwrap();
        Case { session: s, ids, removed, height }
    }

    fn run(space: f64, crossing: bool) {
        let mut case = fixture();
        let ids = case.ids;
        let before = doc(&case.session);
        let input = probe(&mut case.session, "footnote.options", json!({"spaceBefore": space}));
        option_only(&before, &input, space);
        let old = independent(&before, ids);
        let expected = independent(&input, ids);
        assert!(old.source_fits && expected.source_fits && old.note_stays_in_first && expected.note_stays_in_first);
        assert_eq!(old.page, Some(0));
        assert_eq!(expected.page, Some(if crossing { 1 } else { 0 }));
        assert_eq!(old.text != expected.text, crossing, "PRECONDITION: actual consumed Page Number value");
        let old_flow = fresh(&before, ids.primary);
        assert!(old_flow.is_overset());
        let mark = before.story(ids.primary).unwrap().text.find(designcraft_doc::XREF_MARK).unwrap();
        assert!(old_flow.overset_at.is_some_and(|at| mark < at), "PRECONDITION: visible consumed reference");
        if crossing {
            assert_ne!(
                format!("{old_flow:?}"),
                format!("{:?}", fresh(&input, ids.primary)),
                "PRECONDITION: independent primary composition consumes the changed reference"
            );
        } else {
            same_composition(&old_flow, &fresh(&input, ids.primary));
        }

        // Rewarm a known old bank after every canceled input/calibration probe.
        case.session.cache.clear();
        let bank_before = case.session.cache.xref_index(&before);
        let cached_before = case.session.cache.get(&before, ids.primary, None);
        same_composition(&old_flow, &cached_before);
        assert_eq!(bank_before.page_index(ids.anchor), old.page);
        let undo = case.session.doc().unwrap().history.undo.len();
        let journal = case.session.journal.len();
        case.session.execute("footnote.options", &json!({"spaceBefore": space})).unwrap();
        let after = doc(&case.session);
        after.check().unwrap();
        assert_eq!(after.footnote_options.space_before, space);
        assert!(case.session.prefs.smart_text_reflow);
        assert!(after.item(case.removed).is_none(), "removed object identity must not be restored");
        let actual_fresh = independent(&after, ids);
        assert_eq!(actual_fresh.page, expected.page, "PRECONDITION: crossing survives the real command");
        assert_eq!(actual_fresh.text, expected.text);
        assert!(Arc::ptr_eq(&before.stories[&ids.source], &after.stories[&ids.source]), "source story must stay shared");
        assert!(Arc::ptr_eq(&before.styles, &after.styles), "styles must stay shared");
        assert_eq!(after.page_of_item(ids.first), Some(0));
        assert_eq!(after.page_of_item(ids.second), Some(1));
        let bank_after = case.session.cache.xref_index(&after);
        let warm_text = compose::xref::xref_text(&after, &after.story(ids.primary).unwrap().xrefs[0], Some(&bank_after));
        let cached_after = case.session.cache.get(&after, ids.primary, None);
        let refs = case.session.execute("xref.list", &json!({})).unwrap();
        assert_eq!(refs.as_array().unwrap().len(), 1);
        assert_eq!(refs[0]["text"].as_str(), Some(warm_text.as_str()), "public query must expose the measured cache");
        let frames_before = before.story(ids.primary).unwrap().frames.len();
        let frames_after = after.story(ids.primary).unwrap().frames.len();
        let resumed = frames_after > frames_before && !cached_after.is_overset();
        let all_shared = same_allocations(&before, &after);
        let history = &case.session.doc().unwrap().history;
        let expected_undo = if space == 0.0 { undo } else { (undo + 1).min(history.limit.max(1)) };
        assert_eq!(history.undo.len(), expected_undo);
        if space != 0.0 {
            assert!(Arc::ptr_eq(&history.undo.last().unwrap().doc, &before));
        }
        assert_eq!(case.session.journal.len(), journal + 1);
        eprintln!(
            "RF30_OBSERVED {}",
            json!({
                "space_before": space, "crossing_case": crossing, "calibrated_height": case.height,
                "fresh_page_before": old.page, "fresh_page_after": actual_fresh.page,
                "fresh_text_before": old.text, "fresh_text_after": actual_fresh.text,
                "warm_page_after": bank_after.page_index(ids.anchor), "warm_text_after": warm_text,
                "public_xref_text": refs[0]["text"],
                "same_bank_arc": Arc::ptr_eq(&bank_before, &bank_after),
                "same_primary_composition_arc": Arc::ptr_eq(&cached_before, &cached_after),
                "all_story_spread_style_arcs_shared_after_command": all_shared,
                "all_story_spread_style_arcs_shared_in_input_probe": true,
                "primary_frames_before": frames_before, "primary_frames_after": frames_after,
                "initial_primary_overset": true, "final_primary_overset": cached_after.is_overset(),
                "reflow_resumed": resumed
            })
        );
        if crossing {
            // A different failure does not prove the retained-allocation BEFORE defect.
            if !resumed {
                assert!(all_shared, "PRECONDITION: non-resumed BEFORE must retain all story/spread/style Arcs");
            }
            assert!(
                bank_after.page_index(ids.anchor) == actual_fresh.page && warm_text == actual_fresh.text && resumed,
                "R29-RF-1 ORACLE: validated crossing must refresh the consumed page and resume primary reflow; see RF30_OBSERVED"
            );
            same_composition(&fresh(&after, ids.primary), &cached_after);
        } else {
            assert!(all_shared, "CONTROL: story/spread/style Arcs stay shared");
            assert_eq!(frames_after, frames_before);
            assert!(cached_after.is_overset(), "CONTROL: preserve removed capacity");
            assert_eq!(bank_after.page_index(ids.anchor), actual_fresh.page);
            assert_eq!(warm_text, actual_fresh.text);
            same_composition(&old_flow, &cached_after);
            if space == 0.0 {
                assert!(Arc::ptr_eq(&before, &after), "CONTROL: same-value edit is document-no-op");
            }
        }
    }

    #[test]
    fn crossing_refreshes_consumed_page_and_resumes_primary() {
        run(CROSSING_SPACE, true);
    }
    #[test]
    fn non_crossing_spacing_preserves_removed_capacity() {
        run(NON_CROSSING_SPACE, false);
    }
    #[test]
    fn same_value_preserves_removed_capacity() {
        run(0.0, false);
    }
}

// Renaming used styles must not recreate deliberately removed primary capacity.
fn r30_nondefault_style_rename_case(character: bool, cold_commit: bool, changed_attrs: bool) {
    // Ignore only the edit counter when comparing composed output across the rename.
    fn same_layout(mut a: ComposedStory, b: ComposedStory) {
        assert_eq!(a.story, b.story);
        a.rev = b.rev;
        same_composition(&a, &b);
    }

    assert!(!character || !changed_attrs, "the combined edit uses the existing paragraph command");
    let kind = if character { "character" } else { "paragraph" };
    let name = "R30 Nondefault";
    let renamed = if character { "R30 Renamed/R30 Nondefault" } else { "R30 Nondefault Renamed" };
    let (mut s, sid) = primary_chain(false);
    s.execute(&format!("style.{kind}.create"), &json!({"name": name, "chars": {"size": 15, "tracking": 20}})).unwrap();
    select_all(&mut s, sid);
    s.execute(&format!("style.{kind}.apply"), &json!({"name": name})).unwrap();
    assert!(!fresh(&doc(&s), sid).is_overset(), "fixture: nondefault styled source must fit before deletion");
    let removed = remove_tail(&mut s, sid);
    let before = doc(&s);
    let old_story = before.story(sid).unwrap();
    assert!(!compose::vars::has_running(&before, old_story));
    assert!(!compose::xref::has_xrefs(old_story));
    assert!(old_story.tables.is_empty() && old_story.notes.is_empty() && old_story.endnotes.is_empty());

    // The fixture has one paragraph; resolve every nonempty character run, including the overset tail.
    let resolved = |d: &Document| {
        let story = d.story(sid).unwrap();
        assert_eq!(story.paras.len(), 1);
        let paragraph = d.styles.resolve_para(&story.paras[0]);
        let runs: Vec<_> =
            story.runs().filter(|(range, _)| !range.is_empty()).map(|(_, format)| d.styles.resolve_char(&paragraph.1, format)).collect();
        assert!(!runs.is_empty());
        (paragraph, runs)
    };
    let old_formats = resolved(&before);
    assert!(
        old_formats.1.iter().all(|chars| chars.size == 15.0 && chars.tracking == 20.0),
        "fixture: used attributes must differ from the 12pt/zero-tracking defaults"
    );

    // Character names are changed by the existing public group/rename route.
    let (command, mut params) = if character {
        ("style.group", json!({"kind": "character", "names": [name], "group": "R30 Renamed"}))
    } else {
        ("style.paragraph.edit", json!({"name": name, "rename": renamed}))
    };
    if changed_attrs {
        params["chars"] = json!({"size": 18});
    }
    let expected = probe(&mut s, command, params.clone());
    let expected_story = expected.story(sid).unwrap();
    assert_eq!(expected.page_count(), before.page_count());
    assert_eq!(expected_story.frames, old_story.frames);
    assert_eq!(expected_story.text, old_story.text);
    assert!(expected.item(removed).is_none());
    assert!(fresh(&expected, sid).is_overset(), "fixture: preview must retain the occupied-capacity deletion");
    if character {
        assert!(expected.styles.char_style(name).is_none() && expected.styles.char_style(renamed).is_some());
        assert!(expected_story.runs().filter(|(range, _)| !range.is_empty()).all(|(_, format)| format.style == renamed));
    } else {
        assert!(expected.styles.para(name).is_none() && expected.styles.para(renamed).is_some());
        assert!(expected_story.paras.iter().all(|para| para.style == renamed));
    }
    let new_formats = resolved(&expected);
    if changed_attrs {
        assert_ne!(old_formats, new_formats, "control must change consumed formatting as well as the name");
        assert!(new_formats.1.iter().all(|chars| chars.size == 18.0 && chars.tracking == 20.0));
    } else {
        assert_eq!(old_formats, new_formats, "pure rename must preserve each snapshot's resolved properties");
        same_layout(fresh(&before, sid), fresh(&expected, sid));
    }

    // Warm execute uses this populated bank; apply clears before begin/commit for the cold mode.
    public_native_controls(&mut s, sid);
    let undo = s.doc().unwrap().history.undo.len();
    apply(&mut s, command, params, cold_commit, cold_commit);
    let after = doc(&s);
    let new_story = after.story(sid).unwrap();
    assert_eq!(after.settings.primary_story, Some(sid));
    assert_eq!(new_story.text, old_story.text);
    assert!(after.item(removed).is_none(), "the deleted frame ID must remain absent");
    assert_eq!(after.styles, expected.styles);
    assert_eq!((&new_story.paras, &new_story.chars), (&expected_story.paras, &expected_story.chars));
    if changed_attrs {
        growth(&mut s, sid, Some(removed), &before, undo);
    } else {
        assert_eq!(after.page_count(), before.page_count(), "rename oracle: preserve explicitly deleted primary capacity");
        assert_eq!(new_story.frames, old_story.frames, "rename oracle: no replacement frames");
        assert!(fresh(&after, sid).is_overset());
        assert_eq!(full_doc(&after), full_doc(&expected), "only the previewed rename may change the full document");
        same_layout(fresh(&before, sid), fresh(&after, sid));
        history(&mut s, sid, &before, &after, undo);
    }
}

#[test]
fn r30_nondefault_paragraph_style_rename_preserves_removed_capacity() {
    for cold_commit in [false, true] {
        r30_nondefault_style_rename_case(false, cold_commit, false);
    }
}

#[test]
fn r30_nondefault_character_style_rename_preserves_removed_capacity() {
    for cold_commit in [false, true] {
        r30_nondefault_style_rename_case(true, cold_commit, false);
    }
}

#[test]
fn r30_paragraph_style_rename_with_changed_attributes_resumes_growth() {
    for cold_commit in [false, true] {
        r30_nondefault_style_rename_case(false, cold_commit, true);
    }
}

// RF-owned controls: equal plain-base properties can hide a consumed layered-style edit.
fn r30_layered_tail(text: &str) -> (Session, StoryId, usize) {
    let (mut s, sid) = primary_chain(false);
    let end = doc(&s).story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": end, "focus": end})).unwrap();
    s.execute("text.insert", &json!({"text": text})).unwrap();
    (s, sid, end + 1)
}

// A separate, non-document frame-spec copy exposes the tail for metric calibration only.
fn r30_layered_complete(d: &Document, sid: StoryId) -> ComposedStory {
    let mut frames = compose::frame_specs(d, sid);
    frames.truncate(1);
    frames[0].area.y1 = frames[0].area.y0 + 100_000.0;
    let flow = compose::compose(d, d.story(sid).unwrap(), &frames, &Default::default());
    assert!(!flow.is_overset(), "calibration must actually compose the entire tail");
    flow
}

fn r30_layered_glyph_size(flow: &ComposedStory, byte: usize) -> f64 {
    let glyph = flow.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).find(|g| g.byte == byte && g.visible).unwrap();
    flow.styles[glyph.style as usize].size
}

fn r30_layered_plain(d: &Document, sid: StoryId, at: usize) -> designcraft_doc::CharProps {
    let story = d.story(sid).unwrap();
    let (_, base) = d.styles.resolve_para(&story.paras[story.para_at(at)]);
    d.styles.resolve_char(&base, story.format_after(at))
}

fn r30_layered_preview(
    s: &mut Session,
    sid: StoryId,
    range: std::ops::Range<usize>,
    command: &str,
    params: &Value,
) -> (ItemId, Arc<Document>, Arc<Document>) {
    let removed = remove_tail(s, sid);
    s.execute("text.select", &json!({"story": sid.0, "anchor": range.start, "focus": range.end})).unwrap();
    let before = doc(s);
    let mut old_flow = fresh(&before, sid);
    assert!(s.prefs.smart_text_reflow);
    assert!(old_flow.overset_at.is_some_and(|at| at < range.start), "the entire changed tail must be overset");
    assert!(compose::compose_story(&before, sid, &Default::default()).is_overset(), "autoflow must need capacity");
    public_native_controls(s, sid);
    let expected = probe(s, command, params.clone());
    assert_eq!(expected.page_count(), before.page_count());
    assert_eq!(expected.story(sid).unwrap().frames, before.story(sid).unwrap().frames);
    assert_eq!(expected.story(sid).unwrap().text, before.story(sid).unwrap().text);
    assert_eq!(
        compose::advanced_type_inputs(&before, before.story(sid).unwrap()),
        compose::advanced_type_inputs(&expected, expected.story(sid).unwrap())
    );
    let new_flow = fresh(&expected, sid);
    old_flow.rev = new_flow.rev; // The public edit advances the source revision, not visible layout.
    same_composition(&old_flow, &new_flow);
    (removed, before, expected)
}

#[test]
fn r30_layered_grep_existing_style_application_resumes_capacity() {
    let (mut s, sid, tail) = r30_layered_tail("\nTail");
    for (name, chars) in [("R30 Explicit", json!({"size": 12})), ("R30 Empty", json!({})), ("R30 Overlay", json!({"size": 24}))] {
        s.execute("style.character.create", &json!({"name": name, "chars": chars})).unwrap();
    }
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": "R30 Overlay", "pattern": "."}]}})).unwrap();
    let end = doc(&s).story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": tail, "focus": end})).unwrap();
    s.execute("style.character.apply", &json!({"name": "R30 Explicit"})).unwrap();

    let params = json!({"name": "R30 Empty"});
    let (removed, before, expected) = r30_layered_preview(&mut s, sid, tail..end, "style.character.apply", &params);
    assert_eq!(before.styles, expected.styles, "this applies an existing style; it does not rename a definition");
    assert_eq!(before.styles.char_style("R30 Explicit").unwrap().chars.size, Some(12.0));
    assert!(before.styles.char_style("R30 Empty").unwrap().chars.is_empty());
    assert_eq!(before.story(sid).unwrap().paras, expected.story(sid).unwrap().paras);
    assert_eq!(before.story(sid).unwrap().chars.len(), expected.story(sid).unwrap().chars.len());
    assert_eq!(r30_layered_plain(&before, sid, tail), r30_layered_plain(&expected, sid, tail));
    assert_eq!(r30_layered_plain(&before, sid, tail).size, 12.0);
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&before, sid), tail), 12.0);
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&expected, sid), tail), 24.0);

    let count = s.doc().unwrap().history.undo.len();
    s.execute("style.character.apply", &params).unwrap();
    assert_eq!(doc(&s).story(sid).unwrap().chars, expected.story(sid).unwrap().chars);
    growth(&mut s, sid, Some(removed), &before, count);
}

#[test]
fn r30_layered_footnote_reference_style_rename_resumes_capacity() {
    let (mut s, sid, tail) = r30_layered_tail("\nTail ");
    s.execute("style.character.create", &json!({"name": "R30 Ref", "chars": {"size": 12}})).unwrap();
    s.execute("style.character.create", &json!({"name": "R30 Overlay", "chars": {"size": 24}})).unwrap();
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": "R30 Overlay", "pattern": "."}]}})).unwrap();
    s.execute("footnote.options", &json!({"refCharStyle": "R30 Ref"})).unwrap();
    let marker = doc(&s).story(sid).unwrap().len();
    s.execute("footnote.insert", &json!({"text": "A short note."})).unwrap();
    let end = marker + designcraft_doc::FOOTNOTE_REF.len_utf8();
    s.execute("text.select", &json!({"story": sid.0, "anchor": marker, "focus": end})).unwrap();
    s.execute("style.character.apply", &json!({"name": "R30 Ref"})).unwrap();

    // A pure rename retargets the reference option and preserves its explicit size.
    let grouped = probe(&mut s, "style.group", json!({"kind": "character", "names": ["R30 Ref"], "group": "R30 Renamed"}));
    assert_eq!(grouped.footnote_options.ref_char_style, "R30 Renamed/R30 Ref");
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&grouped, sid), marker), 12.0);

    // Removing the size override during rename exposes the GREP layer's 24pt size.
    // Plain-base resolution remains 12pt, so the overset dependency check still matters.
    let params = json!({"name": "R30 Ref", "rename": "R30 Renamed/R30 Ref", "chars": {"size": null}});
    let (removed, before, expected) = r30_layered_preview(&mut s, sid, tail..end, "style.character.edit", &params);
    let old_story = before.story(sid).unwrap();
    let new_story = expected.story(sid).unwrap();
    assert_eq!(old_story.notes.len(), 1);
    assert_eq!(old_story.text[marker..end], designcraft_doc::FOOTNOTE_REF.to_string());
    assert!(fresh(&before, sid).frames.iter().all(|f| f.notes.is_empty()));
    assert!(before.styles.char_style("R30 Renamed/R30 Ref").is_none());
    assert!(expected.styles.char_style("R30 Ref").is_none());
    let mut renamed_chars = before.styles.char_style("R30 Ref").unwrap().chars.clone();
    assert_eq!(renamed_chars.size, Some(12.0));
    renamed_chars.size = None;
    assert_eq!(expected.styles.char_style("R30 Renamed/R30 Ref").unwrap().chars, renamed_chars);
    assert_eq!(new_story.format_after(marker).style, "R30 Renamed/R30 Ref");
    assert_eq!(before.footnote_options.ref_char_style, "R30 Ref");
    let mut renamed_options = before.footnote_options.clone();
    renamed_options.ref_char_style = "R30 Renamed/R30 Ref".into();
    assert_eq!(expected.footnote_options, renamed_options, "only the renamed reference target changes");
    assert_eq!(r30_layered_plain(&before, sid, marker), r30_layered_plain(&expected, sid, marker));
    let (_, base) = before.styles.resolve_para(&old_story.paras[old_story.para_at(marker)]);
    assert_eq!(base.size, 12.0);
    let reference = designcraft_doc::CharFormat { style: "R30 Ref".into(), ..Default::default() };
    let renamed_reference = designcraft_doc::CharFormat { style: "R30 Renamed/R30 Ref".into(), ..Default::default() };
    assert_eq!(before.styles.resolve_char(&base, &reference), expected.styles.resolve_char(&base, &renamed_reference));
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&before, sid), marker), 12.0);
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&expected, sid), marker), 24.0);

    let count = s.doc().unwrap().history.undo.len();
    s.execute("style.character.edit", &params).unwrap();
    assert_eq!(doc(&s).story(sid).unwrap().chars, new_story.chars);
    assert_eq!(doc(&s).footnote_options, renamed_options);
    growth(&mut s, sid, Some(removed), &before, count);
}

#[test]
fn r32_layered_same_name_character_style_edit_resumes_capacity() {
    let (mut s, sid, tail) = r30_layered_tail("\nTail");
    s.execute("style.character.create", &json!({"name": "R32 Mask"})).unwrap();
    s.execute("style.character.create", &json!({"name": "R32 Overlay", "chars": {"size": 24}})).unwrap();
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": "R32 Overlay", "pattern": "."}]}})).unwrap();
    let end = doc(&s).story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": tail, "focus": end})).unwrap();
    s.execute("style.character.apply", &json!({"name": "R32 Mask"})).unwrap();

    let params = json!({"name": "R32 Mask", "chars": {"size": 12}});
    let (removed, before, expected) = r30_layered_preview(&mut s, sid, tail..end, "style.character.edit", &params);
    assert!(Arc::ptr_eq(&before.stories[&sid], &expected.stories[&sid]), "only the style definition changes");
    assert!(before.styles.char_style("R32 Mask").unwrap().chars.is_empty());
    assert_eq!(expected.styles.char_style("R32 Mask").unwrap().chars.size, Some(12.0));
    assert_eq!(before.story(sid).unwrap().format_after(tail).style, "R32 Mask");
    assert_eq!(r30_layered_plain(&before, sid, tail), r30_layered_plain(&expected, sid, tail));
    assert_eq!(r30_layered_plain(&before, sid, tail).size, 12.0);
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&before, sid), tail), 24.0);
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&expected, sid), tail), 12.0);

    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("style.character.edit", &params).unwrap();
    assert_eq!(doc(&s).styles, expected.styles);
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last().unwrap(), &("style.character.edit".into(), params));
    growth(&mut s, sid, Some(removed), &before, count);
}

#[test]
fn r32_layered_same_name_footnote_reference_style_edit_resumes_capacity() {
    let (mut s, sid, tail) = r30_layered_tail("\nTail ");
    s.execute("style.character.create", &json!({"name": "R32 Ref"})).unwrap();
    s.execute("style.character.create", &json!({"name": "R32 Overlay", "chars": {"size": 24}})).unwrap();
    s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": "R32 Overlay", "pattern": "."}]}})).unwrap();
    s.execute("footnote.options", &json!({"refCharStyle": "R32 Ref"})).unwrap();
    let marker = doc(&s).story(sid).unwrap().len();
    s.execute("footnote.insert", &json!({"text": "A short note."})).unwrap();
    let end = marker + designcraft_doc::FOOTNOTE_REF.len_utf8();

    let params = json!({"name": "R32 Ref", "chars": {"size": 12}});
    let (removed, before, expected) = r30_layered_preview(&mut s, sid, tail..end, "style.character.edit", &params);
    let story = before.story(sid).unwrap();
    assert!(Arc::ptr_eq(&before.stories[&sid], &expected.stories[&sid]));
    assert_eq!(story.notes.len(), 1);
    assert_eq!(story.text[marker..end], designcraft_doc::FOOTNOTE_REF.to_string());
    assert_ne!(story.format_after(marker).style, "R32 Ref", "the dependency is injected by footnote options, not the run");
    assert!(fresh(&before, sid).frames.iter().all(|frame| frame.notes.is_empty()));
    assert_eq!(before.footnote_options, expected.footnote_options);
    assert_eq!(before.footnote_options.ref_char_style, "R32 Ref");
    assert!(before.styles.char_style("R32 Ref").unwrap().chars.is_empty());
    assert_eq!(expected.styles.char_style("R32 Ref").unwrap().chars.size, Some(12.0));
    let (_, base) = before.styles.resolve_para(&story.paras[story.para_at(marker)]);
    assert_eq!(base.size, 12.0);
    let reference = designcraft_doc::CharFormat { style: "R32 Ref".into(), ..Default::default() };
    assert_eq!(before.styles.resolve_char(&base, &reference), expected.styles.resolve_char(&base, &reference));
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&before, sid), marker), 24.0);
    assert_eq!(r30_layered_glyph_size(&r30_layered_complete(&expected, sid), marker), 12.0);

    let count = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    s.execute("style.character.edit", &params).unwrap();
    assert_eq!(doc(&s).styles, expected.styles);
    assert_eq!(s.journal.len(), journal + 1);
    assert_eq!(s.journal.last().unwrap(), &("style.character.edit".into(), params));
    growth(&mut s, sid, Some(removed), &before, count);
}

#[path = "reflow_fc2_tests.rs"]
mod fc2;

#[test]
fn r35_reflow_projection_uses_session_memo_and_exact_consumed_bits() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1, "primaryTextFrame": true, "facingPages": false})).unwrap();
    let sid = doc(&s).settings.primary_story.unwrap();
    s.edit(|d, _| {
        let story = d.story_mut(sid).unwrap();
        story.insert(0, "M2");
        story.format_chars(1..2, |format| format.over.position = Some(designcraft_doc::Position::Superscript));
        d.settings.advanced_type.superscript_position = 0.0;
        Ok(())
    })
    .unwrap();
    s.cache.clear();
    let before = doc(&s);
    let mut units = (*before).clone();
    units.settings.horizontal_units = designcraft_geom::Unit::Millimeters;
    assert_eq!(Arc::weak_count(&before.stories[&sid]), 0);
    assert_eq!(Arc::weak_count(&before.styles), 0);
    assert!(crate::reflow_inputs_same(&before, &units, sid, &s.cache));
    // This fails if the engine takes the free scanner path or uses a temporary cache.
    // Ownership is observable without exposing compose's test-only work counter.
    assert_eq!(Arc::weak_count(&before.stories[&sid]), 1, "the Session memo pins the actual story allocation");
    assert_eq!(Arc::weak_count(&before.styles), 1, "the same entry pins the actual styles allocation");
    for _ in 0..3 {
        assert!(crate::reflow_inputs_same(&before, &units, sid, &s.cache));
    }
    let mut unused = units.clone();
    unused.settings.advanced_type.subscript_position = 99.0;
    assert!(crate::reflow_inputs_same(&before, &unused, sid, &s.cache), "unused settings must not resume reflow");
    for position in [-0.0, f64::from_bits(1)] {
        let mut consumed = units.clone();
        consumed.settings.advanced_type.superscript_position = position;
        assert!(!crate::reflow_inputs_same(&before, &consumed, sid, &s.cache), "consumed sign and low bits remain distinct");
    }
    assert_eq!(Arc::weak_count(&before.stories[&sid]), 1);
    assert_eq!(Arc::weak_count(&before.styles), 1);
    s.cache.clear();
    assert_eq!(Arc::weak_count(&before.stories[&sid]), 0, "the dependency entry belongs to the Session cache");
    assert_eq!(Arc::weak_count(&before.styles), 0);
}

#[test]
fn r35_unit_preferences_preserve_primary_capacity_and_warm_composition() {
    for commit in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 1, "primaryTextFrame": true, "facingPages": false})).unwrap();
        let sid = doc(&s).settings.primary_story.unwrap();
        s.execute("style.character.create", &json!({"name": "R35 Script", "chars": {"position": "superscript"}})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("text.insert", &json!({"text": "Body 2 with a synthesized script.\n".repeat(120)})).unwrap();
        select_all(&mut s, sid);
        s.execute(
            "type.para",
            &json!({"attrs": {"composer": "singleLine", "hyphenate": false, "grepStyles": [{"style": "R35 Script", "pattern": "2"}]}}),
        )
        .unwrap();
        let removed = remove_tail(&mut s, sid);
        s.cache.clear();
        let before = doc(&s);
        let composed = s.cache.get(&before, sid, None);
        assert!(composed.is_overset(), "explicit capacity removal must remain in force");
        let undo = s.doc().unwrap().history.undo.len();
        let journal = s.journal.len();
        let params = json!({"horizontalUnits": "millimeters"});
        apply(&mut s, "document.preferences", params.clone(), commit, commit);
        let after = doc(&s);
        let mut expected = (*before).clone();
        expected.settings.horizontal_units = designcraft_geom::Unit::Millimeters;
        assert_eq!(*after, expected, "only the requested unit preference may change");
        r9_shared_sources(&before, &after, true);
        assert_eq!(after.page_count(), before.page_count());
        assert_eq!(after.story(sid).unwrap().frames, before.story(sid).unwrap().frames);
        assert!(after.item(removed).is_none());
        let current = s.cache.get(&after, sid, None);
        if !commit {
            assert!(Arc::ptr_eq(&composed, &current), "ordinary unrelated edits retain the warm composition");
        }
        assert!(current.is_overset());
        assert!(Arc::ptr_eq(&current, &s.cache.get(&after, sid, None)));
        same_composition(&fresh(&after, sid), &current);
        assert_eq!(Arc::weak_count(&after.stories[&sid]), 1);
        assert_eq!(Arc::weak_count(&after.styles), 1);
        assert_eq!(s.journal.len(), journal + 1);
        assert_eq!(s.journal.last().unwrap(), &("document.preferences".into(), params));
        history(&mut s, sid, &before, &after, undo);
    }
}

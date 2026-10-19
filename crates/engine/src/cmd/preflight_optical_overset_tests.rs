use super::*;
use designcraft_doc::{ItemId, StoryId};

const OPTICAL: &str = "Optical kerning approximated by metrics";
const OPTICAL_INCOMPLETE: &str = "Optical kerning check incomplete: uncomposed text uses nested line styles";

fn fixture(text: &str, composer: &str) -> (Session, StoryId, ItemId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("frame.create", &json!({"rect": [0, 0, 600, 400], "content": "text", "text": text, "caret": false})).unwrap();
    let sid = StoryId(r["story"].as_u64().unwrap());
    let fid = ItemId(r["id"].as_u64().unwrap());
    s.execute("type.para", &json!({"attrs": {"composer": composer, "hyphenate": false}})).unwrap();
    (s, sid, fid)
}

fn select(s: &mut Session, sid: StoryId, start: usize, end: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": start, "focus": end})).unwrap();
}

fn kerning(s: &mut Session, kind: Option<&str>) {
    let value = kind.map_or(Value::Null, |kind| json!({"kind": kind}));
    s.execute("type.char", &json!({"attrs": {"kerning": value}})).unwrap();
}

fn label_styles(s: &mut Session, composer: &str, list: &str, named: bool, bullet: &str, separator: &str) {
    if named {
        s.execute("list.define", &json!({"name": "Review14 Steps"})).unwrap();
    }
    for (name, kind) in [("Label Metrics", "metrics"), ("Label Optical", "optical")] {
        s.execute(
            "style.paragraph.create",
            &json!({
                "name": name, "chars": {"fontFamily": "Source Serif 4", "fontStyle": "Regular", "size": 12, "kerning": {"kind": kind}},
                "para": {"composer": composer, "hyphenate": false, "listType": list, "listName": if named { "Review14 Steps" } else { "" },
                    "bulletChar": bullet, "listSeparator": separator}
            }),
        )
        .unwrap();
    }
}

fn geometry(s: &Session, sid: StoryId) -> Vec<Value> {
    fn record(cs: &ComposedStory, out: &mut Vec<Value>) {
        out.push(json!(["overset", cs.overset_at]));
        for f in &cs.frames {
            out.push(json!(["frame", f.frame.0, f.range.start, f.range.end]));
            for l in &f.lines {
                out.push(json!([
                    "line",
                    l.para,
                    l.column,
                    l.range.start,
                    l.range.end,
                    l.baseline,
                    l.x0,
                    l.x1,
                    l.end_x,
                    l.ascent,
                    l.descent,
                    l.leading
                ]));
                for g in &l.glyphs {
                    out.push(json!(["glyph", g.face.family, g.face.style, g.gid, g.byte, g.len, g.visible, g.x, g.y, g.adv, g.sx, g.sy]));
                }
            }
            for n in &f.notes {
                out.push(json!(["note", n.id, n.origin.x, n.origin.y, n.rect.x0, n.rect.y0, n.rect.x1, n.rect.y1]));
                record(&n.text, out);
            }
            for t in &f.tables {
                for c in &t.cells {
                    out.push(json!(["cell", t.table, c.row, c.col]));
                    record(&c.text, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    record(&s.cache.get(&s.doc().unwrap().doc, sid, None), &mut out);
    out
}

fn warning(s: &mut Session, sid: StoryId, expected: usize) {
    optical_warnings(s, sid, expected, 0);
}

fn optical_warnings(s: &mut Session, sid: StoryId, expected: usize, incomplete: usize) {
    let doc = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let history = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len());
    let cs = s.cache.get(&doc, sid, None);
    let before = geometry(s, sid);
    for _ in 0..2 {
        let result = s.execute("preflight.run", &json!({})).unwrap();
        for (message, count) in [(OPTICAL, expected), (OPTICAL_INCOMPLETE, incomplete)] {
            let found: Vec<_> = result["issues"].as_array().unwrap().iter().filter(|issue| issue["message"] == message).collect();
            assert_eq!(found.len(), count, "one deduplicated document warning: {message}");
            assert!(found.iter().all(|issue| issue["kind"] == "unsupportedTypography" && issue["severity"] == "warning"));
            assert!(found.iter().all(|issue| issue.get("item").is_none() && issue.get("page").is_none()));
        }
        assert_eq!(*s.doc().unwrap().doc, *doc);
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!((s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len()), history);
        assert!(std::sync::Arc::ptr_eq(&cs, &s.cache.get(&doc, sid, None)));
        assert_eq!(geometry(s, sid), before);
    }
}

// The target starts after "plain\n": the height-one frame stops before that paragraph.
// Undo/redo proves preflight did not add an edit above the supported geometry command.
fn fitting_then_overset(s: &mut Session, sid: StoryId, fid: ItemId, expected: usize) {
    let fitted = s.cache.get(&s.doc().unwrap().doc, sid, None);
    assert!(!fitted.is_overset());
    assert!(fitted.frames.iter().flat_map(|f| &f.lines).any(|l| l.para > 0));
    warning(s, sid, expected);
    let before = s.doc().unwrap().doc.clone();
    let fit_geometry = geometry(s, sid);
    s.execute("transform.set", &json!({"ids": [fid.0], "height": 1, "ref": 0})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    let cs = s.cache.get(&after, sid, None);
    assert!(cs.is_overset());
    assert!(cs.optical_kerning.get(1).copied().flatten().is_none(), "exercise source-only fallback, not a shaped target");
    assert!(cs.frames.iter().all(|f| f.notes.is_empty() && f.tables.is_empty()), "children must be unplaced");
    warning(s, sid, expected);
    let overset_geometry = geometry(s, sid);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    warning(s, sid, expected);
    assert_eq!(geometry(s, sid), fit_geometry);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *after);
    warning(s, sid, expected);
    assert_eq!(geometry(s, sid), overset_geometry);
    s.execute("edit.undo", &json!({})).unwrap();
}

#[test]
fn fc2_optical_overset_list_labels_skip_empty_paragraphs() {
    for composer in ["paragraph", "singleLine"] {
        for (list, named) in [("numbers", false), ("numbers", true), ("bullets", false)] {
            for body in ["Body", ""] {
                let text = format!("plain\n{body}");
                let (mut s, sid, fid) = fixture(&text, composer);
                label_styles(&mut s, composer, list, named, "AV", " ");
                select(&mut s, sid, 6, text.len());
                s.execute("style.paragraph.apply", &json!({"name": "Label Metrics"})).unwrap();
                if !body.is_empty() {
                    kerning(&mut s, Some("metrics"));
                }
                let metrics = geometry(&s, sid);
                fitting_then_overset(&mut s, sid, fid, 0);
                s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
                assert_eq!(geometry(&s, sid), metrics, "Optical remains the Metrics approximation");
                fitting_then_overset(&mut s, sid, fid, usize::from(!body.is_empty()));
            }
        }
    }
}

#[test]
fn r14_optical_overset_page_markers_use_effective_styles() {
    for composer in ["paragraph", "singleLine"] {
        for marker in
            [designcraft_doc::PAGE_NUMBER, designcraft_doc::NEXT_PAGE_NUMBER, designcraft_doc::PREV_PAGE_NUMBER, designcraft_doc::SECTION_MARKER]
        {
            let text = format!("plain\n{marker}{marker}");
            let (mut s, sid, fid) = fixture(&text, composer);
            select(&mut s, sid, 6, text.len());
            s.execute("style.character.create", &json!({"name": "Marker Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
            fitting_then_overset(&mut s, sid, fid, 0);
            let metrics = geometry(&s, sid);
            s.execute("style.character.apply", &json!({"name": "Marker Optical"})).unwrap();
            assert_eq!(geometry(&s, sid), metrics);
            fitting_then_overset(&mut s, sid, fid, 1);
            kerning(&mut s, Some("metrics"));
            fitting_then_overset(&mut s, sid, fid, 0);
            kerning(&mut s, None);
            fitting_then_overset(&mut s, sid, fid, 1);
        }
    }
}

#[test]
fn r14_optical_overset_variables_and_xrefs_keep_empty_controls() {
    for composer in ["paragraph", "singleLine"] {
        for xref in [false, true] {
            let (mut s, sid, fid) = fixture("plain\n", composer);
            let dest =
                s.execute("frame.create", &json!({"rect": [0, 410, 600, 700], "content": "text", "text": "Destination", "caret": false})).unwrap();
            select(&mut s, sid, 6, 6);
            if xref {
                s.execute("xref.defineFormat", &json!({"name": "Review14 Value", "definition": "AVATAR"})).unwrap();
                for _ in 0..2 {
                    s.execute("xref.insert", &json!({"story": dest["story"], "para": 0, "format": "Review14 Value"})).unwrap();
                }
            } else {
                s.execute("variables.define", &json!({"name": "Review14 Value", "type": "custom", "text": "AVATAR"})).unwrap();
                for _ in 0..2 {
                    s.execute("variables.insert", &json!({"name": "Review14 Value"})).unwrap();
                }
            }
            select(&mut s, sid, 6, 12);
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().len(), 12);
            kerning(&mut s, Some("optical"));
            warning(&mut s, sid, 1);
            if xref {
                s.execute("xref.defineFormat", &json!({"name": "Review14 Value", "definition": ""})).unwrap();
            } else {
                s.execute("variables.define", &json!({"name": "Review14 Value", "type": "custom", "text": ""})).unwrap();
            }
            fitting_then_overset(&mut s, sid, fid, 0);
            if xref {
                s.execute("xref.defineFormat", &json!({"name": "Review14 Value", "definition": "AVATAR"})).unwrap();
            } else {
                s.execute("variables.define", &json!({"name": "Review14 Value", "type": "custom", "text": "AVATAR"})).unwrap();
            }
            fitting_then_overset(&mut s, sid, fid, 1);
        }
    }
}

#[test]
fn r14_optical_overset_nonrendering_and_absent_labels_stay_quiet() {
    for composer in ["paragraph", "singleLine"] {
        // Unresolved variables and nonrendering controls have no replacement.
        // A raw XREF_MARK creates a broken xref rendering "??"; resolved empty xrefs are tested separately.
        let text = "plain\n\u{E1FF}\u{E00B}\u{E00D}\u{E00E}\u{E010}\t";
        let (mut s, sid, fid) = fixture(text, composer);
        assert!(s.doc().unwrap().doc.text_variables.len() < 256);
        select(&mut s, sid, 6, text.len());
        kerning(&mut s, Some("optical"));
        fitting_then_overset(&mut s, sid, fid, 0);
        for (list, bullet) in [("none", ""), ("bullets", ""), ("bullets", "\t")] {
            let (mut s, sid, fid) = fixture("plain\nBody", composer);
            label_styles(&mut s, composer, list, false, bullet, "");
            select(&mut s, sid, 6, 10);
            s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
            kerning(&mut s, Some("metrics"));
            fitting_then_overset(&mut s, sid, fid, 0);
        }
    }
}

#[test]
fn r14_optical_overset_hidden_and_deleted_marker_text_stays_quiet() {
    for composer in ["paragraph", "singleLine"] {
        for deleted in [false, true] {
            let (mut s, sid, fid) = fixture("plain\n\u{E000}", composer);
            select(&mut s, sid, 6, 9);
            kerning(&mut s, Some("optical"));
            warning(&mut s, sid, 1);
            if deleted {
                s.execute("type.char", &json!({"attrs": {"change": "deleted"}})).unwrap();
            } else {
                s.execute("condition.new", &json!({"name": "Hidden", "visible": false})).unwrap();
                s.execute("condition.apply", &json!({"name": "Hidden"})).unwrap();
            }
            fitting_then_overset(&mut s, sid, fid, 0);
        }
    }
}

#[test]
fn r14_optical_overset_final_reference_style_precedence() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid, fid) = fixture("plain\n", composer);
        for (name, kind) in [("Ref Metrics", "metrics"), ("Ref Optical", "optical")] {
            s.execute("style.character.create", &json!({"name": name, "chars": {"kerning": {"kind": kind}}})).unwrap();
        }
        select(&mut s, sid, 6, 6);
        s.execute("footnote.insert", &json!({"text": "Metrics body"})).unwrap();
        s.execute("footnote.goToReference", &json!({})).unwrap();
        select(&mut s, sid, 6, 9);
        s.execute("footnote.options", &json!({"refCharStyle": "Ref Optical"})).unwrap();
        warning(&mut s, sid, 1);
        kerning(&mut s, Some("metrics"));
        fitting_then_overset(&mut s, sid, fid, 0);
        kerning(&mut s, None);
        s.execute("style.character.apply", &json!({"name": "Ref Optical"})).unwrap();
        s.execute("footnote.options", &json!({"refCharStyle": "Ref Metrics"})).unwrap();
        fitting_then_overset(&mut s, sid, fid, 0);
        s.execute("footnote.options", &json!({"refCharStyle": "Ref Optical"})).unwrap();
        fitting_then_overset(&mut s, sid, fid, 1);
    }
}

#[test]
fn r14_optical_overset_unplaced_note_and_cell_labels() {
    for composer in ["paragraph", "singleLine"] {
        for (note, body) in [(true, "Body"), (true, ""), (false, "Body")] {
            let (mut s, sid, fid) = fixture("plain\n", composer);
            label_styles(&mut s, composer, if note { "none" } else { "bullets" }, false, "AV", " ");
            select(&mut s, sid, 6, 6);
            if note {
                s.execute("footnote.options", &json!({"paraStyle": "Label Metrics", "prefix": "AV", "affixIn": "text", "separator": " "})).unwrap();
                s.execute("footnote.insert", &json!({"text": body})).unwrap();
            } else {
                s.execute("table.insert", &json!({"rows": 1, "cols": 1})).unwrap();
                s.execute("text.insert", &json!({"text": body})).unwrap();
            }
            s.execute("edit.selectAll", &json!({})).unwrap();
            s.execute("style.paragraph.apply", &json!({"name": "Label Metrics"})).unwrap();
            if !body.is_empty() {
                kerning(&mut s, Some("metrics"));
            }
            let target = s.doc().unwrap().selection.text.unwrap();
            assert!(target.cell.is_some());
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            if note {
                assert_eq!(cs.frames.iter().flat_map(|f| &f.notes).count(), 1);
            } else {
                assert_eq!(cs.frames.iter().flat_map(|f| &f.tables).flat_map(|t| &t.cells).count(), 1);
            }
            let metrics = geometry(&s, sid);
            fitting_then_overset(&mut s, sid, fid, 0);
            s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
            assert_eq!(geometry(&s, sid), metrics);
            fitting_then_overset(&mut s, sid, fid, 1);
        }
    }
}

#[test]
fn r14_optical_overset_unplaced_note_and_cell_markers_persist() {
    for composer in ["paragraph", "singleLine"] {
        for note in [false, true] {
            let (mut s, sid, fid) = fixture("plain\n", composer);
            select(&mut s, sid, 6, 6);
            if note {
                s.execute("footnote.insert", &json!({"text": "\u{E000}\u{E000}"})).unwrap();
            } else {
                s.execute("table.insert", &json!({"rows": 1, "cols": 1})).unwrap();
                s.execute("text.insert", &json!({"text": "\u{E000}\u{E000}"})).unwrap();
            }
            s.execute("edit.selectAll", &json!({})).unwrap();
            s.execute("type.para", &json!({"attrs": {"composer": composer, "hyphenate": false}})).unwrap();
            let metrics = geometry(&s, sid);
            kerning(&mut s, Some("optical"));
            assert_eq!(geometry(&s, sid), metrics);
            fitting_then_overset(&mut s, sid, fid, 1);
            s.execute("transform.set", &json!({"ids": [fid.0], "height": 1, "ref": 0})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            let saved = s.execute("file.serialize", &json!({})).unwrap();
            let mut reopened = Session::new();
            reopened.execute("file.openBytes", &json!({"base64": saved["base64"]})).unwrap();
            assert_eq!(serde_json::to_value(&*reopened.doc().unwrap().doc).unwrap(), serde_json::to_value(&*before).unwrap());
            warning(&mut reopened, sid, 1);
            assert_eq!(geometry(&reopened, sid), geometry(&s, sid));
        }
    }
}

#[test]
fn r14_optical_overset_marker_automatic_overlays_and_running_affixes() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid, fid) = fixture("plain\n\u{E000}\u{E000}", composer);
        select(&mut s, sid, 6, 12);
        for (name, kind) in [("Auto Optical", "optical"), ("Auto Metrics", "metrics")] {
            s.execute("style.character.create", &json!({"name": name, "chars": {"kerning": {"kind": kind}}})).unwrap();
        }
        s.execute(
            "type.para",
            &json!({"attrs": {
                "nestedStyles": [{"style": "Auto Optical", "count": 2, "through": true, "until": {"kind": "characters"}}]
            }}),
        )
        .unwrap();
        fitting_then_overset(&mut s, sid, fid, 1);
        s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": "Auto Metrics", "pattern": ".+"}]}})).unwrap();
        fitting_then_overset(&mut s, sid, fid, 0);

        let (mut s, sid, fid) = fixture("plain\n", composer);
        s.execute("style.paragraph.create", &json!({"name": "Unused Heading"})).unwrap();
        s.execute("variables.define", &json!({"name": "Review14 Running", "type": "runningHeader", "style": "Unused Heading"})).unwrap();
        select(&mut s, sid, 6, 6);
        s.execute("variables.insert", &json!({"name": "Review14 Running"})).unwrap();
        select(&mut s, sid, 6, 9);
        kerning(&mut s, Some("optical"));
        fitting_then_overset(&mut s, sid, fid, 0);
        s.execute(
            "variables.define",
            &json!({
                "name": "Review14 Running", "type": "runningHeader", "style": "Unused Heading", "before": "AV", "after": "!"
            }),
        )
        .unwrap();
        fitting_then_overset(&mut s, sid, fid, 1);
    }
}

#[test]
fn r14_optical_overset_endnote_marker_keeps_direct_precedence() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid, fid) = fixture("plain\n", composer);
        select(&mut s, sid, 6, 6);
        s.execute("endnote.insert", &json!({"text": "Metrics body"})).unwrap();
        select(&mut s, sid, 6, 9);
        let metrics = geometry(&s, sid);
        kerning(&mut s, Some("optical"));
        assert_eq!(geometry(&s, sid), metrics);
        fitting_then_overset(&mut s, sid, fid, 1);
        kerning(&mut s, Some("metrics"));
        fitting_then_overset(&mut s, sid, fid, 0);
    }
}

#[test]
fn fc2_optical_overset_uses_expanded_list_labels() {
    for composer in ["paragraph", "singleLine"] {
        for named in [false, true] {
            for expression in ["", "^t", "^#^t", "Step ^#^t"] {
                let (mut s, sid, fid) = fixture("plain\nBody", composer);
                label_styles(&mut s, composer, "numbers", named, "", "^t");
                select(&mut s, sid, 6, 10);
                s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
                s.execute("type.para", &json!({"attrs": {"numberExpression": expression}})).unwrap();
                kerning(&mut s, Some("metrics"));
                fitting_then_overset(&mut s, sid, fid, usize::from(expression.contains("^#")));
            }
        }
        for (bullet, separator, expected) in [("", "^t", 0), ("\n\r", "^t", 0), ("AV", "^t", 1), ("", "Label^t", 1)] {
            let (mut s, sid, fid) = fixture("plain\nBody", composer);
            label_styles(&mut s, composer, "bullets", false, bullet, separator);
            select(&mut s, sid, 6, 10);
            s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
            kerning(&mut s, Some("metrics"));
            fitting_then_overset(&mut s, sid, fid, expected);
        }
    }
}

#[test]
fn r34_optical_child_index_misses_preserve_source_fallback() {
    for composer in ["paragraph", "singleLine"] {
        for is_note in [false, true] {
            for optical in [false, true] {
                let (mut s, sid, _) = fixture("", composer);
                select(&mut s, sid, 0, 0);
                if is_note {
                    s.execute("footnote.insert", &json!({"text": "Metrics body"})).unwrap();
                    s.execute("footnote.goToReference", &json!({})).unwrap();
                    s.execute("footnote.insert", &json!({"text": "Target body"})).unwrap();
                } else {
                    s.execute("table.insert", &json!({"rows": 1, "cols": 2})).unwrap();
                    s.execute("text.insert", &json!({"text": "Metrics body"})).unwrap();
                    s.execute("table.nextCell", &json!({})).unwrap();
                    s.execute("text.insert", &json!({"text": "Target body"})).unwrap();
                }
                s.execute("type.para", &json!({"attrs": {"composer": composer, "hyphenate": false}})).unwrap();
                s.execute("edit.selectAll", &json!({})).unwrap();
                kerning(&mut s, Some(if optical { "optical" } else { "metrics" }));
                let doc = &s.doc().unwrap().doc;
                let story = doc.story(sid).unwrap();
                let mut partial = (*s.cache.get(doc, sid, None)).clone();
                assert!(!partial.optical_kerning.contains(&Some(true)), "only the missing child may use Optical");
                // Remove one placement, keeping the source intact and another indexed child present.
                if is_note {
                    assert_eq!(story.notes.len(), 2);
                    let missing = story.notes.last().unwrap().id;
                    assert_eq!(partial.frames.iter().flat_map(|f| &f.notes).count(), 2);
                    for frame in &mut partial.frames {
                        frame.notes.retain(|note| note.id != missing);
                    }
                    assert_eq!(partial.frames.iter().flat_map(|f| &f.notes).count(), 1);
                } else {
                    assert_eq!(partial.frames.iter().flat_map(|f| &f.tables).flat_map(|t| &t.cells).count(), 2);
                    for table in partial.frames.iter_mut().flat_map(|f| &mut f.tables) {
                        table.cells.retain(|cell| cell.col == 0);
                    }
                    assert_eq!(partial.frames.iter().flat_map(|f| &f.tables).flat_map(|t| &t.cells).count(), 1);
                }
                for composed in [Some(&partial), None] {
                    assert_eq!(
                        designcraft_compose::source_optical_kerning(doc, story, composed, &Default::default()),
                        if optical { OpticalKerningUse::Used } else { OpticalKerningUse::NotUsed },
                        "{composer}, note={is_note}, optical={optical}, partial={}",
                        composed.is_some()
                    );
                }
            }
        }
    }
}

// RUN47 revised contract: actual composition is definite; ambiguous source is incomplete.
fn r47_unresolved_line_case(composer: &str, line_optical: bool) {
    use designcraft_doc::Kerning;
    let expected = usize::from(line_optical);
    let base_kind = if line_optical { "metrics" } else { "optical" };
    let line_kind = if line_optical { "optical" } else { "metrics" };
    let (mut s, sid, fid) = fixture("plain\nX", composer);
    s.execute("style.character.create", &json!({"name": "R47 first line", "chars": {"kerning": {"kind": line_kind}}})).unwrap();
    s.execute(
        "style.paragraph.create",
        &json!({
            "name": "R47 target",
            "chars": {"fontFamily": "Source Serif 4", "fontStyle": "Regular", "size": 12, "kerning": {"kind": base_kind}},
            "para": {"composer": composer, "hyphenate": false,
                "nestedLineStyles": [{"style": "R47 first line", "lines": 1}]}
        }),
    )
    .unwrap();
    select(&mut s, sid, 6, 7);
    s.execute("style.paragraph.apply", &json!({"name": "R47 target", "clearOverrides": true})).unwrap();

    let fitted_doc = s.doc().unwrap().doc.clone();
    let story = fitted_doc.story(sid).unwrap();
    assert_eq!(story.text, "plain\nX");
    assert_eq!(story.paras.len(), 2);
    assert_eq!(&story.text[story.para_ranges()[1].clone()], "X");
    let (para, base) = fitted_doc.styles.resolve_para(&story.paras[1]);
    assert_eq!(para.nested_line_styles.len(), 1);
    assert_eq!(para.nested_line_styles[0].style, "R47 first line");
    assert_eq!(para.nested_line_styles[0].lines, 1);
    let expected_base = if line_optical { Kerning::Metrics } else { Kerning::Optical };
    assert_eq!(base.kerning, expected_base);
    assert_eq!(story.format_after(6).over.kerning, None);
    assert_eq!(fitted_doc.styles.resolve_char(&base, story.format_after(6)).kerning, expected_base);
    let (_, first_base) = fitted_doc.styles.resolve_para(&story.paras[0]);
    assert_eq!(fitted_doc.styles.resolve_char(&first_base, story.format_after(0)).kerning, Kerning::Metrics);
    let fitted = s.cache.get(&fitted_doc, sid, None);
    assert!(!fitted.is_overset());
    assert_eq!(fitted.optical_kerning.first(), Some(&Some(false)));
    assert_eq!(fitted.optical_kerning.get(1), Some(&Some(line_optical)));
    let target_lines: Vec<_> = fitted.frames.iter().flat_map(|f| &f.lines).filter(|line| line.para == 1).collect();
    assert_eq!(target_lines.len(), 1, "the single X really occupies the first line");
    assert!(target_lines[0].glyphs.iter().any(|g| g.byte == 6 && g.len == 1 && g.visible));
    warning(&mut s, sid, expected);
    let fit_geometry = geometry(&s, sid);

    s.execute("transform.set", &json!({"ids": [fid.0], "height": 1, "ref": 0})).unwrap();
    let overset_doc = s.doc().unwrap().doc.clone();
    assert_eq!(overset_doc.story(sid), fitted_doc.story(sid), "resizing preserves all source formatting");
    assert_eq!(overset_doc.styles, fitted_doc.styles);
    let cs = s.cache.get(&overset_doc, sid, None);
    assert!(cs.is_overset());
    assert!(cs.overset_at.is_some_and(|at| at < 6));
    assert_eq!(cs.optical_kerning.first(), Some(&Some(false)));
    assert_eq!(cs.optical_kerning.get(1), None, "paragraph 1 must have no composition diagnostic at all");
    assert!(cs.frames.iter().flat_map(|f| &f.lines).all(|line| line.para == 0));

    assert_eq!(
        designcraft_compose::source_optical_kerning(&overset_doc, overset_doc.story(sid).unwrap(), Some(&cs), &Default::default()),
        OpticalKerningUse::Unresolved
    );
    optical_warnings(&mut s, sid, 0, 1);
    let overset_geometry = geometry(&s, sid);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *fitted_doc);
    warning(&mut s, sid, expected);
    assert_eq!(geometry(&s, sid), fit_geometry);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *overset_doc);
    optical_warnings(&mut s, sid, 0, 1);
    assert_eq!(geometry(&s, sid), overset_geometry);
}

#[test]
fn r47_optical_unresolved_metrics_base_optical_first_line_paragraph() {
    r47_unresolved_line_case("paragraph", true);
}

#[test]
fn r47_optical_unresolved_metrics_base_optical_first_line_single_line() {
    r47_unresolved_line_case("singleLine", true);
}

#[test]
fn r47_optical_unresolved_optical_base_metrics_first_line_paragraph() {
    r47_unresolved_line_case("paragraph", false);
}

#[test]
fn r47_optical_unresolved_optical_base_metrics_first_line_single_line() {
    r47_unresolved_line_case("singleLine", false);
}

fn r47_styles(s: &mut Session) {
    for (name, attrs) in [
        ("R47 Optical", json!({"kerning": {"kind": "optical"}})),
        ("R47 Metrics", json!({"kerning": {"kind": "metrics"}})),
        ("R47 tracking", json!({"tracking": 20})),
        ("R47 empty", json!({})),
    ] {
        s.execute("style.character.create", &json!({"name": name, "chars": attrs})).unwrap();
    }
}

#[test]
fn r47_optical_unresolved_multiline_and_precedence() {
    for composer in ["paragraph", "singleLine"] {
        let text = format!("plain\n{}", "Alpha beta gamma delta epsilon zeta eta theta. ".repeat(8));
        let (mut s, sid, fid) = fixture(&text, composer);
        r47_styles(&mut s);
        select(&mut s, sid, 6, text.len());
        s.execute("type.char", &json!({"attrs": {"fontFamily": "Source Serif 4", "fontStyle": "Regular", "size": 12}})).unwrap();
        s.execute(
            "type.para",
            &json!({"attrs": {"nestedLineStyles": [
                {"style": designcraft_doc::NO_CHAR_STYLE, "lines": 1}, {"style": "R47 Optical", "lines": 1}
            ]}}),
        )
        .unwrap();
        let mut first_ends = Vec::new();
        for width in [120, 220] {
            s.execute("transform.set", &json!({"ids": [fid.0], "width": width, "height": 800, "ref": 0})).unwrap();
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            assert!(!cs.is_overset());
            let lines: Vec<_> = cs.frames.iter().flat_map(|f| &f.lines).filter(|line| line.para == 1).collect();
            assert!(lines.len() >= 3, "exercise a real later-line style and unstyled suffix");
            first_ends.push(lines[0].range.end);
            assert_eq!(cs.optical_kerning.get(1), Some(&Some(true)));
            warning(&mut s, sid, 1);
        }
        assert_ne!(first_ends[0], first_ends[1], "line-style byte ranges really change with width");
        s.execute("transform.set", &json!({"ids": [fid.0], "height": 1, "ref": 0})).unwrap();
        assert_eq!(s.cache.get(&s.doc().unwrap().doc, sid, None).optical_kerning.get(1), None);
        optical_warnings(&mut s, sid, 0, 1);
        // Named run formatting and direct formatting both sit above every candidate.
        s.execute("style.character.apply", &json!({"name": "R47 Metrics"})).unwrap();
        warning(&mut s, sid, 0);
        s.execute("edit.undo", &json!({})).unwrap();
        optical_warnings(&mut s, sid, 0, 1);
        kerning(&mut s, Some("metrics"));
        warning(&mut s, sid, 0);
        kerning(&mut s, None);
        optical_warnings(&mut s, sid, 0, 1);
        s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": "R47 Metrics", "pattern": ".+"}]}})).unwrap();
        warning(&mut s, sid, 0);
        kerning(&mut s, Some("optical"));
        warning(&mut s, sid, 1);
    }
}

#[test]
fn r47_optical_unresolved_mixed_use_deduplicates() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid, fid) = fixture("plain\nAmbiguous", composer);
        r47_styles(&mut s);
        select(&mut s, sid, 6, 15);
        s.execute("type.para", &json!({"attrs": {"nestedLineStyles": [{"style": "R47 Optical", "lines": 1}]}})).unwrap();
        s.execute("transform.set", &json!({"ids": [fid.0], "height": 1, "ref": 0})).unwrap();
        optical_warnings(&mut s, sid, 0, 1);
        select(&mut s, sid, 0, 5);
        kerning(&mut s, Some("optical"));
        let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
        assert_eq!(cs.optical_kerning.first(), Some(&Some(true)));
        assert_eq!(cs.optical_kerning.get(1), None);
        warning(&mut s, sid, 1);
        s.execute("edit.undo", &json!({})).unwrap();
        optical_warnings(&mut s, sid, 0, 1);
        let other = s.execute("frame.create", &json!({"rect": [0, 410, 600, 700], "content": "text", "text": "Known", "caret": false})).unwrap();
        let other_sid = StoryId(other["story"].as_u64().unwrap());
        select(&mut s, other_sid, 0, 5);
        kerning(&mut s, Some("optical"));
        let doc = &s.doc().unwrap().doc;
        assert_eq!(
            designcraft_compose::source_optical_kerning(doc, doc.story(sid).unwrap(), Some(&s.cache.get(doc, sid, None)), &Default::default()),
            OpticalKerningUse::Unresolved
        );
        assert!(s.cache.get(doc, other_sid, None).optical_kerning.contains(&Some(true)));
        warning(&mut s, sid, 1);
    }
}

#[test]
fn r47_optical_irrelevant_line_styles_stay_quiet() {
    for composer in ["paragraph", "singleLine"] {
        for (body, lines) in [
            ("Body", json!([])), // An unused Optical definition is not a use.
            ("Body", json!([{"style": "R47 Metrics", "lines": 0}, {"style": "R47 Optical", "lines": 1}])),
            ("Body", json!([{"style": designcraft_doc::NO_CHAR_STYLE, "lines": 1}])),
            ("Body", json!([{"style": "", "lines": 1}])),
            ("Body", json!([{"style": "Missing style", "lines": 1}])),
            ("Body", json!([{"style": "R47 empty", "lines": 1}])),
            ("Body", json!([{"style": "R47 tracking", "lines": 1}])),
            ("Body", json!([{"style": "R47 Metrics", "lines": 1}])),
            ("", json!([{"style": "R47 Optical", "lines": 1}])),
            ("\t\u{E1FF}", json!([{"style": "R47 Optical", "lines": 1}])),
        ] {
            let text = format!("plain\n{body}");
            let (mut s, sid, fid) = fixture(&text, composer);
            r47_styles(&mut s);
            select(&mut s, sid, 6, text.len());
            s.execute("type.para", &json!({"attrs": {"nestedLineStyles": lines}})).unwrap();
            warning(&mut s, sid, 0);
            s.execute("transform.set", &json!({"ids": [fid.0], "height": 1, "ref": 0})).unwrap();
            assert_eq!(s.cache.get(&s.doc().unwrap().doc, sid, None).optical_kerning.get(1), None);
            warning(&mut s, sid, 0);
        }
    }
}

#[test]
fn r47_optical_nested_line_source_children_and_labels() {
    for composer in ["paragraph", "singleLine"] {
        for kind in ["cell", "footnote"] {
            let (mut s, sid, fid) = fixture("plain\n", composer);
            r47_styles(&mut s);
            select(&mut s, sid, 6, 6);
            match kind {
                "cell" => {
                    s.execute("table.insert", &json!({"rows": 1, "cols": 1})).unwrap();
                    s.execute("text.insert", &json!({"text": "Child text"})).unwrap();
                }
                _ => {
                    assert_eq!(kind, "footnote");
                    s.execute("footnote.insert", &json!({"text": "Child text"})).unwrap();
                }
            }
            s.execute("edit.selectAll", &json!({})).unwrap();
            s.execute(
                "type.para",
                &json!({"attrs": {"composer": composer, "hyphenate": false,
                "nestedLineStyles": [{"style": "R47 Optical", "lines": 1}]}}),
            )
            .unwrap();
            let target = s.doc().unwrap().selection.text.unwrap();
            assert!(target.cell.is_some());
            s.execute("transform.set", &json!({"ids": [fid.0], "height": 1, "ref": 0})).unwrap();
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            assert!(cs.is_overset());
            assert!(cs.frames.iter().all(|frame| frame.notes.is_empty() && frame.tables.is_empty()));
            assert!(!cs.optical_kerning.contains(&Some(true)));
            optical_warnings(&mut s, sid, 0, 1);
            // All-Metrics alternatives stay quiet even inside retained child sources.
            kerning(&mut s, Some("metrics"));
            warning(&mut s, sid, 0);
            if kind == "cell" {
                let story = s.doc().unwrap().doc.story(sid).unwrap();
                let pi = story.paras.iter().enumerate().find(|(pi, _)| story.para_table(*pi).is_some()).unwrap().0;
                let range = story.para_ranges()[pi].clone();
                select(&mut s, sid, range.start, range.end);
                s.execute("type.para", &json!({"attrs": {"nestedLineStyles": [{"style": "R47 Optical", "lines": 1}]}})).unwrap();
                warning(&mut s, sid, 0); // Table-wrapper formatting does not shape text.
            }
        }
        let (mut s, sid, fid) = fixture("plain\nBody", composer);
        r47_styles(&mut s);
        label_styles(&mut s, composer, "bullets", false, "AV", " ");
        select(&mut s, sid, 6, 10);
        s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
        s.execute("type.para", &json!({"attrs": {"nestedLineStyles": [{"style": "R47 Metrics", "lines": 1}]}})).unwrap();
        fitting_then_overset(&mut s, sid, fid, 1); // The generated label is independent of line styles.
    }
}

use super::*;
use designcraft_doc::{ItemId, StoryId};

const OPTICAL: &str = "Optical kerning approximated by metrics";

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
    let doc = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let history = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len());
    let cs = s.cache.get(&doc, sid, None);
    let before = geometry(s, sid);
    for _ in 0..2 {
        let result = s.execute("preflight.run", &json!({})).unwrap();
        let found: Vec<_> = result["issues"].as_array().unwrap().iter().filter(|issue| issue["message"] == OPTICAL).collect();
        assert_eq!(found.len(), expected, "one deduplicated warning for all effective uses");
        assert!(found.iter().all(|issue| issue["kind"] == "unsupportedTypography" && issue["severity"] == "warning"));
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

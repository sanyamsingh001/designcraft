use super::*;
use designcraft_doc::{Kerning, StoryId};

const OPTICAL_MESSAGE: &str = "Optical kerning approximated by metrics";

fn new_story(text: &str, height: f64) -> (Session, StoryId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("frame.create", &json!({"rect": [0, 0, 600, height], "content": "text", "text": text, "caret": false})).unwrap();
    (s, StoryId(r["story"].as_u64().unwrap()))
}

fn warning_count(s: &mut Session) -> usize {
    let before = s.doc().unwrap().doc.clone();
    let r = s.execute("preflight.run", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before, "preflight must not edit the document");
    let issues: Vec<_> = r["issues"].as_array().unwrap().iter().filter(|i| i["message"] == OPTICAL_MESSAGE).collect();
    assert!(issues.iter().all(|i| i["severity"] == "warning" && i["kind"] == "unsupportedTypography"));
    issues.len()
}

fn geometry(s: &Session, sid: StoryId) -> Vec<Value> {
    let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
    cs.frames
        .iter()
        .flat_map(|f| &f.lines)
        .flat_map(|l| l.glyphs.iter().map(move |g| json!([l.baseline, l.x0, l.x1, g.gid, g.byte, g.len, g.x, g.y, g.adv, g.sx, g.sy])))
        .collect()
}

#[test]
fn review9_optical_warning_is_deduplicated_and_keeps_metrics_shaping() {
    let (mut s, sid) = new_story("Text “Park”", 400.0);
    assert_eq!(warning_count(&mut s), 0);
    for kerning in [json!({"kind": "metrics"}), json!({"kind": "none"}), json!({"kind": "manual", "value": -40})] {
        s.execute("type.char", &json!({"attrs": {"kerning": kerning}})).unwrap();
        assert_eq!(warning_count(&mut s), 0);
    }
    s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
    let metrics = geometry(&s, sid);
    s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
    assert_eq!(warning_count(&mut s), 1);
    assert_eq!(geometry(&s, sid), metrics, "warning must not implement a different shaper");
    assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().format_after(0).over.kerning, Some(Kerning::Optical));
    s.execute("frame.create", &json!({"rect": [0, 410, 600, 600], "content": "text", "text": "Te", "caret": false})).unwrap();
    s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
    assert_eq!(warning_count(&mut s), 1, "multiple used runs/stories produce one warning");
}

#[test]
fn review9_optical_warning_history_and_native_zip_keep_the_stored_flag() {
    let (mut s, sid) = new_story("Te", 400.0);
    let before = s.doc().unwrap().doc.clone();
    s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    assert_eq!(warning_count(&mut s), 1);
    let bytes = designcraft_format::save(&after).unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert_eq!(crate::cmd::from_bytes(&bytes).unwrap(), *after);
    let mut reopened = Session::new();
    reopened.execute("file.openBytes", &json!({"base64": crate::cmd::base64_encode(&bytes)})).unwrap();
    assert_eq!(serde_json::to_value(&*reopened.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
    assert_eq!(warning_count(&mut reopened), 1);
    assert_eq!(reopened.doc().unwrap().doc.story(sid).unwrap().format_after(0).over.kerning, Some(Kerning::Optical));
    let imported = designcraft_idml::import_idml(&designcraft_idml::export_idml(&after)).unwrap();
    assert!(imported.stories.values().any(|st| st.runs().any(|(_, f)| f.over.kerning == Some(Kerning::Optical))));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    assert_eq!(warning_count(&mut s), 0);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *after);
    assert_eq!(warning_count(&mut s), 1);
}

#[test]
fn review9_optical_warning_uses_style_inheritance_and_respects_direct_overrides() {
    for paragraph in [false, true] {
        let (mut s, _) = new_story("Te Park", 400.0);
        s.execute("style.character.create", &json!({"name": "Unused Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 0, "unused definitions are not used formatting");
        let create = if paragraph { "style.paragraph.create" } else { "style.character.create" };
        let apply = if paragraph { "style.paragraph.apply" } else { "style.character.apply" };
        s.execute(create, &json!({"name": "Parent Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
        s.execute(create, &json!({"name": "Inherited Optical", "basedOn": "Parent Optical"})).unwrap();
        s.execute(apply, &json!({"name": "Inherited Optical"})).unwrap();
        assert_eq!(warning_count(&mut s), 1);
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 0);
        s.execute("type.char", &json!({"attrs": {"kerning": null}})).unwrap();
        assert_eq!(warning_count(&mut s), 1);
    }
}

#[test]
fn review9_optical_nested_grep_and_line_styles_are_effective_used_formatting() {
    for composer in ["paragraph", "singleLine"] {
        for field in ["nestedStyles", "grepStyles", "nestedLineStyles"] {
            let (mut s, _) = new_story("Te Park Te Park Te Park", 400.0);
            s.execute("style.character.create", &json!({"name": "Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
            s.execute("style.character.create", &json!({"name": "Metrics", "chars": {"kerning": {"kind": "metrics"}}})).unwrap();
            let mut attrs = json!({"composer": composer});
            attrs[field] = match field {
                "nestedStyles" => json!([{"style": "Optical", "count": 2, "through": true, "until": {"kind": "characters"}}]),
                "grepStyles" => json!([{"style": "Optical", "pattern": "Te"}]),
                _ => json!([{"style": "Optical", "lines": 1}]),
            };
            s.execute("type.para", &json!({"attrs": attrs})).unwrap();
            assert_eq!(warning_count(&mut s), 1, "{composer}/{field}");
            s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
            assert_eq!(warning_count(&mut s), 0, "direct formatting overrides {field}");
            s.execute("type.char", &json!({"attrs": {"kerning": null}})).unwrap();
            assert_eq!(warning_count(&mut s), 1);
            attrs[field] = match field {
                "nestedStyles" => json!([{"style": "Metrics", "count": 10000, "until": {"kind": "characters"}}, {"style": "Optical"}]),
                "grepStyles" => json!([{"style": "Optical", "pattern": "NEVERMATCH"}]),
                _ => json!([{"style": "Optical", "lines": 0}]),
            };
            s.execute("type.para", &json!({"attrs": attrs})).unwrap();
            assert_eq!(warning_count(&mut s), 0, "unused automatic style {field}");
        }
    }
}

#[test]
fn review9_optical_automatic_overlay_precedence_avoids_false_warnings() {
    let (mut s, _) = new_story("Te Park", 400.0);
    s.execute("style.character.create", &json!({"name": "Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
    s.execute("style.character.create", &json!({"name": "Metrics", "chars": {"kerning": {"kind": "metrics"}}})).unwrap();
    s.execute(
        "type.para",
        &json!({"attrs": {
            "nestedLineStyles": [{"style": "Optical", "lines": 1}],
            "nestedStyles": [{"style": "Optical", "count": 100, "until": {"kind": "characters"}}],
            "grepStyles": [{"style": "Metrics", "pattern": ".+"}]
        }}),
    )
    .unwrap();
    assert_eq!(warning_count(&mut s), 0, "GREP supersedes nested and line overlays");
}

#[test]
fn review9_optical_table_and_note_text_warn_and_respect_local_metrics() {
    for note in [false, true] {
        let (mut s, sid) = new_story("", 400.0);
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        if note {
            s.execute("footnote.insert", &json!({"text": "Te Park"})).unwrap();
        } else {
            s.execute("table.insert", &json!({"rows": 1, "cols": 1})).unwrap();
            s.execute("text.insert", &json!({"text": "Te Park"})).unwrap();
        }
        s.execute("edit.selectAll", &json!({})).unwrap();
        assert_eq!(warning_count(&mut s), 0);
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 1, "nested text note={note}");
        let after = s.doc().unwrap().doc.clone();
        assert_eq!(crate::cmd::from_bytes(&designcraft_format::save(&after).unwrap()).unwrap(), *after);
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 0);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *after);
        assert_eq!(warning_count(&mut s), 1);
    }
}

#[test]
fn review9_optical_uncomposed_overset_text_is_checked_from_effective_source() {
    for automatic in [false, true] {
        let text = "plain\nTe Park";
        let (mut s, sid) = new_story(text, 1.0);
        s.execute("text.select", &json!({"story": sid.0, "anchor": 6, "focus": text.len()})).unwrap();
        if automatic {
            s.execute("style.character.create", &json!({"name": "Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
            s.execute("type.para", &json!({"attrs": {"grepStyles": [{"style": "Optical", "pattern": "Te"}]}})).unwrap();
        } else {
            s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
        }
        assert!(s.cache.get(&s.doc().unwrap().doc, sid, None).is_overset());
        assert_eq!(warning_count(&mut s), 1, "uncomposed overset automatic={automatic}");
        let r = s.execute("preflight.run", &json!({})).unwrap();
        assert!(r["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "overset" && i["severity"] == "error"));
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 0);
    }
}

#[test]
fn review9_optical_empty_text_and_inactive_idml_numeric_values_are_preserved() {
    let (mut s, _) = new_story("", 400.0);
    s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
    assert_eq!(warning_count(&mut s), 0, "an empty typing format has no used text");
    let (mut nonempty, _) = new_story("Te", 400.0);
    nonempty.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
    let idml = designcraft_idml::export_idml(&nonempty.doc().unwrap().doc);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(idml)).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut replaced = false;
    for i in 0..archive.len() {
        use std::io::{Read, Write};
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if name.starts_with("Stories/") && name.ends_with(".xml") {
            let text = String::from_utf8(data).unwrap();
            let changed = text.replace(r#"KerningMethod="$ID/Optical""#, r#"KerningMethod="$ID/Optical" KerningValue="1e+11""#);
            replaced |= changed != text;
            data = changed.into_bytes();
        }
        output.start_file(name, zip::write::SimpleFileOptions::default().compression_method(entry.compression())).unwrap();
        output.write_all(&data).unwrap();
    }
    assert!(replaced);
    let bytes = output.finish().unwrap().into_inner();
    let d = designcraft_idml::import_idml(&bytes).unwrap();
    let st = d.stories.values().find(|st| st.text == "Te").unwrap();
    assert_eq!(st.format_after(0).over.kerning, Some(Kerning::Optical));
    let mut reopened = Session::new();
    reopened.execute("file.openBytes", &json!({"name": "inactive.idml", "base64": crate::cmd::base64_encode(&bytes)})).unwrap();
    assert_eq!(warning_count(&mut reopened), 1);
    let sid = reopened.doc().unwrap().doc.stories.values().find(|st| st.text == "Te").unwrap().id;
    let cs = reopened.cache.get(&reopened.doc().unwrap().doc, sid, None);
    assert!(cs.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).all(|g| g.adv.is_finite() && g.adv.abs() < 100.0));
}

fn review11_custom_variable_story(value: &str, composer: &str) -> (Session, StoryId) {
    let (mut s, sid) = new_story("", 400.0);
    s.execute("type.para", &json!({"attrs": {"composer": composer}})).unwrap();
    s.execute("variables.define", &json!({"name": "Review11 Header", "type": "custom", "text": value})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    for _ in 0..2 {
        s.execute("variables.insert", &json!({"name": "Review11 Header"})).unwrap();
    }
    let len = s.doc().unwrap().doc.story(sid).unwrap().len();
    assert_eq!(len, 6, "two UTF-8 variable markers, with no literal text");
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": len})).unwrap();
    (s, sid)
}

#[test]
fn review11_optical_variable_warning_preserves_geometry_history_and_native_persistence() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid) = review11_custom_variable_story("AVATAR", composer);
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 0);
        let before = s.doc().unwrap().doc.clone();
        let metrics = geometry(&s, sid);
        assert!(!metrics.is_empty(), "the variable must have composed output");
        assert!(s.cache.get(&before, sid, None).frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).any(|g| g.adv > 0.0));
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        let history = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len());
        assert_eq!(warning_count(&mut s), 1, "two rendered variable values produce one warning: {composer}");
        assert_eq!(warning_count(&mut s), 1, "cached composition retains the warning");
        assert_eq!((s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len()), history);
        assert_eq!(geometry(&s, sid), metrics, "warning detection cannot change Metrics approximation geometry");
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().format_after(0).over.kerning, Some(Kerning::Optical));

        let bytes = designcraft_format::save(&after).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(crate::cmd::from_bytes(&bytes).unwrap(), *after);
        let mut reopened = Session::new();
        reopened.execute("file.openBytes", &json!({"base64": crate::cmd::base64_encode(&bytes)})).unwrap();
        assert_eq!(serde_json::to_value(&*reopened.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
        assert_eq!(warning_count(&mut reopened), 1, "native persistence keeps effective Optical on variables");
        assert_eq!(geometry(&reopened, sid), metrics);
        assert_eq!(reopened.doc().unwrap().doc.story(sid).unwrap().format_after(0).over.kerning, Some(Kerning::Optical));

        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *before);
        let history = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len());
        assert_eq!(warning_count(&mut s), 0);
        assert_eq!((s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len()), history);
        assert_eq!(geometry(&s, sid), metrics);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *after);
        assert_eq!(warning_count(&mut s), 1);
        assert_eq!(geometry(&s, sid), metrics);
    }
}

#[test]
fn review11_optical_variable_warning_uses_effective_formatting_and_resolved_value() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid) = review11_custom_variable_story("AVATAR", composer);
        s.execute("style.character.create", &json!({"name": "Variable Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
        s.execute("style.character.apply", &json!({"name": "Variable Optical"})).unwrap();
        assert_eq!(warning_count(&mut s), 1);
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 0, "direct Metrics overrides inherited Optical");
        s.execute("type.char", &json!({"attrs": {"kerning": null}})).unwrap();
        assert_eq!(warning_count(&mut s), 1);
        s.execute("variables.define", &json!({"name": "Review11 Header", "type": "custom", "text": ""})).unwrap();
        assert_eq!(warning_count(&mut s), 0, "the now-empty value becomes a control glyph");
        assert!(s.cache.get(&s.doc().unwrap().doc, sid, None).frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).all(|g| g.adv == 0.0));
        s.execute("variables.define", &json!({"name": "Review11 Header", "type": "custom", "text": "", "before": "AV", "after": "ATAR"})).unwrap();
        assert_eq!(warning_count(&mut s), 1, "SubstCtx includes the nonempty before/after value");
    }
}

#[test]
fn review11_optical_empty_variables_and_nonrendering_markers_do_not_warn() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid) = review11_custom_variable_story("", composer);
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
        assert_eq!(warning_count(&mut s), 0, "empty variable values have no rendered text");
        assert!(s.cache.get(&s.doc().unwrap().doc, sid, None).frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).all(|g| g.adv == 0.0));

        let missing_variable = designcraft_doc::vars::var_char(designcraft_doc::vars::VAR_MAX - 1).unwrap();
        let text = format!("{}{}{}", designcraft_doc::ANCHOR_MARK, designcraft_doc::INDEX_MARK, missing_variable);
        let (mut markers, sid) = new_story(&text, 400.0);
        assert_eq!(markers.doc().unwrap().doc.story(sid).unwrap().text, text);
        assert!(markers.doc().unwrap().doc.text_variables.len() < designcraft_doc::vars::VAR_MAX);
        markers.execute("type.para", &json!({"attrs": {"composer": composer}})).unwrap();
        markers.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
        assert_eq!(warning_count(&mut markers), 0, "nonrendering private markers must remain excluded");
        assert!(
            markers
                .cache
                .get(&markers.doc().unwrap().doc, sid, None)
                .frames
                .iter()
                .flat_map(|f| &f.lines)
                .flat_map(|l| &l.glyphs)
                .all(|g| g.adv == 0.0)
        );
    }
}

fn review12_warning_count(s: &mut Session) -> usize {
    let history = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len());
    let count = warning_count(s);
    assert_eq!((s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len()), history);
    count
}

fn review12_has_rendered_text(s: &Session, sid: StoryId) -> bool {
    let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
    cs.frames.iter().flat_map(|f| &f.lines).flat_map(|l| &l.glyphs).any(|g| g.visible && g.adv > 0.0)
}

fn review12_optical_edit_roundtrip(s: &mut Session, sid: StoryId, command: &str, params: &Value) {
    assert_eq!(review12_warning_count(s), 0);
    assert!(review12_has_rendered_text(s, sid), "the marker must already render text under Metrics");
    let before = s.doc().unwrap().doc.clone();
    let metrics = geometry(s, sid);
    s.execute(command, params).unwrap();
    let after = s.doc().unwrap().doc.clone();
    assert_eq!(review12_warning_count(s), 1, "rendered replacements must report effective Optical");
    assert_eq!(review12_warning_count(s), 1, "cached composition keeps one deduplicated warning");
    assert_eq!(geometry(s, sid), metrics, "the diagnostic must retain Metrics approximation geometry");

    let bytes = designcraft_format::save(&after).unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert_eq!(crate::cmd::from_bytes(&bytes).unwrap(), *after);
    let mut reopened = Session::new();
    reopened.execute("file.openBytes", &json!({"base64": crate::cmd::base64_encode(&bytes)})).unwrap();
    assert_eq!(serde_json::to_value(&*reopened.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
    assert_eq!(review12_warning_count(&mut reopened), 1);
    assert_eq!(geometry(&reopened, sid), metrics);

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    assert_eq!(review12_warning_count(s), 0);
    assert_eq!(geometry(s, sid), metrics);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *after);
    assert_eq!(review12_warning_count(s), 1);
    assert_eq!(geometry(s, sid), metrics);
}

#[test]
fn review12_optical_page_and_section_replacements_use_effective_formatting() {
    use designcraft_doc::{NEXT_PAGE_NUMBER, PAGE_NUMBER, PREV_PAGE_NUMBER, SECTION_MARKER};
    for composer in ["paragraph", "singleLine"] {
        for marker in [PAGE_NUMBER, NEXT_PAGE_NUMBER, PREV_PAGE_NUMBER, SECTION_MARKER] {
            let text = format!("{marker}{marker}");
            let (mut s, sid) = new_story(&text, 400.0);
            s.execute("type.para", &json!({"attrs": {"composer": composer}})).unwrap();
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
            s.execute("style.character.create", &json!({"name": "Marker Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
            if marker == SECTION_MARKER {
                review12_optical_edit_roundtrip(&mut s, sid, "style.character.apply", &json!({"name": "Marker Optical"}));
            } else {
                assert_eq!(review12_warning_count(&mut s), 0);
                assert!(review12_has_rendered_text(&s, sid));
                let metrics = geometry(&s, sid);
                s.execute("style.character.apply", &json!({"name": "Marker Optical"})).unwrap();
                assert_eq!(review12_warning_count(&mut s), 1, "{composer}/{marker:?}");
                assert_eq!(review12_warning_count(&mut s), 1, "two replacements still produce one cached warning");
                assert_eq!(geometry(&s, sid), metrics);
            }
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, text, "rendered values must not replace source bytes");
            s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
            assert_eq!(review12_warning_count(&mut s), 0, "direct Metrics overrides the marker style");
            s.execute("type.char", &json!({"attrs": {"kerning": null}})).unwrap();
            assert_eq!(review12_warning_count(&mut s), 1, "clearing the direct override restores effective Optical");
        }
    }
}

#[test]
fn review12_optical_xref_warning_tracks_rendered_and_empty_values() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, destination) = new_story("Destination", 400.0);
        let r = s.execute("frame.create", &json!({"rect": [0, 410, 600, 700], "content": "text", "text": "", "caret": false})).unwrap();
        let sid = StoryId(r["story"].as_u64().unwrap());
        s.execute("type.para", &json!({"attrs": {"composer": composer}})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("xref.defineFormat", &json!({"name": "Review12 Reference", "definition": "AVATAR"})).unwrap();
        for _ in 0..2 {
            s.execute("xref.insert", &json!({"story": destination.0, "para": 0, "format": "Review12 Reference"})).unwrap();
        }
        let text = designcraft_doc::XREF_MARK.to_string().repeat(2);
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, text);
        assert!(s.execute("xref.list", &json!({})).unwrap().as_array().unwrap().iter().all(|x| x["text"] == "AVATAR"));
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
        s.execute("style.character.create", &json!({"name": "Xref Optical", "chars": {"kerning": {"kind": "optical"}}})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 0);
        assert!(review12_has_rendered_text(&s, sid));
        let metrics = geometry(&s, sid);
        s.execute("style.character.apply", &json!({"name": "Xref Optical"})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 1, "two rendered cross-references: {composer}");
        assert_eq!(geometry(&s, sid), metrics);
        s.execute("xref.defineFormat", &json!({"name": "Review12 Reference", "definition": ""})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 0, "empty resolved references are only control glyphs");
        assert!(!review12_has_rendered_text(&s, sid));
        s.execute("xref.defineFormat", &json!({"name": "Review12 Reference", "definition": "AVATAR"})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 1);
        assert_eq!(geometry(&s, sid), metrics);
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 0);
        s.execute("type.char", &json!({"attrs": {"kerning": null}})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 1);
    }
}

#[test]
fn review12_optical_footnote_reference_style_is_checked_after_resolution() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid) = new_story("", 400.0);
        s.execute("type.para", &json!({"attrs": {"composer": composer}})).unwrap();
        for (name, kind) in [("Reference Metrics", "metrics"), ("Reference Optical", "optical")] {
            s.execute("style.character.create", &json!({"name": name, "chars": {"kerning": {"kind": kind}}})).unwrap();
        }
        s.execute("footnote.options", &json!({"refCharStyle": "Reference Metrics", "prefix": "AVATAR ", "affixIn": "reference"})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        for _ in 0..2 {
            s.execute("footnote.insert", &json!({"text": "Note body"})).unwrap();
            s.execute("footnote.goToReference", &json!({})).unwrap();
        }
        let text = designcraft_doc::FOOTNOTE_REF.to_string().repeat(2);
        assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, text);
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
        review12_optical_edit_roundtrip(&mut s, sid, "footnote.options", &json!({"refCharStyle": "Reference Optical"}));
        assert!(s.doc().unwrap().doc.story(sid).unwrap().runs().all(|(_, f)| f.over.kerning.is_none()));
        s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 0, "direct Metrics really overrides the Optical reference style");
        s.execute("type.char", &json!({"attrs": {"kerning": null}})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 1);
        s.execute("style.character.apply", &json!({"name": "Reference Optical"})).unwrap();
        s.execute("footnote.options", &json!({"refCharStyle": "Reference Metrics"})).unwrap();
        assert_eq!(review12_warning_count(&mut s), 0, "the final Metrics reference style replaces the source Optical style");
    }
}

#[test]
fn review12_optical_footnote_and_endnote_labels_use_direct_formatting() {
    for composer in ["paragraph", "singleLine"] {
        for endnote in [false, true] {
            let (mut s, sid) = new_story("", 400.0);
            s.execute("type.para", &json!({"attrs": {"composer": composer}})).unwrap();
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
            s.execute(if endnote { "endnote.insert" } else { "footnote.insert" }, &json!({"text": "Metrics note body"})).unwrap();
            let marker = if endnote { designcraft_doc::ENDNOTE_REF } else { designcraft_doc::FOOTNOTE_REF };
            assert_eq!(s.doc().unwrap().doc.story(sid).unwrap().text, marker.to_string());
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": marker.len_utf8()})).unwrap();
            assert_eq!(review12_warning_count(&mut s), 0);
            assert!(review12_has_rendered_text(&s, sid));
            let metrics = geometry(&s, sid);
            s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "optical"}}})).unwrap();
            assert_eq!(review12_warning_count(&mut s), 1, "{composer}/endnote={endnote}: only the reference has Optical");
            assert_eq!(geometry(&s, sid), metrics);
            s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
            assert_eq!(review12_warning_count(&mut s), 0);
        }
    }
}

fn review13_label_styles(s: &mut Session, composer: &str, list_type: &str, list_name: &str) {
    for (name, kerning) in [("Label Metrics", "metrics"), ("Label Optical", "optical")] {
        s.execute(
            "style.paragraph.create",
            &json!({
                "name": name,
                "chars": {"fontFamily": "Source Serif 4", "fontStyle": "Regular", "size": 12, "kerning": {"kind": kerning}},
                "para": {"composer": composer, "hyphenate": false, "listType": list_type, "listName": list_name,
                    "bulletChar": "AV", "listSeparator": " "}
            }),
        )
        .unwrap();
    }
}

fn review13_list_labels(text: &str, composer: &str, list_type: &str, named: bool) -> (Session, StoryId) {
    let (mut s, sid) = new_story(text, 400.0);
    let list_name = if named { "Review13 Steps" } else { "" };
    if named {
        s.execute("list.define", &json!({"name": list_name})).unwrap();
    }
    review13_label_styles(&mut s, composer, list_type, list_name);
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
    s.execute("style.paragraph.apply", &json!({"name": "Label Metrics"})).unwrap();
    s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
    if list_type == "numbers" {
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("type.para", &json!({"attrs": {"startAt": 10}})).unwrap();
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": text.len()})).unwrap();
    }
    assert!(s.doc().unwrap().doc.story(sid).unwrap().chars.iter().all(|r| r.format.over.kerning == Some(Kerning::Metrics)));
    (s, sid)
}

fn review13_note_label(text: &str, composer: &str) -> (Session, StoryId) {
    let (mut s, sid) = new_story("", 400.0);
    s.execute("type.para", &json!({"attrs": {"composer": composer}})).unwrap();
    review13_label_styles(&mut s, composer, "none", "");
    s.execute("footnote.options", &json!({"paraStyle": "Label Metrics", "prefix": "AV", "affixIn": "text", "separator": " "})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
    s.execute("footnote.insert", &json!({"text": text})).unwrap();
    s.execute("edit.selectAll", &json!({})).unwrap();
    s.execute("type.char", &json!({"attrs": {"kerning": {"kind": "metrics"}}})).unwrap();
    let d = &s.doc().unwrap().doc;
    let story = d.story(sid).unwrap();
    assert_eq!(story.text, designcraft_doc::FOOTNOTE_REF.to_string());
    let base = d.styles.resolve_para(&story.paras[0]).1;
    assert_eq!(d.styles.resolve_char(&base, story.format_after(0)).kerning, Kerning::Metrics, "the reference is a Metrics control");
    assert_eq!(story.notes.len(), 1);
    assert_eq!(story.notes[0].text.text, text);
    assert!(story.notes[0].text.chars.iter().all(|r| r.format.over.kerning == Some(Kerning::Metrics)));
    assert!(s.doc().unwrap().selection.text.unwrap().cell.unwrap().footnote_id().is_some());
    (s, sid)
}

fn review13_lines_with_labels(cs: &ComposedStory) -> usize {
    cs.frames.iter().flat_map(|f| &f.lines).filter(|l| l.glyphs.iter().any(|g| g.len == 0 && g.visible && g.adv > 0.0)).count()
}

// Include the note body's own glyph metrics and placement; the older geometry helper sees only the parent story.
fn review13_label_geometry(s: &Session, sid: StoryId) -> Vec<Value> {
    fn record(cs: &ComposedStory, out: &mut Vec<Value>) {
        assert!(!cs.is_overset(), "fixture must compose all source text and generated labels");
        for frame in &cs.frames {
            out.push(json!(["frame", frame.frame.0, frame.vertical, frame.range.start, frame.range.end]));
            for line in &frame.lines {
                out.push(json!([
                    "line",
                    line.column,
                    line.range.start,
                    line.range.end,
                    line.baseline,
                    line.x0,
                    line.x1,
                    line.end_x,
                    line.ascent,
                    line.descent,
                    line.leading
                ]));
                for g in &line.glyphs {
                    out.push(json!(["glyph", g.face.family, g.face.style, g.gid, g.byte, g.len, g.visible, g.x, g.y, g.adv, g.sx, g.sy]));
                }
            }
            for note in &frame.notes {
                out.push(json!(["note", note.id, note.column, note.origin.x, note.origin.y, note.rect.x0, note.rect.y0, note.rect.x1, note.rect.y1]));
                record(&note.text, out);
            }
        }
    }
    let mut out = Vec::new();
    record(&s.cache.get(&s.doc().unwrap().doc, sid, None), &mut out);
    out
}

fn review13_label_warning(s: &mut Session, sid: StoryId, expected: usize) {
    let selection = s.doc().unwrap().selection.clone();
    let doc = s.doc().unwrap().doc.clone();
    let first = s.cache.get(&doc, sid, None);
    assert!(std::sync::Arc::ptr_eq(&first, &s.cache.get(&doc, sid, None)), "fixture must reach a warm cache");
    assert_eq!(review12_warning_count(s), expected);
    assert_eq!(review12_warning_count(s), expected, "warm preflight must retain a single deduplicated warning");
    assert!(std::sync::Arc::ptr_eq(&first, &s.cache.get(&doc, sid, None)));
    assert_eq!(s.doc().unwrap().selection, selection, "preflight must preserve the selection");
}

fn review13_label_roundtrip(s: &mut Session, sid: StoryId) {
    review13_label_warning(s, sid, 0);
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo_len = s.doc().unwrap().history.undo.len();
    let metrics = review13_label_geometry(s, sid);
    s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    assert_ne!(*after, *before, "the paragraph base actually changes");
    let target = s.doc().unwrap().selection.text.unwrap();
    let source = after.text_story(target.story, target.cell).unwrap();
    assert!(source.paras.iter().all(|p| after.styles.resolve_para(p).1.kerning == Kerning::Optical));
    assert!(source.chars.iter().all(|r| r.format.over.kerning == Some(Kerning::Metrics)), "only generated labels use Optical");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_len + 1);
    review13_label_warning(s, sid, 1);
    assert_eq!(review13_label_geometry(s, sid), metrics, "the warning preserves Metrics approximation geometry, including note labels");

    let serialized = s.execute("file.serialize", &json!({})).unwrap();
    let bytes = crate::cmd::file::base64_decode(serialized["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"));
    assert_eq!(crate::cmd::from_bytes(&bytes).unwrap(), *after);
    let mut reopened = Session::new();
    reopened.execute("file.openBytes", &json!({"base64": serialized["base64"]})).unwrap();
    assert_eq!(serde_json::to_value(&*reopened.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
    review13_label_warning(&mut reopened, sid, 1);
    assert_eq!(review13_label_geometry(&reopened, sid), metrics);

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before, "undo restores the entire document");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_len);
    review13_label_warning(s, sid, 0);
    assert_eq!(review13_label_geometry(s, sid), metrics);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *after, "redo restores the entire document");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_len + 1);
    review13_label_warning(s, sid, 1);
    assert_eq!(review13_label_geometry(s, sid), metrics);
}

#[test]
fn review13_optical_labels_numbered_lists_warn_in_both_composers() {
    for composer in ["paragraph", "singleLine"] {
        for named in [false, true] {
            let (mut s, sid) = review13_list_labels("First\nSecond", composer, "numbers", named);
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            assert_eq!(review13_lines_with_labels(&cs), 2, "both numbered paragraphs must render generated labels");
            review13_label_roundtrip(&mut s, sid);
        }
    }
}

#[test]
fn review13_optical_labels_bulleted_lists_warn_in_both_composers() {
    for composer in ["paragraph", "singleLine"] {
        let (mut s, sid) = review13_list_labels("First\nSecond", composer, "bullets", false);
        let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
        assert_eq!(review13_lines_with_labels(&cs), 2, "two actual AV labels must yield only one warning");
        review13_label_roundtrip(&mut s, sid);
    }
}

#[test]
fn fc2_optical_empty_list_paragraphs_have_no_label_or_warning() {
    for composer in ["paragraph", "singleLine"] {
        for list_type in ["numbers", "bullets"] {
            let (mut s, sid) = review13_list_labels("", composer, list_type, false);
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            assert!(s.doc().unwrap().doc.story(sid).unwrap().text.is_empty());
            assert_eq!(review13_lines_with_labels(&cs), 0, "fc2 suppresses labels on empty list paragraphs");
            review13_label_warning(&mut s, sid, 0);
            s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
            review13_label_warning(&mut s, sid, 0);
            let saved = s.execute("file.serialize", &json!({})).unwrap();
            let mut reopened = Session::new();
            reopened.execute("file.openBytes", &json!({"base64": saved["base64"]})).unwrap();
            review13_label_warning(&mut reopened, sid, 0);
        }
    }
}

#[test]
fn review13_optical_labels_footnote_body_labels_warn_in_both_composers() {
    for composer in ["paragraph", "singleLine"] {
        for body in ["Note body", ""] {
            let (mut s, sid) = review13_note_label(body, composer);
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            let notes: Vec<_> = cs.frames.iter().flat_map(|f| &f.notes).collect();
            assert_eq!(notes.len(), 1, "exercise a placed footnote, not source-only fallback");
            assert_eq!(review13_lines_with_labels(&notes[0].text), 1, "the note body actually has its generated label");
            review13_label_roundtrip(&mut s, sid);
        }
    }
}

#[test]
fn review13_optical_labels_metrics_and_unused_styles_do_not_warn() {
    for composer in ["paragraph", "singleLine"] {
        for text in ["First\nSecond", ""] {
            for list_type in ["numbers", "bullets"] {
                let (mut s, sid) = review13_list_labels(text, composer, list_type, false);
                assert_eq!(review13_lines_with_labels(&s.cache.get(&s.doc().unwrap().doc, sid, None)), if text.is_empty() { 0 } else { 2 });
                review13_label_warning(&mut s, sid, 0);
            }
        }
        for body in ["Note body", ""] {
            let (mut s, sid) = review13_note_label(body, composer);
            let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
            assert!(cs.frames.iter().flat_map(|f| &f.notes).any(|n| review13_lines_with_labels(&n.text) > 0));
            review13_label_warning(&mut s, sid, 0);
        }
    }
}

#[test]
fn review13_optical_labels_absent_or_empty_labels_do_not_warn() {
    for composer in ["paragraph", "singleLine"] {
        for text in ["Body", ""] {
            for list_type in ["none", "bullets"] {
                let (mut s, sid) = review13_list_labels(text, composer, list_type, false);
                if list_type == "bullets" {
                    s.execute("type.para", &json!({"attrs": {"bulletChar": "", "listSeparator": ""}})).unwrap();
                }
                let metrics = review13_label_geometry(&s, sid);
                s.execute("style.paragraph.apply", &json!({"name": "Label Optical"})).unwrap();
                let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
                assert_eq!(review13_lines_with_labels(&cs), 0);
                review13_label_warning(&mut s, sid, 0);
                assert_eq!(review13_label_geometry(&s, sid), metrics, "an Optical base with no rendered label stays a Metrics-body control");
            }
        }
    }
}

use super::*;

#[test]
fn review9_markdown_shortcut_place_preserves_source_and_full_document_history_zip() {
    let cases = [
        ("[**label**]\n\n[**label**]: /target", "[**label**]\n[**label**]: /target", false, false),
        ("[**label**]: /target\n\n[**label**]", "[**label**]: /target\n[**label**]", false, false),
        ("![**label**]\n\n[**label**]: /target", "![**label**]\n[**label**]: /target", false, false),
        ("[**é中**]\n\n[**é中**]: /target", "[**é中**]\n[**é中**]: /target", false, false),
        ("[**label**][ref]\n\n[ref]: /target", "[**label**][ref]\n[ref]: /target", false, false),
        ("[**label**][]\n\n[**label**]: /target", "[**label**][]\n[**label**]: /target", false, false),
        ("[**label**]", "[label]", true, false),
        ("[**label**]\n\n[other]: /target", "[label]\n[other]: /target", true, false),
        (
            r"\[**label**\]

[**label**]: /target",
            "[label]\n[**label**]: /target",
            true,
            false,
        ),
        ("\u{0060}[**label**]\u{0060}\n\n[**label**]: /target", "[**label**]\n[**label**]: /target", false, true),
    ];
    for (source, expected, strong, code) in cases {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("frame.create", &json!({"rect": [20, 20, 250, 70], "content": "text", "text": "Untouched guard"})).unwrap();
        s.execute("selection.set", &json!({"ids": []})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Mapped Body", "chars": {"size": 12, "fontStyle": "Regular"}})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let b64 = crate::cmd::base64_encode(source.as_bytes());
        let info = s.execute("place.styles", &json!({"name": "shortcut.md", "base64": b64})).unwrap();
        assert_eq!(info["character"].as_array().unwrap().contains(&json!("Strong")), strong, "{source}");
        assert_eq!(info["character"].as_array().unwrap().contains(&json!("Code")), code, "{source}");
        assert_eq!(*s.doc().unwrap().doc, *before, "style preview is read-only");
        let placed = s
            .execute(
                "file.place",
                &json!({"name": "shortcut.md", "base64": b64, "rect": [20, 100, 400, 500],
            "styleMap": {"Body Text": "Mapped Body"}}),
            )
            .unwrap();
        let after = s.doc().unwrap().doc.clone();
        let story = after.story(StoryId(placed["story"].as_u64().unwrap())).unwrap();
        assert_eq!(story.text, expected, "{source}");
        assert!(story.paras.iter().all(|p| p.style == "Mapped Body"));
        assert_eq!(story.runs().any(|(_, f)| f.style == "Strong"), strong);
        assert_eq!(story.runs().any(|(_, f)| f.style == "Code"), code);
        story.check().unwrap();
        let bytes = designcraft_format::save(&after).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(crate::cmd::from_bytes(&bytes).unwrap(), *after);
        let mut reopened = Session::new();
        reopened.execute("file.openBytes", &json!({"base64": crate::cmd::base64_encode(&bytes)})).unwrap();
        assert_eq!(serde_json::to_value(&*reopened.doc().unwrap().doc).unwrap(), serde_json::to_value(&*after).unwrap());
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *before);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *after);
    }
}

#[test]
fn review9_markdown_shortcut_caret_placement_preserves_neighbors_and_history() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = s.execute("frame.create", &json!({"rect": [20, 20, 400, 500], "content": "text", "text": "before  after"})).unwrap();
    s.execute("text.select", &json!({"story": r["story"], "anchor": 7, "focus": 7})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    s.execute("file.place", &json!({"name": "shortcut.md", "base64": crate::cmd::base64_encode(b"[**label**]\n\n[**label**]: /target")})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    let story = after.story(StoryId(r["story"].as_u64().unwrap())).unwrap();
    assert_eq!(story.text, "before [**label**]\n[**label**]: /target after");
    assert!(!story.runs().any(|(_, f)| f.style == "Strong"));
    story.check().unwrap();
    assert_eq!(crate::cmd::from_bytes(&designcraft_format::save(&after).unwrap()).unwrap(), *after);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *after);
}

#[test]
fn review15_markdown_reference_continuation_placement_preserves_native_document_and_history() {
    for (source, expected, emphasis) in [
        ("[ref]:\n  /path*with*stars\n\n[ref]", "[ref]:\n  /path*with*stars\n[ref]", false),
        ("[ref]: /path*with*stars\n\n[ref]", "[ref]: /path*with*stars\n[ref]", false),
        ("[ref]: /target\n  \"**caption**\"\n\n[ref]", "[ref]: /target\n  \"**caption**\"\n[ref]", false),
        (
            r#"[ref]:
  /target
  "**é中** \"quoted\""

[ref]"#,
            r#"[ref]:
  /target
  "**é中** \"quoted\""
[ref]"#,
            false,
        ),
        ("ordinary *prose*", "ordinary prose", true),
    ] {
        for caret in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", &json!({})).unwrap();
            let guard = s.execute("frame.create", &json!({"rect": [20, 20, 400, 80], "content": "text", "text": "before  after"})).unwrap();
            if caret {
                s.execute("text.select", &json!({"story": guard["story"], "anchor": 7, "focus": 7})).unwrap();
            } else {
                s.execute("selection.set", &json!({"ids": []})).unwrap();
            }
            let before = s.doc().unwrap().doc.clone();
            let base64 = crate::cmd::base64_encode(source.as_bytes());
            let params = json!({"name": "multiline-reference.md", "base64": base64, "rect": [20, 120, 400, 400]});
            let info = s.execute("place.styles", &params).unwrap();
            assert_eq!(info["character"], if emphasis { json!(["Emphasis"]) } else { json!([]) });
            assert_eq!(*s.doc().unwrap().doc, *before, "style preview must preserve the document");
            let result = s.execute("file.place", &params).unwrap();
            let after = s.doc().unwrap().doc.clone();
            let sid = StoryId(result["story"].as_u64().unwrap());
            let story = after.story(sid).unwrap();
            let expected = if caret { format!("before {expected} after") } else { expected.to_string() };
            assert_eq!(story.text, expected, "caret={caret}, source={source:?}");
            if emphasis {
                assert_eq!(story.format_after(expected.find("prose").unwrap()).style, "Emphasis");
            } else {
                assert!(story.chars.iter().all(|run| run.format == designcraft_doc::CharFormat::default()));
                assert_eq!(after.styles.character, before.styles.character);
            }
            if !caret {
                let guard_id = StoryId(guard["story"].as_u64().unwrap());
                assert_eq!(after.story(guard_id), before.story(guard_id), "placement must preserve the other story");
            }
            after.check().unwrap();
            let serialized = s.execute("file.serialize", &json!({})).unwrap();
            let bytes = crate::cmd::file::base64_decode(serialized["base64"].as_str().unwrap());
            assert!(bytes.starts_with(b"PK"), "exercise the native ZIP format");
            let mut reopened = Session::new();
            reopened.execute("file.openBytes", &json!({"base64": serialized["base64"]})).unwrap();
            assert_eq!(*reopened.doc().unwrap().doc, *after);
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *after);
        }
    }
}

#[test]
fn run34_markdown_unicode_spacing_survives_placement_and_history() {
    for (source, expected, first_style) in [
        ("\u{2003}Indented\u{a0}", "\u{2003}Indented\u{a0}", "Body Text"),
        ("  \u{a0}Indented\u{2003} \t", "\u{a0}Indented\u{2003}", "Body Text"),
        ("\u{2003}\u{a0}", "\u{2003}\u{a0}", "Body Text"),
        ("before\n\n\u{2003}\u{a0}\n\nafter", "before\n\u{2003}\u{a0}\nafter", "Body Text"),
        (
            "  first\u{a0}\n \t\u{2003}second\u{a0}  \n  \u{2003}third\u{a0}\\\n  \u{2003}fourth\u{a0} \t",
            "first\u{a0} \u{2003}second\u{a0}\u{2028}\u{2003}third\u{a0}\u{2028}\u{2003}fourth\u{a0}",
            "Body Text",
        ),
        ("  \u{2003}**Bold**\u{a0} \t", "\u{2003}Bold\u{a0}", "Body Text"),
        ("# \u{2003}Heading\u{a0} ### \t", "\u{2003}Heading\u{a0}", "Heading 1"),
        ("\u{a0}Heading\u{2003}\n--- \t", "\u{a0}Heading\u{2003}", "Heading 2"),
        ("  first \t\n  second  \n \tthird\\\n  fourth \t\n \t\nlast", "first second\u{2028}third\u{2028}fourth\nlast", "Body Text"),
    ] {
        for caret in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", &json!({})).unwrap();
            let guard = s.execute("frame.create", &json!({"rect": [20, 20, 400, 80], "content": "text", "text": "before  after"})).unwrap();
            let guard_id = StoryId(guard["story"].as_u64().unwrap());
            if caret {
                s.execute("text.select", &json!({"story": guard["story"], "anchor": 7, "focus": 7})).unwrap();
            } else {
                s.execute("selection.set", &json!({"ids": []})).unwrap();
            }
            let before = s.doc().unwrap().doc.clone();
            let params = json!({"name": "spacing.md", "base64": crate::cmd::base64_encode(source.as_bytes()), "rect": [20, 120, 400, 400]});
            s.execute("place.styles", &params).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *before, "style preview must preserve the document");
            let result = s.execute("file.place", &params).unwrap();
            assert_eq!(result["warnings"], json!([]), "{source:?}");
            let after = s.doc().unwrap().doc.clone();
            let story = after.story(StoryId(result["story"].as_u64().unwrap())).unwrap();
            let expected = if caret { format!("before {expected} after") } else { expected.to_string() };
            assert_eq!(story.text, expected, "caret={caret}, source={source:?}");
            if source.contains("**Bold**") {
                assert_eq!(story.format_after(expected.find("Bold").unwrap()).style, "Strong");
            }
            if !caret {
                assert_eq!(story.paras[0].style, first_style);
                assert_eq!(after.story(guard_id), before.story(guard_id), "placement must preserve the other story");
            }
            after.check().unwrap();
            assert_eq!(crate::cmd::from_bytes(&designcraft_format::save(&after).unwrap()).unwrap(), *after);
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *after);
        }
    }
}

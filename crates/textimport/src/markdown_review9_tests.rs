use super::*;

fn check(source: &str, expected: &str, strong: bool, code: bool, warning: bool) {
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert_eq!(imported.story.text, expected, "{source}");
    assert_eq!(imported.char_styles.iter().any(|s| s.name == "Strong"), strong, "{source}");
    assert_eq!(imported.char_styles.iter().any(|s| s.name == "Code"), code, "{source}");
    assert_eq!(imported.warnings.iter().any(|w| w.contains("links, images")), warning, "{source}");
    for (range, _) in imported.story.runs() {
        assert!(imported.story.text.is_char_boundary(range.start));
        assert!(imported.story.text.is_char_boundary(range.end));
    }
}

#[test]
fn review9_shortcut_references_preserve_links_images_and_definition_order() {
    for label in ["**label**", "**é中**"] {
        for prefix in ["", "!"] {
            let usage = format!("{prefix}[{label}]");
            let definition = format!("[{label}]: /target");
            for first in [false, true] {
                let (a, b) = if first { (&definition, &usage) } else { (&usage, &definition) };
                for separator in ["\n", "\n\n"] {
                    check(&format!("{a}{separator}{b}"), &format!("{a}\n{b}"), false, false, true);
                }
            }
        }
    }
}

#[test]
fn review9_shortcut_reference_label_matching_is_case_and_whitespace_insensitive() {
    check("[**É中**]\n\n[**é中**]: /target", "[**É中**]\n[**é中**]: /target", false, false, true);
    check("[**la bel**]\n\n[**LA   BEL**]: /target", "[**la bel**]\n[**LA   BEL**]: /target", false, false, true);
}

#[test]
fn review9_full_collapsed_inline_and_image_fallbacks_remain_literal() {
    for usage in ["[**label**][ref]", "[**label**][]", "![**label**][ref]", "![**label**][]", "[**label**](/target)"] {
        let source = format!("{usage}\n\n[ref]: /target\n[**label**]: /target");
        check(&source, &source.replace("\n\n", "\n"), false, false, true);
    }
}

#[test]
fn review9_bare_unmatched_code_and_escaped_brackets_keep_emphasis() {
    check("[**label**]", "[label]", true, false, false);
    check("[**label**]\n\n[other]: /target", "[label]\n[other]: /target", true, false, false);
    check("[**label**]\n\n[**label**]:", "[label]\n[**label**]:", true, false, false);
    check("\u{0060}[**label**]\u{0060}\n\n[**label**]: /target", "[**label**]\n[**label**]: /target", false, true, false);
    check(
        r"\[**label**\]

[**label**]: /target",
        "[label]\n[**label**]: /target",
        true,
        false,
        false,
    );
    check("[**label**\n\n[**label**]: /target", "[label\n[**label**]: /target", true, false, false);
}

#[test]
fn review9_definitions_inside_code_html_and_tables_do_not_resolve_shortcuts() {
    for container in [
        "\u{0060}\u{0060}\u{0060}\n[**label**]: /target\n\u{0060}\u{0060}\u{0060}",
        "    [**label**]: /target",
        "intro \u{0060}[**label**]: /target\nend\u{0060}",
        "<div>\n[**label**]: /target\n</div>",
        "| Header |\n| --- |\n[**label**]: /target",
        "> [**label**]: /target",
    ] {
        let source = format!("[**label**]\n\n{container}");
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text.lines().next(), Some("[label]"), "{source}");
        assert!(imported.char_styles.iter().any(|s| s.name == "Strong"), "{source}");
        assert!(!imported.warnings.iter().any(|w| w.contains("links, images")), "{source}");
    }
}

#[test]
fn review9_shortcut_in_a_prose_paragraph_keeps_the_complete_source() {
    check("before *em* [**label**] after\n\n[**label**]: /target", "before *em* [**label**] after\n[**label**]: /target", false, false, true);
}

#[test]
fn review9_reference_discovery_retains_existing_import_limits() {
    let source = (0..10_001).map(|i| format!("[label{i}]: /target\n")).collect::<String>();
    assert!(matches!(import(source.as_bytes()), Err(ImportError::Unsupported(_))));
    let label = "a".repeat(1000);
    let imported = import(format!("[**{label}**]\n\n[**{label}**]: /target").as_bytes()).unwrap();
    assert!(imported.char_styles.iter().any(|s| s.name == "Strong"));
    imported.story.check().unwrap();
}

#[test]
fn review15_multiline_reference_destinations_remain_literal_in_both_passes() {
    for (source, expected) in [
        ("[ref]:\n  /path*with*stars\n\n[ref]", "[ref]:\n  /path*with*stars\n[ref]"),
        ("[**ref**]\n\n[**ref**]:\n  /path*with*stars", "[**ref**]\n[**ref**]:\n  /path*with*stars"),
        ("[ref]:\n  /path\\*with\\*stars\n\n[ref]", "[ref]:\n  /path\\*with\\*stars\n[ref]"),
    ] {
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text, expected, "{source:?}");
        assert_eq!(imported.story.paras.len(), 3);
        assert!(imported.story.chars.iter().all(|run| run.format == CharFormat::default()));
        assert!(imported.char_styles.is_empty());
        assert_eq!(imported.warnings.len(), 2);
        assert_eq!(
            imported.warnings.iter().map(String::as_str).collect::<std::collections::BTreeSet<_>>(),
            [
                "Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.",
                "Markdown paragraphs containing links, images or HTML were kept as source text.",
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
        );
    }
}

#[test]
fn review15_deferred_reference_destinations_are_emitted_once_and_literal() {
    for destination in ["  /path*with*stars", "  [nested]:/path*with*stars"] {
        let source = format!("\u{0060}unclosed\n[ref]:\n{destination}\n\n[ref]");
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text, format!("\u{0060}unclosed\n[ref]:\n{destination}\n[ref]"));
        assert_eq!(imported.story.paras.len(), 4);
        assert!(imported.story.chars.iter().all(|run| run.format == CharFormat::default()));
        assert!(imported.char_styles.is_empty());
        assert_eq!(imported.warnings.len(), 2);
    }
}

#[test]
fn review15_reference_continuation_does_not_capture_ordinary_prose() {
    for (source, expected, warning) in [
        ("ordinary *prose* and **bold**", "ordinary prose and bold", false),
        ("[ref]:\n  ordinary *prose* and **bold**", "[ref]:\nordinary prose and bold", true),
        ("[ref]:\n\nordinary *prose* and **bold**", "[ref]:\nordinary prose and bold", true),
        ("[ref]: /path*with*stars\n  ordinary *prose* and **bold**", "[ref]: /path*with*stars\nordinary prose and bold", true),
    ] {
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text, expected, "{source:?}");
        assert_eq!(imported.story.format_after(expected.find("prose").unwrap()).style, "Emphasis");
        assert_eq!(imported.story.format_after(expected.find("bold").unwrap()).style, "Strong");
        assert_eq!(imported.warnings.len(), usize::from(warning));
        for (at, _) in expected[..expected.find("ordinary").unwrap()].char_indices() {
            assert_eq!(imported.story.format_after(at), &CharFormat::default());
        }
    }
}

#[test]
fn review15_reference_destination_consumes_only_the_recognized_row() {
    let source = "[ref]:\n  /path*with*stars\nordinary *prose* and **bold**";
    let expected = "[ref]:\n  /path*with*stars\nordinary prose and bold";
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert_eq!(imported.story.text, expected);
    assert_eq!(imported.story.paras.len(), 3);
    assert_eq!(imported.story.format_after(expected.find("prose").unwrap()).style, "Emphasis");
    assert_eq!(imported.story.format_after(expected.find("bold").unwrap()).style, "Strong");
    for (at, _) in expected[..expected.find("ordinary").unwrap()].char_indices() {
        assert_eq!(imported.story.format_after(at), &CharFormat::default());
    }
    assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
}

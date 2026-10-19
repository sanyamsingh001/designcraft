use super::*;

fn check_literal(source: &str, expected: &str) {
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert_eq!(imported.story.text, expected, "{source:?}");
    assert_eq!(imported.story.paras.len(), expected.lines().count(), "{source:?}");
    assert!(imported.story.chars.iter().all(|run| run.format == CharFormat::default()), "{source:?}");
    assert!(imported.char_styles.is_empty(), "{source:?}");
    assert_eq!(imported.warnings.len(), 2, "{source:?}");
    assert!(imported.warnings.iter().any(|w| w.contains("references")), "{source:?}");
    assert!(imported.warnings.iter().any(|w| w.contains("links, images")), "{source:?}");
}

#[test]
fn reference_titles_preserve_complete_rows_in_both_orders_and_paths() {
    for definition in ["[**ref**]: /target", "[**ref**]: <target>", "[**ref**]:\n  /target", "[**ref**]:\n  <target>"] {
        for title in [
            "  \"**caption**\"",
            "   '*é中*'",
            "  (**caption**)",
            "  \"\"",
            "  ''",
            "  ()",
            r#"  "**é中** \"quoted\" \*escaped\* \\ end"   "#,
            r"  '*é中* \'quoted\' \*escaped\* \\ end'",
            r"  (**caption** \) \*escaped\* \\ end)",
            "\t\"**caption**\"  ",
        ] {
            for prefix in ["", "\u{0060}unclosed\n"] {
                let definition = format!("{prefix}{definition}\n{title}");
                for before in [false, true] {
                    let (source, expected) = if before {
                        (format!("[**ref**]\n\n{definition}"), format!("[**ref**]\n{definition}"))
                    } else {
                        (format!("{definition}\n\n[**ref**]"), format!("{definition}\n[**ref**]"))
                    };
                    check_literal(&source, &expected);
                }
            }
        }
    }
}

#[test]
fn reference_titles_stop_before_following_prose() {
    for definition in ["[**ref**]: /target", "[**ref**]:\n  /target"] {
        for prefix in ["", "\u{0060}unclosed\n"] {
            let literal = format!("{prefix}{definition}\n  \"**caption**\"");
            let source = format!("{literal}\nordinary *prose* and **bold**\n\n[**ref**]");
            let expected = format!("{literal}\nordinary prose and bold\n[**ref**]");
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.format_after(expected.find("prose").unwrap()).style, "Emphasis");
            assert_eq!(imported.story.format_after(expected.find("bold").unwrap()).style, "Strong");
            for (at, _) in literal.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{source:?}");
            }
        }
    }
}

#[test]
fn reference_titles_reject_ordinary_unclosed_and_trailing_text_rows() {
    for definition in ["[**ref**]: /target", "[**ref**]:\n  /target"] {
        for prefix in ["", "\u{0060}unclosed\n"] {
            for (row, expected_row, word, style) in [
                ("ordinary *prose*", "ordinary prose", "prose", "Emphasis"),
                ("\"**caption**\" trailing *prose*", "\"caption\" trailing prose", "caption", "Strong"),
                ("\"**caption**", "\"caption", "caption", "Strong"),
                ("'**caption**", "'caption", "caption", "Strong"),
                ("(**caption**", "(caption", "caption", "Strong"),
                (r#""**caption**\""#, "\"caption\"", "caption", "Strong"),
            ] {
                let source = format!("{prefix}{definition}\n{row}\n\n[**ref**]");
                let expected = format!("{prefix}{definition}\n{expected_row}\n[**ref**]");
                let imported = import(source.as_bytes()).unwrap();
                imported.story.check().unwrap();
                assert_eq!(imported.story.text, expected, "{source:?}");
                assert_eq!(imported.story.format_after(expected.find(word).unwrap()).style, style, "{source:?}");
                assert!(imported.warnings.iter().any(|w| w.contains("links, images")), "{source:?}");
            }
        }
    }
}

#[test]
fn reference_titles_do_not_cross_blank_rows() {
    for definition in ["[**ref**]: /target", "[**ref**]:\n  /target"] {
        for prefix in ["", "\u{0060}unclosed\n"] {
            let source = format!("{prefix}{definition}\n\n\"**caption**\"\n\n[**ref**]");
            let expected = format!("{prefix}{definition}\n\"caption\"\n[**ref**]");
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.format_after(expected.find("caption").unwrap()).style, "Strong");
        }
    }
}

#[test]
fn reference_titles_accept_only_one_title() {
    for definition in [
        "[**ref**]: /target \"**caption**\"",
        "[**ref**]:\n  /target \"**caption**\"",
        "[**ref**]: /target\n  \"**caption**\"",
        "[**ref**]:\n  /target\n  \"**caption**\"",
    ] {
        for prefix in ["", "\u{0060}unclosed\n"] {
            let literal = format!("{prefix}{definition}");
            let source = format!("{literal}\n\"**ordinary**\"\n\n[**ref**]");
            let expected = format!("{literal}\n\"ordinary\"\n[**ref**]");
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.format_after(expected.find("ordinary").unwrap()).style, "Strong");
            for (at, _) in literal.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{source:?}");
            }
        }
    }
}

#[test]
fn reference_titles_consume_candidate_looking_rows_once() {
    for prefix in ["", "\u{0060}unclosed\n"] {
        let literal = format!("{prefix}[**ref**]:\n  [nested]:/path*with*stars\n  \"[inner]: **caption**\"");
        check_literal(&format!("{literal}\n\n[**ref**]"), &format!("{literal}\n[**ref**]"));
    }
}

#[test]
fn reference_titles_preserve_the_paragraph_limit() {
    let definitions = "[ref]:\n  /target\n  \"**caption**\"\n".repeat(3333);
    let source = format!("{definitions}ordinary");
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert_eq!(imported.story.text, source);
    assert_eq!(imported.story.paras.len(), BLOCK_LIMIT);
    assert!(imported.char_styles.is_empty());
    assert!(matches!(import(format!("{source}\n\noverflow").as_bytes()), Err(ImportError::Unsupported(_))));
}

#[test]
fn reference_titles_have_equal_literal_extent_in_discovery_and_replay() {
    for definition in ["[ref]: /target", "[ref]:\n  /target"] {
        for prefix in ["", "\u{0060}unclosed\n"] {
            let source = format!("{prefix}{definition}\n  \"**caption**\"");
            let (discovery, references) = parse(&source, std::collections::BTreeSet::new()).unwrap();
            assert!(references.contains("ref"));
            let (replay, _) = parse(&source, references).unwrap();
            for imported in [discovery, replay] {
                imported.story.check().unwrap();
                assert_eq!(imported.story.text, source);
                assert_eq!(imported.story.paras.len(), source.lines().count());
                assert!(imported.story.chars.iter().all(|run| run.format == CharFormat::default()));
                assert!(imported.char_styles.is_empty());
                assert_eq!(imported.warnings.len(), 1);
            }
        }
    }
}

#[test]
fn reference_title_backtick_does_not_hide_following_definition() {
    let source = "``unclosed\n[ref]: /target\n  \"**caption** `\"\n[**next**]: /path*with*stars\nordinary ` text\n\n[**next**]";
    check_literal(source, &source.replace("\n\n", "\n"));
}

#[test]
fn reference_title_backticks_keep_later_definitions_in_both_paths_and_orders() {
    for definition in ["[ref]: /target", "[ref]:\n  /target"] {
        for title in [r#"  "**é中** \"quoted\" `""#, "\t'**caption** `'", r"  (**caption** \) `)"] {
            for prefix in ["", "``unclosed\n"] {
                let literal = format!("{prefix}{definition}\n{title}\n[**next**]: /path*with*stars\nordinary ` text");
                for before in [false, true] {
                    let (source, expected) = if before {
                        (format!("[**next**]\n\n{literal}"), format!("[**next**]\n{literal}"))
                    } else {
                        (format!("{literal}\n\n[**next**]"), format!("{literal}\n[**next**]"))
                    };
                    check_literal(&source, &expected);
                    let (_, references) = parse(&source, std::collections::BTreeSet::new()).unwrap();
                    assert_eq!(
                        references,
                        ["ref".into(), "**next**".into()].into_iter().collect::<std::collections::BTreeSet<String>>(),
                        "{source:?}"
                    );
                }
            }
        }
    }
}

fn check_code_hides_next_reference(source: &str, expected_references: &[&str]) {
    let (_, references) = parse(source, std::collections::BTreeSet::new()).unwrap();
    assert_eq!(references, expected_references.iter().map(|label| (*label).to_owned()).collect::<std::collections::BTreeSet<_>>(), "{source:?}");
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert!(imported.story.text.ends_with("\n[next]"), "{source:?}: {:?}", imported.story.text);
    let definition = imported.story.text.find("[**next**]:").unwrap();
    assert_eq!(imported.story.format_after(definition).style, "Code", "{source:?}");
    let shortcut = imported.story.text.rfind("next").unwrap();
    assert_eq!(imported.story.format_after(shortcut).style, "Strong", "{source:?}");
    assert!(!imported.warnings.iter().any(|w| w.contains("links, images")), "{source:?}");
}

#[test]
fn reference_title_scanning_preserves_genuine_multiline_code() {
    for definition in ["[**next**]: /path*with*stars", "[**next**]:\n  /path*with*stars"] {
        // A real code opener before the definition may close inside a title-looking row.
        let before = format!("genuine `code\n{definition}\n  \"**caption** `\"\n\n[**next**]");
        check_code_hides_next_reference(&before, &[]);
        // A real opener after an accepted title must still hide a definition inside its span.
        let after = format!("``unclosed\n[ref]: /target\n  \"**caption** `\"\nordinary ` code\n{definition}\nstill ` text\n\n[**next**]");
        check_code_hides_next_reference(&after, &["ref"]);
    }
}

#[test]
fn rejected_reference_title_rows_retain_real_code_spans() {
    for definition in ["[ref]: /target", "[ref]:\n  /target"] {
        for title in ["  \"**caption** `\" trailing", "  \"**caption** `"] {
            let source = format!("``unclosed\n{definition}\n{title}\n[**next**]: /path*with*stars\nordinary ` text\n\n[**next**]");
            check_code_hides_next_reference(&source, &["ref"]);
        }
    }
}

#[test]
fn run31_multiline_reference_title_preserves_the_reported_source() {
    let source = "[ref]: /target\n  \"first\n  **second**\"\n\n[ref]";
    check_literal(source, "[ref]: /target\n  \"first\n  **second**\"\n[ref]");
    // The already-supported single-row title remains literal too.
    check_literal("[ref]: /target\n  \"first **second**\"\n\n[ref]", "[ref]: /target\n  \"first **second**\"\n[ref]");
}

#[test]
fn run31_multiline_titles_keep_definition_order_and_deferred_literal_extent() {
    for definition in [
        "[**ref**]: /target \"first\n  **second**\"",
        "[**ref**]:\n  /target\n  \"first\n  **second**\"",
        "[**ref**]: /target\n  '**first**\n  *second*'",
        "[**ref**]: /target\n  (**first**\n  *second*)",
    ] {
        for prefix in ["", "``unclosed\n"] {
            for source in [format!("[**ref**]\n\n{prefix}{definition}"), format!("{prefix}{definition}\n\n[**ref**]")] {
                check_literal(&source, &source.replace("\n\n", "\n"));
            }
        }
    }
}

#[test]
fn run31_multiline_title_lookahead_preserves_matched_code_and_following_prose() {
    let source = "`[ref]: /target\n  \"first\n  **second**\"`\n\n[ref]";
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert_eq!(imported.story.text, "[ref]: /target   \"first   **second**\"\n[ref]");
    assert_eq!(imported.story.format_after(imported.story.text.find("second").unwrap()).style, "Code");
    assert!(imported.warnings.is_empty(), "a matched code span cannot define a reference");

    let source = "[ref]: /target\n  \"first\n  **second**\"\nordinary *prose*\n\n[ref]";
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert_eq!(imported.story.text, "[ref]: /target\n  \"first\n  **second**\"\nordinary prose\n[ref]");
    assert_eq!(imported.story.format_after(imported.story.text.find("second").unwrap()), &CharFormat::default());
    assert_eq!(imported.story.format_after(imported.story.text.find("prose").unwrap()).style, "Emphasis");
}

#[test]
fn run31_unclosed_blank_separated_and_invalid_titles_remain_prose() {
    for (title, expected) in [
        ("  \"first\n  **second**", "\"first second"),
        ("  \"first\n  **second**\" trailing", "\"first second\" trailing"),
        ("  \"first\n\n  **second**\"", "\"first\nsecond\""),
    ] {
        let imported = import(format!("[ref]: /target\n{title}\n\n[ref]").as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text, format!("[ref]: /target\n{expected}\n[ref]"), "{title:?}");
        assert_eq!(imported.story.format_after(imported.story.text.find("second").unwrap()).style, "Strong");
    }
}

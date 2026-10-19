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
            for prefix in ["", "\u{0060}unclosed\n\n"] {
                let definition = format!("{prefix}{definition}\n{title}");
                for before in [false, true] {
                    let (source, expected) = if before {
                        (format!("[**ref**]\n\n{definition}"), format!("[**ref**]\n{definition}"))
                    } else {
                        (format!("{definition}\n\n[**ref**]"), format!("{definition}\n[**ref**]"))
                    };
                    check_literal(&source, &expected.replace("\n\n", "\n"));
                }
            }
        }
    }
}

#[test]
fn reference_titles_stop_before_following_prose() {
    for definition in ["[**ref**]: /target", "[**ref**]:\n  /target"] {
        for prefix in ["", "\u{0060}unclosed\n\n"] {
            let literal = format!("{prefix}{definition}\n  \"**caption**\"");
            let source = format!("{literal}\nordinary *prose* and **bold**\n\n[**ref**]");
            let literal = literal.replace("\n\n", "\n");
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
        for prefix in ["", "\u{0060}unclosed\n\n"] {
            for (row, expected_row, word, style) in [
                ("ordinary *prose*", "ordinary prose", "prose", "Emphasis"),
                ("\"**caption**\" trailing *prose*", "\"caption\" trailing prose", "caption", "Strong"),
                ("\"**caption**", "\"caption", "caption", "Strong"),
                ("'**caption**", "'caption", "caption", "Strong"),
                ("(**caption**", "(caption", "caption", "Strong"),
                (r#""**caption**\""#, "\"caption\"", "caption", "Strong"),
            ] {
                let source = format!("{prefix}{definition}\n{row}\n\n[**ref**]");
                let expected = format!("{prefix}{definition}\n{expected_row}\n[**ref**]").replace("\n\n", "\n");
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
        for prefix in ["", "\u{0060}unclosed\n\n"] {
            let source = format!("{prefix}{definition}\n\n\"**caption**\"\n\n[**ref**]");
            let expected = format!("{prefix}{definition}\n\"caption\"\n[**ref**]").replace("\n\n", "\n");
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
        for prefix in ["", "\u{0060}unclosed\n\n"] {
            let literal = format!("{prefix}{definition}");
            let source = format!("{literal}\n\"**ordinary**\"\n\n[**ref**]");
            let literal = literal.replace("\n\n", "\n");
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
    for prefix in ["", "\u{0060}unclosed\n\n"] {
        let literal = format!("{prefix}[**ref**]:\n  [nested]:/path*with*stars\n  \"[inner]: **caption**\"");
        let source = format!("{literal}\n\n[**ref**]");
        check_literal(&source, &source.replace("\n\n", "\n"));
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
fn reference_titles_preserve_literal_extent_through_public_import() {
    for definition in ["[ref]: /target", "[ref]:\n  /target"] {
        for prefix in ["", "\u{0060}unclosed\n\n"] {
            let source = format!("{prefix}{definition}\n  \"**caption**\"");
            let expected = source.replace("\n\n", "\n");
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, expected);
            assert_eq!(imported.story.paras.len(), expected.lines().count());
            assert!(imported.story.chars.iter().all(|run| run.format == CharFormat::default()));
            assert!(imported.char_styles.is_empty());
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }
}

#[test]
fn reference_title_backtick_does_not_hide_following_definition() {
    let source = "``unclosed\n\n[ref]: /target\n  \"**caption** `\"\n[**next**]: /path*with*stars\nordinary ` text\n\n[**next**]";
    check_literal(source, &source.replace("\n\n", "\n"));
}

#[test]
fn reference_title_backticks_keep_later_definitions_in_both_paths_and_orders() {
    for definition in ["[ref]: /target", "[ref]:\n  /target"] {
        for title in ["  \"**é中** \\\"quoted\\\" \u{0060}\"", "\t'**caption** \u{0060}'", "  (**caption** \\) \u{0060})"] {
            for prefix in ["", "\u{0060}\u{0060}unclosed\n\n"] {
                let literal = format!("{prefix}{definition}\n{title}\n[**next**]: /path*with*stars\nordinary \u{0060} text");
                for usage in ["*[ref]*", "[**next**]"] {
                    for before in [false, true] {
                        let source = if before { format!("{usage}\n\n{literal}") } else { format!("{literal}\n\n{usage}") };
                        check_literal(&source, &source.replace("\n\n", "\n"));
                    }
                }
            }
        }
    }
}

fn check_code_hides_next_reference(source: &str, has_reference_definition: bool) {
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    assert!(imported.story.text.ends_with("\n[next]"), "{source:?}: {:?}", imported.story.text);
    let definition = imported.story.text.find("[**next**]:").unwrap();
    assert_eq!(imported.story.format_after(definition).style, "Code");
    let shortcut = imported.story.text.rfind("next").unwrap();
    assert_eq!(imported.story.format_after(shortcut).style, "Strong");
    if has_reference_definition {
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
    } else {
        assert!(imported.warnings.is_empty());
    }
}

#[test]
fn reference_title_scanning_preserves_genuine_multiline_code() {
    for definition in ["[**next**]: /path*with*stars", "[**next**]:\n  /path*with*stars"] {
        // A real code opener before the definition may close inside a title-looking row.
        let before = format!("genuine `code\n{definition}\n  \"**caption** `\"\n\n[**next**]");
        check_code_hides_next_reference(&before, false);
        // A real opener after an accepted title must still hide a definition inside its span.
        let after = format!("``unclosed\n\n[ref]: /target\n  \"**caption** `\"\nordinary ` code\n{definition}\nstill ` text\n\n[**next**]");
        check_code_hides_next_reference(&after, true);
    }
}

#[test]
fn rejected_reference_title_rows_retain_real_code_spans() {
    for definition in ["[ref]: /target", "[ref]:\n  /target"] {
        for title in ["  \"**caption** `\" trailing", "  \"**caption** `"] {
            let source = format!("``unclosed\n\n{definition}\n{title}\n[**next**]: /path*with*stars\nordinary ` text\n\n[**next**]");
            check_code_hides_next_reference(&source, true);
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
        for prefix in ["", "``unclosed\n\n"] {
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

#[test]
fn reference_looking_rows_do_not_interrupt_unclosed_code_prose() {
    for (prose, rendered, strong_word) in [
        ("\u{0060}unclosed\n[**ref**]: /target\n  \"**caption**\"", "\u{0060}unclosed [ref]: /target \"caption\"", "caption"),
        (
            "\u{0060}\u{0060}unclosed\n[**ref**]: /target \"first\n  **second**\"",
            "\u{0060}\u{0060}unclosed [ref]: /target \"first second\"",
            "second",
        ),
    ] {
        for before in [false, true] {
            let (source, expected) = if before {
                (format!("[**ref**]\n\n{prose}"), format!("[ref]\n{rendered}"))
            } else {
                (format!("{prose}\n\n[**ref**]"), format!("{rendered}\n[ref]"))
            };
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, expected);
            assert_eq!(imported.story.paras.len(), 2);
            for (at, _) in expected.char_indices() {
                let strong = ["ref", strong_word]
                    .iter()
                    .any(|word| expected.match_indices(*word).any(|(start, matched)| (start..start + matched.len()).contains(&at)));
                if strong {
                    assert_eq!(imported.story.format_after(at).style, "Strong");
                } else {
                    assert_eq!(imported.story.format_after(at), &CharFormat::default());
                }
            }
            assert_eq!(imported.char_styles.len(), 1);
            assert_eq!(imported.char_styles[0].name, "Strong");
            assert!(imported.warnings.is_empty());
        }
    }
}

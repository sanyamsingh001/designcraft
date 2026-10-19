use super::*;

const WARNING: &str = "Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.";

fn read(source: &str) -> Imported {
    let imported = import(source.as_bytes()).unwrap();
    imported.story.check().unwrap();
    imported
}

fn literal(source: &str, expected: &str) {
    let imported = read(source);
    assert_eq!(imported.story.text, expected, "{source:?}");
    assert_eq!(imported.story.paras.len(), expected.lines().count());
    assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.para.is_empty() && p.chars.is_empty()));
    assert!(imported.story.chars.iter().all(|r| r.format == CharFormat::default()));
    assert!(imported.char_styles.is_empty());
    assert_eq!(imported.warnings, [WARNING]);
}

#[test]
fn r22_quote_lazy_paragraph_rows_preserve_markup_unicode_and_whitespace() {
    for source in [
        "> quoted\ncontinued **verbatim**",
        "> *quoted*\nlazy *em* and **strong**\nlast ~~strike~~ and \u{0060}code\u{0060}",
        "> quoted\n> marked **row**\nlazy **row**\nlast \u{0060}unmatched",
        "  > é中\n  lazy **é中**  \n\tmore \\*literal\\*\\",
        ">\tquoted\nlazy **raw**",
        "> \tquoted\nlazy **raw**",
        "> quoted\n    indented **lazy**",
        "> quoted\n[link](target) and **raw**\nmore **raw**",
        "> quoted\nspaced | pipe and **raw**\nmore **raw**",
        "> quoted\n> [**label**]: /target\nlazy **raw**",
        "> quoted\n<custom>\nmore **raw**",
        "> quoted\n#not-a-heading **raw**",
    ] {
        literal(source, source);
    }
    literal("> quoted\r\nlazy **raw**\r\nlast *raw*\r\n", "> quoted\nlazy **raw**\nlast *raw*");
}

#[test]
fn r22_quote_blank_boundaries_resume_ordinary_paragraphs() {
    for (source, expected) in [
        ("> quoted\nlazy **raw**\n\nordinary **parsed**", "> quoted\nlazy **raw**\nordinary parsed"),
        ("> quoted\n>\nordinary **parsed**", "> quoted\n>\nordinary parsed"),
        ("> quoted\n> \nordinary **parsed**", "> quoted\n> \nordinary parsed"),
        ("> quoted\nlazy **raw**\n \t\nordinary **parsed**", "> quoted\nlazy **raw**\nordinary parsed"),
    ] {
        let imported = read(source);
        assert_eq!(imported.story.text, expected);
        assert_eq!(imported.story.format_after(expected.find("parsed").unwrap()).style, "Strong");
        assert_eq!(imported.warnings, [WARNING]);
    }
    let imported = read("> quoted\n\n    **code**");
    assert_eq!(imported.story.text, "> quoted\n**code**");
    assert_eq!(imported.story.paras[1].style, "Code Block");
    assert!(imported.char_styles.is_empty());
}

#[test]
fn r22_quote_block_starts_keep_existing_heading_list_code_and_literal_paths() {
    let prefix = "> quoted\nlazy **raw**";
    for (suffix, expected, style) in [
        ("# **parsed**", "parsed", "Heading 1"),
        ("- **parsed**", "parsed", "Bullet List"),
        ("1. **parsed**", "parsed", "Numbered List"),
        ("7. **parsed**", "parsed", "Numbered List"),
        ("\u{0060}\u{0060}\u{0060}\n**raw**\n\u{0060}\u{0060}\u{0060}", "**raw**", "Code Block"),
        ("~~~\n**raw**\n~~~", "**raw**", "Code Block"),
        ("---\n**parsed**", "---\nparsed", "Body Text"),
        ("<div>\n**raw**\n</div>\n\n**parsed**", "<div>\n**raw**\n</div>\nparsed", "Body Text"),
        ("**A**|B\n---|---\n**C**|D\n\n**parsed**", "**A**|B\n---|---\n**C**|D\nparsed", "Body Text"),
    ] {
        let imported = read(&format!("{prefix}\n{suffix}"));
        let expected = format!("{prefix}\n{expected}");
        assert_eq!(imported.story.text, expected, "{suffix}");
        assert_eq!(imported.story.paras.last().unwrap().style, style, "{suffix}");
        for (at, _) in prefix.char_indices() {
            assert_eq!(imported.story.format_after(at), &CharFormat::default());
        }
        if let Some(at) = expected.find("parsed") {
            assert_eq!(imported.story.format_after(at).style, "Strong");
        } else {
            assert!(imported.char_styles.is_empty());
        }
        assert_eq!(imported.warnings, [WARNING]);
    }
}

#[test]
fn r22_quote_nonparagraph_rows_do_not_capture_following_ordinary_text() {
    for quoted in [
        "> # heading",
        "> ---",
        ">     **code**",
        "> quoted\n> ===",
        "> \u{0060}\u{0060}\u{0060}\n> **code**\n> \u{0060}\u{0060}\u{0060}",
        "> \u{0060}\u{0060}\u{0060}\n>\n> **unclosed code**",
        "> <!--\n> **HTML**\n> -->",
        "> <div>\n> **HTML**\n> </div>",
        "> **A**|B\n> ---|---\n> **C**|D",
        "> [label]:\n> /target",
    ] {
        let imported = read(&format!("{quoted}\nordinary **parsed**"));
        let expected = format!("{quoted}\nordinary parsed");
        assert_eq!(imported.story.text, expected, "{quoted}");
        assert_eq!(imported.story.format_after(expected.find("parsed").unwrap()).style, "Strong");
        for (at, _) in quoted.char_indices() {
            assert_eq!(imported.story.format_after(at), &CharFormat::default());
        }
        assert_eq!(imported.warnings, [WARNING]);
    }
}

#[test]
fn r22_quote_explicit_new_paragraph_after_a_quoted_block_can_continue_lazily() {
    for prefix in ["> # heading", "> \u{0060}\u{0060}\u{0060}\n> code\n> \u{0060}\u{0060}\u{0060}", "> <!-- closed -->", "> nested\n>", "> - list\n>"]
    {
        let source = format!("{prefix}\n> quoted\nlazy **raw**");
        literal(&source, &source);
    }
}

#[test]
fn r22_quote_reference_discovery_keeps_paragraph_content_local_and_blank_separated_definitions_global() {
    let source = "> quoted\n[**label**]: /target\n\n[**label**]";
    let imported = read(source);
    assert_eq!(imported.story.text, "> quoted\n[**label**]: /target\n[label]");
    assert_eq!(imported.story.format_after(imported.story.text.rfind("label").unwrap()).style, "Strong");
    assert_eq!(imported.warnings, [WARNING]);

    let source = "> quoted\nlazy [**label**]\n\n[**label**]: /target\n\n[**label**]";
    let imported = read(source);
    assert_eq!(imported.story.text, "> quoted\nlazy [**label**]\n[**label**]: /target\n[**label**]");
    assert!(imported.char_styles.is_empty());
    assert_eq!(imported.warnings.len(), 2, "outside shortcut retains the existing link warning");
}

#[test]
fn r22_quote_literal_rows_keep_existing_bounds_and_two_pass_work_budget() {
    let quote = format!("> quoted\n{}", "lazy **raw**\n".repeat(128));
    let source = format!("{quote}\n[ref]: /target\n\n[ref]");
    let imported = read(&source);
    assert_eq!(imported.story.text, format!("{}\n[ref]: /target\n[ref]", quote.trim_end_matches('\n')));
    assert!(imported.char_styles.is_empty());
    assert_eq!(imported.warnings.len(), 2);
    let oversized = format!("> quoted\n{}", "lazy **raw**\n".repeat(BLOCK_LIMIT));
    assert!(matches!(import(oversized.as_bytes()), Err(ImportError::Unsupported(_))));
}

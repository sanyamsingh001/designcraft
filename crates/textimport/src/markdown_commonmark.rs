//! Maintained inline grammar behind the importer's source-preservation policy.
use super::{Builder, DEPTH_LIMIT, Inline, SOURCE_LIMIT, TOKEN_LIMIT, limit};
use crate::ImportError;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

const PREFIX: &str = "x ";
pub(super) const LITERAL_WARNING: &str = "Markdown paragraphs containing links, images or HTML were kept as source text.";

fn boundary() -> ImportError {
    ImportError::Corrupt("Markdown inline parser boundary".into())
}

/// Probe code spans with one offset-preserving physical line, then apply the
/// application's hard/soft-break spacing to original source outside those spans.
fn prepare(source: &str, builder: &mut Builder) -> Result<String, ImportError> {
    if source.len() > SOURCE_LIMIT {
        return Err(limit("file size"));
    }
    let probe_len = source.len().checked_add(PREFIX.len()).ok_or_else(|| limit("file size"))?;
    let capacity = source.len().checked_mul(2).and_then(|n| n.checked_add(PREFIX.len())).ok_or_else(|| limit("file size"))?;
    let preparation_work = probe_len.checked_add(source.len()).ok_or_else(|| limit("delimiter processing"))?;
    super::reference::reserve_parser_work(builder, preparation_work)?;
    let spans = {
        let mut probe = String::with_capacity(probe_len);
        probe.push_str(PREFIX);
        probe.extend(source.chars().map(|c| if c == '\n' { ' ' } else { c }));
        let mut spans: Vec<Range<usize>> = Vec::new();
        for (event, range) in Parser::new_ext(&probe, Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
            builder.spend()?;
            if !matches!(event, Event::Code(_)) {
                continue;
            }
            let start = range.start.checked_sub(PREFIX.len()).ok_or_else(boundary)?;
            let end = range.end.checked_sub(PREFIX.len()).ok_or_else(boundary)?;
            source.get(start..end).ok_or_else(boundary)?;
            if start >= end || spans.last().is_some_and(|previous| previous.end > start) {
                return Err(boundary());
            }
            if spans.len() >= TOKEN_LIMIT {
                return Err(limit("inline code spans"));
            }
            spans.push(start..end);
        }
        spans
    };
    let mut spans = spans.into_iter().peekable();
    let mut input = String::with_capacity(capacity);
    input.push_str(PREFIX);
    let mut at = 0usize;
    while let Some(rest) = source.get(at..).filter(|s| !s.is_empty()) {
        if let Some(span) = spans.peek() {
            if span.start < at {
                return Err(boundary());
            }
            if span.start == at {
                let end = span.end;
                let code = source.get(span.clone()).ok_or_else(boundary)?;
                input.extend(code.chars().map(|c| if matches!(c, '\n' | '\u{2028}') { ' ' } else { c }));
                at = end;
                spans.next();
                continue;
            }
        }
        let Some(c) = rest.chars().next() else { break };
        if c == '\n' || rest.starts_with("\\\n") {
            let backslash = c == '\\';
            let previous = source.get(..at).ok_or_else(boundary)?;
            let spaces = previous.len().saturating_sub(previous.trim_end_matches(' ').len());
            let hard = backslash || spaces >= 2;
            if !backslash {
                let end = if hard { input.len().saturating_sub(spaces) } else { input.trim_end_matches([' ', '\t']).len() };
                input.truncate(end);
            }
            input.push(if hard { '\u{2028}' } else { ' ' });
            at = at.checked_add(if backslash { 2 } else { 1 }).ok_or_else(boundary)?;
            let tail = source.get(at..).ok_or_else(boundary)?;
            at = at.checked_add(tail.len() - tail.trim_start_matches([' ', '\t']).len()).ok_or_else(boundary)?;
            continue;
        }
        if c == '\\'
            && let Some(next) = rest.chars().nth(1).filter(char::is_ascii_punctuation)
        {
            let width = 1 + next.len_utf8();
            input.push_str(rest.get(..width).ok_or_else(boundary)?);
            at = at.checked_add(width).ok_or_else(boundary)?;
            continue;
        }
        if c == '~' {
            let width = rest.bytes().take_while(|&b| b == b'~').count();
            if width == 2 {
                input.push_str("~~");
            } else {
                for _ in 0..width {
                    input.push_str("\\~");
                }
            }
            at = at.checked_add(width).ok_or_else(boundary)?;
            continue;
        }
        input.push(c);
        at = at.checked_add(c.len_utf8()).ok_or_else(boundary)?;
    }
    if spans.next().is_some() {
        return Err(boundary());
    }
    Ok(input)
}

struct Run {
    text: String,
    inline: Inline,
}

fn format_key(inline: Inline) -> (bool, bool, bool, bool, bool) {
    (inline.bold > 0, inline.italic > 0, inline.strike > 0, inline.code, inline.base_bold)
}

fn push(runs: &mut Vec<Run>, text: &str, inline: Inline) -> Result<(), ImportError> {
    if text.is_empty() {
        return Ok(());
    }
    if let Some(last) = runs.last_mut().filter(|last| format_key(last.inline) == format_key(inline)) {
        last.text.push_str(text);
    } else {
        if runs.len() >= TOKEN_LIMIT {
            return Err(limit("character runs"));
        }
        runs.push(Run { text: text.into(), inline });
    }
    Ok(())
}

fn code_text(input: &str, range: Range<usize>) -> Result<&str, ImportError> {
    let raw = input.get(range).ok_or_else(boundary)?;
    let width = raw.bytes().take_while(|&b| b == b'\x60').count();
    if width == 0 {
        return Err(boundary());
    }
    let end = raw.len().checked_sub(width).ok_or_else(boundary)?;
    let content = raw.get(width..end).ok_or_else(boundary)?;
    if content.starts_with(' ') && content.ends_with(' ') && content.chars().any(|c| c != ' ') {
        content.get(1..content.len().saturating_sub(1)).ok_or_else(boundary)
    } else {
        Ok(content)
    }
}

/// Inspect root-filtered maintained events before literal pipe rows split a paragraph.
/// This pass admits parser work but creates no formatting or output runs.
pub(super) fn requires_literal(source: &str, allowed: Option<&Parser<'_>>, builder: &mut Builder) -> Result<bool, ImportError> {
    let input = prepare(source, builder)?;
    super::reference::reserve_parser_work(builder, input.len())?;
    let callback = |link| allowed.and_then(|parser| super::reference::resolve(parser, link));
    let parser = Parser::new_with_broken_link_callback(&input, Options::ENABLE_STRIKETHROUGH, Some(callback)).into_offset_iter();
    for (event, _) in parser {
        builder.spend()?;
        if matches!(event, Event::Start(Tag::Link { .. } | Tag::Image { .. }) | Event::Html(_) | Event::InlineHtml(_)) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn append(source: &str, base_bold: bool, allowed: Option<&Parser<'_>>, builder: &mut Builder) -> Result<(), ImportError> {
    let input = prepare(source, builder)?;
    super::reference::reserve_parser_work(builder, input.len())?;
    let callback = |link| allowed.and_then(|parser| super::reference::resolve(parser, link));
    let parser = Parser::new_with_broken_link_callback(&input, Options::ENABLE_STRIKETHROUGH, Some(callback)).into_offset_iter();
    let mut runs = Vec::new();
    let mut frames = Vec::new();
    let mut inline = Inline { base_bold, ..Default::default() };
    for (event, range) in parser {
        builder.spend()?;
        if matches!(event, Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph)) || range.end <= PREFIX.len() {
            continue;
        }
        match event {
            Event::Start(tag @ (Tag::Emphasis | Tag::Strong | Tag::Strikethrough)) => {
                if frames.len() >= DEPTH_LIMIT {
                    return Err(limit("inline nesting"));
                }
                frames.push(inline);
                inline.add(match tag {
                    Tag::Emphasis => Inline { italic: 1, ..Default::default() },
                    Tag::Strong => Inline { bold: 1, ..Default::default() },
                    Tag::Strikethrough => Inline { strike: 1, ..Default::default() },
                    _ => return Err(boundary()),
                });
            }
            Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => {
                inline = frames.pop().ok_or_else(boundary)?;
            }
            Event::Text(text) => {
                let raw = input.get(range.clone()).ok_or_else(boundary)?;
                // Preserve source spelling for synthesized entities/NUL; ordinary
                // payloads retain the library's escaping behavior.
                let text =
                    if (raw.starts_with('&') && raw.ends_with(';') && raw != text.as_ref()) || raw.contains('\0') { raw } else { text.as_ref() };
                let skip = PREFIX.len().saturating_sub(range.start);
                push(&mut runs, text.get(skip..).ok_or_else(boundary)?, inline)?;
            }
            Event::Code(_) => push(&mut runs, code_text(&input, range)?, Inline { code: true, ..inline })?,
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) | Event::Html(_) | Event::InlineHtml(_) => {
                builder.warn(LITERAL_WARNING);
                return builder.append(&source.replace('\n', "\u{2028}"), Inline::default());
            }
            _ => return Err(boundary()),
        }
    }
    if !frames.is_empty() {
        return Err(boundary());
    }
    for run in runs {
        builder.append(&run.text, run.inline)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import;

    #[test]
    fn maintained_inline_parser_retains_entity_spelling_and_nul() {
        let source = "\\*literal\\* &amp; &#42; &#x2A; \\&amp;\0é";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, "*literal* &amp; &#42; &#x2A; &amp;\0é");
        assert!(imported.char_styles.is_empty());
        assert!(imported.warnings.is_empty());
        imported.story.check().unwrap();
    }

    #[test]
    fn parser_prefix_does_not_reinterpret_list_content_as_blocks() {
        let imported = import("source.md", b"- [x]: **value**\n\n- text\n  ===").unwrap();
        assert_eq!(imported.story.text, "[x]: value\ntext ===");
        assert_eq!(imported.story.paras.len(), 2);
        assert!(imported.story.paras.iter().all(|p| p.style == "Bullet List"));
        assert_eq!(imported.story.format_after("[x]: ".len()).style, "Strong");
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn unsupported_tilde_widths_keep_supported_inner_emphasis() {
        let source = "~**bold**~ \\~~~strike~~ \u{60}~code~\u{60}";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, "~bold~ ~strike ~code~");
        assert_eq!(imported.story.format_after(1).style, "Strong");
        assert_eq!(imported.story.format_after("~bold~ ~".len()).style, "Strikethrough");
        assert_eq!(imported.story.format_after("~bold~ ~strike ".len()).style, "Code");
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn double_tilde_uses_maintained_punctuation_flanking() {
        for (source, expected, struck) in [("a~~!b~~c", "a!bc", "!b"), ("a~~b!~~c", "ab!c", "b!")] {
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert_eq!(imported.story.text, expected);
            assert_eq!(imported.story.format_after(1).style, "Strikethrough");
            assert!(imported.story.format_after(0).style.is_empty());
            assert!(imported.story.format_after(1 + struck.len()).style.is_empty());
            assert!(imported.warnings.is_empty());
            imported.story.check().unwrap();
        }
        for source in ["a~!b~c", "a~~~!b~~~c"] {
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert_eq!(imported.story.text, source);
            assert!(imported.char_styles.is_empty());
            assert!(imported.warnings.is_empty());
        }
    }

    #[test]
    fn unsupported_unicode_paragraph_still_keeps_the_whole_source() {
        let source = "é \\*literal\\* &amp; [x](u) **中** 👩‍👩‍👧‍👦";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, source);
        assert!(imported.char_styles.is_empty());
        assert_eq!(imported.warnings, ["Markdown paragraphs containing links, images or HTML were kept as source text."]);
        imported.story.check().unwrap();
    }

    #[test]
    fn unmatched_delimiters_remain_literal_without_spending_nesting() {
        for opener in ["*a ", "**a ", "_a ", "~~a ", "(*a ", "*! ", "…_a "] {
            let source = opener.repeat(DEPTH_LIMIT + 1);
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert_eq!(imported.story.text, source.trim_end());
            assert!(imported.char_styles.is_empty());
            assert!(imported.warnings.is_empty());
        }
        let source = format!("{}**b*", "*a ".repeat(DEPTH_LIMIT - 1));
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, format!("{}*b", "*a ".repeat(DEPTH_LIMIT - 1)));
        assert_eq!(imported.story.format_after(imported.story.text.len() - 1).style, "Emphasis");
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn benign_escaped_text_and_entities_do_not_consume_format_run_admission() {
        let source = "\\* &amp; ".repeat(TOKEN_LIMIT);
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, "* &amp; ".repeat(TOKEN_LIMIT).trim_end());
        assert_eq!(imported.story.chars.len(), 1);
        assert!(imported.char_styles.is_empty());
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn bare_carriage_returns_are_normalized_before_inline_parsing() {
        let imported = import("source.md", b"one\rtwo\r\r# *Heading*\r").unwrap();
        assert_eq!(imported.story.text, "one two\nHeading");
        assert_eq!(imported.story.paras.len(), 2);
        assert_eq!(imported.story.paras[1].style, "Heading 1");
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn inline_code_retains_nul_and_entity_spelling() {
        let source = "\u{60} a\0&amp; b \u{60}";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, "a\0&amp; b");
        assert_eq!(imported.story.format_after(0).style, "Code");
        assert!(imported.warnings.is_empty());
        imported.story.check().unwrap();
    }
    #[test]
    fn maintained_blocks_keep_escaped_heading_and_native_list_content() {
        let source = "# \\*literal\\* **bold**\n\n- \\*item\\* **strong**\n  - child";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, "*literal* bold\n*item* strong\nchild");
        assert_eq!(imported.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Heading 1", "Bullet List", "Bullet List"]);
        assert_eq!(imported.story.format_after("*literal* ".len()).style, "Strong");
        assert_eq!(imported.story.format_after("*literal* bold\n*item* ".len()).style, "Strong");
        assert_eq!(imported.story.paras[2].para.left_indent, Some(36.0));
        assert!(imported.warnings.is_empty());
        imported.story.check().unwrap();
    }

    #[test]
    fn hidden_definition_between_item_blocks_keeps_the_complete_item_source() {
        let source = "- first\n\n  [r]: /target\n\n  **last**";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, source);
        assert!(imported.story.paras.iter().all(|p| p.style == "Body Text"));
        assert!(imported.char_styles.is_empty());
        assert_eq!(
            imported.warnings,
            [
                "Multiple blocks inside Markdown list items were imported as separate paragraphs.",
                "Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.",
            ]
        );
        imported.story.check().unwrap();
    }

    #[test]
    fn a_hidden_item_definition_before_a_quote_cannot_disappear() {
        let source = "- [r]: /target\n  > **quoted**";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, source);
        assert!(imported.char_styles.is_empty());
        assert_eq!(imported.story.paras.len(), 2);
        assert_eq!(
            imported.warnings,
            [
                "Multiple blocks inside Markdown list items were imported as separate paragraphs.",
                "Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.",
            ]
        );
        imported.story.check().unwrap();
    }

    #[test]
    fn definitions_in_literal_containers_do_not_resolve_other_paragraphs() {
        for container in
            ["> [**r**]: /target", "\u{0060}\u{0060}\u{0060}\n[**r**]: /target\n\u{0060}\u{0060}\u{0060}", "<div>\n[**r**]: /target\n</div>"]
        {
            let source = format!("{container}\n\n[**r**]");
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert!(imported.story.text.ends_with("[r]"));
            assert_eq!(imported.story.format_after(imported.story.text.len() - 2).style, "Strong");
            assert!(!imported.warnings.iter().any(|warning| warning.contains("paragraphs containing links")));
            imported.story.check().unwrap();
        }
    }

    #[test]
    fn bare_pipe_rows_use_maintained_code_and_escape_boundaries() {
        for (source, expected, style, warning) in [
            ("| **raw** |", "| **raw** |", "", true),
            ("\\| **bold** |", "| bold |", "Strong", false),
            ("\u{0060}| **raw** |\u{0060}", "| **raw** |", "Code", false),
        ] {
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert_eq!(imported.story.text, expected);
            assert_eq!(imported.story.format_after(2).style, style);
            assert_eq!(!imported.warnings.is_empty(), warning);
            if warning {
                assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
            }
            imported.story.check().unwrap();
        }
    }

    #[test]
    fn maintained_code_blocks_keep_nul_and_entity_spelling() {
        let imported = import("source.md", b"\x60\x60\x60\na\0&amp;\n\x60\x60\x60").unwrap();
        assert_eq!(imported.story.text, "a\0&amp;");
        assert_eq!(imported.story.paras[0].style, "Code Block");
        assert!(imported.char_styles.is_empty());
        assert!(imported.warnings.is_empty());
        imported.story.check().unwrap();
    }

    #[test]
    fn unsupported_paragraph_fallback_precedes_literal_pipe_rows() {
        for ending in ["a [b | c](/url) d", "a ![b | c](/url) d", "a <span>b | c</span> d"] {
            let source = format!("before **bold**\n{ending}");
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert_eq!(imported.story.text, source.replace('\n', "\u{2028}"));
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert!(imported.char_styles.is_empty());
            assert!(imported.story.chars.iter().all(|run| run.format == designcraft_doc::CharFormat::default()));
            assert_eq!(imported.warnings, [LITERAL_WARNING]);
            imported.story.check().unwrap();
        }
    }

    #[test]
    fn root_reference_pipe_labels_keep_the_whole_paragraph_in_both_orders() {
        let paragraph = "before **bold**\na [**b | c**][r] d";
        let definition = "[r]: /url";
        let block_warning = "Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.";
        for before in [false, true] {
            let (source, expected, warnings) = if before {
                (
                    format!("{definition}\n\n{paragraph}"),
                    format!("{definition}\n{}", paragraph.replace('\n', "\u{2028}")),
                    [block_warning, LITERAL_WARNING],
                )
            } else {
                (
                    format!("{paragraph}\n\n{definition}"),
                    format!("{}\n{definition}", paragraph.replace('\n', "\u{2028}")),
                    [LITERAL_WARNING, block_warning],
                )
            };
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert_eq!(imported.story.text, expected);
            assert_eq!(imported.story.paras.len(), 2);
            assert!(imported.story.paras.iter().all(|para| para.style == "Body Text"));
            assert!(imported.char_styles.is_empty());
            assert!(imported.story.chars.iter().all(|run| run.format == designcraft_doc::CharFormat::default()));
            assert_eq!(imported.warnings, warnings);
            imported.story.check().unwrap();
        }
    }

    #[test]
    fn container_reference_pipe_labels_do_not_force_link_fallback() {
        let source = "> [r]: /url\n\nbefore **bold**\na [**b | c**][r] d";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, "> [r]: /url\nbefore bold\na [**b | c**][r] d");
        assert_eq!(imported.story.paras.len(), 3);
        assert!(imported.story.paras.iter().all(|para| para.style == "Body Text"));
        let bold = imported.story.text.find("bold").unwrap();
        let last_row = imported.story.text.rfind('\n').unwrap() + 1;
        for (at, _) in imported.story.text.char_indices() {
            if (bold..bold + 4).contains(&at) {
                assert_eq!(imported.story.format_after(at).style, "Strong");
            } else {
                assert_eq!(imported.story.format_after(at), &designcraft_doc::CharFormat::default());
            }
        }
        assert_eq!(imported.story.format_after(last_row), &designcraft_doc::CharFormat::default());
        assert_eq!(imported.char_styles.len(), 1);
        assert_eq!(imported.char_styles[0].name, "Strong");
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        imported.story.check().unwrap();
    }

    #[test]
    fn matching_root_reference_label_is_a_positive_resolution_control() {
        let source = "[**r**]: /target\n\n[**r**]";
        let imported = import("source.md", source.as_bytes()).unwrap();
        assert_eq!(imported.story.text, "[**r**]: /target\n[**r**]");
        assert_eq!(imported.story.paras.len(), 2);
        assert!(imported.char_styles.is_empty());
        assert!(imported.story.chars.iter().all(|run| run.format == designcraft_doc::CharFormat::default()));
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.", LITERAL_WARNING,]);
        imported.story.check().unwrap();
    }

    #[test]
    fn literal_pipe_rows_do_not_spend_rendered_formatting_depth() {
        for depth in [DEPTH_LIMIT, DEPTH_LIMIT + 1] {
            let source = format!("| {}leaf{} |", "**a ".repeat(depth), " z**".repeat(depth));
            let imported = import("source.md", source.as_bytes()).unwrap();
            assert_eq!(imported.story.text, source);
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert!(imported.char_styles.is_empty());
            assert!(imported.story.chars.iter().all(|run| run.format == designcraft_doc::CharFormat::default()));
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
            imported.story.check().unwrap();
        }
    }
}

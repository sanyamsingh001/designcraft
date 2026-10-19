//! Bounded Markdown import through the same named-style mapping as Word.
//! Supports ATX/setext headings, paragraphs, nested lists, emphasis/strong, strikethrough and
//! code. Paragraphs containing links/images/HTML and quote/table blocks stay literal, with
//! warnings. Complex list containers are simplified to separate paragraphs.
//! This is a one-time import; it does not create a live-sync link.

use crate::{ImportError, Imported, ImportedStyle};
use designcraft_doc::{CharAttrs, CharFormat, CharRun, ListType, ParaAttrs, ParaFormat, Story, StoryId};

const SOURCE_LIMIT: usize = 16 * 1024 * 1024;
const BLOCK_LIMIT: usize = 10_000;
const TOKEN_LIMIT: usize = 100_000;
const DEPTH_LIMIT: usize = 32;
const MONO: &str = "JetBrains Mono";

#[path = "markdown_blocks.rs"]
mod blocks;
#[path = "markdown_commonmark.rs"]
mod commonmark;
#[path = "markdown_document.rs"]
mod document;
#[path = "markdown_pipes.rs"]
mod pipes;
#[path = "markdown_tables.rs"]
mod tables;

#[path = "markdown_reference.rs"]
mod reference;

fn limit(what: &str) -> ImportError {
    ImportError::Unsupported(format!("Markdown {what} exceeds the import limit"))
}

#[derive(Clone, Copy, Default)]
struct Inline {
    bold: usize,
    italic: usize,
    strike: usize,
    code: bool,
    base_bold: bool,
}

impl Inline {
    fn style(self) -> Option<ImportedStyle> {
        let mut parts = Vec::new();
        // Only explicit strong markup belongs to the character style. Inherited heading
        // weight is provisional until placement selects the document's paragraph style.
        let bold = self.bold > 0;
        if bold {
            parts.push("Strong");
        }
        if self.italic > 0 {
            parts.push("Emphasis");
        }
        if self.strike > 0 {
            parts.push("Strikethrough");
        }
        if self.code {
            parts.push("Code");
        }
        if parts.is_empty() {
            return None;
        }
        let font_style = match (bold, self.italic > 0) {
            (true, true) => Some("Bold Italic".into()),
            (true, false) => Some("Bold".into()),
            (false, true) => Some("Italic".into()),
            _ => None,
        };
        Some(ImportedStyle {
            name: parts.join(" "),
            chars: CharAttrs {
                font_style,
                strikethrough: (self.strike > 0).then_some(true),
                font_family: self.code.then(|| MONO.into()),
                ..Default::default()
            },
            ..Default::default()
        })
    }
    fn add(&mut self, other: Self) {
        self.bold += other.bold;
        self.italic += other.italic;
        self.strike += other.strike;
    }
}

struct Builder {
    text: String,
    runs: Vec<CharRun>,
    paras: Vec<ParaFormat>,
    para_styles: Vec<ImportedStyle>,
    char_styles: Vec<ImportedStyle>,
    warnings: Vec<String>,
    work: usize,
}
impl Builder {
    fn new(source_len: usize) -> Self {
        Self {
            text: String::new(),
            runs: Vec::new(),
            paras: Vec::new(),
            para_styles: Vec::new(),
            char_styles: Vec::new(),
            warnings: Vec::new(),
            work: source_len.saturating_mul(16).saturating_add(1024),
        }
    }
    fn spend(&mut self) -> Result<(), ImportError> {
        self.work = self.work.checked_sub(1).ok_or_else(|| limit("delimiter processing"))?;
        Ok(())
    }
    fn warn(&mut self, message: &str) {
        if !self.warnings.iter().any(|w| w == message) {
            self.warnings.push(message.into());
        }
    }
    fn append(&mut self, text: &str, inline: Inline) -> Result<(), ImportError> {
        if text.is_empty() {
            return Ok(());
        }
        let mut format = CharFormat::default();
        if let Some(style) = inline.style() {
            format.style = style.name.clone();
            if !self.char_styles.iter().any(|s| s.name == style.name) {
                self.char_styles.push(style);
            }
        }
        if inline.base_bold && inline.italic > 0 && inline.bold == 0 {
            format.over.font_style = Some("Bold Italic".into());
        }
        if let Some(last) = self.runs.last_mut().filter(|r| r.format == format) {
            last.len += text.len();
        } else {
            if self.runs.len() >= TOKEN_LIMIT {
                return Err(limit("character runs"));
            }
            self.runs.push(CharRun { len: text.len(), format });
        }
        self.text.push_str(text);
        Ok(())
    }
    fn block_with_references(
        &mut self,
        source: &str,
        style: ImportedStyle,
        attrs: ParaAttrs,
        literal: bool,
        allowed: Option<&pulldown_cmark::Parser<'_>>,
    ) -> Result<(), ImportError> {
        if self.paras.len() >= BLOCK_LIMIT {
            return Err(limit("paragraph count"));
        }
        if !self.paras.is_empty() {
            self.append("\n", Inline::default())?;
        }
        let name = style.name.clone();
        let base_bold = matches!(style.chars.font_style.as_deref(), Some("Bold" | "Bold Italic"));
        if !self.para_styles.iter().any(|s| s.name == name) {
            self.para_styles.push(style);
        }
        self.paras.push(ParaFormat { style: name, para: attrs, ..Default::default() });
        if literal {
            self.append(source, Inline::default())?;
        } else {
            commonmark::append(source.trim_end_matches([' ', '\t']), base_bold, allowed, self)?;
        }
        Ok(())
    }
    fn finish(self) -> Imported {
        // Insert once to initialize document-special characters. Repeated insert_with would
        // rescan the growing story; the accumulated runs and paragraphs already cover it.
        let mut story = Story::with_text(StoryId(0), &self.text, ParaFormat::default());
        if !self.paras.is_empty() {
            story.paras = self.paras;
        }
        if !self.runs.is_empty() {
            story.chars = self.runs;
        }
        Imported { story, para_styles: self.para_styles, char_styles: self.char_styles, warnings: self.warnings }
    }
}

fn body_style(name: &str) -> ImportedStyle {
    ImportedStyle {
        name: name.into(),
        based_on: Some(designcraft_doc::BASIC_PARAGRAPH.into()),
        para: ParaAttrs { space_after: Some(6.0), ..Default::default() },
        ..Default::default()
    }
}
fn heading_style(level: usize) -> ImportedStyle {
    let size = match level {
        1 => 24.0,
        2 => 20.0,
        3 => 18.0,
        4 => 16.0,
        5 => 14.0,
        _ => 12.0,
    };
    ImportedStyle {
        name: format!("Heading {level}"),
        para: ParaAttrs { space_before: Some(12.0), space_after: Some(6.0), keep_with_next: Some(1), ..Default::default() },
        chars: CharAttrs { size: Some(size), font_style: Some("Bold".into()), ..Default::default() },
        ..body_style("")
    }
}
fn list_style(kind: ListType) -> ImportedStyle {
    ImportedStyle {
        para: ParaAttrs {
            list_type: Some(kind),
            left_indent: Some(18.0),
            first_line_indent: Some(-12.0),
            space_after: Some(3.0),
            ..Default::default()
        },
        ..body_style(if kind == ListType::Numbers { "Numbered List" } else { "Bullet List" })
    }
}
fn code_style() -> ImportedStyle {
    ImportedStyle {
        chars: CharAttrs { font_family: Some(MONO.into()), ..Default::default() },
        para: ParaAttrs { hyphenate: Some(false), ..Default::default() },
        ..body_style("Code Block")
    }
}

pub fn import(bytes: &[u8]) -> Result<Imported, ImportError> {
    import_capped(bytes, SOURCE_LIMIT)
}
fn import_capped(bytes: &[u8], cap: usize) -> Result<Imported, ImportError> {
    if bytes.len() > cap {
        return Err(limit("file size"));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| ImportError::Corrupt(format!("Markdown is not UTF-8: {e}")))?;
    let text = text.trim_start_matches('\u{FEFF}').replace("\r\n", "\n").replace('\r', "\n");
    document::parse(&text)
}

impl Imported {
    /// Remove provisional inherited heading weight before placement maps styles. Each flag
    /// identifies an italic run without explicit strong markup; renaming must retain run order.
    /// Sniffing matches `import`, so RTF/Word/tagged data named `.md` is left untouched.
    pub fn prepare_markdown_emphasis(&mut self, name: &str, bytes: &[u8]) -> Vec<bool> {
        if !crate::is_markdown(&name.to_ascii_lowercase())
            || bytes.starts_with(b"PK\x03\x04")
            || bytes.starts_with(b"{\\rtf")
            || crate::tagged::is_tagged(bytes)
        {
            return Vec::new();
        }
        self.story
            .chars
            .iter_mut()
            .map(|run| {
                let emphasis =
                    matches!(run.format.style.as_str(), "Emphasis" | "Emphasis Strikethrough" | "Emphasis Code" | "Emphasis Strikethrough Code");
                if emphasis {
                    run.format.over.font_style = None;
                }
                emphasis
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use designcraft_doc::{CharacterStyle, ParagraphStyle, Styles};

    fn read(source: &str) -> Imported {
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        imported
    }

    fn assert_character_regions(imported: &Imported, styled: &[(&str, &str)]) {
        let regions: Vec<_> = styled
            .iter()
            .map(|(text, style)| {
                let start = imported.story.text.find(*text).unwrap();
                (start..start + text.len(), *style)
            })
            .collect();
        for (at, _) in imported.story.text.char_indices() {
            let mut expected = CharFormat::default();
            if let Some((_, style)) = regions.iter().find(|(range, _)| range.contains(&at)) {
                expected.style = (*style).into();
            }
            assert_eq!(imported.story.format_after(at), &expected, "{:?} at {at}", imported.story.text);
        }
    }

    fn styles(imported: &Imported) -> Styles {
        let mut styles = Styles::default();
        for s in &imported.para_styles {
            styles.paragraph.push(ParagraphStyle {
                name: s.name.clone(),
                based_on: s.based_on.clone(),
                next_style: None,
                para: s.para.clone(),
                chars: s.chars.clone(),
                shortcut: String::new(),
            });
        }
        for s in &imported.char_styles {
            styles.character.push(CharacterStyle {
                name: s.name.clone(),
                based_on: s.based_on.clone(),
                chars: s.chars.clone(),
                shortcut: String::new(),
            });
        }
        styles
    }

    #[test]
    fn headings_and_setext_have_mappable_style_definitions() {
        let imported = read(
            "\u{FEFF}# One ###\r\n## Two\r### Three\n#### Four\n##### Five\n###### Six\n\nSetext one\n===\n\nSetext two\n---\n\n#not a heading\n####### neither",
        );
        assert_eq!(imported.story.text, "One\nTwo\nThree\nFour\nFive\nSix\nSetext one\nSetext two\n#not a heading ####### neither");
        assert_eq!(
            imported.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(),
            ["Heading 1", "Heading 2", "Heading 3", "Heading 4", "Heading 5", "Heading 6", "Heading 1", "Heading 2", "Body Text"]
        );
        let styles = styles(&imported);
        for (para, size) in imported.story.paras.iter().take(6).zip([24.0, 20.0, 18.0, 16.0, 14.0, 12.0]) {
            let (p, c) = styles.resolve_para(para);
            assert_eq!(c.size, size);
            assert_eq!(c.font_style, "Bold");
            assert_eq!(p.keep_with_next, 1);
            assert!(para.chars.is_empty() && para.para.is_empty(), "style mapping must control heading defaults");
        }
        assert_eq!(imported.para_styles.len(), 7, "only used styles, without duplicates");
    }

    #[test]
    fn soft_lines_hard_breaks_and_blank_lines() {
        let imported = read("First line\r\ncontinues  \rnext\\\nlast\n\nSecond\n\n\nThird\n");
        assert_eq!(imported.story.text, "First line continues\u{2028}next\u{2028}last\nSecond\nThird");
        assert_eq!(imported.story.paras.len(), 3);
        assert!(imported.warnings.is_empty());
        let escaped = read("Two\\\\\nlines");
        assert_eq!(escaped.story.text, "Two\\ lines", "an escaped backslash is not a hard break");
    }

    #[test]
    fn run34_unicode_spacing_is_paragraph_content() {
        for (source, expected) in [
            ("\u{2003}Indented", "\u{2003}Indented"),
            ("Indented\u{a0}", "Indented\u{a0}"),
            ("  \u{a0}Indented\u{2003} \t", "\u{a0}Indented\u{2003}"),
            ("\u{2003}\u{a0}", "\u{2003}\u{a0}"),
            ("  \u{2003}\u{a0} \t", "\u{2003}\u{a0}"),
            ("before\n\u{2003}\u{a0}\nafter", "before \u{2003}\u{a0} after"),
            ("before\n\n\u{2003}\u{a0}\n\nafter", "before\n\u{2003}\u{a0}\nafter"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text"));
            assert!(imported.warnings.is_empty());
        }
    }

    #[test]
    fn run34_unicode_spacing_survives_continuations_and_ascii_breaks() {
        for (source, expected) in [
            ("  first\u{a0}\n  \u{2003}second\u{a0} \t", "first\u{a0} \u{2003}second\u{a0}"),
            ("first\u{a0}  \n \t\u{2003}second\u{a0}", "first\u{a0}\u{2028}\u{2003}second\u{a0}"),
            ("first\u{a0}\\\n  \u{2003}second\u{a0}", "first\u{a0}\u{2028}\u{2003}second\u{a0}"),
            ("  first \t\n  second  \n \tthird\\\n  fourth \t", "first second\u{2028}third\u{2028}fourth"),
            ("  First\n \t\n  Second  ", "First\nSecond"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert!(imported.warnings.is_empty());
        }
        // Unicode whitespace still prevents a delimiter from closing emphasis.
        for source in ["*word\u{a0}*", "_word\u{2003}_"] {
            let imported = read(source);
            assert_eq!(imported.story.text, source);
            assert!(imported.char_styles.is_empty());
        }
        let imported = read("\u{2003}**Bold**\u{a0}");
        assert_eq!(imported.story.text, "\u{2003}Bold\u{a0}");
        assert_eq!(imported.story.format_after("\u{2003}".len()).style, "Strong");
    }

    #[test]
    fn run34_headings_preserve_unicode_spacing_and_ascii_markers() {
        for (source, expected, style) in [
            ("# \u{2003}Heading\u{a0} ### \t", "\u{2003}Heading\u{a0}", "Heading 1"),
            ("  ## \u{a0}Heading\u{2003} \t", "\u{a0}Heading\u{2003}", "Heading 2"),
            ("\u{2003}Heading\u{a0}\n=== \t", "\u{2003}Heading\u{a0}", "Heading 1"),
            ("\u{2003}Heading\u{a0}\n  --- \t", "\u{2003}Heading\u{a0}", "Heading 2"),
            ("# \u{2003}\u{a0}", "\u{2003}\u{a0}", "Heading 1"),
            ("# Title ###\u{a0}", "Title ###\u{a0}", "Heading 1"),
            ("Title\n===\u{a0}", "Title ===\u{a0}", "Body Text"),
            ("Title\n---\u{2003}", "Title ---\u{2003}", "Body Text"),
            ("\u{2003}# Title", "\u{2003}# Title", "Body Text"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, style, "{source:?}");
            assert!(imported.warnings.is_empty());
        }
    }

    #[test]
    fn run34_unicode_spacing_does_not_end_code_or_literal_blocks() {
        for (source, expected) in [
            ("    code\n    \u{2003}\u{a0}\n\nnext", "code\u{2028}\u{2003}\u{a0}\nnext"),
            ("~~~\ncode\n~~~\u{a0}\nstill code\n~~~ \t", "code\u{2028}~~~\u{a0}\u{2028}still code"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras[0].style, "Code Block");
            assert!(imported.warnings.is_empty());
        }
        for (source, expected) in [
            ("<div>\n\u{2003}\u{a0}\n**raw**\n\n*after*", "<div>\n\u{2003}\u{a0}\n**raw**\nafter"),
            ("a|b\n-|-\n\u{2003}\u{a0}\n**raw**\n\n*after*", "a|b\n-|-\n\u{2003}\u{a0}\n**raw**\nafter"),
            ("> first\n\u{2003}\u{a0}\n**raw**\n\n*after*", "> first\n\u{2003}\u{a0}\n**raw**\nafter"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert!(!imported.char_styles.iter().any(|s| s.name == "Strong"));
            assert_eq!(imported.story.format_after(expected.find("after").unwrap()).style, "Emphasis");
            assert!(!imported.warnings.is_empty());
        }
    }

    #[test]
    fn multiline_code_preserves_raw_backslashes_and_whitespace() {
        for (source, expected) in [
            ("`a\\\nb`", "a\\ b"),
            ("`a  \nb`", "a   b"),
            ("`a\n  b`", "a   b"),
            ("``a\\\n \tb``", "a\\  \tb"),
            ("`é\\\n  中`", "é\\   中"),
            ("`a\r\n b`", "a  b"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert!(imported.story.chars.iter().all(|run| run.format.style == "Code"));
            assert!(imported.warnings.is_empty());
        }
        let mixed = read("before  \n`a\\\n b`\\\nafter");
        assert_eq!(mixed.story.text, "before\u{2028}a\\  b\u{2028}after");
        assert_eq!(mixed.story.format_after(mixed.story.text.find("a\\").unwrap()).style, "Code");
        assert_eq!(mixed.story.format_after(mixed.story.text.find("after").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
        assert!(mixed.warnings.is_empty());
    }

    #[test]
    fn multiline_code_pipe_on_closing_line_is_not_a_table() {
        for (source, expected) in [("`a\nb | c`", "a b | c"), ("``a\n| b``", "a | b"), ("`é\n中 | c`", "é 中 | c")] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert!(imported.story.chars.iter().all(|run| run.format.style == "Code"));
            assert!(imported.warnings.is_empty());
            imported.story.check().unwrap();
        }
    }

    #[test]
    fn multiline_code_pipe_on_opening_line_is_not_a_table() {
        for (source, expected) in [("`a | b\nc`", "a | b c"), ("``a | b\nc``", "a | b c"), ("`é | 中\nc`", "é | 中 c")] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert!(imported.story.chars.iter().all(|run| run.format.style == "Code"));
            assert!(imported.warnings.is_empty());
            imported.story.check().unwrap();
        }
    }

    #[test]
    fn multiline_code_with_many_pipe_rows_keeps_context_within_work_limit() {
        let rows = "other ``x`` | y\n".repeat(128);
        let imported = read(&format!("`first\n{rows}last`"));
        assert_eq!(imported.story.text, format!("first {}last", rows.replace('\n', " ")));
        assert_eq!(imported.story.paras.len(), 1);
        assert!(imported.story.chars.iter().all(|run| run.format.style == "Code"));
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn multiline_code_reference_continuations_keep_code_context() {
        for (source, expected) in [
            ("`a\n[x]: y`", "a [x]: y"),
            ("`a\n[x]: y\nz`", "a [x]: y z"),
            ("``é\n  [中]: `y`\nfin``", "é   [中]: `y` fin"),
            ("`a\\\n[x]: y`", "a\\ [x]: y"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert!(imported.story.chars.iter().all(|run| run.format.style == "Code"));
            assert!(imported.warnings.is_empty());
        }
        let rows = "[x]: ``y`` | z\n".repeat(128);
        let imported = read(&format!("`first\n{rows}last`"));
        assert_eq!(imported.story.text, format!("first {}last", rows.replace('\n', " ")));
        assert_eq!(imported.story.paras.len(), 1);
        assert!(imported.story.chars.iter().all(|run| run.format.style == "Code"));
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn actual_reference_rows_remain_literal_after_paragraph_code_scan() {
        for source in ["[x]: **y**", "  [x]: `y`", "[x]: `a\n| **b** | c`"] {
            let imported = read(source);
            assert_eq!(imported.story.text, source, "{source:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count());
            assert!(imported.story.paras.iter().all(|para| para.style == "Body Text"));
            assert!(imported.char_styles.is_empty());
            assert_eq!(imported.warnings.len(), 1);
        }
        // Removing a list marker must not turn its content into a reference row.
        let list = read("- [x]: **value**");
        assert_eq!(list.story.text, "[x]: value");
        assert_eq!(styles(&list).resolve_para(&list.story.paras[0]).0.list_type, ListType::Bullets);
        assert_eq!(list.story.format_after(list.story.text.find("value").unwrap()).style, "Strong");
        assert!(list.warnings.is_empty());
        let list_code = read("- [x]: `a\nb`");
        assert_eq!(list_code.story.text, "[x]: a b");
        assert_eq!(styles(&list_code).resolve_para(&list_code.story.paras[0]).0.list_type, ListType::Bullets);
        assert_eq!(list_code.story.format_after(list_code.story.text.find("a b").unwrap()).style, "Code");
        assert!(list_code.warnings.is_empty());
        // The blank row makes the following definition a separate block.
        let mixed = read("`a\n[x]: y`\n\n[real]: **destination**\n# Next");
        assert_eq!(mixed.story.text, "a [x]: y\n[real]: **destination**\nNext");
        assert_eq!(mixed.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Body Text", "Body Text", "Heading 1"]);
        assert_eq!(mixed.story.format_after(0).style, "Code");
        assert_eq!(mixed.story.format_after(mixed.story.text.find("[real]").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
        assert_eq!(mixed.warnings.len(), 1);
        assert_character_regions(&mixed, &[("a [x]: y", "Code")]);
    }

    #[test]
    fn malformed_and_paragraph_internal_reference_rows_keep_inline_formatting() {
        for (source, expected, paragraph, code, strong) in [
            ("[x]: `a | b`", "[x]: a | b", "Body Text", Some("a | b"), None),
            ("``unclosed\n[x]: `a\n| **b** | c`", "``unclosed [x]: a | **b** | c", "Body Text", Some("a | **b** | c"), None),
            ("`unclosed\n[x]: **y**", "`unclosed [x]: y", "Body Text", None, Some("y")),
            ("``unclosed\n[x]: y`", "``unclosed [x]: y`", "Body Text", None, None),
            ("`unclosed\n  [x]: **y**", "`unclosed [x]: y", "Body Text", None, Some("y")),
            ("`a\n[x]: **y**\n---", "`a [x]: y", "Heading 2", None, Some("y")),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1, "{source:?}");
            assert_eq!(imported.story.paras[0].style, paragraph, "{source:?}");
            let mut styled = Vec::new();
            if let Some(code) = code {
                styled.push((code, "Code"));
            }
            if let Some(strong) = strong {
                styled.push((strong, "Strong"));
            }
            assert_character_regions(&imported, &styled);
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn reference_definition_does_not_interrupt_open_code_paragraph() {
        let imported = read("`a\n[x]: y`\n[real]: **destination**\n# Next");
        assert_eq!(imported.story.text, "a [x]: y [real]: destination\nNext");
        assert_eq!(imported.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Body Text", "Heading 1"]);
        assert_character_regions(&imported, &[("a [x]: y", "Code"), ("destination", "Strong")]);
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn reference_like_code_with_bracketed_prefix_keeps_emphasis() {
        for (source, expected, code, word, style) in [
            ("[x] **bold** and `a]: b`", "[x] bold and a]: b", "a]: b", "bold", "Strong"),
            ("[x] **bold** and `]:`", "[x] bold and ]:", "]:", "bold", "Strong"),
            ("[x] `]:` and *em*", "[x] ]: and em", "]:", "em", "Emphasis"),
            ("[prefix `]:` suffix] **bold**", "[prefix ]: suffix] bold", "]:", "bold", "Strong"),
            ("[x] **é** and ``a `]:` 中``", "[x] é and a `]:` 中", "a `]:` 中", "é", "Strong"),
            ("[x] **bold** and `a: b`", "[x] bold and a: b", "a: b", "bold", "Strong"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert_eq!(imported.story.format_after(imported.story.text.find(code).unwrap()).style, "Code");
            assert_eq!(imported.story.format_after(imported.story.text.find(word).unwrap()).style, style);
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn reference_detection_keeps_escaped_and_unmatched_backtick_tokens() {
        for (source, expected) in [
            (r"[x] \` **bold** `a]: b`", "[x] ` bold a]: b"),
            ("[x] `` **bold** `a]: b`", "[x] `` bold a]: b"),
            ("[x] ` **bold** ``a]: b``", "[x] ` bold a]: b"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.format_after(imported.story.text.find("bold").unwrap()).style, "Strong");
            assert_eq!(imported.story.format_after(imported.story.text.find("a]: b").unwrap()).style, "Code");
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
        let escaped = read(r"[x] \]: **bold**");
        assert_eq!(escaped.story.text, "[x] ]: bold");
        assert_eq!(escaped.story.format_after(escaped.story.text.find("bold").unwrap()).style, "Strong");
        assert!(escaped.warnings.is_empty());
    }

    #[test]
    fn reference_like_multiline_code_with_bracketed_prefix_keeps_context() {
        for (source, expected, code) in [
            ("[x] **bold** and `a]:\nb`", "[x] bold and a]: b", "a]: b"),
            ("[x] **bold** and `a\n]: b`", "[x] bold and a ]: b", "a ]: b"),
            ("[x] **bold** and `a\n[x]: b`", "[x] bold and a [x]: b", "a [x]: b"),
            ("[x] **bold** and `a]:\n[x]: b\nc`", "[x] bold and a]: [x]: b c", "a]: [x]: b c"),
            ("[prefix `a\n]: b` suffix] **bold**", "[prefix a ]: b suffix] bold", "a ]: b"),
            ("[x] **bold** and ``a]:\n[x]: `b`\nc``", "[x] bold and a]: [x]: `b` c", "a]: [x]: `b` c"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.format_after(imported.story.text.find(code).unwrap()).style, "Code");
            assert_eq!(imported.story.format_after(imported.story.text.find("bold").unwrap()).style, "Strong");
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
        let rows = "[x]: ``y`` | z\n".repeat(128);
        let imported = read(&format!("[x] **bold** `]:\n{rows}end`"));
        assert_eq!(imported.story.text, format!("[x] bold ]: {}end", rows.replace('\n', " ")));
        assert_eq!(imported.story.paras.len(), 1);
        assert_eq!(imported.story.format_after(imported.story.text.find("bold").unwrap()).style, "Strong");
        assert_eq!(imported.story.format_after(imported.story.text.find("]:").unwrap()).style, "Code");
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn initial_label_delimiter_controls_reference_fallback() {
        for (source, expected, word, code) in [
            ("[x] text ]: **destination**", "[x] text ]: destination", "destination", None),
            ("[x] `a]: b` and [y]: **destination**", "[x] a]: b and [y]: destination", "destination", Some("a]: b")),
            ("[x] `code` ]: **destination**", "[x] code ]: destination", "destination", Some("code")),
            (r"[x] \`a]: b\` **bold**", "[x] `a]: b` bold", "bold", None),
            ("[x] ``a]: b` **bold**", "[x] ``a]: b` bold", "bold", None),
            ("[x] `a]: b`` **bold**", "[x] `a]: b`` bold", "bold", None),
            ("[x] `` **bold** `a]: b` ]: **destination**", "[x] `` bold a]: b ]: destination", "destination", Some("a]: b")),
            ("[x [y]: label] **destination**", "[x [y]: label] destination", "destination", None),
            (r"[x\] label] text ]: **destination**", "[x] label] text ]: destination", "destination", None),
            ("[x `]:` label] text ]: **destination**", "[x ]: label] text ]: destination", "destination", Some("]:")),
            ("[x] text\ny]: **destination**", "[x] text y]: destination", "destination", None),
            ("[x] `a\n]: b` and [y]: **destination**", "[x] a ]: b and [y]: destination", "destination", Some("a ]: b")),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert_eq!(imported.story.format_after(imported.story.text.find(word).unwrap()).style, "Strong");
            if let Some(code) = code {
                assert_eq!(imported.story.format_after(imported.story.text.find(code).unwrap()).style, "Code");
            } else {
                assert!(imported.char_styles.iter().all(|s| s.name != "Code"), "unmatched or escaped backticks must remain literal: {source:?}");
            }
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
        for source in ["[x]: **destination**", r"[x\] label]: **destination**", "[x]: `a]:\n| **table** | b`"] {
            let imported = read(source);
            assert_eq!(imported.story.text, source, "{source:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count());
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text"));
            assert!(imported.char_styles.is_empty());
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn invalid_reference_labels_and_destinations_keep_inline_formatting() {
        for (source, expected, code, strong) in [
            ("  [x]: `a]: b`", "[x]: a]: b", Some("a]: b"), None),
            ("[x `]:` label]: **destination**", "[x ]: label]: destination", Some("]:"), Some("destination")),
            ("[x [y]: label]: **destination**", "[x [y]: label]: destination", None, Some("destination")),
            (r"[x \`a]: b\` **bold**", "[x `a]: b` bold", None, Some("bold")),
            ("[x ``a]: b` **bold**", "[x ``a]: b` bold", None, Some("bold")),
            ("[x `a]: b`` **bold**", "[x `a]: b`` bold", None, Some("bold")),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1, "{source:?}");
            assert_eq!(imported.story.paras[0].style, "Body Text", "{source:?}");
            let mut styled = Vec::new();
            if let Some(code) = code {
                styled.push((code, "Code"));
            }
            if let Some(strong) = strong {
                styled.push((strong, "Strong"));
            }
            assert_character_regions(&imported, &styled);
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn actual_table_rows_remain_literal_after_paragraph_code_scan() {
        for source in ["| **a** | `b` |\n| --- | --- |\n| *c* | d |", "**a** | `b`\n---", "  | **a** | b |", "- **a** | b", " `a | b"] {
            let imported = read(source);
            assert_eq!(imported.story.text, source, "{source:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count());
            assert!(imported.story.paras.iter().all(|para| para.style == "Body Text"));
            assert!(imported.char_styles.is_empty());
            assert_eq!(imported.warnings.len(), 1);
        }
        let mixed = read("**before**\n| **a** | b |\n| --- | --- |\n\nafter *em*");
        assert_eq!(mixed.story.text, "before\n| **a** | b |\n| --- | --- |\nafter em");
        assert_eq!(mixed.story.paras.len(), 4);
        assert_eq!(mixed.story.format_after(0).style, "Strong");
        assert_eq!(mixed.story.format_after(mixed.story.text.find("**a**").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
        assert_eq!(mixed.story.format_after(mixed.story.text.rfind("em").unwrap()).style, "Emphasis");
        assert_eq!(mixed.warnings.len(), 1);
        let resumed = read("`a\nb`\n| **c** | d |\nheading\n---\n\n    *code*");
        assert_eq!(resumed.story.text, "a b\n| **c** | d |\nheading\n*code*");
        assert_eq!(resumed.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Body Text", "Body Text", "Heading 2", "Code Block"]);
        assert_eq!(resumed.story.format_after(0).style, "Code");
        assert_eq!(resumed.story.format_after(resumed.story.text.find("**c**").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
        assert_eq!(resumed.warnings.len(), 1);
    }

    #[test]
    fn r7a_compact_table_rows_remain_literal_with_optional_outer_pipes() {
        for source in [
            "**A**|**B**\n---|---\n*C*|*D*",
            "| **A** | **B** |\n| --- | --- |\n| *C* | *D* |",
            "|**A**|**B**|\n---|---\n|*C*|*D*|",
            "**A**|**B**\n|---|---|\n*C*|*D*",
            "|**A**|**B**\n---|---|\n|*C*|*D*",
            "**A**|**B**|\n|---|---\n*C*|*D*|",
            "  **A**\t|\t**B** \t\n  :---:\t|\t---: \t\n  *C*\t|\t*D* \t",
            "**A**|**B**\n:---:|---:\n*C*|*D*",
            " **é**|**中**\r\n :---:|---:\r\n *c*|*d*",
        ] {
            let expected = source.replace("\r\n", "\n");
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), expected.lines().count(), "{source:?}");
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert_eq!(imported.para_styles.len(), 1);
            assert_eq!(imported.para_styles[0].name, "Body Text");
            assert!(imported.char_styles.is_empty(), "literal table cells must not create emphasis/code styles: {source:?}");
            for (at, _) in imported.story.text.char_indices() {
                let format = imported.story.format_after(at);
                assert_eq!(format.style, designcraft_doc::NO_CHAR_STYLE, "{source:?} at {at}");
                assert!(format.over.is_empty(), "{source:?} at {at}");
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r7a_compact_table_literal_body_keeps_escaped_and_code_cell_source() {
        for source in [
            "**A\\|x**|**B**\n---|---\n*C\\|y*|`d|e`",
            "**A**\\\\|**B**\n---|---\n*C*|*D*",
            "**A**|**B**\n---|---\n*C*|*D*|extra\n*E*|",
            "|**A**|**B**|\n|---|---|\n*E*|`unclosed\n*F*|*G*",
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, source, "{source:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count(), "{source:?}");
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert!(imported.char_styles.is_empty(), "{source:?}");
            for (at, _) in imported.story.text.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{source:?} at {at}");
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r7a_compact_table_candidates_require_matching_delimiter_cells() {
        for (source, expected) in [
            ("**A**|*B*\n---|---|---\n*C*|D", "A|B ---|---|--- C|D"),
            ("**A**|*B*|C\n---|---", "A|B|C ---|---"),
            ("**A**|*B*\n---|x--\n*C*|D", "A|B ---|x-- C|D"),
            ("**A**|*B*\n---||---\n*C*|D", "A|B ---||--- C|D"),
            ("**A**|*B*\n---\\|---\n*C*|D", "A|B ---|--- C|D"),
            ("**A**|*B*\n`---|---`\n*C*|D", "A|B ---|--- C|D"),
            ("**A**|*B*", "A|B"),
            ("**A**|*B*\nplain", "A|B plain"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1, "{source:?}");
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert_eq!(imported.story.format_after(0).style, "Strong", "{source:?}");
            assert_eq!(imported.story.format_after(expected.find('B').unwrap()).style, "Emphasis", "{source:?}");
            if source.contains("`---|---`") {
                assert_eq!(imported.story.format_after(expected.find("---|---").unwrap()).style, "Code");
            }
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn r7a_compact_table_candidates_ignore_escaped_and_inline_code_pipes() {
        for (source, expected, strong, emphasis, code) in [
            ("a|b", "a|b", None, None, None),
            ("a\\|b\n---|---", "a|b ---|---", None, None, None),
            (r"**A**\|*B*", "A|B", Some("A"), Some("B"), None),
            ("**A**\\|*B*\n---|---", "A|B ---|---", Some("A"), Some("B"), None),
            ("**A**\\|*B*|C\n---|---|---", "A|B|C ---|---|---", Some("A"), Some("B"), None),
            ("Use ``x | `y`` and **bold**.", "Use x | `y and bold.", Some("bold"), None, Some("x | `y")),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1, "{source:?}");
            assert_eq!(imported.story.paras[0].style, "Body Text");
            for (word, style) in [(strong, "Strong"), (emphasis, "Emphasis"), (code, "Code")] {
                if let Some(word) = word {
                    assert_eq!(imported.story.format_after(expected.find(word).unwrap()).style, style, "{source:?}: {word}");
                } else {
                    assert!(imported.char_styles.iter().all(|s| s.name != style), "{source:?}: unexpected {style}");
                }
            }
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn r7a_compact_table_blocks_resume_supported_markup() {
        let rows = "**A**|**B**\n---|---\n*C*|*D*";
        let joined = "before A|B ---|--- C|D";
        for (suffix, expected_suffix, same_paragraph, para_style, word, char_style) in [
            ("\nafter *em*", "after em", false, "Body Text", "em", "Emphasis"),
            ("# After", "After", false, "Heading 1", "After", designcraft_doc::NO_CHAR_STYLE),
            ("- *after*", "after", false, "Bullet List", "after", "Emphasis"),
            ("\n**after**|*em*", "after|em", false, "Body Text", "after", "Strong"),
            ("\u{0060}\u{0060}\u{0060}\n*x*|y\n\u{0060}\u{0060}\u{0060}", "*x*|y", false, "Code Block", "*x*", designcraft_doc::NO_CHAR_STYLE),
            ("    *x*|y", "x|y", true, "Body Text", "x", "Emphasis"),
            ("\nafter \u{0060}x|y\u{0060}", "after x|y", false, "Body Text", "x|y", "Code"),
        ] {
            let source = format!("**before**\n{rows}\n{suffix}");
            let separator = if same_paragraph { " " } else { "\n" };
            let expected = format!("{joined}{separator}{expected_suffix}");
            let imported = read(&source);
            assert_eq!(imported.story.text, expected, "{suffix:?}");
            assert_eq!(imported.story.paras.len(), if same_paragraph { 1 } else { 2 }, "{suffix:?}");
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert_eq!(imported.story.paras.last().unwrap().style, para_style, "{suffix:?}");
            for word in ["before", "A", "B"] {
                assert_eq!(imported.story.format_after(expected.find(word).unwrap()).style, "Strong", "{suffix:?}: {word}");
            }
            for word in ["C", "D"] {
                assert_eq!(imported.story.format_after(expected.find(word).unwrap()).style, "Emphasis", "{suffix:?}: {word}");
            }
            assert_eq!(imported.story.format_after(expected.find("---|---").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
            assert_eq!(imported.story.format_after(expected.rfind(word).unwrap()).style, char_style, "{suffix:?}");
            if para_style == "Code Block" {
                assert_eq!(styles(&imported).resolve_para(imported.story.paras.last().unwrap()).1.font_family, MONO);
            }
            assert!(imported.warnings.is_empty(), "{suffix:?}");
        }
        let source = format!("**before**\n\n{rows}\n\n*after*");
        let expected = format!("before\n{rows}\nafter");
        let imported = read(&source);
        assert_eq!(imported.story.text, expected);
        assert_eq!(imported.story.paras.len(), 5);
        assert!(imported.story.paras.iter().all(|p| p.style == "Body Text"));
        assert_eq!(imported.story.format_after(0).style, "Strong");
        for (at, _) in rows.char_indices() {
            assert_eq!(imported.story.format_after("before\n".len() + at), &CharFormat::default());
        }
        assert_eq!(imported.story.format_after(expected.rfind("after").unwrap()).style, "Emphasis");
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        let source = format!("{rows}\n\n**E**|**F**\n---|---\n*G*|*H*");
        let imported = read(&source);
        assert_eq!(imported.story.text, source.replace("\n\n", "\n"));
        assert_eq!(imported.story.paras.len(), 6);
        assert!(imported.char_styles.is_empty());
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
    }

    #[test]
    fn r7a_single_line_code_pipes_follow_maintained_table_grammar() {
        for source in ["\u{0060}**A**|*B*\u{0060}\n---|---", "**A**|\u{0060}B|C\u{0060}\n---|---|---"] {
            let imported = read(source);
            assert_eq!(imported.story.text, source);
            assert_eq!(imported.story.paras.len(), 2);
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert!(imported.char_styles.is_empty());
            assert_eq!(imported.para_styles.len(), 1);
            for (at, _) in source.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default());
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r7a_code_pipe_header_widths_use_maintained_table_grammar() {
        for (source, expected, code) in [
            ("\u{0060}a|b\u{0060}|**B**\n---|---\n*C*|*D*", "a|b|B ---|--- C|D", "a|b"),
            ("\u{0060}\u{0060}a|\u{0060}b\u{0060}\u{0060}|**B**\n---|---\n*C*|*D*", "a|\u{0060}b|B ---|--- C|D", "a|\u{0060}b"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected);
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Body Text");
            for (at, _) in code.char_indices() {
                assert_eq!(imported.story.format_after(at).style, "Code");
            }
            assert_eq!(imported.story.format_after(expected.find('B').unwrap()).style, "Strong");
            for word in ['C', 'D'] {
                assert_eq!(imported.story.format_after(expected.find(word).unwrap()).style, "Emphasis");
            }
            assert_eq!(imported.story.format_after(expected.find("---|---").unwrap()), &CharFormat::default());
            assert!(imported.warnings.is_empty());
        }
    }

    #[test]
    fn r7a_compact_table_candidates_preserve_block_and_multiline_code_context() {
        for source in ["```\n**A**|**B**\n---|---\n*C*|*D*\n```", "    **A**|**B**\n    ---|---\n    *C*|*D*"] {
            let imported = read(source);
            assert_eq!(imported.story.text, "**A**|**B**\u{2028}---|---\u{2028}*C*|*D*");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Code Block");
            assert!(imported.char_styles.is_empty() && imported.warnings.is_empty());
        }
        for (source, expected, code) in [
            ("before `**A**|**B**\n---|---\n*C*|*D*` **after**", "before **A**|**B** ---|--- *C*|*D* after", "**A**|**B**"),
            ("`open\n**A**|**B**\n---|---\n*C*|*D* close` **after**", "open **A**|**B** ---|--- *C*|*D* close after", "open"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Body Text");
            assert_eq!(imported.story.format_after(expected.find(code).unwrap()).style, "Code");
            assert_eq!(imported.story.format_after(expected.find("after").unwrap()).style, "Strong");
            assert!(imported.warnings.is_empty());
        }
        let list = read("- **A**|**B**\n---|---\n*C*|*D*");
        assert_eq!(list.story.text, "A|B ---|--- C|D");
        assert_eq!(list.story.paras.len(), 1);
        assert_eq!(list.story.paras[0].style, "Bullet List");
        assert_eq!(styles(&list).resolve_para(&list.story.paras[0]).0.list_type, ListType::Bullets);
        assert_eq!(list.story.format_after(0).style, "Strong");
        assert_eq!(list.story.format_after(list.story.text.find('C').unwrap()).style, "Emphasis");
        assert!(list.warnings.is_empty());
        let heading = read("# **A**|**B**\n---|---\n*C*|*D*");
        assert_eq!(heading.story.text, "A|B\n---|--- C|D");
        assert_eq!(heading.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Heading 1", "Body Text"]);
        assert_eq!(heading.story.format_after(0).style, "Strong");
        assert_eq!(heading.story.format_after(heading.story.text.find('C').unwrap()).style, "Emphasis");
        assert!(heading.warnings.is_empty());
    }

    #[test]
    fn r7a_compact_one_cell_bold_body_remains_literal() {
        for body in ["**C**", "*C*", "~~C~~", "**C**\nplain", "C\n*D*|**E**\n**F**", "  **é**  "] {
            let source = format!("**A**|**B**\n---|---\n{body}");
            let imported = read(&source);
            assert_eq!(imported.story.text, source, "{body:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count(), "{body:?}");
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert_eq!(imported.para_styles.len(), 1);
            assert_eq!(imported.para_styles[0].name, "Body Text");
            assert!(imported.char_styles.is_empty(), "{body:?}");
            for (at, _) in imported.story.text.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{body:?} at {at}");
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r7a_compact_one_cell_code_body_remains_literal() {
        for body in ["`C`", "``C ` D``", "`C|D`", "`C\nD`", "`unclosed", "code **C** and `é`"] {
            let source = format!("**A**|**B**\n---|---\n{body}");
            let imported = read(&source);
            assert_eq!(imported.story.text, source, "{body:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count(), "{body:?}");
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert_eq!(imported.para_styles.len(), 1);
            assert_eq!(imported.para_styles[0].name, "Body Text");
            assert!(imported.char_styles.is_empty(), "{body:?}");
            for (at, _) in imported.story.text.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{body:?} at {at}");
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r7a_compact_one_cell_escaped_body_remains_literal() {
        for body in [r"\*C\*", r"\[C\]\(D\)", r"**C**\|D", r"\`C\`", r"C\\D", r"\<em\>C\</em\>"] {
            let source = format!("**A**|**B**\n---|---\n{body}");
            let imported = read(&source);
            assert_eq!(imported.story.text, source, "{body:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count(), "{body:?}");
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert_eq!(imported.para_styles.len(), 1);
            assert_eq!(imported.para_styles[0].name, "Body Text");
            assert!(imported.char_styles.is_empty(), "{body:?}");
            for (at, _) in imported.story.text.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{body:?} at {at}");
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r7a_compact_one_cell_extent_stops_at_blank_or_block() {
        let literal = "**A**|**B**\n---|---\n**C**";
        for (suffix, expected_suffix, para_style, word, char_style) in [
            ("\n**outside**", "outside", "Body Text", "outside", "Strong"),
            ("# **outside**", "outside", "Heading 1", "outside", "Strong"),
            ("- *outside*", "outside", "Bullet List", "outside", "Emphasis"),
            ("7. **outside**", "outside", "Numbered List", "outside", "Strong"),
            ("```\n*outside*|x\n```", "*outside*|x", "Code Block", "*outside*", designcraft_doc::NO_CHAR_STYLE),
            ("    *outside*", "    *outside*", "Body Text", "*outside*", designcraft_doc::NO_CHAR_STYLE),
            ("> **outside**", "> **outside**", "Body Text", "**outside**", designcraft_doc::NO_CHAR_STYLE),
            ("<div>**outside**</div>", "<div>**outside**</div>", "Body Text", "**outside**", designcraft_doc::NO_CHAR_STYLE),
            ("---", "---", "Body Text", "---", designcraft_doc::NO_CHAR_STYLE),
        ] {
            let source = format!("{literal}\n{suffix}");
            let expected = format!("{literal}\n{expected_suffix}");
            let imported = read(&source);
            assert_eq!(imported.story.text, expected, "{suffix:?}");
            assert_eq!(imported.story.paras.len(), 4, "{suffix:?}");
            assert!(imported.story.paras[..3].iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert_eq!(imported.story.paras[3].style, para_style, "{suffix:?}");
            for (at, _) in literal.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{suffix:?} at {at}");
            }
            assert_eq!(imported.story.format_after(expected.rfind(word).unwrap()).style, char_style, "{suffix:?}");
            if para_style == "Code Block" {
                assert_eq!(styles(&imported).resolve_para(&imported.story.paras[3]).1.font_family, MONO);
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r7a_compact_one_cell_outside_block_controls_preserve_formatting() {
        let literal = "**A**|**B**\n---|---";
        for (suffix, expected_suffix, para_style, word, char_style) in [
            ("\n**outside**", "outside", "Body Text", "outside", "Strong"),
            ("\n`x|y`", "x|y", "Body Text", "x|y", "Code"),
            ("\n\\*outside\\*", "*outside*", "Body Text", "*outside*", designcraft_doc::NO_CHAR_STYLE),
            ("# **outside**", "outside", "Heading 1", "outside", "Strong"),
            ("- *outside*", "outside", "Bullet List", "outside", "Emphasis"),
            ("7. **outside**", "outside", "Numbered List", "outside", "Strong"),
            ("```\n*outside*|x\n```", "*outside*|x", "Code Block", "*outside*", designcraft_doc::NO_CHAR_STYLE),
            ("    *outside*", "    *outside*", "Body Text", "*outside*", designcraft_doc::NO_CHAR_STYLE),
            ("> **outside**", "> **outside**", "Body Text", "**outside**", designcraft_doc::NO_CHAR_STYLE),
            ("<div>**outside**</div>", "<div>**outside**</div>", "Body Text", "**outside**", designcraft_doc::NO_CHAR_STYLE),
            ("---", "---", "Body Text", "---", designcraft_doc::NO_CHAR_STYLE),
            ("\na|b", "a|b", "Body Text", "a|b", designcraft_doc::NO_CHAR_STYLE),
        ] {
            let source = format!("{literal}\n{suffix}");
            let expected = format!("{literal}\n{expected_suffix}");
            let imported = read(&source);
            assert_eq!(imported.story.text, expected, "{suffix:?}");
            assert_eq!(imported.story.paras.len(), 3, "{suffix:?}");
            assert!(imported.story.paras[..2].iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
            assert_eq!(imported.story.paras[2].style, para_style, "{suffix:?}");
            for (at, _) in literal.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{suffix:?} at {at}");
            }
            assert_eq!(imported.story.format_after(expected.rfind(word).unwrap()).style, char_style, "{suffix:?}");
            if para_style == "Code Block" {
                assert_eq!(styles(&imported).resolve_para(&imported.story.paras[2]).1.font_family, MONO);
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn multiline_unsupported_markup_keeps_raw_code_and_line_boundaries() {
        let source = "Text `a\\\n b` and [link](target)";
        let imported = read(source);
        assert_eq!(imported.story.text, "Text `a\\\u{2028} b` and [link](target)");
        assert_eq!(imported.story.paras.len(), 1);
        assert!(imported.char_styles.is_empty());
        assert!(!imported.warnings.is_empty());
    }

    #[test]
    fn nested_lists_keep_native_labels_and_parent_numbering() {
        let imported =
            read("- First\n  continues\n  - Child\n- Second\n\n7. Seven\n9. Eight\n\n   2) Sub\n   2) Next\n9. Nine\n\nPlain\n\n3. Restart");
        assert_eq!(imported.story.text, "First continues\nChild\nSecond\nSeven\nEight\nSub\nNext\nNine\nPlain\nRestart");
        let styles = styles(&imported);
        let resolved: Vec<_> = imported.story.paras.iter().map(|p| styles.resolve_para(p).0).collect();
        for p in &resolved[..3] {
            assert_eq!(p.list_type, ListType::Bullets);
            assert_eq!(p.first_line_indent, -12.0);
        }
        assert_eq!(resolved[0].left_indent, 18.0);
        assert_eq!(resolved[1].left_indent, 36.0);
        assert_eq!(resolved[2].left_indent, 18.0);
        for (i, start) in [(3, 7), (4, 8), (5, 2), (6, 3), (7, 9), (9, 3)] {
            assert_eq!(resolved[i].list_type, ListType::Numbers);
            assert_eq!(resolved[i].start_at, Some(start));
        }
        assert_eq!(resolved[5].left_indent, 36.0);
        assert_eq!(resolved[7].left_indent, 18.0);
        assert_eq!(resolved[8].list_type, ListType::None);
    }

    #[test]
    fn ordered_sublist_above_one_does_not_interrupt_a_paragraph() {
        let imported = read("- First\n  continues\n  - Child\n- Second\n\n7. Seven\n9. Eight\n   2) Sub\n   2) Next\n9. Nine\n\nPlain\n\n3. Restart");
        assert_eq!(imported.story.text, "First continues\nChild\nSecond\nSeven\nEight 2) Sub 2) Next\nNine\nPlain\nRestart");
        assert_eq!(imported.story.paras.len(), 8);
        let styles = styles(&imported);
        let resolved: Vec<_> = imported.story.paras.iter().map(|p| styles.resolve_para(p).0).collect();
        for (index, indent) in [(0, 18.0), (1, 36.0), (2, 18.0)] {
            assert_eq!(resolved[index].list_type, ListType::Bullets);
            assert_eq!(resolved[index].left_indent, indent);
            assert_eq!(resolved[index].first_line_indent, -12.0);
        }
        for (index, start) in [(3, 7), (4, 8), (5, 9), (7, 3)] {
            assert_eq!(resolved[index].list_type, ListType::Numbers);
            assert_eq!(resolved[index].start_at, Some(start));
            assert_eq!(resolved[index].left_indent, 18.0);
            assert_eq!(resolved[index].first_line_indent, -12.0);
        }
        assert_eq!(resolved[6].list_type, ListType::None);
        assert_character_regions(&imported, &[]);
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn emphasis_combines_and_resets_across_utf8_runs() {
        let imported = read("*em* **strong** ***both*** **outer *nested*** ~~gone~~ ~~**bold gone**~~ _café_ **日本語** tail");
        assert_eq!(imported.story.text, "em strong both outer nested gone bold gone café 日本語 tail");
        let styles = styles(&imported);
        for (word, expected) in [
            ("em", "Italic"),
            ("strong", "Bold"),
            ("both", "Bold Italic"),
            ("outer", "Bold"),
            ("nested", "Bold Italic"),
            ("café", "Italic"),
            ("日本語", "Bold"),
        ] {
            let at = imported.story.text.find(word).unwrap();
            let (_, base) = styles.resolve_para(&imported.story.paras[0]);
            let format = imported.story.format_after(at);
            assert_eq!(styles.resolve_char(&base, format).font_style, expected, "{word}");
            assert!(format.over.is_empty(), "document styles can replace imported formatting");
        }
        let at = imported.story.text.find("bold gone").unwrap();
        let (_, base) = styles.resolve_para(&imported.story.paras[0]);
        let c = styles.resolve_char(&base, imported.story.format_after(at));
        assert_eq!(c.font_style, "Bold");
        assert!(c.strikethrough);
        assert_eq!(imported.story.format_after(imported.story.text.find("tail").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
    }

    #[test]
    fn escapes_unmatched_markers_and_intraword_underscores_survive() {
        let imported = read(r"my_variable_name \*literal\* lone* **unclosed and a_b_c ~single~");
        assert_eq!(imported.story.text, "my_variable_name *literal* lone* **unclosed and a_b_c ~single~");
        assert!(imported.char_styles.is_empty());
        let crossing = read("*outer _unfinished* tail_");
        assert_eq!(crossing.story.text, "outer _unfinished tail_");
        assert_eq!(crossing.story.format_after(crossing.story.text.find("tail").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
    }

    #[test]
    fn run35_composed_and_decomposed_accents_keep_intraword_underscores() {
        for source in ["caf\u{e9}_menu_", "cafe\u{301}_menu_", "caf\u{e9}__menu__", "cafe\u{301}__menu__"] {
            let imported = read(source);
            assert_eq!(imported.story.text.as_bytes(), source.as_bytes(), "{source:?}");
            assert!(imported.char_styles.is_empty(), "{source:?}");
            assert!(imported.story.chars.iter().all(|run| run.format == CharFormat::default()), "{source:?}");
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn run35_unicode_flanks_keep_supported_emphasis_and_source_bytes() {
        for (source, expected, word, style, weight) in [
            ("*caf\u{e9}*menu", "caf\u{e9}menu", "caf\u{e9}", "Emphasis", "Italic"),
            ("*cafe\u{301}*menu", "cafe\u{301}menu", "cafe\u{301}", "Emphasis", "Italic"),
            ("(*word*)", "(word)", "word", "Emphasis", "Italic"),
            ("\u{2014}_word_!", "\u{2014}word!", "word", "Emphasis", "Italic"),
            ("\u{ab}__word__\u{bb}", "\u{ab}word\u{bb}", "word", "Strong", "Bold"),
            ("\u{20ac}_word_\u{d7}", "\u{20ac}word\u{d7}", "word", "Emphasis", "Italic"),
            ("^_word_^", "^word^", "word", "Emphasis", "Italic"),
            ("\u{1f642}_word_\u{1f642}", "\u{1f642}word\u{1f642}", "word", "Emphasis", "Italic"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text.as_bytes(), expected.as_bytes(), "{source:?}");
            assert!(imported.warnings.is_empty(), "{source:?}");
            let styles = styles(&imported);
            let (_, base) = styles.resolve_para(&imported.story.paras[0]);
            let start = expected.find(word).unwrap();
            let span = start..start + word.len();
            for (at, _) in expected.char_indices() {
                let format = imported.story.format_after(at);
                let (name, font_style) =
                    if span.contains(&at) { (style, weight) } else { (designcraft_doc::NO_CHAR_STYLE, base.font_style.as_str()) };
                assert_eq!(format.style, name, "{source:?} at {at}");
                assert_eq!(styles.resolve_char(&base, format).font_style, font_style, "{source:?} at {at}");
                assert!(format.over.is_empty(), "{source:?} at {at}");
            }
        }
    }

    #[test]
    fn code_spans_and_blocks_keep_literal_contents() {
        let imported = read("A `` **literal** ` tick `` and *`code`*.\n\n```rust\n# literal\n**not bold**\n\n```\n\n    indented *code*\n    second");
        assert_eq!(imported.story.text, "A **literal** ` tick and code.\n# literal\u{2028}**not bold**\u{2028}\nindented *code*\u{2028}second");
        let at = imported.story.text.find("**literal**").unwrap();
        assert_eq!(imported.story.format_after(at).style, "Code");
        assert_eq!(imported.story.format_after(imported.story.text.find("and code").unwrap() + 4).style, "Emphasis Code");
        assert_eq!(imported.story.paras[1].style, "Code Block");
        assert_eq!(imported.story.paras[2].style, "Code Block");
        let (_, c) = styles(&imported).resolve_para(&imported.story.paras[1]);
        assert_eq!(c.font_family, MONO);
        assert!(read("````\nshort\n```\nstill code").story.text.contains("```"));
        assert_eq!(read("A `unclosed").story.text, "A `unclosed");
    }

    #[test]
    fn fenced_code_removes_the_opening_fence_indentation() {
        for indent in 1..=3 {
            for fence in ["```", "~~~"] {
                let prefix = " ".repeat(indent);
                let source = format!("{prefix}{fence}rust\n{prefix}print(1)\n{fence}");
                let imported = read(&source);
                assert_eq!(imported.story.text, "print(1)", "{source:?}");
                assert_eq!(imported.story.paras.len(), 1);
                assert_eq!(imported.story.paras[0].style, "Code Block");
                assert!(imported.char_styles.is_empty() && imported.warnings.is_empty());
            }
        }
    }

    #[test]
    fn fenced_code_keeps_extra_indentation_short_lines_blanks_and_tabs() {
        for (source, expected) in [
            ("```\n\t  literal\n    deeper\n```", "\t  literal\u{2028}    deeper"),
            (
                "  ```\n  alpha\n    deeper\n x\nplain\n\n \n  \n   \n  omega\n  ```",
                "alpha\u{2028}  deeper\u{2028}x\u{2028}plain\u{2028}\u{2028}\u{2028}\u{2028} \u{2028}omega",
            ),
            (" ```\n\talpha\n ```", "   alpha"),
            ("  ```\n\talpha\n \té中\n  \tdeeper\n\t\tlast\n  ```", "  alpha\u{2028}  é中\u{2028}\tdeeper\u{2028}  \tlast"),
            ("   ~~~\n\talpha\n \t\n   ~~~", " alpha\u{2028} "),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Code Block");
            assert!(imported.char_styles.is_empty() && imported.warnings.is_empty());
            let (para, chars) = styles(&imported).resolve_para(&imported.story.paras[0]);
            assert_eq!(chars.font_family, MONO);
            assert!(!para.hyphenate);
        }
    }

    #[test]
    fn indented_code_keeps_interior_blank_lines() {
        for (source, expected) in [
            ("    alpha\n\n    beta", "alpha\u{2028}\u{2028}beta"),
            ("    alpha\n\n\n    beta", "alpha\u{2028}\u{2028}\u{2028}beta"),
            ("    *alpha*\n      \n    **beta**", "*alpha*\u{2028}  \u{2028}**beta**"),
            ("    alpha\n \n    beta", "alpha\u{2028}\u{2028}beta"),
            ("    alpha\n\t\t\n    beta", "alpha\u{2028}\t\u{2028}beta"),
            ("\té\r\n \t\r\n\t中", "é\u{2028}\u{2028}中"),
        ] {
            let imported = read(source);
            let fenced = read(&format!("```\n{}\n```", expected.replace('\u{2028}', "\n")));
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.text, fenced.story.text);
            assert_eq!(imported.story.paras.len(), 1);
            assert_eq!(imported.story.paras[0].style, "Code Block");
            assert!(imported.char_styles.is_empty() && imported.warnings.is_empty());
            let (para, chars) = styles(&imported).resolve_para(&imported.story.paras[0]);
            assert_eq!(chars.font_family, MONO);
            assert!(!para.hyphenate);
        }
    }

    #[test]
    fn indented_code_drops_edge_blanks_and_resumes_blocks() {
        for (source, expected, middle_style) in [
            ("\n    alpha\n    \n\n# Heading\n\n    beta\n\n    \n", "alpha\nHeading\nbeta", "Heading 1"),
            ("\n    alpha\n\nBody\n\n    beta\n\n", "alpha\nBody\nbeta", "Body Text"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Code Block", middle_style, "Code Block"]);
            assert!(imported.char_styles.is_empty() && imported.warnings.is_empty());
        }
    }

    #[test]
    fn indented_code_precedes_list_and_unsupported_markers() {
        for (source, expected) in [
            ("    - **literal**", "- **literal**"),
            ("    7. *literal*", "7. *literal*"),
            ("\t- **literal**", "- **literal**"),
            (" \t  <b>literal</b>", "  <b>literal</b>"),
            ("    > **quote**", "> **quote**"),
            ("    ***", "***"),
            ("    | a | b |", "| a | b |"),
            ("    [link](a*path*)", "[link](a*path*)"),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras[0].style, "Code Block");
            assert!(imported.char_styles.is_empty() && imported.warnings.is_empty());
        }
        let mixed = read("\tfirst\n    - **literal**\n \t\tlast");
        assert_eq!(mixed.story.text, "first\u{2028}- **literal**\u{2028}\tlast");
        assert_eq!(mixed.story.paras.len(), 1);
        let continuation = read("Body\n    - **not a list**");
        assert_eq!(continuation.story.text, "Body - not a list");
        assert_eq!(continuation.story.paras[0].style, "Body Text");
        let nested = read("- Parent\n    - Child\n- Next");
        assert_eq!(nested.story.text, "Parent\nChild\nNext");
        assert_eq!(styles(&nested).resolve_para(&nested.story.paras[1]).0.left_indent, 36.0);
    }

    #[test]
    fn code_punctuation_does_not_trigger_unsupported_markup() {
        for (source, expected, code) in [
            ("Use `a < b` and **bold**.", "Use a < b and bold.", "a < b"),
            ("Use `x | y` and **bold**.", "Use x | y and bold.", "x | y"),
            ("Use ``[x](a*path*)`` and **bold**.", "Use [x](a*path*) and bold.", "[x](a*path*)"),
            ("Use `![x](a)` and **bold**.", "Use ![x](a) and bold.", "![x](a)"),
            ("Use `<tag>**raw**</tag>` and **bold**.", "Use <tag>**raw**</tag> and bold.", "<tag>**raw**</tag>"),
            ("Use `][` and **bold**.", "Use ][ and bold.", "]["),
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, expected, "{source}");
            assert!(imported.warnings.is_empty());
            assert_eq!(imported.story.format_after(imported.story.text.find(code).unwrap()).style, "Code");
            assert_eq!(imported.story.format_after(imported.story.text.find("bold").unwrap()).style, "Strong");
        }
        let escaped = read(r"**bold** < 10 and \[text\]\(target\) \<b\>.");
        assert_eq!(escaped.story.text, "bold < 10 and [text](target) <b>.");
        assert!(escaped.warnings.is_empty());
        assert_eq!(escaped.story.format_after(0).style, "Strong");
    }

    #[test]
    fn genuine_unsupported_markup_keeps_mixed_code_source_literal() {
        for source in [
            "Use `a < b` [**link**](a*path*) and **bold**.",
            "[**link**](a*path*) with `a < b` and **bold**.",
            "Use `[x](y)` ![alt](image.png) **bold**.",
            "Use `x | y` with <b>**raw**</b> and **bold**.",
            "Use `a < b` with <https://example.org> and **bold**.",
            r"Use \*escaped\* `a < b` and [link](destination).",
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, source, "{source}");
            assert!(imported.char_styles.is_empty());
            assert_eq!(imported.warnings.len(), 1);
        }
    }

    #[test]
    fn multiline_html_blocks_preserve_source_and_resume_at_their_boundary() {
        for (literal, separator) in [
            ("<div data-x=\">\">\n**literal**\n- item\n`[x](y)`\n</div>", "\n\n"),
            ("<!--\n**literal**\n\n# heading\n```rust\n[link](dest)\n-->", "\n"),
            ("<PRE>\n**literal**\n\n- item\n</pRe>", "\n"),
            ("<![CDATA[\n**literal**\n]]>", "\n"),
            ("<?instruction\n**literal**\n?>", "\n"),
            ("<!DOCTYPE\n**literal**\n>", "\n"),
            ("<custom>\n**literal**\n</custom>", "\n\n"),
            ("<pre/>\n**literal**", "\n\n"),
        ] {
            let imported = read(&format!("{literal}{separator}**parsed**"));
            assert_eq!(imported.story.text, format!("{literal}\nparsed"), "{literal}");
            assert_eq!(imported.story.format_after(0).style, designcraft_doc::NO_CHAR_STYLE);
            assert_eq!(imported.story.format_after(imported.story.text.find("**literal**").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
            assert_eq!(imported.story.format_after(imported.story.text.rfind("parsed").unwrap()).style, "Strong");
            assert_eq!(imported.warnings.len(), 1);
        }
        let unclosed = "<!--\n**literal**\n\n```\n# still literal";
        let imported = read(unclosed);
        assert_eq!(imported.story.text, unclosed);
        assert!(imported.char_styles.is_empty());
    }

    #[test]
    fn heading_emphasis_retains_the_imported_base_weight() {
        for level in 1..=6 {
            let imported = read(&format!("{} plain *em* *`code`* tail\n\n*body*", "#".repeat(level)));
            assert_eq!(imported.story.text, "plain em code tail\nbody");
            let definitions = styles(&imported);
            let (_, heading) = definitions.resolve_para(&imported.story.paras[0]);
            for word in ["em", "code"] {
                let format = imported.story.format_after(imported.story.text.find(word).unwrap());
                assert_eq!(definitions.resolve_char(&heading, format).font_style, "Bold Italic");
                assert!(format.style.starts_with("Emphasis"), "inherited weight must not masquerade as explicit strong markup");
                assert_eq!(format.over.font_style.as_deref(), Some("Bold Italic"));
            }
            for word in ["plain", "tail"] {
                let format = imported.story.format_after(imported.story.text.find(word).unwrap());
                assert_eq!(definitions.resolve_char(&heading, format).font_style, "Bold");
                assert_eq!(format.style, designcraft_doc::NO_CHAR_STYLE);
            }
            let (_, body) = definitions.resolve_para(&imported.story.paras[1]);
            assert_eq!(definitions.resolve_char(&body, imported.story.format_after(imported.story.text.find("body").unwrap())).font_style, "Italic");
        }
        let setext = read("*Heading*\n===");
        let definitions = styles(&setext);
        let (_, base) = definitions.resolve_para(&setext.story.paras[0]);
        assert_eq!(definitions.resolve_char(&base, setext.story.format_after(0)).font_style, "Bold Italic");

        // Read the actual paragraph attributes; the style's name alone must not force bold.
        let mut builder = Builder {
            text: String::new(),
            runs: Vec::new(),
            paras: Vec::new(),
            para_styles: Vec::new(),
            char_styles: Vec::new(),
            warnings: Vec::new(),
            work: 1024,
        };
        let mut regular_heading = heading_style(1);
        regular_heading.chars.font_style = Some("Regular".into());
        builder.block_with_references("*Regular*", regular_heading, ParaAttrs::default(), false, None).unwrap();
        let regular = builder.finish();
        regular.story.check().unwrap();
        let definitions = styles(&regular);
        let (_, base) = definitions.resolve_para(&regular.story.paras[0]);
        assert_eq!(definitions.resolve_char(&base, regular.story.format_after(0)).font_style, "Italic");
    }

    #[test]
    fn style_defaults_can_be_replaced_without_local_formatting() {
        let imported = read("# Heading\n\n**Strong**\n\n- Item");
        let mut styles = styles(&imported);
        styles.para_mut("Heading 1").unwrap().chars.size = Some(42.0);
        styles.char_style_mut("Strong").unwrap().chars.font_style = Some("Regular".into());
        styles.para_mut("Bullet List").unwrap().para.left_indent = Some(27.0);
        assert_eq!(styles.resolve_para(&imported.story.paras[0]).1.size, 42.0);
        let (_, base) = styles.resolve_para(&imported.story.paras[1]);
        let at = imported.story.text.find("Strong").unwrap();
        assert_eq!(styles.resolve_char(&base, imported.story.format_after(at)).font_style, "Regular");
        assert_eq!(styles.resolve_para(&imported.story.paras[2]).0.left_indent, 27.0);
    }

    #[test]
    fn unsupported_markup_stays_visible_with_deduplicated_warnings() {
        let imported =
            read("> **quote**\n\n| a | b |\n| --- | --- |\n\n[link](https://example.org) ![alt](image.png)\n\n[other](destination)\n\n---");
        assert_eq!(
            imported.story.text,
            "> **quote**\n| a | b |\n| --- | --- |\n[link](https://example.org) ![alt](image.png)\n[other](destination)\n---"
        );
        assert_eq!(imported.warnings.len(), 2);
        assert!(imported.char_styles.is_empty());
        let linked = read("[**label**](a*path*) with **emphasis**");
        assert_eq!(linked.story.text, "[**label**](a*path*) with **emphasis**");
        assert!(linked.char_styles.is_empty());
        let zero = read("0. Zero\n1. Next");
        assert_eq!(styles(&zero).resolve_para(&zero.story.paras[0]).0.start_at, Some(1));
        assert_eq!(zero.warnings.len(), 1);
    }

    #[test]
    fn empty_documents_and_empty_blocks_are_valid_stories() {
        for source in ["", "\u{FEFF} \r\n\r", "#", "```\n```"] {
            let imported = read(source);
            assert!(imported.story.text.is_empty());
            assert_eq!(imported.story.paras.len(), 1);
        }
        assert!(read("").para_styles.is_empty());
        assert_eq!(read("#").story.paras[0].style, "Heading 1");
    }

    #[test]
    fn bad_encoding_and_input_limits_return_errors() {
        assert!(matches!(import(&[0xff]), Err(ImportError::Corrupt(_))));
        assert!(matches!(import_capped(b"1234", 3), Err(ImportError::Unsupported(_))));
        for depth in [DEPTH_LIMIT, DEPTH_LIMIT + 1] {
            let source = format!("{}leaf{}", "**a ".repeat(depth), " z**".repeat(depth));
            if depth == DEPTH_LIMIT {
                let imported = read(&source);
                assert_eq!(imported.story.text, format!("{}leaf{}", "a ".repeat(depth), " z".repeat(depth)));
                assert_eq!(imported.story.paras.len(), 1);
                assert!(imported.story.chars.iter().all(|run| run.format.style == "Strong"));
            } else {
                assert!(matches!(import(source.as_bytes()), Err(ImportError::Unsupported(_))));
            }
        }
        let mut nested = String::new();
        for depth in 0..=DEPTH_LIMIT {
            nested.push_str(&format!("{}- item\n", "  ".repeat(depth)));
        }
        assert!(matches!(import(nested.as_bytes()), Err(ImportError::Unsupported(_))));
        assert!(matches!(import("# p\n".repeat(BLOCK_LIMIT + 1).as_bytes()), Err(ImportError::Unsupported(_))));
        assert!(matches!(import("| a | b |\n".repeat(BLOCK_LIMIT + 1).as_bytes()), Err(ImportError::Unsupported(_))));
    }

    #[test]
    fn short_malformed_unicode_inputs_keep_story_invariants() {
        let pieces = ["é", "日本", "*", "**", "_", "~~", "`", "\\", "\n"];
        for a in pieces {
            for b in pieces {
                for c in pieces {
                    read(&format!("Text {a}{b}{c} end"));
                }
            }
        }
    }
}

#[cfg(test)]
mod r8_unmatched_table_tests {
    use super::*;

    fn literal(source: &str) {
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text, source);
        assert_eq!(imported.story.paras.len(), source.lines().count());
        assert!(imported.story.paras.iter().all(|p| p.style == "Body Text" && p.chars.is_empty() && p.table.is_none()));
        assert!(imported.char_styles.is_empty());
        for (at, _) in source.char_indices() {
            assert_eq!(imported.story.format_after(at), &CharFormat::default());
        }
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
    }

    #[test]
    fn r8_unmatched_header_preserves_exact_literal_rows_and_run_variants() {
        for source in [
            "**A**|`B\n---|---\n*C*|*D*",
            "**A**|``B\n---|---\n*C*|*D*",
            "**A**|```B\n---|---\n*C*|*D*",
            "**A**|`B ``C\n---|---\n*D*|*E*",
            "**A**|``B `C\n---|---\n*D*|*E*",
            "**A**|`B|C\n---|---|---\n*D*|*E*|F",
            "**A**|`B ``\n---|---\n*C*|*D*",
            "**A**|`local` ``open\n---|---\n*C*|*D*",
            "\\`**A**|``B\n---|---\n*C*|*D*",
            "  |**é**|``中|\n  |:---:|---:|\n  **C**\n  `cell`",
        ] {
            literal(source);
        }
    }

    #[test]
    fn r8_unmatched_header_keeps_single_cells_and_resumes_at_block_boundaries() {
        let rows = "**A**|``B\n---|---\n**C**\n`cell`";
        for (suffix, expected, style, word, char_style) in [
            ("\n*after* ``tail", "after ``tail", "Body Text", "after", "Emphasis"),
            ("# *after* ``tail", "after ``tail", "Heading 1", "after", "Emphasis"),
            ("- *after* ``tail", "after ``tail", "Bullet List", "after", "Emphasis"),
            ("\n7. **after** ``tail", "after ``tail", "Numbered List", "after", "Strong"),
            ("~~~\n*after* ``tail\n~~~", "*after* ``tail", "Code Block", "*after*", designcraft_doc::NO_CHAR_STYLE),
            ("    *after* `tail`", "    *after* `tail`", "Body Text", "*after*", designcraft_doc::NO_CHAR_STYLE),
            ("> **after** ``tail", "> **after** ``tail", "Body Text", "**after**", designcraft_doc::NO_CHAR_STYLE),
            ("<div>**after** ``tail</div>", "<div>**after** ``tail</div>", "Body Text", "**after**", designcraft_doc::NO_CHAR_STYLE),
            ("---\n# after", "---\nafter", "Heading 1", "after", designcraft_doc::NO_CHAR_STYLE),
        ] {
            let source = format!("{rows}\n{suffix}");
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, format!("{rows}\n{expected}"), "{suffix:?}");
            assert!(imported.story.paras[..4].iter().all(|p| p.style == "Body Text"));
            for (at, _) in rows.char_indices() {
                assert_eq!(imported.story.format_after(at), &CharFormat::default(), "{suffix:?}");
            }
            assert_eq!(imported.story.paras.last().unwrap().style, style, "{suffix:?}");
            let at = imported.story.text.rfind(word).unwrap();
            assert_eq!(imported.story.format_after(at).style, char_style, "{suffix:?}");
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
    }

    #[test]
    fn r8_ordered_marker_above_one_does_not_interrupt_multiline_code() {
        let source = "**A**|``B\n---|---\n**C**\n`cell`\n7. **after** ``tail";
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text, "A|B ---|--- **C** `cell` 7. **after** tail");
        assert_eq!(imported.story.paras.len(), 1);
        assert_eq!(imported.story.paras[0].style, "Body Text");
        let code = 2..imported.story.text.len() - "tail".len();
        for (at, _) in imported.story.text.char_indices() {
            let mut expected = CharFormat::default();
            if at == 0 {
                expected.style = "Strong".into();
            } else if code.contains(&at) {
                expected.style = "Code".into();
            }
            assert_eq!(imported.story.format_after(at), &expected, "{source:?} at {at}");
        }
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn r8_matched_multiline_and_later_run_widths_remain_code_controls() {
        for (source, expected, code) in [
            ("**A**|`B\n---|---\n*C*|*D* close` **after**", "A|B ---|--- *C*|*D* close after", "B"),
            ("**A**|``B\n---|---\n*C*|*D* close`` **after**", "A|B ---|--- *C*|*D* close after", "B"),
            ("**A**|``B `C\n---|---\n*D*|*E* close` **after**", "A|``B C ---|--- *D*|*E* close after", "C"),
            ("**A**|`B\n---|---\n*C*|*D* close\\` **after**", "A|B ---|--- *C*|*D* close\\ after", "B"),
            ("`open\n**A**|**B**\n---|---\n*C*|*D* close` **after**", "open **A**|**B** ---|--- *C*|*D* close after", "open"),
        ] {
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1, "{source:?}");
            assert_eq!(imported.story.format_after(expected.find(code).unwrap()).style, "Code", "{source:?}");
            assert_eq!(imported.story.format_after(expected.rfind("after").unwrap()).style, "Strong", "{source:?}");
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn r8_incomplete_code_and_escaped_or_code_pipes_remain_ordinary_controls() {
        for (source, expected) in [
            ("**before** `open", "before `open"),
            ("**A**\\|`B\n---|---\n*C*|*D*", "A|`B ---|--- C|D"),
            ("**A**|``B\n---\\|---\n*C*|*D*", "A|``B ---|--- C|D"),
        ] {
            let imported = import(source.as_bytes()).unwrap();
            imported.story.check().unwrap();
            assert_eq!(imported.story.text, expected, "{source:?}");
            assert_eq!(imported.story.paras.len(), 1, "{source:?}");
            assert_eq!(imported.story.format_after(0).style, "Strong", "{source:?}");
            assert!(imported.warnings.is_empty(), "{source:?}");
        }
    }

    #[test]
    fn r8_single_line_code_pipes_follow_maintained_table_grammar() {
        literal("**A**|\u{0060}B|C\u{0060}\n---|---|---\n*D*|*E*");
    }

    #[test]
    fn r8_many_unmatched_headers_retain_complete_literal_source() {
        let rows = "**A**|`B\n---|---\n**C**";
        let source = vec![rows; 2000].join("\n\n");
        let imported = import(source.as_bytes()).unwrap();
        imported.story.check().unwrap();
        assert_eq!(imported.story.text, vec![rows; 2000].join("\n"));
        assert_eq!(imported.story.paras.len(), 6000);
        assert!(imported.story.chars.iter().all(|run| run.format == CharFormat::default()));
        assert!(imported.char_styles.is_empty());
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
    }
}

#[cfg(test)]
#[path = "markdown_review9_tests.rs"]
mod markdown_review9_tests;

#[cfg(test)]
#[path = "markdown_quote_tests.rs"]
mod markdown_quote_tests;

#[cfg(test)]
#[path = "markdown_reference_title_tests.rs"]
mod markdown_reference_title_tests;

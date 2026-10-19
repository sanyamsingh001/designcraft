//! Bounded Markdown import through the same named-style mapping as Word.
//! Supports ATX/setext headings, paragraphs, nested lists, emphasis/strong, strikethrough and
//! code. Paragraphs containing links/images/HTML and quote/table blocks stay literal, with
//! warnings. Complex list containers are simplified to separate paragraphs.
//! This is a one-time import; it does not create a live-sync link.

use crate::{ImportError, Imported, ImportedStyle};
use designcraft_doc::{CharAttrs, CharFormat, CharRun, FORCED_LINE_BREAK, ListType, ParaAttrs, ParaFormat, Story, StoryId};
use unicode_general_category::{GeneralCategory, get_general_category};

const SOURCE_LIMIT: usize = 16 * 1024 * 1024;
const BLOCK_LIMIT: usize = 10_000;
const TOKEN_LIMIT: usize = 100_000;
const DEPTH_LIMIT: usize = 32;
const MONO: &str = "JetBrains Mono";

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
    fn subtract(&mut self, other: Self) {
        self.bold = self.bold.saturating_sub(other.bold);
        self.italic = self.italic.saturating_sub(other.italic);
        self.strike = self.strike.saturating_sub(other.strike);
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
    known_references: std::collections::BTreeSet<String>,
    defined_references: std::collections::BTreeSet<String>,
}
impl Builder {
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
    fn block(&mut self, source: &str, style: ImportedStyle, attrs: ParaAttrs, literal: bool) -> Result<(), ImportError> {
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
            self.inline(source.trim_end_matches([' ', '\t']), base_bold)?;
        }
        Ok(())
    }
    fn inline(&mut self, source: &str, base_bold: bool) -> Result<(), ImportError> {
        // Literal reference rows were already split; this first line may be list content.
        if inline_syntax(source, false, self)?.unsupported {
            self.warn("Markdown paragraphs containing links, images or HTML were kept as source text.");
            return self.append(&source.replace('\n', &FORCED_LINE_BREAK.to_string()), Inline::default());
        }
        let mut tokens = tokenize(source, self)?;
        pair(&mut tokens, self)?;
        let mut state = Inline { base_bold, ..Default::default() };
        for token in tokens {
            match token {
                Token::Text(text) => self.append(&text, state)?,
                Token::Code(text) => self.append(&text, Inline { code: true, ..state })?,
                Token::Delimiter(d) => {
                    state.subtract(d.end);
                    let literal: String = std::iter::repeat_n(d.marker, d.remaining).collect();
                    self.append(&literal, state)?;
                    state.add(d.begin);
                }
            }
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

struct Pending {
    text: String,
    /// Original indentation/list marker, retained for a literal first row.
    prefix: String,
    /// First unmatched code run: byte offset and delimiter width.
    open_code: Option<(usize, usize)>,
    style: ImportedStyle,
    attrs: ParaAttrs,
    /// None for a regular paragraph, else the list marker's content column.
    list_column: Option<usize>,
}
impl Pending {
    fn continuation(&mut self, line: &str, open_code: Option<(usize, usize)>, builder: &mut Builder) -> Result<(), ImportError> {
        // Keep raw boundaries and whitespace until code spans have been recognized.
        let line_start = self.text.len() + 1;
        self.text.push('\n');
        self.text.push_str(line);
        if let Some((at, width)) = self.open_code {
            // Revisit the unresolved suffix only when a matching delimiter arrives.
            if code_end(line, width, builder)?.is_some() {
                let source = self.text.get(at..).ok_or_else(|| ImportError::Corrupt("Markdown code boundary".into()))?;
                self.open_code = inline_syntax(source, true, builder)?.open_code.map(|(next, width)| (at + next, width));
            }
        } else {
            self.open_code = open_code.map(|(at, width)| (line_start + at, width));
        }
        Ok(())
    }
}
fn flush(pending: &mut Option<Pending>, lists: &mut Vec<List>, builder: &mut Builder) -> Result<(), ImportError> {
    if let Some(p) = pending.take() {
        // Classify rows only after complete multiline code spans can be skipped.
        let literal_lines = inline_syntax(&p.text, p.list_column.is_none(), builder)?.literal_lines;
        if literal_lines.is_empty() {
            builder.block(&p.text, p.style, p.attrs, false)?;
            return Ok(());
        }
        lists.clear();
        let mut start = 0;
        let mut style = p.style;
        let mut attrs = p.attrs;
        for at in literal_lines {
            if at < start {
                continue;
            }
            if start < at {
                let text = p.text.get(start..at).ok_or_else(|| ImportError::Corrupt("Markdown literal boundary".into()))?;
                builder.block(text.trim_end_matches('\n'), style.clone(), attrs.clone(), false)?;
            }
            let rest = p.text.get(at..).ok_or_else(|| ImportError::Corrupt("Markdown literal boundary".into()))?;
            let mut end = rest.find('\n').unwrap_or(rest.len());
            let line = rest.get(..end).ok_or_else(|| ImportError::Corrupt("Markdown literal boundary".into()))?;
            let literal = if at == 0 { format!("{}{line}", p.prefix) } else { line.into() };
            let next = rest.lines().skip(1);
            let continued = reference::remember(&literal, next.clone(), builder)?;
            builder.warn("Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.");
            builder.block(&literal, body_style("Body Text"), ParaAttrs::default(), true)?;
            for next in next.take(continued) {
                builder.block(next, body_style("Body Text"), ParaAttrs::default(), true)?;
                end += 1 + next.len();
            }
            start = at + end + usize::from(end < rest.len());
            style = body_style("Body Text");
            attrs = ParaAttrs::default();
        }
        let text = p.text.get(start..).ok_or_else(|| ImportError::Corrupt("Markdown literal boundary".into()))?;
        if !text.is_empty() {
            builder.block(text, style, attrs, false)?;
        }
    }
    Ok(())
}
fn heading(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_start_matches(' ');
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = line.get(level..)?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = rest.trim_matches([' ', '\t']);
    let without_hashes = text.trim_end_matches('#');
    if without_hashes.is_empty() || without_hashes.ends_with([' ', '\t']) {
        text = without_hashes.trim_end_matches([' ', '\t']);
    }
    Some((level, text))
}
fn setext(line: &str) -> Option<usize> {
    let text = line.trim_matches([' ', '\t']);
    let marker = text.chars().next()?;
    if matches!(marker, '=' | '-') && text.chars().all(|c| c == marker) { Some(if marker == '=' { 1 } else { 2 }) } else { None }
}
fn thematic(line: &str) -> bool {
    let text = line.trim_matches([' ', '\t']);
    let Some(marker) = text.chars().next() else { return false };
    matches!(marker, '-' | '*' | '_')
        && text.chars().filter(|c| !c.is_whitespace()).count() >= 3
        && text.chars().all(|c| c == marker || c == ' ' || c == '\t')
}
// Only ASCII space/tab is blank-row syntax; Unicode spacing remains literal content.
fn is_blank(line: &str) -> bool {
    line.bytes().all(|b| matches!(b, b' ' | b'\t'))
}

/// (indent columns, first non-whitespace byte).
fn indentation(line: &str) -> (usize, usize) {
    let mut columns: usize = 0;
    let mut bytes = 0;
    for c in line.chars() {
        match c {
            ' ' => columns = columns.saturating_add(1),
            '\t' => columns = columns.saturating_add(4 - columns % 4),
            _ => break,
        }
        bytes += c.len_utf8();
    }
    (columns, bytes)
}
struct Marker<'a> {
    kind: ListType,
    marker: char,
    number: u32,
    indent: usize,
    column: usize,
    text: &'a str,
}
fn list_marker(line: &str) -> Option<Marker<'_>> {
    let (indent, bytes) = indentation(line);
    let text = line.get(bytes..)?;
    let first = text.chars().next()?;
    let (kind, marker, number, width) = if matches!(first, '-' | '+' | '*') {
        (ListType::Bullets, first, 1, 1)
    } else {
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        let marker = text.get(digits..)?.chars().next()?;
        if !matches!(marker, '.' | ')') {
            return None;
        }
        (ListType::Numbers, marker, text.get(..digits)?.parse().ok()?, digits + 1)
    };
    let rest = text.get(width..)?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let (spaces, start) = indentation(rest);
    Some(Marker { kind, marker, number, indent, column: indent.saturating_add(width).saturating_add(spaces.max(1)), text: rest.get(start..)? })
}
struct List {
    indent: usize,
    marker: char,
    next: u32,
}
fn list_attrs(marker: &Marker<'_>, stack: &mut Vec<List>) -> Result<ParaAttrs, ImportError> {
    while stack.last().is_some_and(|l| l.indent > marker.indent) {
        stack.pop();
    }
    if stack.last().is_some_and(|l| l.indent == marker.indent && l.marker != marker.marker) {
        stack.pop();
    }
    if stack.last().is_none_or(|l| l.indent < marker.indent) {
        if stack.len() >= DEPTH_LIMIT {
            return Err(limit("list nesting"));
        }
        stack.push(List { indent: marker.indent, marker: marker.marker, next: marker.number.max(1) });
    }
    let mut attrs = ParaAttrs::default();
    if stack.len() > 1 {
        attrs.left_indent = Some(18.0 * stack.len() as f64);
    }
    if marker.kind == ListType::Numbers
        && let Some(list) = stack.last_mut()
    {
        attrs.start_at = Some(Some(list.next));
        list.next = list.next.checked_add(1).ok_or_else(|| limit("list numbering"))?;
    }
    Ok(attrs)
}
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let (indent, bytes) = indentation(line);
    if indent > 3 {
        return None;
    }
    let text = line.get(bytes..)?;
    let marker = text.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = text.chars().take_while(|&c| c == marker).count();
    let rest = text.get(count..)?;
    if count < 3 || (marker == '`' && rest.contains('`')) {
        return None;
    }
    Some((marker, count, rest))
}
/// Remove at most the opening fence's indentation, retaining a partially consumed tab.
fn append_fenced_line(out: &mut String, line: &str, indent: usize) {
    let mut columns = 0usize;
    let mut rest = line;
    while columns < indent {
        if let Some(tail) = rest.strip_prefix(' ') {
            columns += 1;
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix('\t') {
            columns += 4 - columns % 4;
            rest = tail;
        } else {
            break;
        }
    }
    out.extend(std::iter::repeat_n(' ', columns.saturating_sub(indent)));
    out.push_str(rest);
}

/// Remove four indentation columns, including tabs, while retaining any extra indentation.
fn code_content(line: &str) -> Option<&str> {
    let mut columns = 0;
    for (at, c) in line.char_indices() {
        columns += match c {
            ' ' => 1,
            '\t' => 4 - columns % 4,
            _ => return None,
        };
        if columns == 4 {
            return line.get(at + c.len_utf8()..);
        }
    }
    None
}

fn html_tag(source: &str) -> Option<(&str, &str)> {
    let rest = source.strip_prefix('<')?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    if !rest.bytes().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let count = rest.bytes().take_while(|c| c.is_ascii_alphanumeric() || *c == b'-').count();
    let tail = rest.get(count..)?;
    if (!tail.is_empty() && !tail.starts_with([' ', '\t', '\n', '/', '>'])) || (tail.starts_with('/') && !tail.starts_with("/>")) {
        return None;
    }
    Some((rest.get(..count)?, tail))
}

/// Closing angle brackets in quoted attributes do not end the tag.
fn html_tag_end(source: &str, builder: &mut Builder) -> Result<Option<usize>, ImportError> {
    if html_tag(source).is_none() {
        return Ok(None);
    }
    let mut quote = None;
    for (at, c) in source.char_indices() {
        builder.spend()?;
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if matches!(c, '\'' | '"') {
            quote = Some(c);
        } else if c == '>' {
            return Ok(Some(at + 1));
        }
    }
    Ok(None)
}

fn inline_html(source: &str, builder: &mut Builder) -> Result<bool, ImportError> {
    if ["<!--", "<?", "<![CDATA["].iter().any(|prefix| source.starts_with(*prefix))
        || source.strip_prefix("<!").and_then(|s| s.bytes().next()).is_some_and(|c| c.is_ascii_uppercase())
        || html_tag_end(source, builder)?.is_some()
    {
        return Ok(true);
    }
    let Some(rest) = source.strip_prefix('<') else { return Ok(false) };
    let mut end = None;
    for (at, c) in rest.char_indices() {
        builder.spend()?;
        if c == '>' {
            end = Some(at);
            break;
        }
        if c.is_whitespace() || c == '<' {
            return Ok(false);
        }
    }
    let Some(inside) = end.and_then(|at| rest.get(..at)) else { return Ok(false) };
    let uri = inside.split_once(':').is_some_and(|(scheme, _)| {
        (2..=32).contains(&scheme.len())
            && scheme.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
            && scheme.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
    });
    let email = inside.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty());
    Ok(uri || email)
}

#[derive(Clone, Copy)]
enum HtmlBlock {
    Until(&'static str),
    Raw(&'static str),
    Blank,
}
impl HtmlBlock {
    fn ended_by(self, line: &str) -> bool {
        match self {
            Self::Until(end) => line.contains(end),
            Self::Raw(end) => line.as_bytes().windows(end.len()).any(|w| w.eq_ignore_ascii_case(end.as_bytes())),
            Self::Blank => is_blank(line),
        }
    }
}
fn html_block(line: &str, can_interrupt: bool, builder: &mut Builder) -> Result<Option<HtmlBlock>, ImportError> {
    let (indent, bytes) = indentation(line);
    if indent > 3 {
        return Ok(None);
    }
    let Some(source) = line.get(bytes..) else { return Ok(None) };
    if source.starts_with("<!--") {
        return Ok(Some(HtmlBlock::Until("-->")));
    }
    if source.starts_with("<?") {
        return Ok(Some(HtmlBlock::Until("?>")));
    }
    if source.starts_with("<![CDATA[") {
        return Ok(Some(HtmlBlock::Until("]]>")));
    }
    if source.strip_prefix("<!").and_then(|s| s.bytes().next()).is_some_and(|c| c.is_ascii_uppercase()) {
        return Ok(Some(HtmlBlock::Until(">")));
    }
    let Some((name, tail)) = html_tag(source) else { return Ok(None) };
    if !source.starts_with("</") && !tail.starts_with('/') {
        for (tag, end) in [("script", "</script>"), ("pre", "</pre>"), ("style", "</style>"), ("textarea", "</textarea>")] {
            if name.eq_ignore_ascii_case(tag) {
                return Ok(Some(HtmlBlock::Raw(end)));
            }
        }
    }
    const BLOCK_TAGS: &[&str] = &[
        "address",
        "article",
        "aside",
        "base",
        "basefont",
        "blockquote",
        "body",
        "caption",
        "center",
        "col",
        "colgroup",
        "dd",
        "details",
        "dialog",
        "dir",
        "div",
        "dl",
        "dt",
        "fieldset",
        "figcaption",
        "figure",
        "footer",
        "form",
        "frame",
        "frameset",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "head",
        "header",
        "hr",
        "html",
        "iframe",
        "legend",
        "li",
        "link",
        "main",
        "menu",
        "menuitem",
        "nav",
        "noframes",
        "ol",
        "optgroup",
        "option",
        "p",
        "param",
        "search",
        "section",
        "summary",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "title",
        "tr",
        "track",
        "ul",
    ];
    if BLOCK_TAGS.iter().any(|tag| name.eq_ignore_ascii_case(tag))
        || (can_interrupt && html_tag_end(source, builder)?.and_then(|end| source.get(end..)).is_some_and(|s| s.trim().is_empty()))
    {
        return Ok(Some(HtmlBlock::Blank));
    }
    Ok(None)
}

// Only paragraph tails can omit the quote marker. Quoted block contents remain literal;
// their small amount of state prevents code/HTML rows from opening a lazy paragraph.
#[derive(Default)]
enum QuoteTail {
    #[default]
    Empty,
    Paragraph,
    Fence(char, usize),
    Html(HtmlBlock),
    Other,
}

impl QuoteTail {
    fn marked(&mut self, content: &str, builder: &mut Builder) -> Result<(), ImportError> {
        match self {
            Self::Fence(marker, width) => {
                if fence(content).is_some_and(|(m, n, rest)| m == *marker && n >= *width && is_blank(rest)) {
                    *self = Self::Empty;
                }
                return Ok(());
            }
            Self::Html(block) => {
                if block.ended_by(content) {
                    *self = Self::Empty;
                }
                return Ok(());
            }
            Self::Other if !is_blank(content) => return Ok(()),
            _ => {}
        }
        let paragraph = matches!(self, Self::Paragraph);
        *self = Self::Empty;
        if is_blank(content) {
            return Ok(());
        }
        if let Some((marker, width, _)) = fence(content) {
            *self = Self::Fence(marker, width);
        } else if let Some(block) = html_block(content, !paragraph, builder)? {
            if !block.ended_by(content) {
                *self = Self::Html(block);
            }
        } else if (!paragraph && code_content(content).is_some())
            || (indentation(content).0 <= 3 && (heading(content).is_some() || thematic(content) || (paragraph && setext(content).is_some())))
        {
            // A heading, thematic break or indented code row has no lazy paragraph tail.
        } else if content.trim_start_matches([' ', '\t']).starts_with('>')
            || list_marker(content).is_some_and(|m| m.indent <= 3)
            || table_delimiter_columns(content).is_some()
            || (!paragraph && inline_syntax(content, true, builder)?.reference)
        {
            // Complex quoted containers retain their existing literal fallback. A quoted blank
            // separates a following plain paragraph; do not infer one from container body rows.
            *self = Self::Other;
        } else {
            *self = Self::Paragraph;
        }
        Ok(())
    }
}

/// Strip one quote prefix only for classification; emission always uses the original row.
/// Tabs use the source column. Four remaining indentation columns suffice to identify code.
fn quote_content(line: &str) -> Option<std::borrow::Cow<'_, str>> {
    let (indent, start) = indentation(line);
    if indent > 3 {
        return None;
    }
    let rest = line.get(start..)?.strip_prefix('>')?;
    let mut column = indent + 1;
    let mut end = 0;
    for (at, c) in rest.char_indices() {
        column = match c {
            ' ' => column.saturating_add(1),
            '\t' => column.saturating_add(4 - column % 4),
            _ => break,
        };
        end = at + c.len_utf8();
    }
    if rest.get(..end)?.contains('\t') {
        let padding = column.saturating_sub(indent + 2).min(4);
        Some(std::borrow::Cow::Owned(format!("{}{}", " ".repeat(padding), rest.get(end..)?)))
    } else {
        Some(std::borrow::Cow::Borrowed(rest.strip_prefix(' ').unwrap_or(rest)))
    }
}

/// Count only separators outside escapes and code spans, excluding optional outer pipes.
fn table_columns(line: &str, syntax: &InlineSyntax) -> Option<usize> {
    if indentation(line).0 > 3 || syntax.pipes == 0 {
        return None;
    }
    let start = line.len() - line.trim_start_matches([' ', '\t']).len();
    let end = line.trim_end_matches([' ', '\t']).len();
    let mut columns = syntax.pipes + 1;
    if syntax.first_pipe == Some(start) {
        columns -= 1;
    }
    if syntax.last_pipe.is_some_and(|at| at + 1 == end) {
        columns -= 1;
    }
    (columns > 0).then_some(columns)
}

fn table_delimiter_columns(line: &str) -> Option<usize> {
    if indentation(line).0 > 3 {
        return None;
    }
    let line = line.trim_matches([' ', '\t']);
    if !line.contains('|') {
        return None;
    }
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let mut columns = 0;
    for cell in line.split('|') {
        let cell = cell.trim_matches([' ', '\t']);
        let cell = cell.strip_prefix(':').unwrap_or(cell);
        let cell = cell.strip_suffix(':').unwrap_or(cell);
        if cell.is_empty() || !cell.bytes().all(|b| b == b'-') {
            return None;
        }
        columns += 1;
    }
    Some(columns)
}

fn table_body_row(line: &str, builder: &mut Builder) -> Result<bool, ImportError> {
    // Confirmed table rows may have fewer cells, including a single cell without
    // a pipe. Only a blank line or another block ends the literal table extent.
    if is_blank(line)
        || indentation(line).0 > 3
        || fence(line).is_some()
        || heading(line).is_some()
        || thematic(line)
        || line.trim_start_matches([' ', '\t']).starts_with('>')
        || list_marker(line).is_some()
        || html_block(line, true, builder)?.is_some()
    {
        return Ok(false);
    }
    Ok(true)
}

/// Cache future raw backtick runs in a continuation region. A closer inside code is
/// raw (even an escaped backtick can close it), and must have exactly the opener's width.
/// Regions are scanned once and retain only the last line for each distinct width.
#[derive(Default)]
struct CodeLookahead {
    end_line: usize,
    last_run: std::collections::HashMap<usize, usize>,
}

impl CodeLookahead {
    fn closes_later(
        &mut self,
        line_index: usize,
        width: usize,
        lines: &std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
        builder: &mut Builder,
    ) -> Result<bool, ImportError> {
        if line_index >= self.end_line {
            self.last_run.clear();
            self.end_line = line_index + 1;
            for (index, line) in (*lines).clone() {
                self.end_line = index;
                // These blocks interrupt a pending paragraph even with an open code run.
                // Indentation alone does not interrupt its ordinary lazy continuation.
                if is_blank(line)
                    || fence(line).is_some()
                    || (indentation(line).0 <= 3 && heading(line).is_some())
                    || thematic(line)
                    || line.trim_start_matches([' ', '\t']).starts_with('>')
                    || list_marker(line).is_some_and(|m| m.indent <= 3)
                    || html_block(line, false, builder)?.is_some()
                {
                    break;
                }
                self.end_line = index + 1;
                let mut bytes = line.bytes().peekable();
                while let Some(c) = bytes.next() {
                    builder.spend()?;
                    if c == b'`' {
                        let mut count = 1;
                        while bytes.peek() == Some(&b'`') {
                            bytes.next();
                            builder.spend()?;
                            count += 1;
                        }
                        self.last_run.insert(count, index);
                    }
                }
            }
        }
        Ok(self.last_run.get(&width).is_some_and(|&index| index > line_index))
    }

    fn header_spans_lines(
        &mut self,
        line_index: usize,
        syntax: &InlineSyntax,
        lines: &std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
        builder: &mut Builder,
    ) -> Result<bool, ImportError> {
        // An earlier unmatched width can be plain text while a later width opens
        // real multiline code. The existing scanner already respected local spans/escapes.
        for &width in &syntax.unmatched_code {
            if self.closes_later(line_index, width, lines, builder)? {
                return Ok(true);
            }
        }
        Ok(false)
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
    let (imported, references) = parse(&text, std::collections::BTreeSet::new())?;
    if references.is_empty() {
        return Ok(imported);
    }
    drop(imported);
    parse(&text, references).map(|(imported, _)| imported)
}

fn parse(text: &str, references: std::collections::BTreeSet<String>) -> Result<(Imported, std::collections::BTreeSet<String>), ImportError> {
    let mut builder = Builder {
        text: String::new(),
        runs: Vec::new(),
        paras: Vec::new(),
        para_styles: Vec::new(),
        char_styles: Vec::new(),
        warnings: Vec::new(),
        work: text.len().saturating_mul(16).saturating_add(1024),
        known_references: references,
        defined_references: std::collections::BTreeSet::new(),
    };
    let mut pending: Option<Pending> = None;
    let mut lists = Vec::new();
    let mut html: Option<HtmlBlock> = None;
    let mut table = false;
    let mut quote = QuoteTail::default();
    let mut code_lookahead = CodeLookahead::default();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((line_index, line)) = lines.next() {
        if let Some(block) = html {
            if matches!(block, HtmlBlock::Blank) && block.ended_by(line) {
                html = None;
                continue;
            }
            builder.block(line, body_style("Body Text"), ParaAttrs::default(), true)?;
            if block.ended_by(line) {
                html = None;
            }
            continue;
        }
        if table {
            if table_body_row(line, &mut builder)? {
                builder.block(line, body_style("Body Text"), ParaAttrs::default(), true)?;
                continue;
            }
            table = false;
        }
        let quoted = quote_content(line);
        let quote_continuation = matches!(quote, QuoteTail::Paragraph) && quoted.is_none();
        if quoted.is_none() {
            quote = QuoteTail::default();
        }
        if is_blank(line) {
            flush(&mut pending, &mut lists, &mut builder)?;
            continue;
        }
        if let Some((marker, count, _)) = fence(line) {
            let indent = indentation(line).0;
            flush(&mut pending, &mut lists, &mut builder)?;
            lists.clear();
            let mut code = String::new();
            let mut first = true;
            for (_, line) in lines.by_ref() {
                if fence(line).is_some_and(|(m, n, rest)| m == marker && n >= count && is_blank(rest)) {
                    break;
                }
                if !first {
                    code.push(FORCED_LINE_BREAK);
                }
                append_fenced_line(&mut code, line, indent);
                first = false;
            }
            builder.block(&code, code_style(), ParaAttrs::default(), true)?;
            continue;
        }
        let (indent, _) = indentation(line);
        if pending.is_none()
            && !quote_continuation
            && lists.is_empty()
            && let Some(content) = code_content(line)
        {
            let mut code = content.to_string();
            let mut content_end = code.len();
            while lines.peek().is_some_and(|(_, l)| is_blank(l) || code_content(l).is_some()) {
                if let Some((_, next)) = lines.next() {
                    code.push(FORCED_LINE_BREAK);
                    code.push_str(code_content(next).unwrap_or(""));
                    if !is_blank(next) {
                        content_end = code.len();
                    }
                }
            }
            // Blank lines belong to the block only when indented content follows.
            code.truncate(content_end);
            builder.block(&code, code_style(), ParaAttrs::default(), true)?;
            continue;
        }
        if indent <= 3
            && let Some((level, content)) = heading(line)
        {
            flush(&mut pending, &mut lists, &mut builder)?;
            lists.clear();
            builder.block(content, heading_style(level), ParaAttrs::default(), false)?;
            continue;
        }
        if indent <= 3
            && let Some(level) = setext(line)
            && pending.as_ref().is_some_and(|p| p.list_column.is_none())
            && let Some(p) = pending.as_ref()
            && inline_syntax(&p.text, true, &mut builder)?.literal_lines.is_empty()
            && let Some(p) = pending.take()
        {
            lists.clear();
            builder.block(p.text.trim_matches([' ', '\t']), heading_style(level), ParaAttrs::default(), false)?;
            continue;
        }
        if pending.is_none() && !lists.is_empty() && indent > 0 {
            builder.warn("Multiple blocks inside Markdown list items were imported as separate paragraphs.");
        }
        if let Some(block) = html_block(line, pending.is_none() && !quote_continuation, &mut builder)? {
            flush(&mut pending, &mut lists, &mut builder)?;
            lists.clear();
            builder.warn("Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.");
            builder.block(line, body_style("Body Text"), ParaAttrs::default(), true)?;
            html = (!block.ended_by(line)).then_some(block);
            continue;
        }
        let syntax = inline_syntax(line, true, &mut builder)?;
        if pending.as_ref().is_none_or(|p| p.list_column.is_none() && p.open_code.is_none())
            && !line.trim_start_matches([' ', '\t']).starts_with('>')
            && list_marker(line).is_none()
            && let Some(columns) = table_columns(line, &syntax)
            && lines.peek().and_then(|(_, next)| table_delimiter_columns(next)) == Some(columns)
            && !code_lookahead.header_spans_lines(line_index, &syntax, &lines, &mut builder)?
        {
            flush(&mut pending, &mut lists, &mut builder)?;
            lists.clear();
            builder.warn("Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.");
            builder.block(line, body_style("Body Text"), ParaAttrs::default(), true)?;
            if let Some((_, delimiter)) = lines.next() {
                builder.block(delimiter, body_style("Body Text"), ParaAttrs::default(), true)?;
            }
            table = true;
            continue;
        }
        if let Some(content) = quoted {
            quote.marked(&content, &mut builder)?;
        } else if quote_continuation && !thematic(line) && !list_marker(line).is_some_and(|m| m.indent <= 3) {
            // Existing block-start branches above still win. Keep lazy paragraph rows byte-for-byte,
            // before reference discovery or inline parsing can reinterpret their source markup.
            builder.block(line, body_style("Body Text"), ParaAttrs::default(), true)?;
            quote = QuoteTail::Paragraph;
            continue;
        }
        let defer_reference = pending.as_ref().is_some_and(|p| p.open_code.is_some());
        let defer_literal = syntax.open_code.is_some() || defer_reference;
        let unsupported = thematic(line)
            || line.trim_start_matches([' ', '\t']).starts_with('>')
            || (!defer_literal && !syntax.literal_lines.is_empty())
            || (!defer_reference && syntax.reference);
        if unsupported {
            let continued = if syntax.reference { reference::remember(line, lines.clone().map(|(_, next)| next), &mut builder)? } else { 0 };
            flush(&mut pending, &mut lists, &mut builder)?;
            lists.clear();
            builder.warn("Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.");
            builder.block(line, body_style("Body Text"), ParaAttrs::default(), true)?;
            for (_, next) in lines.by_ref().take(continued) {
                builder.block(next, body_style("Body Text"), ParaAttrs::default(), true)?;
            }
            continue;
        }
        if let Some(marker) = list_marker(line).filter(|m| m.indent <= 3 || !lists.is_empty()) {
            flush(&mut pending, &mut lists, &mut builder)?;
            if marker.kind == ListType::Numbers && marker.number == 0 {
                builder.warn("Markdown ordered lists starting at zero were imported starting at one.");
            }
            let attrs = list_attrs(&marker, &mut lists)?;
            let prefix = line.get(..line.len() - marker.text.len()).ok_or_else(|| ImportError::Corrupt("Markdown list boundary".into()))?;
            pending = Some(Pending {
                text: marker.text.into(),
                prefix: prefix.into(),
                open_code: syntax.open_code.map(|(at, width)| (at.saturating_sub(prefix.len()), width)),
                style: list_style(marker.kind),
                attrs,
                list_column: Some(marker.column),
            });
            continue;
        }
        if let Some(p) = pending.as_mut() {
            p.continuation(line, syntax.open_code, &mut builder)?;
        } else {
            lists.clear();
            let text = line.trim_start_matches([' ', '\t']);
            let prefix = line.get(..line.len() - text.len()).ok_or_else(|| ImportError::Corrupt("Markdown paragraph boundary".into()))?;
            pending = Some(Pending {
                text: text.into(),
                prefix: prefix.into(),
                open_code: syntax.open_code.map(|(at, width)| (at.saturating_sub(prefix.len()), width)),
                style: body_style("Body Text"),
                attrs: ParaAttrs::default(),
                list_column: None,
            });
        }
    }
    flush(&mut pending, &mut lists, &mut builder)?;
    let references = std::mem::take(&mut builder.defined_references);
    let imported = builder.finish();
    imported.story.check().map_err(ImportError::Corrupt)?;
    Ok((imported, references))
}

enum Token {
    Text(String),
    Code(String),
    Delimiter(Delimiter),
}
struct Delimiter {
    marker: char,
    remaining: usize,
    original: usize,
    open: bool,
    close: bool,
    begin: Inline,
    end: Inline,
}
fn text_token(tokens: &mut Vec<Token>, text: &str) {
    if let Some(Token::Text(last)) = tokens.last_mut() {
        last.push_str(text);
    } else {
        tokens.push(Token::Text(text.into()));
    }
}
fn punctuation(c: char) -> bool {
    // Delimiter flanking uses Unicode punctuation/symbols, not combining marks.
    matches!(
        get_general_category(c),
        GeneralCategory::ClosePunctuation
            | GeneralCategory::ConnectorPunctuation
            | GeneralCategory::DashPunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::OpenPunctuation
            | GeneralCategory::OtherPunctuation
            | GeneralCategory::CurrencySymbol
            | GeneralCategory::MathSymbol
            | GeneralCategory::ModifierSymbol
            | GeneralCategory::OtherSymbol
    )
}
/// Backtick runs close only runs of the same length; code contents aren't parsed for emphasis.
fn code_end(source: &str, width: usize, builder: &mut Builder) -> Result<Option<usize>, ImportError> {
    let mut indices = source.char_indices().peekable();
    while let Some((at, c)) = indices.next() {
        builder.spend()?;
        if c == '`' {
            let mut count = 1;
            while indices.peek().is_some_and(|&(_, c)| c == '`') {
                indices.next();
                builder.spend()?;
                count += 1;
            }
            if count == width {
                return Ok(Some(at));
            }
        }
    }
    Ok(None)
}
#[derive(Default)]
struct InlineSyntax {
    unsupported: bool,
    /// The first row's initial label closes with ':' before any unmatched code opener.
    reference: bool,
    literal_lines: Vec<usize>,
    open_code: Option<(usize, usize)>,
    /// Widths with no closing run in this source; a later width may open multiline code.
    unmatched_code: Vec<usize>,
    /// Unescaped separators outside matched code spans.
    pipes: usize,
    first_pipe: Option<usize>,
    last_pipe: Option<usize>,
}

/// Inspect original source so a literal fallback retains code delimiters and escapes too.
fn inline_syntax(source: &str, initial_reference: bool, builder: &mut Builder) -> Result<InlineSyntax, ImportError> {
    let mut syntax = InlineSyntax::default();
    let mut brackets = 0usize;
    let mut bracket_starts = Vec::new();
    let mut at = 0;
    let mut line_start = 0;
    let mut line_content = false;
    let mut reference_candidate = None;
    let mut reference_line = false;
    while let Some(rest) = source.get(at..).filter(|s| !s.is_empty()) {
        builder.spend()?;
        let Some(c) = rest.chars().next() else { break };
        if c == '\\'
            && let Some(next) = rest.chars().nth(1).filter(char::is_ascii_punctuation)
        {
            at += 1 + next.len_utf8();
            line_content = true;
            continue;
        }
        if c == '`' {
            let width = rest.bytes().take_while(|&b| b == b'`').count();
            let content = rest.get(width..).ok_or_else(|| ImportError::Corrupt("Markdown code boundary".into()))?;
            // A literal reference cannot lend its backticks to a following row.
            let content = if reference_line { content.lines().next().unwrap_or("") } else { content };
            let end = at
                + match code_end(content, width, builder)? {
                    Some(end) => width + end + width,
                    None => {
                        syntax.open_code.get_or_insert((at, width));
                        syntax.unmatched_code.push(width);
                        width
                    }
                };
            if let Some(newline) = source.get(at..end).and_then(|s| s.rfind('\n')) {
                line_start = at + newline + 1;
                reference_candidate = None;
                reference_line = false;
            }
            at = end;
            line_content = true;
            continue;
        }
        match c {
            '[' => {
                brackets += 1;
                if !builder.known_references.is_empty() {
                    if bracket_starts.len() >= TOKEN_LIMIT {
                        return Err(limit("bracket nesting"));
                    }
                    bracket_starts.push(at);
                }
                if !line_content && (line_start > 0 || initial_reference) {
                    reference_candidate = Some(brackets);
                }
            }
            ']' => {
                // Only the matching close of the row's initial label can define a
                // reference. Code and escaped brackets do not close that label.
                let reference_close = reference_candidate == Some(brackets);
                if reference_close {
                    reference_candidate = None;
                }
                if reference_close && rest.starts_with("]:") {
                    if syntax.literal_lines.last().is_none_or(|&start| start != line_start) {
                        if syntax.literal_lines.len() >= BLOCK_LIMIT {
                            return Err(limit("literal rows"));
                        }
                        syntax.literal_lines.push(line_start);
                    }
                    // A preceding opener on this row may still close on a later row.
                    // Keep its signature deferred until the complete paragraph is scanned.
                    if syntax.open_code.is_none_or(|(open, _)| open < line_start) {
                        reference_line = true;
                        syntax.reference |= line_start == 0;
                    }
                }
                if brackets > 0 {
                    brackets -= 1;
                    let shortcut = match bracket_starts.pop().and_then(|start| source.get(start + 1..at)) {
                        Some(label) => reference::key(label, builder)?.is_some_and(|key| builder.known_references.contains(&key)),
                        None => false,
                    };
                    if shortcut || rest.get(1..).is_some_and(|s| s.starts_with(['(', '['])) {
                        syntax.unsupported = true;
                    }
                }
            }
            '<' if !syntax.unsupported => syntax.unsupported = inline_html(rest, builder)?,
            '|' => {
                syntax.pipes += 1;
                syntax.first_pipe.get_or_insert(at);
                syntax.last_pipe = Some(at);
                if (!line_content || (source.get(..at).is_some_and(|s| s.ends_with(' ')) && rest.get(1..).is_some_and(|s| s.starts_with(' '))))
                    && syntax.literal_lines.last().is_none_or(|&start| start != line_start)
                {
                    if syntax.literal_lines.len() >= BLOCK_LIMIT {
                        return Err(limit("literal rows"));
                    }
                    syntax.literal_lines.push(line_start);
                }
            }
            _ => {}
        }
        at += c.len_utf8();
        if c == '\n' {
            if reference_line {
                // Accepted continuation rows are literal too: their backticks cannot
                // hide a later definition. Earlier matched code already skipped this row.
                let line = source.get(line_start..at.saturating_sub(1)).ok_or_else(|| ImportError::Corrupt("Markdown reference boundary".into()))?;
                let next = source.get(at..).ok_or_else(|| ImportError::Corrupt("Markdown reference boundary".into()))?;
                let continued = reference::continued_rows(line, next.lines(), builder)?;
                for row in next.split_inclusive('\n').take(continued) {
                    at += row.len();
                }
            }
            line_start = at;
            line_content = false;
            reference_candidate = None;
            reference_line = false;
        } else if !c.is_whitespace() {
            line_content = true;
        }
    }
    Ok(syntax)
}

fn tokenize(source: &str, builder: &mut Builder) -> Result<Vec<Token>, ImportError> {
    let mut tokens = Vec::new();
    let mut at = 0;
    while let Some(rest) = source.get(at..).filter(|s| !s.is_empty()) {
        builder.spend()?;
        if tokens.len() >= TOKEN_LIMIT {
            return Err(limit("inline token count"));
        }
        let Some(c) = rest.chars().next() else { break };
        if c == '\n' || rest.starts_with("\\\n") {
            let backslash = c == '\\';
            let previous = source.get(..at).ok_or_else(|| ImportError::Corrupt("Markdown line boundary".into()))?;
            let spaces = previous.len().saturating_sub(previous.trim_end_matches(' ').len());
            let hard = backslash || spaces >= 2;
            if !backslash && let Some(Token::Text(text)) = tokens.last_mut() {
                let end = if hard { text.len().saturating_sub(spaces) } else { text.trim_end_matches([' ', '\t']).len() };
                text.truncate(end);
            }
            text_token(&mut tokens, if hard { "\u{2028}" } else { " " });
            at += if backslash { 2 } else { 1 };
            let tail = source.get(at..).ok_or_else(|| ImportError::Corrupt("Markdown continuation boundary".into()))?;
            let trimmed = tail.trim_start_matches([' ', '\t']);
            at += tail.len() - trimmed.len();
            continue;
        }
        if c == '\\'
            && let Some(next) = rest.chars().nth(1).filter(char::is_ascii_punctuation)
        {
            text_token(&mut tokens, &next.to_string());
            at += 1 + next.len_utf8();
            continue;
        }
        if c == '`' {
            let width = rest.bytes().take_while(|&b| b == b'`').count();
            let content = rest.get(width..).ok_or_else(|| ImportError::Corrupt("Markdown code boundary".into()))?;
            if let Some(end) = code_end(content, width, builder)? {
                let mut code =
                    content.get(..end).ok_or_else(|| ImportError::Corrupt("Markdown code boundary".into()))?.replace(['\n', FORCED_LINE_BREAK], " ");
                if code.starts_with(' ') && code.ends_with(' ') && code.chars().any(|c| c != ' ') {
                    code = code.get(1..code.len().saturating_sub(1)).ok_or_else(|| ImportError::Corrupt("Markdown code padding".into()))?.into();
                }
                tokens.push(Token::Code(code));
                at += width + end + width;
            } else {
                text_token(&mut tokens, rest.get(..width).ok_or_else(|| ImportError::Corrupt("Markdown code delimiter".into()))?);
                at += width;
            }
            continue;
        }
        if matches!(c, '*' | '_' | '~') {
            let count = rest.chars().take_while(|&x| x == c).count();
            if c != '~' || count == 2 {
                let previous = source.get(..at).and_then(|s| s.chars().next_back());
                let next = rest.get(count..).and_then(|s| s.chars().next());
                let prev_space = previous.is_none_or(char::is_whitespace);
                let next_space = next.is_none_or(char::is_whitespace);
                let prev_punct = previous.is_some_and(punctuation);
                let next_punct = next.is_some_and(punctuation);
                let left = !next_space && (!next_punct || prev_space || prev_punct);
                let right = !prev_space && (!prev_punct || next_space || next_punct);
                tokens.push(Token::Delimiter(Delimiter {
                    marker: c,
                    remaining: count,
                    original: count,
                    open: left && (c != '_' || !right || prev_punct),
                    close: right && (c != '_' || !left || next_punct),
                    begin: Inline::default(),
                    end: Inline::default(),
                }));
            } else {
                text_token(&mut tokens, rest.get(..count).ok_or_else(|| ImportError::Corrupt("Markdown delimiter".into()))?);
            }
            at += count;
            continue;
        }
        let end = rest.char_indices().skip(1).find(|&(_, c)| matches!(c, '\n' | '\\' | '`' | '*' | '_' | '~')).map_or(rest.len(), |(i, _)| i);
        let text = rest.get(..end).ok_or_else(|| ImportError::Corrupt("Markdown text boundary".into()))?;
        text_token(&mut tokens, text);
        at += end;
    }
    Ok(tokens)
}
/// Pair before emitting so unmatched markers survive. The stack prevents crossing pairs and
/// bounds nesting without recursive input traversal.
fn pair(tokens: &mut [Token], builder: &mut Builder) -> Result<(), ImportError> {
    let mut openers: Vec<usize> = Vec::new();
    for i in 0..tokens.len() {
        while let Some(Token::Delimiter(close)) = tokens.get(i) {
            if !close.close || close.remaining == 0 {
                break;
            }
            let mut found = None;
            for (k, &j) in openers.iter().enumerate().rev() {
                builder.spend()?;
                let Some(Token::Delimiter(open)) = tokens.get(j) else { continue };
                let rule_of_three = close.marker != '~'
                    && (open.close || close.open)
                    && (open.original + close.original).is_multiple_of(3)
                    && (!open.original.is_multiple_of(3) || !close.original.is_multiple_of(3));
                if open.marker == close.marker && open.remaining > 0 && !rule_of_three {
                    found = Some((k, j, open.remaining.min(close.remaining).min(2), close.marker));
                    break;
                }
            }
            let Some((k, j, width, marker)) = found else { break };
            let delta = match (marker, width) {
                ('~', _) => Inline { strike: 1, ..Default::default() },
                (_, 2) => Inline { bold: 1, ..Default::default() },
                _ => Inline { italic: 1, ..Default::default() },
            };
            if let Some(Token::Delimiter(open)) = tokens.get_mut(j) {
                open.remaining -= width;
                open.begin.add(delta);
            }
            if let Some(Token::Delimiter(close)) = tokens.get_mut(i) {
                close.remaining -= width;
                close.end.add(delta);
            }
            openers.truncate(k + 1);
            if matches!(tokens.get(j), Some(Token::Delimiter(d)) if d.remaining == 0) {
                openers.pop();
            }
        }
        if matches!(tokens.get(i), Some(Token::Delimiter(d)) if d.open && d.remaining > 0) {
            if openers.len() >= DEPTH_LIMIT {
                return Err(limit("inline nesting"));
            }
            openers.push(i);
        }
    }
    Ok(())
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
        for source in [
            "[x]: **y**",
            "  [x]: `y`",
            "[x]: `a | b`",
            "[x]: `a\n| **b** | c`",
            "``unclosed\n[x]: `a\n| **b** | c`",
            "`unclosed\n[x]: **y**",
            "``unclosed\n[x]: y`",
            "`unclosed\n  [x]: **y**",
            "`a\n[x]: **y**\n---",
        ] {
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
        let mixed = read("`a\n[x]: y`\n[real]: **destination**\n# Next");
        assert_eq!(mixed.story.text, "a [x]: y\n[real]: **destination**\nNext");
        assert_eq!(mixed.story.paras.iter().map(|p| p.style.as_str()).collect::<Vec<_>>(), ["Body Text", "Body Text", "Heading 1"]);
        assert_eq!(mixed.story.format_after(0).style, "Code");
        assert_eq!(mixed.story.format_after(mixed.story.text.find("[real]").unwrap()).style, designcraft_doc::NO_CHAR_STYLE);
        assert_eq!(mixed.warnings.len(), 1);
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
        for source in [
            "[x]: **destination**",
            "  [x]: `a]: b`",
            r"[x\] label]: **destination**",
            "[x `]:` label]: **destination**",
            "[x [y]: label]: **destination**",
            r"[x \`a]: b\` **bold**",
            "[x ``a]: b` **bold**",
            "[x `a]: b`` **bold**",
            "[x]: `a]:\n| **table** | b`",
        ] {
            let imported = read(source);
            assert_eq!(imported.story.text, source, "{source:?}");
            assert_eq!(imported.story.paras.len(), source.lines().count());
            assert!(imported.story.paras.iter().all(|p| p.style == "Body Text"));
            assert!(imported.char_styles.is_empty());
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
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
            "`a|b`|**B**\n---|---\n*C*|*D*",
            "``a|`b``|**B**\n---|---\n*C*|*D*",
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
            ("`**A**|*B*`\n---|---", "**A**|*B* ---|---", None, None, Some("**A**|*B*")),
            ("**A**|`B|C`\n---|---|---", "A|B|C ---|---|---", Some("A"), None, Some("B|C")),
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
        for (suffix, expected_suffix, para_style, word, char_style) in [
            ("\nafter *em*", "after em", "Body Text", "em", "Emphasis"),
            ("# After", "After", "Heading 1", "After", designcraft_doc::NO_CHAR_STYLE),
            ("- *after*", "after", "Bullet List", "after", "Emphasis"),
            ("\n**after**|*em*", "after|em", "Body Text", "after", "Strong"),
            ("```\n*x*|y\n```", "*x*|y", "Code Block", "*x*", designcraft_doc::NO_CHAR_STYLE),
            ("    *x*|y", "*x*|y", "Code Block", "*x*", designcraft_doc::NO_CHAR_STYLE),
            ("\nafter `x|y`", "after x|y", "Body Text", "x|y", "Code"),
        ] {
            let source = format!("**before**\n{rows}\n{suffix}");
            let expected = format!("before\n{rows}\n{expected_suffix}");
            let imported = read(&source);
            assert_eq!(imported.story.text, expected, "{suffix:?}");
            assert_eq!(imported.story.paras.len(), 5, "{suffix:?}");
            assert!(imported.story.paras[..4].iter().all(|p| p.style == "Body Text"));
            assert_eq!(imported.story.paras[4].style, para_style, "{suffix:?}");
            assert_eq!(imported.story.format_after(0).style, "Strong");
            let start = "before\n".len();
            for (at, _) in rows.char_indices() {
                assert_eq!(imported.story.format_after(start + at), &CharFormat::default(), "{suffix:?} at {at}");
            }
            assert_eq!(imported.story.format_after(expected.rfind(word).unwrap()).style, char_style, "{suffix:?}");
            if para_style == "Code Block" {
                assert_eq!(styles(&imported).resolve_para(&imported.story.paras[4]).1.font_family, MONO);
            }
            assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
        }
        let source = format!("{rows}\n\n**E**|**F**\n---|---\n*G*|*H*");
        let imported = read(&source);
        assert_eq!(imported.story.text, source.replace("\n\n", "\n"));
        assert_eq!(imported.story.paras.len(), 6);
        assert!(imported.char_styles.is_empty());
        assert_eq!(imported.warnings, ["Markdown quotes, tables, HTML, references and thematic breaks were kept as source text."]);
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
            ("    *outside*", "*outside*", "Code Block", "*outside*", designcraft_doc::NO_CHAR_STYLE),
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
            ("    *outside*", "*outside*", "Code Block", "*outside*", designcraft_doc::NO_CHAR_STYLE),
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
        let imported = read("- First\n  continues\n  - Child\n- Second\n\n7. Seven\n9. Eight\n   2) Sub\n   2) Next\n9. Nine\n\nPlain\n\n3. Restart");
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
            known_references: std::collections::BTreeSet::new(),
            defined_references: std::collections::BTreeSet::new(),
        };
        let mut regular_heading = heading_style(1);
        regular_heading.chars.font_style = Some("Regular".into());
        builder.block("*Regular*", regular_heading, ParaAttrs::default(), false).unwrap();
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
        assert!(matches!(import("*a ".repeat(DEPTH_LIMIT + 1).as_bytes()), Err(ImportError::Unsupported(_))));
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
            ("7. **after** ``tail", "after ``tail", "Numbered List", "after", "Strong"),
            ("~~~\n*after* ``tail\n~~~", "*after* ``tail", "Code Block", "*after*", designcraft_doc::NO_CHAR_STYLE),
            ("    *after* `tail`", "*after* `tail`", "Code Block", "*after*", designcraft_doc::NO_CHAR_STYLE),
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
            ("**A**|`B|C`\n---|---|---\n*D*|*E*", "A|B|C ---|---|--- D|E"),
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

//! Map maintained block ranges to the importer's named styles and visible-source policy.
use super::{
    Builder, ImportError, Imported, ImportedStyle, SOURCE_LIMIT,
    blocks::{self, Kind, Node, Part, Plan},
    body_style, code_style, heading_style, limit, list_style, pipes, reference, tables,
};
use designcraft_doc::{FORCED_LINE_BREAK, ListType, ParaAttrs};
use pulldown_cmark::{Event, Options, Parser};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ops::Range;

const BLOCK_WARNING: &str = "Markdown quotes, tables, HTML, references and thematic breaks were kept as source text.";
const LIST_WARNING: &str = "Multiple blocks inside Markdown list items were imported as separate paragraphs.";
const ZERO_WARNING: &str = "Markdown ordered lists starting at zero were imported starting at one.";

fn boundary() -> ImportError {
    ImportError::Corrupt("Markdown block source boundary".into())
}

fn source_at(source: &str, range: Range<usize>) -> Result<&str, ImportError> {
    source.get(range).ok_or_else(boundary)
}

fn blank(source: &str) -> bool {
    source.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n'))
}

fn line_start(source: &str, at: usize) -> Result<usize, ImportError> {
    Ok(source_at(source, 0..at)?.rfind('\n').map_or(0, |at| at + 1))
}

fn trim_last_lf(source: &str) -> &str {
    source.strip_suffix('\n').unwrap_or(source)
}

fn has_blank_row(source: &str) -> bool {
    source.split('\n').any(|row| row.bytes().all(|b| matches!(b, b' ' | b'\t')))
}

// Keep the importer's escaped-backslash table boundary while leaving ordinary
// inline/code parsing to the unchanged baseline. Only table discovery sees the
// same-length neutral byte; every accepted table still emits original source.
fn table_escape_view<'a>(source: &'a str, builder: &mut Builder) -> Result<Cow<'a, str>, ImportError> {
    reference::reserve_parser_work(builder, source.len())?;
    if !source.as_bytes().windows(3).any(|bytes| bytes == b"\\\\|") {
        return Ok(Cow::Borrowed(source));
    }
    reference::reserve_parser_work(builder, source.len().checked_mul(2).ok_or_else(|| limit("delimiter processing"))?)?;
    let mut view = source.as_bytes().to_vec();
    let mut slashes = 0usize;
    for at in 0..view.len() {
        match view[at] {
            b'\\' => slashes += 1,
            b'|' => {
                if slashes > 0 && slashes.is_multiple_of(2) {
                    view[at - 1] = b'x';
                }
                slashes = 0;
            }
            _ => slashes = 0,
        }
    }
    String::from_utf8(view).map(Cow::Owned).map_err(|_| boundary())
}

// Keep the pinned block parser's ASCII suffix handling separate from source bytes.
// Indentation and entirely whitespace rows retain their original tab columns.
fn block_parser_view(source: &str, builder: &mut Builder) -> Result<String, ImportError> {
    let work = source.len().checked_mul(5).ok_or_else(|| limit("delimiter processing"))?;
    reference::reserve_parser_work(builder, work)?;
    let mut view = source.as_bytes().to_vec();
    let mut start = 0usize;
    for row in source.split_inclusive('\n') {
        let next = start.checked_add(row.len()).ok_or_else(boundary)?;
        let end = next.checked_sub(usize::from(row.ends_with('\n'))).ok_or_else(boundary)?;
        let mut suffix = end;
        while suffix > start && view.get(suffix - 1).is_some_and(|byte| matches!(byte, b' ' | b'\t')) {
            suffix -= 1;
        }
        if suffix > start {
            for byte in view.get_mut(suffix..end).ok_or_else(boundary)? {
                if *byte == b'\t' {
                    *byte = b' ';
                }
            }
        }
        // Keep indentation columns intact while admitting ASCII tab spacing
        // in an otherwise punctuation-only, unprefixed table delimiter row.
        let mut content = start;
        while content < end && matches!(view[content], b' ' | b'\t') {
            content += 1;
        }
        let delimiter = view.get(content..end).ok_or_else(boundary)?;
        let mut has_tabs = false;
        let mut has_pipe = false;
        let mut punctuation_only = true;
        for byte in delimiter {
            builder.spend()?;
            match byte {
                b'\t' => has_tabs = true,
                b'|' => has_pipe = true,
                b' ' | b':' | b'-' => {}
                _ => {
                    punctuation_only = false;
                    break;
                }
            }
        }
        if punctuation_only && has_tabs && has_pipe {
            reference::reserve_parser_work(builder, delimiter.len())?;
            for byte in view.get_mut(content..end).ok_or_else(boundary)? {
                if *byte == b'\t' {
                    *byte = b' ';
                }
            }
        }
        start = next;
    }
    // The maintained parser recognizes these openers case-insensitively, but its
    // closing-tag search is byte-sensitive. Canonicalize only those fixed tokens;
    // literal and code output still uses the original, same-length source.
    let closing_tags: [&[u8]; 4] = [b"</pre>", b"</style>", b"</script>", b"</textarea>"];
    let mut at = 0usize;
    while at < view.len() {
        if view[at] == b'<' {
            for closing in closing_tags {
                builder.spend()?;
                let end = at.checked_add(closing.len()).ok_or_else(boundary)?;
                if let Some(candidate) = view.get_mut(at..end)
                    && candidate.eq_ignore_ascii_case(closing)
                {
                    candidate.copy_from_slice(closing);
                    at = end - 1;
                    break;
                }
            }
        }
        at += 1;
    }
    String::from_utf8(view).map_err(|_| boundary())
}

struct Render<'source, 'allowed, 'definition, 'builder> {
    source: &'source str,
    parser_source: &'source str,
    allowed: &'allowed Parser<'definition>,
    builder: &'builder mut Builder,
    literal_rows: BTreeSet<usize>,
}

impl Render<'_, '_, '_, '_> {
    fn block(&mut self, text: &str, style: ImportedStyle, attrs: ParaAttrs, literal: bool) -> Result<(), ImportError> {
        self.builder.block_with_references(text, style, attrs, literal, Some(self.allowed))
    }

    fn literal(&mut self, range: Range<usize>, keep_blank: bool) -> Result<(), ImportError> {
        let physical = line_start(self.source, range.start)?;
        let prefix = source_at(self.source, physical..range.start)?;
        let start = if prefix.bytes().all(|b| matches!(b, b' ' | b'\t')) { physical } else { range.start };
        let raw = source_at(self.source, start..range.end)?;
        self.builder.warn(BLOCK_WARNING);
        let mut at = start;
        for row in raw.split_inclusive('\n') {
            self.builder.spend()?;
            let row = trim_last_lf(row);
            if (keep_blank || !blank(row)) && self.literal_rows.insert(at) {
                self.block(row, body_style("Body Text"), ParaAttrs::default(), true)?;
            }
            at = at
                .checked_add(row.len())
                .and_then(|n| n.checked_add(usize::from(self.source.as_bytes().get(n) == Some(&b'\n'))))
                .ok_or_else(boundary)?;
        }
        Ok(())
    }

    fn gap(&mut self, range: Range<usize>) -> Result<(), ImportError> {
        if !blank(source_at(self.source, range.clone())?) {
            self.literal(range, false)?;
        }
        Ok(())
    }

    fn content(
        &mut self,
        parts: &[Part<'_>],
        range: Range<usize>,
        style: ImportedStyle,
        attrs: ParaAttrs,
        heading_source: Option<Range<usize>>,
    ) -> Result<bool, ImportError> {
        source_at(self.source, range.clone())?;
        let rows = pipes::literal_pipe_rows(
            self.source,
            parts.iter().filter_map(|part| match part {
                Part::Inline(event, span) => Some((event, span)),
                Part::Block(_) => None,
            }),
            self.builder,
        )?;
        if !rows.is_empty() {
            let text = source_at(self.source, range.clone())?.trim_end_matches([' ', '\t']);
            if super::commonmark::requires_literal(text, Some(self.allowed), self.builder)? {
                self.builder.warn(super::commonmark::LITERAL_WARNING);
                self.block(&text.replace('\n', "\u{2028}"), style, attrs, true)?;
                return Ok(false);
            }
        }
        // Pipe rows divide heading source into prose and a possible final heading.
        let heading_format = heading_source.as_ref().map(|_| (style.clone(), attrs.clone()));
        let mut last_literal_end = range.start;
        let mut cursor = range.start;
        let mut style = style;
        let mut attrs = attrs;
        let mut literal = false;
        for row in rows {
            if row.end <= range.start || row.start >= range.end {
                continue;
            }
            if row.start > cursor {
                let text = trim_last_lf(source_at(self.source, cursor..row.start)?);
                if !text.is_empty() {
                    let (prefix_style, prefix_attrs) =
                        if heading_format.is_some() { (body_style("Body Text"), ParaAttrs::default()) } else { (style.clone(), attrs.clone()) };
                    self.block(text, prefix_style, prefix_attrs, false)?;
                }
            }
            self.literal(row.clone(), true)?;
            last_literal_end = row.end;
            cursor = row.end.min(range.end);
            if self.source.as_bytes().get(cursor) == Some(&b'\n') {
                cursor += 1;
            }
            cursor = cursor.min(range.end);
            style = body_style("Body Text");
            attrs = ParaAttrs::default();
            literal = true;
        }
        if cursor < range.end || !literal {
            let text = trim_last_lf(source_at(self.source, cursor..range.end)?);
            let (style, attrs) = heading_format.unwrap_or((style, attrs));
            self.block(text, style, attrs, false)?;
        } else if let Some(heading_source) = heading_source {
            // With no trailing heading text, its setext underline must remain visible.
            self.literal(last_literal_end.min(heading_source.end)..heading_source.end, false)?;
        }
        Ok(literal)
    }

    fn code(&mut self, node: &Node<'_>) -> Result<(), ImportError> {
        let mut code = String::new();
        for part in &node.parts {
            let Part::Inline(Event::Text(text), range) = part else {
                return Err(boundary());
            };
            self.builder.spend()?;
            let raw = source_at(self.source, range.clone())?;
            let projected = source_at(self.parser_source, range.clone())?;
            // Recover original bytes only for an exact projected event span; synthesized
            // indentation keeps the parser's payload. Preserve the existing NUL handling.
            if projected == text.as_ref() || (raw.contains('\0') && projected.replace('\0', "\u{fffd}") == text.as_ref()) {
                code.push_str(raw);
            } else {
                code.push_str(text);
            }
        }
        let code = trim_last_lf(&code).replace('\n', &FORCED_LINE_BREAK.to_string());
        self.block(&code, code_style(), ParaAttrs::default(), true)
    }

    fn node(&mut self, plan: &Plan<'_>, index: usize, list_depth: usize) -> Result<(), ImportError> {
        let node = plan.nodes.get(index).ok_or_else(boundary)?;
        match node.kind {
            Kind::Paragraph => {
                let range = blocks::content_range(node, self.source)?.ok_or_else(boundary)?;
                self.content(&node.parts, range, body_style("Body Text"), ParaAttrs::default(), None)?;
            }
            Kind::Heading(level) => {
                let range = blocks::content_range(node, self.source)?;
                if let Some(range) = range {
                    self.content(&node.parts, range, heading_style(level), ParaAttrs::default(), Some(node.range.clone()))?;
                } else {
                    self.block("", heading_style(level), ParaAttrs::default(), false)?;
                }
            }
            Kind::Code => self.code(node)?,
            Kind::List(start) => self.list(plan, node, start, list_depth + 1)?,
            Kind::Quote | Kind::Html | Kind::Table | Kind::Rule => self.literal(node.range.clone(), node.kind == Kind::Html)?,
            Kind::Item => return Err(boundary()),
        }
        Ok(())
    }

    fn list(&mut self, plan: &Plan<'_>, node: &Node<'_>, start: Option<u64>, depth: usize) -> Result<(), ImportError> {
        if depth > super::DEPTH_LIMIT {
            return Err(limit("list nesting"));
        }
        let kind = if start.is_some() { ListType::Numbers } else { ListType::Bullets };
        let mut next = start.map(|value| value.max(1));
        for part in &node.parts {
            let Part::Block(index) = part else { return Err(boundary()) };
            let item = plan.nodes.get(*index).filter(|node| node.kind == Kind::Item).ok_or_else(boundary)?;
            let metadata = blocks::item_metadata(item, self.source, start.is_some())?;
            let Some(metadata) = metadata else {
                self.builder.warn(LIST_WARNING);
                self.literal(item.range.clone(), true)?;
                next = None;
                continue;
            };
            if metadata.number == Some(0) {
                self.builder.warn(ZERO_WARNING);
            }
            let mut attrs = ParaAttrs::default();
            if depth > 1 {
                attrs.left_indent = Some(18.0 * depth as f64);
            }
            if start.is_some() {
                let value = next.or(metadata.number).unwrap_or(1).max(1);
                attrs.start_at = Some(Some(u32::try_from(value).map_err(|_| limit("list numbering"))?));
                next = Some(value.checked_add(1).ok_or_else(|| limit("list numbering"))?);
            }
            if self.item(plan, item, metadata.content_start, list_style(kind), attrs, depth)? {
                next = None;
            }
        }
        Ok(())
    }

    fn item(
        &mut self,
        plan: &Plan<'_>,
        item: &Node<'_>,
        content_start: usize,
        style: ImportedStyle,
        attrs: ParaAttrs,
        depth: usize,
    ) -> Result<bool, ImportError> {
        // A marker-only opening row is not an empty logical content row.
        let mut content_start = content_start;
        if content_start < item.range.end && self.source.as_bytes().get(content_start) == Some(&b'\n') {
            content_start += 1;
            let tail = source_at(self.source, content_start..item.range.end)?;
            content_start += tail.len() - tail.trim_start_matches([' ', '\t']).len();
        }
        enum Segment {
            Inline { parts: Range<usize>, source: Range<usize> },
            Block(usize),
        }
        let mut segments = Vec::new();
        let mut at = 0usize;
        while let Some(part) = item.parts.get(at) {
            match part {
                Part::Block(index) => {
                    segments.push(Segment::Block(*index));
                    at += 1;
                }
                Part::Inline(_, _) => {
                    let start = at;
                    while matches!(item.parts.get(at), Some(Part::Inline(_, _))) {
                        at += 1;
                    }
                    let parts = item.parts.get(start..at).ok_or_else(boundary)?;
                    if let Some(source) = blocks::inline_range(parts, self.source, &item.range)? {
                        segments.push(Segment::Inline { parts: start..at, source });
                    }
                }
            }
        }
        let segment_range = |segment: &Segment| -> Result<Range<usize>, ImportError> {
            match segment {
                Segment::Inline { source, .. } => Ok(source.clone()),
                Segment::Block(index) => Ok(plan.nodes.get(*index).ok_or_else(boundary)?.range.clone()),
            }
        };
        let first_start = segments.first().map(segment_range).transpose()?.map_or(item.range.end, |range| range.start);
        let hidden_prefix = source_at(self.source, content_start..first_start)?;
        let prefix_boundary = if segments.is_empty() {
            hidden_prefix.trim_end_matches([' ', '\t', '\n'])
        } else {
            trim_last_lf(hidden_prefix.trim_end_matches([' ', '\t']))
        };
        let consumes_prefix = match segments.first() {
            None | Some(Segment::Inline { .. }) => true,
            Some(Segment::Block(index)) => matches!(plan.nodes.get(*index).map(|node| node.kind), Some(Kind::Paragraph | Kind::Heading(_))),
        };
        let mut ambiguous = !blank(hidden_prefix) && (has_blank_row(prefix_boundary) || !consumes_prefix);
        let mut previous_end = first_start;
        for (index, segment) in segments.iter().enumerate() {
            let range = segment_range(segment)?;
            if index > 0 {
                ambiguous |= !blank(source_at(self.source, previous_end..range.start)?);
            }
            previous_end = range.end;
        }
        if !segments.is_empty() && previous_end < item.range.end {
            ambiguous |= !blank(source_at(self.source, previous_end..item.range.end)?);
        }
        // A swallowed definition between real blocks has no public node span.
        // Keep its entire item visible rather than silently discard that source.
        if ambiguous {
            self.builder.warn(LIST_WARNING);
            self.literal(item.range.clone(), true)?;
            return Ok(true);
        }
        if segments.is_empty() {
            let text = source_at(self.source, content_start..item.range.end)?.trim_end_matches([' ', '\t', '\n']);
            self.block(text, style, attrs, false)?;
            return Ok(false);
        }

        let mut emitted = false;
        let mut reset = false;
        for segment in segments {
            match segment {
                Segment::Inline { parts, mut source } => {
                    let parts = item.parts.get(parts).ok_or_else(boundary)?;
                    if !emitted {
                        source.start = content_start;
                        reset |= self.content(parts, source, style.clone(), attrs.clone(), None)?;
                    } else {
                        self.builder.warn(LIST_WARNING);
                        self.content(parts, source, body_style("Body Text"), ParaAttrs::default(), None)?;
                        reset = true;
                    }
                    emitted = true;
                }
                Segment::Block(index) => {
                    let child = plan.nodes.get(index).ok_or_else(boundary)?;
                    match child.kind {
                        Kind::Paragraph if !emitted => {
                            let mut range = blocks::content_range(child, self.source)?.ok_or_else(boundary)?;
                            range.start = content_start;
                            reset |= self.content(&child.parts, range, style.clone(), attrs.clone(), None)?;
                            emitted = true;
                        }
                        Kind::Heading(_) if !emitted => {
                            // The first marker selects native list formatting; setext-looking
                            // continuation source remains part of that same logical item.
                            let raw = source_at(self.source, content_start..child.range.end)?;
                            let end = content_start.checked_add(trim_last_lf(raw).len()).ok_or_else(boundary)?;
                            reset |= self.content(&child.parts, content_start..end, style.clone(), attrs.clone(), None)?;
                            emitted = true;
                        }
                        Kind::List(_) => {
                            if !emitted {
                                self.block("", style.clone(), attrs.clone(), false)?;
                                emitted = true;
                            }
                            self.node(plan, index, depth)?;
                        }
                        Kind::Paragraph => {
                            self.builder.warn(LIST_WARNING);
                            self.node(plan, index, depth)?;
                            reset = true;
                        }
                        _ => {
                            self.node(plan, index, depth)?;
                            emitted = true;
                            reset = true;
                        }
                    }
                }
            }
        }
        Ok(reset)
    }

    fn fragment(&mut self, range: Range<usize>) -> Result<(), ImportError> {
        let raw = source_at(self.source, range.clone())?;
        if blank(raw) {
            return Ok(());
        }
        // The fragment uses matching ranges in the original and existing baseline
        // views; do not rebuild or substitute the TABLES-only discovery view.
        let parser_source = source_at(self.parser_source, range)?;
        let plan = Plan::parse(parser_source, Options::ENABLE_STRIKETHROUGH, self.builder)?;
        let mut nested = Render { source: raw, parser_source, allowed: self.allowed, builder: self.builder, literal_rows: BTreeSet::new() };
        nested.roots(&plan, &[])
    }

    fn roots(&mut self, plan: &Plan<'_>, literal_ranges: &[Range<usize>]) -> Result<(), ImportError> {
        let mut cursor = 0usize;
        let mut literal_index = 0usize;
        for index in &plan.roots {
            let node = plan.nodes.get(*index).ok_or_else(boundary)?;
            if node.range.end <= cursor {
                continue;
            }
            self.gap(cursor..node.range.start.max(cursor))?;
            let mut at = cursor.max(node.range.start);
            let mut split = at != node.range.start;
            while let Some(literal) = literal_ranges.get(literal_index) {
                if literal.start >= node.range.end {
                    break;
                }
                if literal.end <= at {
                    literal_index += 1;
                    continue;
                }
                if at < literal.start {
                    self.fragment(at..literal.start)?;
                }
                self.literal(literal.clone(), false)?;
                at = literal.end;
                literal_index += 1;
                split = true;
            }
            if split {
                if at < node.range.end {
                    self.fragment(at..node.range.end)?;
                    at = node.range.end;
                }
                cursor = at;
            } else {
                self.node(plan, *index, 0)?;
                cursor = node.range.end;
            }
        }
        self.gap(cursor..self.source.len())?;
        Ok(())
    }
}

pub(super) fn parse(text: &str) -> Result<Imported, ImportError> {
    if text.len() > SOURCE_LIMIT {
        return Err(limit("file size"));
    }
    let mut builder = Builder::new(text.len());
    let parser_source = block_parser_view(text, &mut builder)?;
    let baseline = Plan::parse(&parser_source, Options::ENABLE_STRIKETHROUGH, &mut builder)?;
    let table_source = table_escape_view(&parser_source, &mut builder)?;
    let table_plan = Plan::parse(&table_source, Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES, &mut builder)?;
    let (literal_ranges, released_quote_tails) = tables::literal_ranges(text, &baseline, &table_plan, &mut builder)?;
    let mut roots = Vec::new();
    for index in &baseline.roots {
        roots.push(baseline.nodes.get(*index).ok_or_else(boundary)?.range.clone());
    }
    let gaps = reference::root_gaps(text, roots)?;
    let mut gaps = tables::subtract_literal_ranges(text, &gaps, &literal_ranges, &mut builder)?;
    if !released_quote_tails.is_empty() {
        let mut roots = Vec::new();
        for index in &table_plan.roots {
            builder.spend()?;
            roots.push(table_plan.nodes.get(*index).ok_or_else(boundary)?.range.clone());
        }
        reference::reserve_parser_work(&mut builder, text.len())?;
        let maintained_gaps = reference::root_gaps(text, roots)?;
        gaps = reference::include_released_quote_gaps(text, gaps, &maintained_gaps, &released_quote_tails, &mut builder)?;
    }
    let reference_source = reference::reference_source(text, &gaps, &mut builder)?;
    let allowed = reference::allowed_parser(&reference_source, &mut builder)?;
    Render { source: text, parser_source: &parser_source, allowed: &allowed, builder: &mut builder, literal_rows: BTreeSet::new() }
        .roots(&baseline, &literal_ranges)?;
    let imported = builder.finish();
    imported.story.check().map_err(ImportError::Corrupt)?;
    Ok(imported)
}

//! Maintained block events and original byte ranges, without a second block grammar.
use super::{Builder, DEPTH_LIMIT, ImportError, SOURCE_LIMIT, TOKEN_LIMIT, limit};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::ops::Range;

pub(super) struct Plan<'a> {
    pub nodes: Vec<Node<'a>>,
    pub roots: Vec<usize>,
    pub inline_code: Vec<Range<usize>>,
}
pub(super) struct Node<'a> {
    pub kind: Kind,
    pub range: Range<usize>,
    pub parts: Vec<Part<'a>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Paragraph,
    Heading(usize),
    Code,
    List(Option<u64>),
    Item,
    Quote,
    Html,
    Table,
    Rule,
}
pub(super) enum Part<'a> {
    Block(usize),
    Inline(Event<'a>, Range<usize>),
}
pub(super) struct ItemMetadata {
    pub content_start: usize,
    pub number: Option<u64>,
}
enum TagClass {
    Block(Kind),
    TablePart,
    Inline,
}
struct Frame {
    end: TagEnd,
    range: Range<usize>,
    block: bool,
    list: bool,
    table: bool,
    opaque: bool,
    previous_owner: Option<usize>,
    inline_owner: Option<usize>,
}
fn corrupt(message: &str) -> ImportError {
    ImportError::Corrupt(message.into())
}
fn checked_source<'s>(source: &'s str, range: &Range<usize>) -> Result<&'s str, ImportError> {
    source.get(range.clone()).ok_or_else(|| corrupt("Markdown source boundary"))
}
fn decrement(value: usize) -> Result<usize, ImportError> {
    value.checked_sub(1).ok_or_else(|| corrupt("Markdown block stack"))
}
fn heading_number(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}
fn classify_tag(tag: &Tag<'_>) -> Result<TagClass, ImportError> {
    Ok(match tag {
        Tag::Paragraph => TagClass::Block(Kind::Paragraph),
        Tag::Heading { level, .. } => TagClass::Block(Kind::Heading(heading_number(*level))),
        Tag::CodeBlock(_) => TagClass::Block(Kind::Code),
        Tag::List(start) => TagClass::Block(Kind::List(*start)),
        Tag::Item => TagClass::Block(Kind::Item),
        Tag::BlockQuote(_) => TagClass::Block(Kind::Quote),
        Tag::HtmlBlock => TagClass::Block(Kind::Html),
        Tag::Table(_) => TagClass::Block(Kind::Table),
        Tag::TableHead | Tag::TableRow | Tag::TableCell => TagClass::TablePart,
        Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. } => TagClass::Inline,
        _ => return Err(corrupt("Unexpected Markdown extension tag")),
    })
}
impl<'a> Plan<'a> {
    pub(super) fn parse(source: &'a str, options: Options, builder: &mut Builder) -> Result<Self, ImportError> {
        if source.len() > SOURCE_LIMIT {
            return Err(limit("file size"));
        }
        // Input admission precedes the parser's eager construction.
        super::reference::reserve_parser_work(builder, source.len())?;
        let mut plan = Self { nodes: Vec::new(), roots: Vec::new(), inline_code: Vec::new() };
        let mut frames: Vec<Frame> = Vec::new();
        let mut owner: Option<usize> = None;
        let mut list_depth = 0usize;
        let mut table_depth = 0usize;
        let mut opaque_depth = 0usize;
        for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
            builder.spend()?;
            checked_source(source, &range)?;
            if matches!(&event, Event::Code(_)) {
                if plan.inline_code.len() >= TOKEN_LIMIT {
                    return Err(limit("inline code spans"));
                }
                plan.inline_code.push(range.clone());
            }
            match event {
                Event::Start(tag) => {
                    // Storage admission is separate from actual list/formatting nesting.
                    if frames.len() >= TOKEN_LIMIT {
                        return Err(limit("parser frame count"));
                    }
                    let end = tag.to_end();
                    match classify_tag(&tag)? {
                        TagClass::Inline => {
                            let inline_owner = if opaque_depth == 0 {
                                let target = owner.ok_or_else(|| corrupt("Markdown inline event without a block"))?;
                                plan.push_inline(target, Event::Start(tag), range.clone())?;
                                Some(target)
                            } else {
                                None
                            };
                            frames.push(Frame {
                                end,
                                range,
                                block: false,
                                list: false,
                                table: false,
                                opaque: false,
                                previous_owner: owner,
                                inline_owner,
                            });
                        }
                        class => {
                            let kind = match class {
                                TagClass::Block(kind) => Some(kind),
                                TagClass::TablePart => {
                                    if table_depth == 0 || opaque_depth == 0 {
                                        return Err(corrupt("Markdown table wrapper outside a table"));
                                    }
                                    None
                                }
                                TagClass::Inline => return Err(corrupt("Markdown tag classification")),
                            };
                            let is_list = matches!(kind, Some(Kind::List(_)));
                            if is_list && list_depth >= DEPTH_LIMIT {
                                return Err(limit("list nesting"));
                            }
                            let is_table = kind == Some(Kind::Table);
                            let opens_opaque = matches!(kind, Some(Kind::Quote | Kind::Html | Kind::Table));
                            let previous_owner = owner;
                            let node = if opaque_depth == 0 {
                                Some(plan.push_node(
                                    kind.ok_or_else(|| corrupt("Markdown table wrapper without owner"))?,
                                    range.clone(),
                                    previous_owner,
                                )?)
                            } else {
                                None
                            };
                            if is_list {
                                list_depth += 1;
                            }
                            if is_table {
                                table_depth += 1;
                            }
                            if opens_opaque {
                                opaque_depth += 1;
                            }
                            owner = node;
                            frames.push(Frame {
                                end,
                                range,
                                block: true,
                                list: is_list,
                                table: is_table,
                                opaque: opens_opaque,
                                previous_owner,
                                inline_owner: None,
                            });
                        }
                    }
                }
                Event::End(end) => {
                    let frame = frames.pop().ok_or_else(|| corrupt("Markdown closing tag without opener"))?;
                    if frame.end != end || frame.range != range {
                        return Err(corrupt("Markdown closing tag boundary"));
                    }
                    if frame.block {
                        if frame.list {
                            list_depth = decrement(list_depth)?;
                        }
                        if frame.table {
                            table_depth = decrement(table_depth)?;
                        }
                        if frame.opaque {
                            opaque_depth = decrement(opaque_depth)?;
                        }
                        owner = frame.previous_owner;
                    } else if let Some(target) = frame.inline_owner {
                        plan.push_inline(target, Event::End(end), range)?;
                    }
                }
                Event::Rule => {
                    if opaque_depth == 0 {
                        plan.push_node(Kind::Rule, range, owner)?;
                    }
                }
                event @ (Event::Text(_) | Event::Code(_) | Event::Html(_) | Event::InlineHtml(_) | Event::SoftBreak | Event::HardBreak) => {
                    if opaque_depth == 0 {
                        let target = owner.ok_or_else(|| corrupt("Markdown inline event without a block"))?;
                        plan.push_inline(target, event, range)?;
                    }
                }
                _ => return Err(corrupt("Unexpected Markdown extension event")),
            }
        }
        if !frames.is_empty() || owner.is_some() || list_depth != 0 || table_depth != 0 || opaque_depth != 0 {
            return Err(corrupt("Unclosed Markdown tag"));
        }
        Ok(plan)
    }
    fn push_node(&mut self, kind: Kind, range: Range<usize>, parent: Option<usize>) -> Result<usize, ImportError> {
        if self.nodes.len() >= TOKEN_LIMIT {
            return Err(limit("block plan nodes"));
        }
        let index = self.nodes.len();
        self.nodes.push(Node { kind, range, parts: Vec::new() });
        if let Some(parent) = parent {
            self.nodes.get_mut(parent).ok_or_else(|| corrupt("Markdown parent node boundary"))?.parts.push(Part::Block(index));
        } else {
            self.roots.push(index);
        }
        Ok(index)
    }
    fn push_inline(&mut self, owner: usize, event: Event<'a>, range: Range<usize>) -> Result<(), ImportError> {
        self.nodes.get_mut(owner).ok_or_else(|| corrupt("Markdown inline owner boundary"))?.parts.push(Part::Inline(event, range));
        Ok(())
    }
}
pub(super) fn content_range(node: &Node<'_>, source: &str) -> Result<Option<Range<usize>>, ImportError> {
    let raw = checked_source(source, &node.range)?;
    match node.kind {
        Kind::Paragraph => {
            let mut range = node.range.clone();
            if raw.ends_with('\n') {
                range.end -= 1;
            }
            return Ok(Some(range));
        }
        Kind::Heading(_) | Kind::Item => {}
        _ => return Ok(None),
    }
    let count = if node.kind == Kind::Item {
        node.parts.iter().position(|part| matches!(part, Part::Block(_))).map_or(node.parts.len(), |at| at)
    } else {
        node.parts.len()
    };
    let parts = node.parts.get(..count).ok_or_else(|| corrupt("Markdown inline group boundary"))?;
    inline_range(parts, source, &node.range)
}
pub(super) fn inline_range(parts: &[Part<'_>], source: &str, enclosing: &Range<usize>) -> Result<Option<Range<usize>>, ImportError> {
    checked_source(source, enclosing)?;
    let mut envelope: Option<Range<usize>> = None;
    let mut leading_text = false;
    for part in parts {
        let Part::Inline(event, range) = part else {
            return Err(corrupt("Markdown child block inside inline group"));
        };
        checked_source(source, range)?;
        let is_text = matches!(event, Event::Text(_));
        match envelope.as_mut() {
            None => {
                envelope = Some(range.clone());
                leading_text = is_text;
            }
            Some(bounds) => {
                if range.start < bounds.start {
                    bounds.start = range.start;
                    leading_text = is_text;
                } else if range.start == bounds.start {
                    leading_text &= is_text;
                }
                bounds.end = bounds.end.max(range.end);
            }
        }
    }
    let Some(mut range) = envelope else {
        return Ok(None);
    };
    // The pinned parser omits one escape byte from leading punctuation Text.
    if leading_text
        && source.as_bytes().get(range.start).is_some_and(u8::is_ascii_punctuation)
        && let Some(previous) = range.start.checked_sub(1)
        && previous >= enclosing.start
        && source.as_bytes().get(previous) == Some(&b'\\')
    {
        range.start = previous;
    }
    checked_source(source, &range)?;
    Ok(Some(range))
}
/// Extract presentation metadata only after the maintained parser recognizes Item/List.
pub(super) fn item_metadata(node: &Node<'_>, source: &str, ordered: bool) -> Result<Option<ItemMetadata>, ImportError> {
    if node.kind != Kind::Item {
        return Err(corrupt("Markdown item metadata boundary"));
    }
    let raw = checked_source(source, &node.range)?;
    let row = raw.split_once('\n').map_or(raw, |(row, _)| row);
    let bytes = row.as_bytes();
    let mut at = 0usize;
    while matches!(bytes.get(at).copied(), Some(b' ' | b'\t')) {
        at += 1;
    }
    let number = if ordered {
        let digits = at;
        let mut number = 0u64;
        while let Some(digit) = bytes.get(at).copied().filter(u8::is_ascii_digit) {
            number = number.checked_mul(10).and_then(|n| n.checked_add(u64::from(digit - b'0'))).ok_or_else(|| limit("list numbering"))?;
            at += 1;
        }
        if at == digits || !matches!(bytes.get(at).copied(), Some(b'.' | b')')) {
            return Ok(None);
        }
        at += 1;
        Some(number)
    } else if matches!(bytes.get(at).copied(), Some(b'-' | b'+' | b'*')) {
        at += 1;
        None
    } else {
        return Ok(None);
    };
    if let Some(next) = bytes.get(at).copied()
        && !matches!(next, b' ' | b'\t')
    {
        return Ok(None);
    }
    while matches!(bytes.get(at).copied(), Some(b' ' | b'\t')) {
        at += 1;
    }
    let content_start = node.range.start.checked_add(at).ok_or_else(|| corrupt("Markdown item content boundary"))?;
    checked_source(source, &(content_start..node.range.end))?;
    Ok(Some(ItemMetadata { content_start, number }))
}

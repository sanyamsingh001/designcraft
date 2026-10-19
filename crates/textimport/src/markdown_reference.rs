//! Reference labels used only to select the existing literal unsupported-link fallback.
use super::{BLOCK_LIMIT, Builder, ImportError, indentation, limit};

pub(super) fn key(label: &str, builder: &mut Builder) -> Result<Option<String>, ImportError> {
    let mut out = String::new();
    let mut chars = label.chars().peekable();
    let mut count = 0usize;
    let mut space = false;
    while let Some(c) = chars.next() {
        builder.spend()?;
        count += 1;
        if count > 999 {
            return Ok(None);
        }
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        if c == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            out.push(c);
            if let Some(next) = chars.next() {
                builder.spend()?;
                count += 1;
                if count > 999 {
                    return Ok(None);
                }
                out.push(next);
            }
        } else {
            if matches!(c, '[' | ']') {
                return Ok(None);
            }
            out.extend(c.to_uppercase().flat_map(char::to_lowercase));
        }
    }
    Ok((!out.is_empty()).then_some(out))
}

fn destination<'a>(source: &'a str, builder: &mut Builder) -> Result<Option<&'a str>, ImportError> {
    let mut chars = source.char_indices().peekable();
    let angled = source.starts_with('<');
    let mut depth = 0usize;
    let mut end = None;
    if angled {
        chars.next();
    }
    while let Some((at, c)) = chars.next() {
        builder.spend()?;
        if c == '\\' && chars.peek().is_some_and(|(_, next)| next.is_ascii_punctuation()) {
            chars.next();
            builder.spend()?;
            continue;
        }
        if angled {
            if c == '>' {
                end = Some(at + 1);
                break;
            }
            if c == '<' || c == '\n' || c == '\r' {
                return Ok(None);
            }
        } else {
            if c.is_whitespace() {
                end = Some(at);
                break;
            }
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => return Ok(None),
                ')' => depth -= 1,
                '<' | '>' => return Ok(None),
                _ => {}
            }
        }
        if c.is_control() {
            return Ok(None);
        }
    }
    let end = match end {
        Some(end) => end,
        None if !angled => source.len(),
        None => return Ok(None),
    };
    if end == 0 || depth != 0 {
        return Ok(None);
    }
    let tail = source.get(end..).ok_or_else(|| ImportError::Corrupt("Markdown reference boundary".into()))?.trim();
    Ok(Some(tail))
}

fn title(source: &str, builder: &mut Builder) -> Result<bool, ImportError> {
    let tail = source.trim();
    let Some(open) = tail.chars().next() else { return Ok(false) };
    let close = match open {
        '"' | '\'' => open,
        '(' => ')',
        _ => return Ok(false),
    };
    let mut chars = tail.char_indices().skip(1).peekable();
    while let Some((at, c)) = chars.next() {
        builder.spend()?;
        if c == '\\' && chars.peek().is_some_and(|(_, next)| next.is_ascii_punctuation()) {
            chars.next();
            builder.spend()?;
        } else if c == close {
            return Ok(tail.get(at + c.len_utf8()..).is_some_and(|s| s.trim().is_empty()));
        }
    }
    Ok(false)
}

struct Definition {
    label: String,
    has_title: bool,
}

fn definition(source: &str, builder: &mut Builder) -> Result<Option<Definition>, ImportError> {
    if indentation(source).0 > 3 {
        return Ok(None);
    }
    let source = source.trim_start_matches([' ', '\t']);
    if !source.starts_with('[') {
        return Ok(None);
    }
    let mut chars = source.char_indices().skip(1).peekable();
    while let Some((at, c)) = chars.next() {
        builder.spend()?;
        if c == '\\' && chars.peek().is_some_and(|(_, next)| next.is_ascii_punctuation()) {
            chars.next();
            builder.spend()?;
            continue;
        }
        match c {
            '[' | '\n' => return Ok(None),
            ']' => {
                let Some(tail) = source.get(at + 1..).and_then(|s| s.strip_prefix(':')) else { return Ok(None) };
                let tail = tail.trim_start();
                let Some(title_source) = destination(tail, builder)? else { return Ok(None) };
                let has_title = !title_source.is_empty();
                if has_title && !title(title_source, builder)? {
                    return Ok(None);
                }
                let label = source.get(1..at).ok_or_else(|| ImportError::Corrupt("Markdown reference label boundary".into()))?;
                return Ok(key(label, builder)?.map(|label| Definition { label, has_title }));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Inspect the existing definition grammar without registering a reference.
fn inspect<'a>(line: &str, mut next: impl Iterator<Item = &'a str>, builder: &mut Builder) -> Result<Option<(String, usize)>, ImportError> {
    let mut continued = 0;
    let definition = match definition(line, builder)? {
        Some(definition) => Some(definition),
        None if line.trim_end().ends_with(':') => match next.next() {
            Some(next) => {
                let definition = definition(&format!("{line}\n{next}"), builder)?;
                continued = usize::from(definition.is_some());
                definition
            }
            None => None,
        },
        None => None,
    };
    if let Some(Definition { label, has_title }) = definition {
        if !has_title
            && let Some(next) = next.next()
            && title(next, builder)?
        {
            continued += 1;
        }
        return Ok(Some((label, continued)));
    }
    Ok(None)
}

/// Return the number of following literal rows (at most two) without discovering labels.
/// A destination may occupy the next row, followed by one complete title row.
pub(super) fn continued_rows<'a>(line: &str, next: impl Iterator<Item = &'a str>, builder: &mut Builder) -> Result<usize, ImportError> {
    Ok(inspect(line, next, builder)?.map_or(0, |(_, continued)| continued))
}

pub(super) fn remember<'a>(line: &str, next: impl Iterator<Item = &'a str>, builder: &mut Builder) -> Result<usize, ImportError> {
    // Both passes need the literal extent; only the discovery pass records labels.
    let Some((label, continued)) = inspect(line, next, builder)? else { return Ok(0) };
    if builder.known_references.is_empty() {
        if builder.defined_references.len() >= BLOCK_LIMIT && !builder.defined_references.contains(&label) {
            return Err(limit("reference definitions"));
        }
        builder.defined_references.insert(label);
    }
    Ok(continued)
}

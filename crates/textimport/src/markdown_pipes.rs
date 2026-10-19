//! Literal pipe-row policy, using maintained Text ranges to exclude code and escapes.
use super::{BLOCK_LIMIT, Builder, limit};
use crate::ImportError;
use pulldown_cmark::Event;
use std::ops::Range;

pub(super) fn literal_pipe_rows<'input: 'events, 'events>(
    source: &str,
    events: impl IntoIterator<Item = (&'events Event<'input>, &'events Range<usize>)>,
    builder: &mut Builder,
) -> Result<Vec<Range<usize>>, ImportError> {
    fn charge(builder: &mut Builder, bytes: usize) -> Result<(), ImportError> {
        builder.work = builder.work.checked_sub(bytes).ok_or_else(|| limit("delimiter processing"))?; Ok(())
    }
    let boundary = || ImportError::Corrupt("Markdown pipe source boundary".into());
    let bytes = source.as_bytes();
    let mut rows: Vec<Range<usize>> = Vec::new();
    let mut content_start = None;
    let mut current_row = 0..0;
    let mut leading_pipe = false;
    for (event, range) in events {
        builder.spend()?;
        let raw = source.get(range.clone()).ok_or_else(boundary)?;
        let is_text = matches!(event, Event::Text(_));
        let escaped_first = is_text && range.start > 0 && bytes.get(range.start - 1) == Some(&b'\\')
            && raw.as_bytes().first().is_some_and(u8::is_ascii_punctuation);
        let logical_start = *content_start.get_or_insert(range.start.saturating_sub(usize::from(escaped_first)));
        if !is_text { continue; }
        charge(builder, raw.len())?;
        for (relative, _) in raw.match_indices('|') {
            if relative == 0 && escaped_first { continue; }
            let at = range.start.checked_add(relative).ok_or_else(boundary)?;
            if !current_row.contains(&at) {
                let before = source.get(..at).ok_or_else(boundary)?;
                let after = source.get(at..).ok_or_else(boundary)?;
                let start = before.rfind('\n').map_or(0, |newline| newline + 1);
                let end = match after.find('\n') { Some(end) => at.checked_add(end).ok_or_else(boundary)?, None => source.len() };
                charge(builder, (end - start).checked_mul(2).ok_or_else(|| limit("delimiter processing"))?)?;
                current_row = start..end;
                leading_pipe = source.get(start.max(logical_start)..at).ok_or_else(boundary)?.chars().all(char::is_whitespace);
            }
            if rows.last().is_some_and(|row| row.start == current_row.start) { continue; }
            let surrounded = at > 0 && bytes.get(at - 1) == Some(&b' ') && bytes.get(at + 1) == Some(&b' ');
            if leading_pipe || surrounded {
                if rows.len() >= BLOCK_LIMIT { return Err(limit("literal rows")); }
                rows.push(current_row.clone());
            }
        }
    }
    Ok(rows)
}

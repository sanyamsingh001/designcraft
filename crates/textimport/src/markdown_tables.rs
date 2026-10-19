//! Preserve original rows of maintained tables while retaining code/container ownership.
use super::{
    BLOCK_LIMIT, Builder, ImportError, SOURCE_LIMIT, TOKEN_LIMIT,
    blocks::{Kind, Plan},
    limit,
};
use std::ops::Range;
fn boundary() -> ImportError {
    ImportError::Corrupt("Markdown table source boundary".into())
}
fn spend_bytes(builder: &mut Builder, count: usize) -> Result<(), ImportError> {
    builder.work = builder.work.checked_sub(count).ok_or_else(|| limit("delimiter processing"))?;
    Ok(())
}
fn push(ranges: &mut Vec<Range<usize>>, range: Range<usize>) -> Result<(), ImportError> {
    if ranges.len() >= TOKEN_LIMIT {
        return Err(limit("table source ranges"));
    }
    ranges.try_reserve(1).map_err(|_| limit("table source storage"))?;
    ranges.push(range);
    Ok(())
}
fn merged(mut ranges: Vec<Range<usize>>, builder: &mut Builder) -> Result<Vec<Range<usize>>, ImportError> {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        builder.spend()?;
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
            continue;
        }
        push(&mut merged, range)?;
    }
    Ok(merged)
}
fn overlaps(ranges: &[Range<usize>], cursor: &mut usize, candidate: &Range<usize>, builder: &mut Builder) -> Result<bool, ImportError> {
    while let Some(range) = ranges.get(*cursor) {
        builder.spend()?;
        if range.end > candidate.start {
            return Ok(range.start < candidate.end);
        }
        *cursor += 1;
    }
    Ok(false)
}
fn header_range(source: &str, range: &Range<usize>, builder: &mut Builder) -> Result<Range<usize>, ImportError> {
    let raw = source.get(range.clone()).ok_or_else(boundary)?;
    let mut end = range.start;
    let mut rows = 0;
    for row in raw.split_inclusive('\n').take(2) {
        spend_bytes(builder, row.len())?;
        end = end.checked_add(row.len()).ok_or_else(boundary)?;
        rows += 1;
    }
    if rows != 2 {
        return Err(boundary());
    }
    Ok(range.start..end)
}
pub(super) fn table_ranges(
    source: &str,
    baseline: &Plan<'_>,
    table_plan: &Plan<'_>,
    builder: &mut Builder,
) -> Result<Vec<Range<usize>>, ImportError> {
    if source.len() > SOURCE_LIMIT {
        return Err(limit("source bytes"));
    }
    let mut protected = Vec::new();
    for node in &baseline.nodes {
        builder.spend()?;
        if matches!(node.kind, Kind::List(_) | Kind::Code | Kind::Quote | Kind::Html) {
            push(&mut protected, node.range.clone())?;
        }
    }
    let protected = merged(protected, builder)?;
    let mut multiline_code = Vec::new();
    for range in &baseline.inline_code {
        builder.spend()?;
        let raw = source.get(range.clone()).ok_or_else(boundary)?;
        spend_bytes(builder, raw.len())?;
        if raw.contains('\n') {
            push(&mut multiline_code, range.clone())?;
        }
    }
    let multiline_code = merged(multiline_code, builder)?;
    let mut candidates = Vec::new();
    for node in &table_plan.nodes {
        builder.spend()?;
        if node.kind == Kind::Table {
            push(&mut candidates, node.range.clone())?;
        }
    }
    candidates.sort_unstable_by_key(|range| (range.start, range.end));
    let mut accepted = Vec::new();
    let mut protected_cursor = 0;
    let mut code_cursor = 0;
    let mut previous_end = 0;
    for range in candidates {
        builder.spend()?;
        if range.is_empty() || range.start < previous_end || source.get(range.clone()).is_none() {
            return Err(boundary());
        }
        previous_end = range.end;
        if overlaps(&protected, &mut protected_cursor, &range, builder)? {
            continue;
        }
        let header = header_range(source, &range, builder)?;
        if overlaps(&multiline_code, &mut code_cursor, &header, builder)? {
            continue;
        }
        if accepted.len() >= BLOCK_LIMIT {
            return Err(limit("paragraph count"));
        }
        push(&mut accepted, range)?;
    }
    Ok(accepted)
}
/// A table body owns its raw rows, including definition-looking source.
pub(super) fn subtract_table_ranges(
    source: &str,
    gaps: &[Range<usize>],
    tables: &[Range<usize>],
    builder: &mut Builder,
) -> Result<Vec<Range<usize>>, ImportError> {
    fn piece(source: &str, range: Range<usize>, output: &mut Vec<Range<usize>>, builder: &mut Builder) -> Result<(), ImportError> {
        let raw = source.get(range.clone()).ok_or_else(boundary)?;
        spend_bytes(builder, raw.len())?;
        if raw.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n')) {
            return Ok(());
        }
        if output.len() >= BLOCK_LIMIT {
            return Err(limit("reference source ranges"));
        }
        push(output, range)
    }
    for ranges in [gaps, tables] {
        let mut previous_end = 0;
        for range in ranges {
            builder.spend()?;
            if range.start < previous_end || source.get(range.clone()).is_none() {
                return Err(boundary());
            }
            previous_end = range.end;
        }
    }
    let mut output = Vec::new();
    let mut table_cursor = 0;
    for gap in gaps {
        while let Some(table) = tables.get(table_cursor) {
            builder.spend()?;
            if table.end > gap.start {
                break;
            }
            table_cursor += 1;
        }
        let mut at = gap.start;
        let mut scan = table_cursor;
        while let Some(table) = tables.get(scan) {
            builder.spend()?;
            if table.start >= gap.end {
                break;
            }
            if table.start > at {
                piece(source, at..table.start.min(gap.end), &mut output, builder)?;
            }
            at = at.max(table.end.min(gap.end));
            if table.end >= gap.end {
                break;
            }
            scan += 1;
        }
        table_cursor = scan;
        if at < gap.end {
            piece(source, at..gap.end, &mut output, builder)?;
        }
    }
    Ok(output)
}

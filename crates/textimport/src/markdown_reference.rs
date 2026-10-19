//! Reference recognition belongs to pulldown; original root gaps remain visible.
use super::{BLOCK_LIMIT, Builder, ImportError, SOURCE_LIMIT, TOKEN_LIMIT, limit};
use pulldown_cmark::{BrokenLink, CowStr, Parser};
use std::ops::Range;

const REFERENCE_BUFFER_LIMIT: usize = SOURCE_LIMIT + 2 * BLOCK_LIMIT;
fn boundary() -> ImportError {
    ImportError::Corrupt("Markdown reference source boundary".into())
}
fn blank_gap(source: &str) -> bool {
    source.bytes().all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
}
fn push_gap(source: &str, range: Range<usize>, gaps: &mut Vec<Range<usize>>) -> Result<(), ImportError> {
    let raw = source.get(range.clone()).ok_or_else(boundary)?;
    if blank_gap(raw) {
        return Ok(());
    }
    if gaps.len() >= BLOCK_LIMIT {
        return Err(limit("reference gaps"));
    }
    gaps.try_reserve(1).map_err(|_| limit("reference gaps"))?;
    gaps.push(range);
    Ok(())
}
pub(super) fn root_gaps(source: &str, roots: impl IntoIterator<Item = Range<usize>>) -> Result<Vec<Range<usize>>, ImportError> {
    if source.len() > SOURCE_LIMIT {
        return Err(limit("source size"));
    }
    let mut gaps = Vec::new();
    let mut cursor = 0usize;
    for (index, range) in roots.into_iter().enumerate() {
        if index >= TOKEN_LIMIT {
            return Err(limit("block count"));
        }
        if range.start < cursor || source.get(range.clone()).is_none() {
            return Err(boundary());
        }
        push_gap(source, cursor..range.start, &mut gaps)?;
        cursor = range.end;
    }
    push_gap(source, cursor..source.len(), &mut gaps)?;
    Ok(gaps)
}
pub(super) fn reserve_parser_work(builder: &mut Builder, source_len: usize) -> Result<(), ImportError> {
    builder.work = builder.work.checked_sub(source_len).ok_or_else(|| limit("delimiter processing"))?;
    Ok(())
}
fn definition_parser<'a>(source: &'a str, require_definition: bool, builder: &mut Builder) -> Result<Parser<'a>, ImportError> {
    if source.len() > REFERENCE_BUFFER_LIMIT {
        return Err(limit("reference source size"));
    }
    reserve_parser_work(builder, source.len())?;
    let mut parser = Parser::new(source);
    if parser.reference_definitions().iter().count() > BLOCK_LIMIT {
        return Err(limit("reference definitions"));
    }
    let has_definitions = parser.reference_definitions().iter().next().is_some();
    if parser.next().is_some() {
        builder.spend()?;
        return Err(ImportError::Corrupt("Markdown reference gap contains parsed content".into()));
    }
    if require_definition && !has_definitions {
        return Err(ImportError::Corrupt("Markdown reference gap contains no recognized definition".into()));
    }
    Ok(parser)
}
/// Separate independently recognized gaps so titles cannot cross their boundaries.
pub(super) fn reference_source(source: &str, gaps: &[Range<usize>], builder: &mut Builder) -> Result<String, ImportError> {
    if source.len() > SOURCE_LIMIT || gaps.len() > BLOCK_LIMIT {
        return Err(limit("reference source size"));
    }
    let mut output = String::new();
    let mut previous_end = 0usize;
    for range in gaps {
        if range.start < previous_end {
            return Err(boundary());
        }
        let raw = source.get(range.clone()).ok_or_else(boundary)?;
        previous_end = range.end;
        let _validated = definition_parser(raw, true, builder)?;
        let separator = usize::from(!output.is_empty()) * 2;
        let additional = raw.len().checked_add(separator).ok_or_else(|| limit("reference source size"))?;
        let next_len = output.len().checked_add(additional).ok_or_else(|| limit("reference source size"))?;
        if next_len > REFERENCE_BUFFER_LIMIT {
            return Err(limit("reference source size"));
        }
        output.try_reserve(additional).map_err(|_| limit("reference source size"))?;
        if separator != 0 {
            output.push_str("\n\n");
        }
        output.push_str(raw);
    }
    Ok(output)
}
pub(super) fn allowed_parser<'a>(source: &'a str, builder: &mut Builder) -> Result<Parser<'a>, ImportError> {
    definition_parser(source, !source.is_empty(), builder)
}
pub(super) fn resolve<'input>(allowed: &Parser<'_>, broken: BrokenLink<'input>) -> Option<(CowStr<'input>, CowStr<'input>)> {
    allowed.reference_definitions().get(broken.reference.as_ref()).map(|_| ("".into(), "".into()))
}

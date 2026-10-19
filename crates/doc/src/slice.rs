//! Formatted text slices: copy a range of a story (with paragraph and character formats,
//! footnotes, endnotes, editorial notes, cross-references, index markers and tables) and insert
//! it into another story.

use std::sync::Arc;

use crate::endnotes::{ENDNOTE_REF, NOTE_MARK};
use crate::ids::StoryId;
use crate::index::INDEX_MARK;
use crate::notes::FOOTNOTE_REF;
use crate::story::{CharRun, Story, TABLE_ANCHOR};
use crate::xref::{ANCHOR_MARK, XREF_MARK};

impl Story {
    /// A frameless story holding `range` with its formatting and the objects anchored in it.
    pub fn extract(&self, range: std::ops::Range<usize>) -> Story {
        let a = crate::story::floor_char_boundary(&self.text, range.start.min(self.len()));
        let b = crate::story::floor_char_boundary(&self.text, range.end.min(self.len())).max(a);
        let mut out = Story::new(StoryId(0));
        out.direction = self.direction;
        out.text = self.text[a..b].to_string();
        let (p0, p1) = (self.para_at(a), self.para_at(b));
        out.paras = self.paras[p0..=p1].to_vec();
        out.chars = Vec::new();
        for (r, f) in self.runs() {
            let s = r.start.max(a);
            let e = r.end.min(b);
            if s < e {
                out.chars.push(CharRun { len: e - s, format: f.clone() });
            }
        }
        if out.chars.is_empty() {
            out.chars.push(CharRun { len: 0, format: self.char_format_at(a).clone() });
        }
        let count = |m: char, upto: usize| self.text[..upto].matches(m).count();
        let span = |m: char| count(m, a)..count(m, b);
        out.notes = self.notes[span(FOOTNOTE_REF)].to_vec();
        if !self.endnotes.is_empty() {
            out.endnotes = self.endnotes[span(ENDNOTE_REF)].to_vec();
        }
        if !self.editorial.is_empty() {
            out.editorial = self.editorial[span(NOTE_MARK)].to_vec();
        }
        out.anchors = self.anchors[span(ANCHOR_MARK)].to_vec();
        out.xrefs = self.xrefs[span(XREF_MARK)].to_vec();
        out.index_refs = self.index_refs[span(INDEX_MARK)].to_vec();
        out.objects = self.objects[span(crate::anchored::OBJECT_MARK)].to_vec();
        for p in &out.paras {
            if let Some(id) = p.table
                && let Some(t) = self.tables.get(&id)
                && out.text.contains(TABLE_ANCHOR)
            {
                out.tables.insert(id, t.clone());
            }
        }
        out.fix_tables();
        out
    }

    /// Insert a slice made by [`Story::extract`] at `pos`, keeping its formatting: the first
    /// pasted paragraph merges into the paragraph at `pos` (it keeps that paragraph's format, as
    /// in InDesign), later paragraphs keep their own formats. Returns the end of the insertion.
    pub fn insert_story(&mut self, pos: usize, src: &Story) -> usize {
        let pos = crate::story::floor_char_boundary(&self.text, pos.min(self.len()));
        if src.text.is_empty() {
            return pos;
        }
        // Text first (creates empty marker entries), then formats and marker payloads.
        let mut at = pos;
        for (r, f) in src.runs() {
            if r.is_empty() {
                continue;
            }
            self.insert_with(at, &src.text[r.clone()], f.clone());
            at += r.len();
        }
        let end = at;
        // Paragraph formats of the pasted paragraphs after the first.
        let p0 = self.para_at(pos);
        let consumed_eof = src.slice_has_eof_stage && src.text.ends_with('\n') && end < self.len();
        for (k, pf) in src.paras.iter().enumerate().skip(1) {
            if let Some(p) = self.paras.get_mut(p0 + k) {
                *p = crate::story::ParaFormat { table: None, ..pf.clone() };
                if consumed_eof && k + 1 == src.paras.len() {
                    // The copied EOF now owns destination bytes (possibly an LF).
                    // Its typing stage must not become those bytes' paragraph defaults.
                    p.chars = p.eof_inherited_chars.take().unwrap_or_default();
                }
            }
        }
        // Marker payloads in text order. Note and anchor ids belong to the destination.
        let first = |m: char| self.text[..pos].matches(m).count();
        let (nf, nx, ni, na) = (first(FOOTNOTE_REF), first(XREF_MARK), first(INDEX_MARK), first(ANCHOR_MARK));
        let no = first(crate::anchored::OBJECT_MARK);
        for (k, o) in src.objects.iter().enumerate() {
            if let Some(slot) = self.objects.get_mut(no + k) {
                *slot = o.clone();
            }
        }
        for (k, n) in src.notes.iter().enumerate() {
            if let Some(slot) = self.notes.get_mut(nf + k) {
                let id = slot.id;
                *slot = Arc::new(crate::notes::Footnote { id, text: n.text.clone() });
            }
        }
        if !src.endnotes.is_empty() {
            let ne = first(ENDNOTE_REF);
            for (k, n) in src.endnotes.iter().enumerate() {
                if let Some(slot) = self.endnotes.get_mut(ne + k) {
                    let id = slot.id;
                    *slot = Arc::new(crate::notes::Footnote { id, text: n.text.clone() });
                }
            }
        }
        if !src.editorial.is_empty() {
            let nn = first(NOTE_MARK);
            for (k, n) in src.editorial.iter().enumerate() {
                if let Some(slot) = self.editorial.get_mut(nn + k) {
                    let id = slot.id;
                    *slot = Arc::new(crate::endnotes::EditorialNote { id, ..(**n).clone() });
                }
            }
        }
        for (k, x) in src.xrefs.iter().enumerate() {
            if let Some(slot) = self.xrefs.get_mut(nx + k) {
                *slot = x.clone();
            }
        }
        for (k, r) in src.index_refs.iter().enumerate() {
            if let Some(slot) = self.index_refs.get_mut(ni + k) {
                *slot = r.clone();
            }
        }
        // Anchors are new destinations (fresh ids) with the copied names.
        for (k, a) in src.anchors.iter().enumerate() {
            if let Some(slot) = self.anchors.get_mut(na + k) {
                let id = slot.id;
                *slot = Arc::new(crate::xref::TextAnchor { id, name: a.name.clone() });
            }
        }
        // Tables: re-attach by paragraph (fresh ids so a pasted copy never shares an id).
        if !src.tables.is_empty() {
            let mut next = self.tables.keys().max().copied().unwrap_or(0) + 1;
            for (k, pf) in src.paras.iter().enumerate() {
                let Some(tid) = pf.table else { continue };
                let Some(t) = src.tables.get(&tid) else { continue };
                let pi = p0 + k;
                let mut t = (**t).clone();
                t.id = next;
                next += 1;
                self.tables.insert(t.id, Arc::new(t));
                if let Some(p) = self.paras.get_mut(pi) {
                    p.table = Some(next - 1);
                }
            }
            self.fix_tables();
        }
        self.rev = self.rev.wrapping_add(1);
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endnotes::NOTE_MARK;
    use crate::story::ParaFormat;

    #[test]
    fn copy_paste_keeps_formatting_and_notes() {
        let mut a = Story::with_text(StoryId(1), "Alpha beta\nGamma delta", ParaFormat::default());
        a.format_chars(0..5, |f| f.over.size = Some(20.0));
        a.format_paras(11..11, |p| p.style = "Head".into());
        a.insert_note(8, "A note.", ParaFormat::default());
        a.check().unwrap();
        let slice = a.extract(0..a.len());
        slice.check().unwrap();
        assert_eq!(slice.notes.len(), 1);
        let mut b = Story::with_text(StoryId(2), "Start: end", ParaFormat::default());
        let end = b.insert_story(7, &slice);
        b.check().unwrap();
        assert_eq!(&b.text[7..end], slice.text.as_str());
        assert_eq!(b.char_format_at(8).over.size, Some(20.0));
        assert_eq!(b.paras[1].style, "Head");
        assert_eq!(b.notes.len(), 1);
        assert_eq!(b.notes[0].text.text, "A note.");
        // A partial slice.
        let e = 8 + FOOTNOTE_REF.len_utf8();
        let part = a.extract(2..e);
        part.check().unwrap();
        assert_eq!(part.text, a.text[2..e]);
        assert_eq!(part.notes.len(), 1);
        assert!(a.extract(2..8).notes.is_empty());
    }

    fn append_editorial(st: &mut Story, text: &str, author: &str) {
        st.insert(st.len(), &NOTE_MARK.to_string());
        let n = Arc::make_mut(st.editorial.last_mut().unwrap());
        n.text = text.into();
        n.author = author.into();
    }

    fn rich_citation() -> Story {
        let mut st = Story::with_text(StoryId(0), "Café citation\n第二段", ParaFormat::default());
        st.direction = crate::TextDirection::RightToLeft;
        st.vertical = true;
        st.format_chars(0..5, |f| {
            f.style = "Citation character".into();
            f.over.size = Some(19.0);
            f.over.change = Some(crate::ChangeMark::Inserted);
        });
        st.format_paras(0..st.len(), |p| {
            p.style = "Citation paragraph".into();
            p.chars.tracking = Some(17.0);
        });
        st.insert_note(st.len(), "Nested footnote", ParaFormat::default());
        st.insert_endnote(st.len(), "Nested endnote", ParaFormat::default());
        append_editorial(&mut st, "Nested comment", "Nested author");
        let anchor = st.insert_anchor(st.len(), "Nested destination");
        st.insert_xref(st.len(), crate::xref::CrossRef { target: anchor, format: "Page Number".into() });
        st.insert_index_ref(st.len(), crate::index::IndexRef { topics: vec!["Citation".into()], ..Default::default() });
        st.insert_object(st.len(), crate::anchored::AnchoredObject::default());
        st.insert_table(st.len(), crate::table::Table::new(7, 1, 1, 0, 0, 100.0));
        st.check().unwrap();
        st
    }

    #[test]
    fn clipped_note_payloads_survive_copy_with_destination_ids_and_nested_content() {
        let mut source = Story::with_text(StoryId(1), "Before é", ParaFormat::default());
        source.insert_endnote(source.len(), "Outside before", ParaFormat::default());
        append_editorial(&mut source, "Outside before", "Other");
        let a = source.len();
        source.insert(a, "正文");
        source.insert_note(source.len(), "Footnote control", ParaFormat::default());
        let id = source.insert_endnote(source.len(), "Replaced by rich citation", ParaFormat::default());
        source.endnote_mut(id).unwrap().text = rich_citation();
        append_editorial(&mut source, "Keep this comment — é", "Reviewer 李");
        let b = source.len();
        source.insert_endnote(b, "Outside after", ParaFormat::default());
        append_editorial(&mut source, "Outside after", "Other");
        source.format_chars(a..b, |f| f.over.change = Some(crate::ChangeMark::Deleted));
        let original = source.clone();
        let slice = source.extract(a..b);
        assert_eq!(slice.endnotes.len(), 1, "copy includes the selected citation payload, not just its marker");
        assert_eq!(slice.editorial.len(), 1);
        assert_eq!(slice.endnotes[0].text, rich_citation());
        assert_eq!((slice.editorial[0].text.as_str(), slice.editorial[0].author.as_str()), ("Keep this comment — é", "Reviewer 李"));
        assert_eq!(slice.notes[0].text.text, "Footnote control");
        slice.check().unwrap();
        let mut dest = Story::with_text(StoryId(2), "Destination", ParaFormat::default());
        dest.insert_note(dest.len(), "Existing footnote", ParaFormat::default());
        // Force destination ids above the source ids; copies must not import outer ids.
        for _ in 0..4 {
            dest.insert_endnote(dest.len(), "Existing citation", ParaFormat::default());
            append_editorial(&mut dest, "Existing comment", "Existing author");
        }
        let prefix = dest.clone();
        let at = dest.len();
        let end = dest.insert_story(at, &slice);
        assert_eq!(&dest.text[at..end], slice.text);
        assert_eq!(dest.endnotes[4].text, slice.endnotes[0].text, "all nested Story data, formats, changes and payloads survive");
        assert_eq!(dest.editorial[4].text, slice.editorial[0].text);
        assert_eq!(dest.editorial[4].author, slice.editorial[0].author);
        assert_eq!(dest.endnotes[4].id, 5);
        assert_eq!(dest.editorial[4].id, 5);
        assert_ne!(dest.endnotes[4].id, slice.endnotes[0].id);
        assert_ne!(dest.editorial[4].id, slice.editorial[0].id);
        assert_eq!(dest.endnotes[..4], prefix.endnotes);
        assert_eq!(dest.editorial[..4], prefix.editorial);
        assert_eq!(dest.notes[0], prefix.notes[0]);
        assert_eq!(dest.notes[1].text, slice.notes[0].text);
        assert_eq!(dest.format_after(at), slice.format_after(0));
        dest.insert_story(dest.len(), &slice);
        assert_eq!(dest.endnotes[5].id, 6);
        assert_eq!(dest.editorial[5].id, 6);
        dest.endnote_mut(5).unwrap().text.insert(0, "Edited copy: ");
        Arc::make_mut(&mut dest.editorial[4]).text = "Edited copy".into();
        assert_eq!(source, original, "editing a copy must leave the entire source unchanged");
        assert_eq!(dest.endnotes[5].text, slice.endnotes[0].text);
        assert_eq!(dest.editorial[5].text, slice.editorial[0].text);
        dest.check().unwrap();
    }

    #[test]
    fn note_fix_keeps_existing_slice_payload_and_table_policies() {
        let source = rich_citation();
        let slice = source.extract(0..source.len());
        let mut dest = Story::new(StoryId(3));
        dest.insert_anchor(0, "Existing destination");
        let old_anchor = dest.anchors[0].clone();
        let at = dest.len();
        let end = dest.insert_story(at, &slice);
        assert_eq!(&dest.text[at..end], source.text);
        assert_eq!(dest.notes[0].text, source.notes[0].text);
        assert_eq!(dest.endnotes[0].text, source.endnotes[0].text);
        assert_eq!(dest.editorial[0].text, source.editorial[0].text);
        assert_eq!(dest.editorial[0].author, source.editorial[0].author);
        assert_eq!(dest.anchors[0], old_anchor);
        assert_ne!(dest.anchors[1].id, source.anchors[0].id);
        assert_eq!(dest.anchors[1].name, source.anchors[0].name);
        assert_eq!(dest.xrefs, source.xrefs, "cross-reference targets retain the existing policy");
        assert_eq!(dest.index_refs, source.index_refs);
        assert_eq!(dest.objects, source.objects);
        let copied_table = dest.tables.values().next().unwrap();
        let source_table = source.tables.values().next().unwrap();
        let mut expected = (**source_table).clone();
        expected.id = copied_table.id;
        assert_eq!(**copied_table, expected);
        assert_ne!(copied_table.id, source_table.id);
        dest.check().unwrap();
    }
}

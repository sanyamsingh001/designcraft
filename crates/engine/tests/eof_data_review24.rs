//! D07/D08/D13: outer data deletions retain paragraph defaults after later typing.

use designcraft_doc::{HyperlinkSource, Leading, ParaFormat, PlaceholderAnchor, Story, StoryId};
use designcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn document(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    run(s, "text.select", json!({"story": sid.0, "anchor": anchor, "focus": focus}));
}

fn frame(s: &mut Session, text: &str) -> StoryId {
    StoryId(run(s, "frame.create", json!({"rect": [40, 40, 440, 440], "content": "text", "text": text}))["story"].as_u64().unwrap())
}

fn story(s: &Session, sid: StoryId) -> &Story {
    s.doc().unwrap().doc.story(sid).unwrap()
}

fn edit(s: &mut Session, command: &str, params: Value) {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    run(s, command, params.clone());
    let after = document(s);
    let selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    assert_eq!(&s.journal[journal..], &[(command.to_string(), params)]);
    s.doc().unwrap().doc.check().unwrap();
    run(s, "edit.undo", json!({}));
    assert_eq!(document(s), before, "Undo restores all formats and data metadata");
    run(s, "edit.redo", json!({}));
    assert_eq!(document(s), after, "Redo restores all formats and data metadata");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
}

fn fixture() -> (Session, StoryId, ParaFormat) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"pages": 1}));
    let sid = frame(&mut s, "A");
    select(&mut s, sid, 0, 1);
    run(&mut s, "style.paragraph.create", json!({"name": "Review24 baseline", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}));
    run(&mut s, "style.paragraph.apply", json!({"name": "Review24 baseline"}));
    run(&mut s, "style.breakLink", json!({"kind": "paragraph"}));
    let baseline = story(&s, sid).paras[0].clone();
    assert_eq!(baseline.chars.size, Some(24.0));
    assert_eq!(baseline.chars.leading, Some(Leading::Points(26.0)));
    assert!(baseline.eof_inherited_chars.is_none());
    run(&mut s, "type.char", json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}}));
    select(&mut s, sid, 1, 1);
    run(&mut s, "text.insert", json!({"text": "\n", "raw": true}));
    run(&mut s, "type.char", json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 30}}}));
    assert_eq!(story(&s, sid).text, "A\n");
    assert_eq!(story(&s, sid).paras[1].eof_inherited_chars.as_ref(), Some(&baseline.chars));
    (s, sid, baseline)
}

fn source(s: &mut Session, csv: &str) {
    run(s, "data.source.select", json!({"name": "one.csv", "bytes": designcraft_engine::cmd::base64_encode(csv.as_bytes())}));
}

fn placeholder(s: &mut Session, sid: StoryId) -> u64 {
    run(s, "data.placeholder.add", json!({"field": "Field", "story": sid.0, "at": 2}))["id"].as_u64().unwrap()
}

fn variable(s: &mut Session, name: &str, text: &str) {
    run(s, "variables.define", json!({"name": name, "type": "custom", "text": text}));
    run(s, "variables.insert", json!({"name": name}));
}

fn native_copies(s: &mut Session) -> [Session; 2] {
    let expected = document(s);
    let saved = run(s, "file.serialize", json!({}));
    let zip = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"));
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    [zip, legacy].map(|bytes| {
        let mut back = Session::new();
        run(&mut back, "file.openBytes", json!({"base64": designcraft_engine::cmd::base64_encode(&bytes)}));
        assert_eq!(document(&back), expected);
        back
    })
}

fn type_and_clear(s: &mut Session, sid: StoryId, baseline: &ParaFormat) {
    let before = story(s, sid).clone();
    assert_eq!(before.text, "A\n");
    select(s, sid, 2, 2);
    run(s, "text.insert", json!({"text": "é", "raw": true}));
    select(s, sid, 2, 4);
    run(s, "type.char", json!({"attrs": {"size": null, "leading": null}}));
    let attrs = run(s, "type.selectionAttrs", json!({}));
    assert_eq!(attrs["chars"]["size"], 24.0, "clearing typed overrides exposes the genuine paragraph defaults");
    assert_eq!(attrs["chars"]["leading"], json!({"kind": "points", "value": 26.0}));
    assert_eq!(attrs["charOverrides"], 0);
    assert_eq!(story(s, sid).paras.last(), Some(baseline));
    assert_eq!(story(s, sid).text, "A\né");
    for at in 0..2 {
        assert_eq!(story(s, sid).format_after(at), before.format_after(at), "the prefix and LF keep their formatting");
    }
    assert_eq!(before.paras[1].eof_inherited_chars.as_ref(), Some(&baseline.chars));
    assert_eq!(before.paras[1].chars.size, Some(15.0), "the surviving LF sets typing overrides");
    assert_eq!(before.paras[1].chars.leading, Some(Leading::Points(18.0)));
    s.doc().unwrap().doc.check().unwrap();
}

fn observe(s: &mut Session, sid: StoryId, baseline: &ParaFormat) {
    let copies = native_copies(s);
    type_and_clear(s, sid, baseline);
    for mut copy in copies {
        type_and_clear(&mut copy, sid, baseline);
    }
}

#[test]
fn r24_d07_empty_variable_delete_preserves_eof_defaults() {
    let (mut s, sid, baseline) = fixture();
    variable(&mut s, "Empty", "");
    assert_eq!(story(&s, sid).paras.last(), Some(&baseline));
    edit(&mut s, "variables.delete", json!({"name": "Empty"}));
    assert!(s.doc().unwrap().doc.variable("Empty").is_none());
    observe(&mut s, sid, &baseline);
}

#[test]
fn r24_d08_placeholder_remove_preserves_eof_defaults() {
    let (mut s, sid, baseline) = fixture();
    source(&mut s, "Field\nx\n");
    let id = placeholder(&mut s, sid);
    assert_eq!(story(&s, sid).paras.last(), Some(&baseline));
    edit(&mut s, "data.placeholder.remove", json!({"id": id}));
    assert!(s.doc().unwrap().doc.data_merge.placeholders.is_empty());
    observe(&mut s, sid, &baseline);
}

#[test]
fn r24_d13_empty_merge_preserves_defaults_preview_and_template() {
    for registered in [true, false] {
        let (mut s, sid, baseline) = fixture();
        source(&mut s, "Field,Keep\n,x\né,x\n");
        if registered {
            placeholder(&mut s, sid);
        } else {
            run(&mut s, "text.insert", json!({"text": "<<Field>>", "raw": true}));
        }
        let marker_format = story(&s, sid).format_after(2).clone();
        assert_eq!(story(&s, sid).paras.last(), Some(&baseline));
        let template = document(&s);
        let depth = s.doc().unwrap().history.undo.len();
        run(&mut s, "data.preview", json!({"record": 1}));
        assert_eq!(story(&s, sid).text, "A\n");
        run(&mut s, "data.preview", json!({"record": 2}));
        assert_eq!(story(&s, sid).text, "A\né");
        assert_eq!(story(&s, sid).format_after(2), &marker_format);
        assert_eq!(story(&s, sid).paras.last(), Some(&baseline));
        run(&mut s, "data.preview.stop", json!({}));
        assert_eq!(document(&s), template);
        assert_eq!(s.doc().unwrap().history.undo.len(), depth);
        // Merge itself must restore a live preview before copying the template.
        run(&mut s, "data.preview", json!({"record": 1}));
        let merged = run(&mut s, "data.merge", json!({}));
        assert_eq!(merged["records"], 2);
        assert_eq!(merged["pages"], 2);
        assert_eq!(s.documents().len(), 2);
        assert_eq!(s.active_index(), Some(1));
        assert_eq!(serde_json::to_value(&*s.documents()[0].doc).unwrap(), template);
        assert_eq!(s.documents()[0].history.undo.len(), depth);
        assert!(s.documents()[0].preview_stash.is_none());
        assert!(s.doc().unwrap().history.undo.is_empty());
        assert!(s.doc().unwrap().doc.data_merge.placeholders.is_empty());
        let second = s.doc().unwrap().doc.stories.values().find(|st| st.id != sid && st.text == "A\né").unwrap();
        assert_eq!(second.format_after(2), &marker_format, "nonempty replacement keeps the marker's full format");
        assert_eq!(second.paras.last(), Some(&baseline));
        observe(&mut s, sid, &baseline);
    }
}

#[test]
fn r24_data_nonempty_variable_and_higher_indices_keep_formats() {
    let (mut s, sid, baseline) = fixture();
    variable(&mut s, "Value", "é");
    variable(&mut s, "Keep", "keep");
    let before = story(&s, sid).clone();
    let value = s.doc().unwrap().doc.variable("Value").unwrap();
    let keep = s.doc().unwrap().doc.variable("Keep").unwrap();
    let old_marker = designcraft_doc::vars::var_char(value).unwrap();
    let keep_at = 2 + old_marker.len_utf8();
    edit(&mut s, "variables.delete", json!({"name": "Value"}));
    let new_keep = s.doc().unwrap().doc.variable("Keep").unwrap();
    assert_eq!(new_keep, keep - 1);
    let marker = designcraft_doc::vars::var_char(new_keep).unwrap();
    assert_eq!(story(&s, sid).text, format!("A\né{marker}"));
    assert_eq!(story(&s, sid).format_after(2), before.format_after(2));
    assert_eq!(story(&s, sid).format_after(4), before.format_after(keep_at));
    assert_eq!(story(&s, sid).paras, before.paras);
    assert_eq!(story(&s, sid).paras.last(), Some(&baseline));
}

#[test]
fn r24_data_removing_duplicate_placeholder_shifts_ranges_and_links() {
    let (mut s, sid, baseline) = fixture();
    source(&mut s, "Field\nx\n");
    let first = placeholder(&mut s, sid);
    let removed = placeholder(&mut s, sid);
    let width = "<<Field>>".len();
    select(&mut s, sid, 2 + width, 2 + 2 * width);
    run(&mut s, "hyperlink.create", json!({"url": "https://example.com"}));
    let before = story(&s, sid).clone();
    edit(&mut s, "data.placeholder.remove", json!({"id": removed}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.data_merge.placeholders.len(), 1);
    assert_eq!(d.data_merge.placeholders[0].id, first);
    assert_eq!(d.data_merge.placeholders[0].anchor, PlaceholderAnchor::Text { story: sid, start: 2, end: 2 + width });
    assert_eq!(d.hyperlinks[0].source, HyperlinkSource::Text { story: sid, start: 2, end: 2 + width });
    assert_eq!(story(&s, sid).text, "A\n<<Field>>");
    assert_eq!(story(&s, sid).format_after(2), before.format_after(2 + width));
    assert_eq!(story(&s, sid).paras.last(), Some(&baseline));
}

#[test]
fn r24_data_outer_deletions_preserve_selected_note_and_cell() {
    for note in [false, true] {
        for variable_case in [false, true] {
            let (mut s, sid, baseline) = fixture();
            let (command, params) = if variable_case {
                variable(&mut s, "Empty", "");
                ("variables.delete", json!({"name": "Empty"}))
            } else {
                source(&mut s, "Field\nx\n");
                let id = placeholder(&mut s, sid);
                ("data.placeholder.remove", json!({"id": id}))
            };
            let parent = frame(&mut s, "");
            select(&mut s, parent, 0, 0);
            if note {
                run(&mut s, "footnote.insert", json!({"text": "Child\n"}));
            } else {
                run(&mut s, "table.insert", json!({"rows": 1, "cols": 1, "width": 300}));
                run(&mut s, "text.insert", json!({"text": "Child\n", "raw": true}));
            }
            let selection = s.doc().unwrap().selection.clone();
            assert!(selection.text.unwrap().cell.is_some());
            let before = story(&s, parent).clone();
            edit(&mut s, command, params);
            assert_eq!(story(&s, parent), &before, "the child and its containing outer story stay unchanged");
            assert_eq!(s.doc().unwrap().selection, selection);
            type_and_clear(&mut s, sid, &baseline);
        }
    }
}

#[test]
fn r24_data_interior_deletions_preserve_existing_blank_eof_stage() {
    for variable_case in [false, true] {
        let (mut s, sid, _) = fixture();
        let (command, params) = if variable_case {
            variable(&mut s, "Empty", "");
            ("variables.delete", json!({"name": "Empty"}))
        } else {
            source(&mut s, "Field\nx\n");
            let id = placeholder(&mut s, sid);
            ("data.placeholder.remove", json!({"id": id}))
        };
        let end = story(&s, sid).len();
        select(&mut s, sid, end, end);
        run(&mut s, "text.insert", json!({"text": "\n", "raw": true}));
        run(&mut s, "type.char", json!({"attrs": {"size": 28, "leading": {"kind": "points", "value": 34}}}));
        let before = story(&s, sid).clone();
        edit(&mut s, command, params);
        assert_eq!(story(&s, sid).text, "A\n\n");
        assert_eq!(story(&s, sid).paras.last(), before.paras.last());
        assert_eq!(story(&s, sid).format_after(2), before.format_after(before.len() - 1));
    }
}

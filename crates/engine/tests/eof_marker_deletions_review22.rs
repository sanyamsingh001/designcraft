//! Removing generated markers preserves the surviving paragraph defaults.

use designcraft_doc::{CellAddr, ENDNOTE_REF, FOOTNOTE_REF, ItemId, Leading, NOTE_MARK, OBJECT_MARK, ParaFormat, StoryId, TextSel};
use designcraft_engine::Session;
use serde_json::{Value, json};

fn document(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn select(s: &mut Session, sid: StoryId, anchor: usize, focus: usize) {
    s.execute("text.select", &json!({"story": sid.0, "anchor": anchor, "focus": focus})).unwrap();
}

fn frame(s: &mut Session, text: &str, y: f64) -> StoryId {
    let result = s.execute("frame.create", &json!({"rect": [40.0, y, 440.0, y + 180.0], "content": "text", "text": text})).unwrap();
    StoryId(result["story"].as_u64().unwrap())
}

fn edit(s: &mut Session, command: &str, params: Value) -> Value {
    let before = document(s);
    let depth = s.doc().unwrap().history.undo.len();
    let journal = s.journal.len();
    let result = s.execute(command, &params).unwrap();
    let after = document(s);
    let selection = s.doc().unwrap().selection.clone();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "one public edit owns one history entry");
    assert_eq!(&s.journal[journal..], &[(command.to_string(), params)], "retain command identity without nested dispatch");
    s.doc().unwrap().doc.check().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(document(s), before, "Undo restores all text, formats and marker metadata");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(document(s), after, "Redo restores the complete document");
    assert_eq!(s.doc().unwrap().selection, selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    s.doc().unwrap().doc.check().unwrap();
    result
}

fn native_copies(s: &mut Session) -> [Session; 2] {
    let expected = document(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let zip = designcraft_engine::cmd::base64_decode(saved["base64"].as_str().unwrap());
    assert!(zip.starts_with(b"PK"));
    let legacy = serde_json::to_vec(&*s.doc().unwrap().doc).unwrap();
    [zip, legacy].map(|bytes| {
        let mut back = Session::new();
        back.execute("file.openBytes", &json!({"base64": designcraft_engine::cmd::base64_encode(&bytes)})).unwrap();
        assert_eq!(document(&back), expected, "native ZIP and legacy JSON preserve the full document");
        back.doc().unwrap().doc.check().unwrap();
        back
    })
}

fn metrics(s: &mut Session, sid: StoryId, start: usize, end: usize, size: f64, leading: Leading) -> Value {
    select(s, sid, start, end);
    let attrs = s.execute("type.selectionAttrs", &json!({})).unwrap();
    assert_eq!(attrs["chars"]["size"], json!(size));
    assert_eq!(attrs["chars"]["leading"], serde_json::to_value(leading).unwrap());
    attrs
}

fn fixture(text: &str, blank_eof: bool, inherited: bool) -> (Session, StoryId, ParaFormat) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let sid = frame(&mut s, text, 40.0);
    select(&mut s, sid, 0, text.len());
    if inherited {
        s.execute("style.paragraph.create", &json!({"name": "Review22 baseline", "chars": {"size": 24, "leading": {"kind": "points", "value": 26}}}))
            .unwrap();
        s.execute("style.paragraph.apply", &json!({"name": "Review22 baseline"})).unwrap();
        s.execute("style.breakLink", &json!({"kind": "paragraph"})).unwrap();
    }
    let baseline = s.doc().unwrap().doc.story(sid).unwrap().paras[0].clone();
    assert!(baseline.eof_inherited_chars.is_none());
    if inherited {
        assert_eq!(baseline.chars.size, Some(24.0));
        assert_eq!(baseline.chars.leading, Some(Leading::Points(26.0)));
    } else {
        assert!(baseline.chars.is_empty());
    }
    s.execute("style.character.create", &json!({"name": "Review22 live", "chars": {"underline": true}})).unwrap();
    s.execute("style.character.apply", &json!({"name": "Review22 live"})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 15, "leading": {"kind": "points", "value": 18}}})).unwrap();
    if blank_eof {
        select(&mut s, sid, text.len(), text.len());
        s.execute("text.insert", &json!({"text": "\n", "raw": true})).unwrap();
        s.execute("type.char", &json!({"attrs": {"size": 20, "leading": {"kind": "points", "value": 30}}})).unwrap();
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, format!("{text}\n"));
        assert_eq!(story.format_after(text.len()).over.size, Some(15.0));
        assert_eq!(story.format_after(text.len()).over.leading, Some(Leading::Points(18.0)));
        assert_eq!(story.paras[0], baseline);
        assert_eq!(story.paras[1].chars.size, Some(20.0));
        assert_eq!(story.paras[1].chars.leading, Some(Leading::Points(30.0)));
        assert_eq!(story.paras[1].eof_inherited_chars.as_ref(), inherited.then_some(&baseline.chars));
    }
    (s, sid, baseline)
}

fn type_and_clear(s: &mut Session, sid: StoryId, baseline: &ParaFormat, size: f64, leading: Leading) {
    let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
    let at = before.len();
    metrics(s, sid, at, at, size, leading);
    edit(s, "text.insert", json!({"text": "é", "raw": true}));
    let typed = metrics(s, sid, at, at + "é".len(), size, leading);
    assert_eq!(typed["characterStyle"], json!(before.char_format_at(at).style));
    edit(s, "type.char", json!({"attrs": {"size": null, "leading": null}}));
    let expected = s.doc().unwrap().doc.styles.resolve_para(baseline).1;
    let cleared = metrics(s, sid, at, at + "é".len(), expected.size, expected.leading);
    assert_eq!(cleared["charOverrides"], 0);
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.text, format!("{}é", before.text));
    assert_eq!(story.paras.last(), Some(baseline), "the populated paragraph retains its genuine defaults");
    for (offset, _) in before.text.char_indices() {
        assert_eq!(story.format_after(offset), before.format_after(offset), "typing/clearing preserves every prior byte format");
    }
    assert_eq!(story.anchors, before.anchors);
    assert_eq!(story.xrefs, before.xrefs);
    assert_eq!(story.editorial, before.editorial);
    assert_eq!(story.notes, before.notes);
    assert_eq!(story.endnotes, before.endnotes);
    assert_eq!(story.objects, before.objects);
    if before.char_format_at(at).style == "Review22 live" {
        assert_eq!(cleared["chars"]["underline"], true, "the independent named character style stays live");
    }
    native_copies(s);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Footnote,
    Endnote,
    Object,
    Note,
}

const MARKERS: [Marker; 4] = [Marker::Footnote, Marker::Endnote, Marker::Object, Marker::Note];

impl Marker {
    fn mark(self) -> char {
        match self {
            Self::Footnote => FOOTNOTE_REF,
            Self::Endnote => ENDNOTE_REF,
            Self::Object => OBJECT_MARK,
            Self::Note => NOTE_MARK,
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Footnote => "footnote.delete",
            Self::Endnote => "endnote.delete",
            Self::Object => "anchored.release",
            Self::Note => "note.convertToText",
        }
    }
}

fn insert_marker(s: &mut Session, sid: StoryId, at: usize, kind: Marker, payload: &str) -> Value {
    select(s, sid, at, at);
    let result = match kind {
        Marker::Footnote => s.execute("footnote.insert", &json!({"text": payload})).unwrap(),
        Marker::Endnote => s.execute("endnote.insert", &json!({"text": payload})).unwrap(),
        Marker::Note => s.execute("note.new", &json!({"text": payload, "author": "Review22 李"})).unwrap(),
        Marker::Object => {
            let index = s.doc().unwrap().doc.story(sid).unwrap().text[..at].matches(OBJECT_MARK).count();
            let item = s.execute("frame.create", &json!({"rect": [460, 420, 484, 454]})).unwrap();
            s.execute("anchored.insert", &json!({"ids": [item["id"]], "story": sid.0, "pos": at, "position": "inline", "yOffset": 2})).unwrap();
            return json!({"story": sid.0, "index": index});
        }
    };
    json!({"story": sid.0, "id": result["id"]})
}

// These fixtures target the last marker of its kind; preceding payloads must survive exactly.
fn remove_marker(s: &mut Session, sid: StoryId, kind: Marker, params: Value, replacement: &str) {
    let before = s.doc().unwrap().doc.clone();
    let prior = before.story(sid).unwrap();
    let at = prior.text.rfind(kind.mark()).unwrap();
    let end = at + kind.mark().len_utf8();
    let mut expected_selection = s.doc().unwrap().selection.clone();
    if kind == Marker::Footnote
        && expected_selection.text.is_some_and(|t| t.story == sid && t.cell == Some(CellAddr::footnote(params["id"].as_u64().unwrap())))
    {
        expected_selection.text = Some(TextSel::caret(sid, at));
    }
    let result = edit(s, kind.command(), params);
    let state = s.doc().unwrap();
    let after = state.doc.story(sid).unwrap();
    assert_eq!(after.text, format!("{}{}{}", &prior.text[..at], replacement, &prior.text[end..]));
    for (offset, _) in prior.text.char_indices().filter(|(offset, _)| *offset < at || *offset >= end) {
        let new_offset = if offset < at { offset } else { offset - kind.mark().len_utf8() + replacement.len() };
        assert_eq!(after.format_after(new_offset), prior.format_after(offset), "surviving UTF-8 runs stay intact");
    }
    for (offset, _) in replacement.char_indices() {
        assert_eq!(after.format_after(at + offset), prior.format_after(at), "converted text retains its marker's format");
    }
    assert_eq!(after.notes.as_slice(), &prior.notes[..prior.notes.len() - usize::from(kind == Marker::Footnote)]);
    assert_eq!(after.endnotes.as_slice(), &prior.endnotes[..prior.endnotes.len() - usize::from(kind == Marker::Endnote)]);
    assert_eq!(after.objects.as_slice(), &prior.objects[..prior.objects.len() - usize::from(kind == Marker::Object)]);
    assert_eq!(after.editorial.as_slice(), &prior.editorial[..prior.editorial.len() - usize::from(kind == Marker::Note)]);
    assert_eq!(after.anchors, prior.anchors);
    assert_eq!(after.xrefs, prior.xrefs);
    assert_eq!(after.tables, prior.tables);
    for (other, story) in &before.stories {
        if kind == Marker::Endnote && Some(*other) == before.endnote_story {
            // Removing an endnote refreshes its generated text while retaining the frame.
            let generated = state.doc.story(*other).unwrap();
            let expected_text = if after.endnotes.is_empty() { "Endnotes" } else { "Endnotes\n1\tKeep é body" };
            assert!(after.endnotes.len() <= 1);
            assert_eq!(generated.text, expected_text);
            assert_eq!(generated.id, story.id);
            assert_eq!(generated.frames, story.frames);
            assert_eq!(generated.rev, story.rev.wrapping_add(1));
        } else if *other != sid {
            assert_eq!(state.doc.story(*other), Some(story.as_ref()), "unrelated stories stay intact");
        }
    }
    assert_eq!(state.doc.endnote_story, before.endnote_story, "retain the existing endnote-frame lifecycle");
    assert_eq!(state.doc.footnote_options, before.footnote_options);
    assert_eq!(state.doc.endnote_options, before.endnote_options);
    if kind == Marker::Object {
        let id = ItemId(result["id"].as_u64().unwrap());
        let original = &prior.objects.last().unwrap().item;
        let mut released = state.doc.item(id).unwrap().clone();
        assert_ne!(id, original.id, "release keeps its existing fresh page-item identity");
        assert!(state.doc.item(original.id).is_none());
        released.id = original.id;
        released.xf = original.xf;
        assert_eq!(&released, original, "only page identity and placement change, not item payload");
        assert_eq!(state.selection, designcraft_doc::Selection::items(vec![id]));
    } else {
        assert_eq!(state.selection, expected_selection, "retain each command's existing selection policy");
    }
}

fn final_marker_case(kind: Marker) {
    let (mut s, sid, baseline) = fixture("éA", true, true);
    frame(&mut s, "Unrelated Ω", 300.0);
    insert_marker(&mut s, sid, 0, kind, "Keep é\nbody");
    let at = s.doc().unwrap().doc.story(sid).unwrap().len();
    let payload = if kind == Marker::Note { "" } else { "Remove 李\nbody" };
    let params = insert_marker(&mut s, sid, at, kind, payload);
    let story = s.doc().unwrap().doc.story(sid).unwrap();
    assert_eq!(story.format_after(at).over.size, Some(20.0));
    assert_eq!(story.format_after(at).over.leading, Some(Leading::Points(30.0)));
    assert_eq!(story.paras.last(), Some(&baseline), "marker insertion has already restored genuine 24/26 defaults");
    let [zip, legacy] = native_copies(&mut s);
    for mut back in [s, zip, legacy] {
        remove_marker(&mut back, sid, kind, params.clone(), "");
        let story = back.doc().unwrap().doc.story(sid).unwrap();
        assert!(story.text.ends_with('\n'));
        assert_eq!(story.paras.last().unwrap().chars.size, Some(15.0));
        assert_eq!(story.paras.last().unwrap().chars.leading, Some(Leading::Points(18.0)));
        assert_eq!(story.paras.last().unwrap().eof_inherited_chars.as_ref(), Some(&baseline.chars));
        let [zip, _legacy] = native_copies(&mut back);
        for mut follow in [back, zip] {
            type_and_clear(&mut follow, sid, &baseline, 15.0, Leading::Points(18.0));
        }
    }
}

#[test]
fn r22_marker_d01_footnote_final_delete_native_and_history() {
    final_marker_case(Marker::Footnote);
}

#[test]
fn r22_marker_d02_endnote_final_delete_native_and_history() {
    final_marker_case(Marker::Endnote);
}

#[test]
fn r22_marker_d03_anchored_final_release_native_payload_and_history() {
    final_marker_case(Marker::Object);
}

#[test]
fn r22_marker_d04_empty_note_conversion_native_and_history() {
    final_marker_case(Marker::Note);
}

fn ordinary_case(text: &str, before_last_byte: bool) {
    for kind in MARKERS {
        let (mut s, sid, baseline) = fixture(text, false, true);
        let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let at = prior.len() - usize::from(before_last_byte);
        let payload = if kind == Marker::Note { "" } else { "Ordinary é\npayload" };
        let params = insert_marker(&mut s, sid, at, kind, payload);
        remove_marker(&mut s, sid, kind, params, "");
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, prior.text);
        assert_eq!(story.chars, prior.chars);
        assert_eq!(story.paras, prior.paras, "no new blank EOF means no paragraph-format transition");
        native_copies(&mut s);
        type_and_clear(&mut s, sid, &baseline, 15.0, Leading::Points(18.0));
    }
}

#[test]
fn r22_marker_ordinary_non_lf_eof_controls() {
    ordinary_case("éA", false);
}

#[test]
fn r22_marker_following_b_nonempty_paragraph_controls() {
    ordinary_case("éA\nB", true);
}

#[test]
fn r22_marker_prefix_removal_preserves_an_existing_eof_stage() {
    for kind in MARKERS {
        let (mut s, sid, baseline) = fixture("éA", true, true);
        let prior = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let payload = if kind == Marker::Note { "" } else { "Prefix é\npayload" };
        let params = insert_marker(&mut s, sid, 0, kind, payload);
        remove_marker(&mut s, sid, kind, params, "");
        let story = s.doc().unwrap().doc.story(sid).unwrap();
        assert_eq!(story.text, prior.text);
        assert_eq!(story.chars, prior.chars);
        assert_eq!(story.paras, prior.paras, "a surviving final LF keeps its 20/30 stage");
        native_copies(&mut s);
        type_and_clear(&mut s, sid, &baseline, 20.0, Leading::Points(30.0));
    }
}

#[test]
fn r22_marker_final_removal_without_genuine_paragraph_defaults() {
    for kind in MARKERS {
        let (mut s, sid, baseline) = fixture("éA", true, false);
        let at = s.doc().unwrap().doc.story(sid).unwrap().len();
        let payload = if kind == Marker::Note { "" } else { "Default é\npayload" };
        let params = insert_marker(&mut s, sid, at, kind, payload);
        remove_marker(&mut s, sid, kind, params, "");
        assert!(s.doc().unwrap().doc.story(sid).unwrap().paras.last().unwrap().eof_inherited_chars.is_none());
        native_copies(&mut s);
        type_and_clear(&mut s, sid, &baseline, 15.0, Leading::Points(18.0));
    }
}

#[test]
fn r22_marker_note_conversion_nonempty_literal_and_lf_payloads() {
    for payload in ["é", "\"literal\" é\nZ", "\n", "é\n"] {
        for following_b in [false, true] {
            let (mut s, sid, baseline) = fixture("éA", true, true);
            insert_marker(&mut s, sid, 0, Marker::Note, "Keep 李");
            let at = s.doc().unwrap().doc.story(sid).unwrap().len();
            let params = insert_marker(&mut s, sid, at, Marker::Note, payload);
            if following_b {
                let end = s.doc().unwrap().doc.story(sid).unwrap().len();
                select(&mut s, sid, end, end);
                s.execute("text.insert", &json!({"text": "B", "raw": true})).unwrap();
            }
            remove_marker(&mut s, sid, Marker::Note, params, payload);
            let story = s.doc().unwrap().doc.story(sid).unwrap();
            assert_eq!(story.editorial.len(), 1);
            assert_eq!(story.editorial[0].text, "Keep 李");
            assert_eq!(story.editorial[0].author, "Review22 李");
            if payload.ends_with('\n') && !following_b {
                assert_eq!(story.paras.last().unwrap().eof_inherited_chars.as_ref(), Some(&baseline.chars));
                assert_eq!(story.paras.last().unwrap().chars.size, Some(20.0));
            } else {
                assert_eq!(story.paras.last(), Some(&baseline));
            }
            let [zip, _legacy] = native_copies(&mut s);
            for mut back in [s, zip] {
                type_and_clear(&mut back, sid, &baseline, 20.0, Leading::Points(30.0));
            }
        }
    }
}

#[test]
fn r22_marker_missing_targets_are_atomic() {
    for kind in MARKERS {
        let (mut s, sid, _) = fixture("éA", true, true);
        let at = s.doc().unwrap().doc.story(sid).unwrap().len();
        insert_marker(&mut s, sid, at, kind, "");
        let before = document(&s);
        let selection = s.doc().unwrap().selection.clone();
        let depth = s.doc().unwrap().history.undo.len();
        let journal = s.journal.clone();
        let params = if kind == Marker::Object { json!({"story": sid.0, "index": 9999}) } else { json!({"story": sid.0, "id": 9999}) };
        assert!(s.execute(kind.command(), &params).is_err());
        assert_eq!(document(&s), before);
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(s.doc().unwrap().history.undo.len(), depth);
        assert_eq!(s.journal, journal);
    }
}

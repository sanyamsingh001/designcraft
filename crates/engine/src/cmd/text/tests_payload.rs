//! Public Session regressions for formatted marker payloads across physical drag and clipboard.
//! full_doc is the complete serialized Session document, rather than an inspect preview.

use designcraft_doc::{ItemId, SpreadRef, Story, StoryId};
use designcraft_geom::Point;
use designcraft_tools::{Mods, PointerEvent, PointerKind};
use serde_json::{Value, json};

use crate::{Session, ViewInfo};

#[derive(Clone, Copy, Debug)]
enum Kind {
    Endnote,
    Editorial,
    Footnote,
    Ordinary,
}

impl Kind {
    fn marker(self) -> Option<char> {
        match self {
            Self::Endnote => Some(designcraft_doc::ENDNOTE_REF),
            Self::Editorial => Some(designcraft_doc::NOTE_MARK),
            Self::Footnote => Some(designcraft_doc::FOOTNOTE_REF),
            Self::Ordinary => None,
        }
    }

    fn insert(self, s: &mut Session, text: &str, author: &str) {
        match self {
            Self::Endnote => {
                s.execute("endnote.insert", &json!({"text": text})).unwrap();
            }
            Self::Editorial => {
                s.execute("note.new", &json!({"text": text, "author": author})).unwrap();
            }
            Self::Footnote => {
                s.execute("footnote.insert", &json!({"text": text})).unwrap();
                s.execute("footnote.goToReference", &json!({})).unwrap();
            }
            Self::Ordinary => {}
        }
    }

    fn payloads(self, st: &Story) -> Vec<Value> {
        match self {
            Self::Endnote => st.endnotes.iter().map(|n| json!({"id": n.id, "story": n.text})).collect(),
            Self::Editorial => st.editorial.iter().map(|n| json!({"id": n.id, "text": n.text, "author": n.author})).collect(),
            Self::Footnote => st.notes.iter().map(|n| json!({"id": n.id, "story": n.text})).collect(),
            Self::Ordinary => vec![],
        }
    }

    fn list(self, s: &mut Session, sid: StoryId) -> Vec<Value> {
        let command = match self {
            Self::Endnote => "endnote.list",
            Self::Editorial => "note.list",
            Self::Footnote => "footnote.list",
            Self::Ordinary => return vec![],
        };
        s.execute(command, &json!({})).unwrap().as_array().unwrap().iter().filter(|n| n["story"] == json!(sid.0)).cloned().collect()
    }
}

const PAYLOAD: &str = "Keep this citation/comment — Café 李";
const CONTROL: &str = "Unaffected reference";
const AUTHOR: &str = "Reviewer 李";

fn fixture(kind: Kind) -> (Session, ItemId, StoryId, usize) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"pages": 1})).unwrap();
    let f = s.execute("frame.create", &json!({"rect": [40, 40, 440, 300], "content": "text", "text": "A B C"})).unwrap();
    let fid = ItemId(f["id"].as_u64().unwrap());
    let sid = StoryId(f["story"].as_u64().unwrap());
    s.execute("endnote.options", &json!({"startAt": 7, "prefix": "(", "suffix": ")", "heading": "Sources", "separator": " — "})).unwrap();
    s.execute("footnote.options", &json!({"style": "lowerRoman", "rule": {"width": 100.0}})).unwrap();
    s.execute("text.select", &json!({"story": sid.0, "anchor": 1})).unwrap();
    kind.insert(&mut s, PAYLOAD, AUTHOR);
    let selected_end = 1 + kind.marker().map_or(0, char::len_utf8);
    let len = s.doc().unwrap().doc.story(sid).unwrap().len();
    s.execute("text.select", &json!({"story": sid.0, "anchor": len})).unwrap();
    kind.insert(&mut s, CONTROL, "Control author");
    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": selected_end})).unwrap();
    s.execute("type.char", &json!({"attrs": {"size": 18, "tracking": 31, "change": "inserted"}})).unwrap();
    s.set_tool("type");
    (s, fid, sid, selected_end)
}

fn full_doc(s: &Session) -> Value {
    serde_json::to_value(&*s.doc().unwrap().doc).unwrap()
}

fn zip_oracle(s: &mut Session) {
    let full = full_doc(s);
    let saved = s.execute("file.serialize", &json!({})).unwrap();
    let bytes = crate::cmd::file::base64_decode(saved["base64"].as_str().unwrap());
    assert!(bytes.starts_with(b"PK"));
    let mut back = Session::new();
    back.execute("file.openBytes", &json!({"base64": saved["base64"]})).unwrap();
    assert_eq!(full_doc(&back), full, "native ZIP retains every payload, id, option and format");
    back.doc().unwrap().doc.check().unwrap();
}

fn pointer(s: &mut Session, kind: PointerKind, spread: Point, alt: bool) {
    let canvas = s.layout().xf(SpreadRef::Doc(0)) * spread;
    let event = PointerEvent::new(kind, canvas.x, canvas.y).with_mods(Mods { alt, ..Default::default() });
    s.pointer(&event, ViewInfo::at_zoom(1.0)).unwrap();
}

fn physical_drag(s: &mut Session, fid: ItemId, sid: StoryId, selected_end: usize, copy: bool) {
    let (press, drop) = {
        let d = &s.doc().unwrap().doc;
        let cs = s.cache.get(d, sid, None);
        let ft = cs.frame(fid).unwrap();
        assert_eq!(ft.lines.len(), 1, "single line gives an unambiguous public drop endpoint");
        let l = &ft.lines[0];
        let g = l.glyphs.iter().find(|g| g.byte == 0 && g.len > 0).unwrap();
        assert!(g.adv > 0.0, "A is a real composed glyph");
        let y = l.baseline - l.ascent * 0.5;
        let press = Point::new(g.x + g.adv * 0.25, y);
        let drop = Point::new(l.end_x + 12.0, y);
        assert_eq!(designcraft_compose::hit(&cs, 0, press), Some(0));
        assert_eq!(designcraft_compose::hit(&cs, 0, drop), Some(d.story(sid).unwrap().len()));
        let xf = d.parent_xf(&d.find(fid).unwrap()) * d.text_xf(d.item(fid).unwrap());
        (xf * press, xf * drop)
    };
    let original = full_doc(s);
    pointer(s, PointerKind::Down, press, copy);
    assert_eq!(
        s.doc().unwrap().selection.text.unwrap().range(),
        0..selected_end,
        "press inside the selected A must arm a move without collapsing it"
    );
    assert_eq!(full_doc(s), original);
    pointer(s, PointerKind::Drag, drop, copy);
    assert_eq!(s.doc().unwrap().selection.text.unwrap().range(), 0..selected_end);
    assert_eq!(full_doc(s), original, "drag preview leaves source data intact until release");
    pointer(s, PointerKind::Up, drop, copy);
}

fn payload_oracle(s: &mut Session, sid: StoryId, kind: Kind, before: &Story, copy: bool) {
    let expected_text = match kind.marker() {
        Some(m) if copy => format!("A{m} B C{m}A{m}"),
        Some(m) => format!(" B C{m}A{m}"),
        None if copy => "A B CA".into(),
        None => " B CA".into(),
    };
    let after = s.doc().unwrap().doc.story(sid).unwrap().clone();
    assert_eq!(after.text, expected_text);
    let new_start = after.len() - (1 + kind.marker().map_or(0, char::len_utf8));
    assert_eq!(after.format_after(new_start), before.format_after(0), "moved/copied run style, overrides and change mark survive");
    let payloads = kind.payloads(&after);
    let original = kind.payloads(before);
    if kind.marker().is_some() {
        assert_eq!(payloads.len(), if copy { 3 } else { 2 });
        let control = usize::from(copy);
        assert_eq!(payloads[control], original[1], "the unselected reference and its identity remain exact");
        if copy {
            assert_eq!(payloads[0], original[0], "the original copy source remains exact");
        }
        let mut fresh = payloads.last().unwrap().clone();
        let fresh_id = fresh["id"].clone();
        assert_eq!(fresh_id, json!(3), "use the fresh destination slot, consistent with footnote copies");
        fresh["id"] = original[0]["id"].clone();
        assert_eq!(fresh, original[0], "copy all payload data while changing only its outer id");
        let list = kind.list(s, sid);
        assert_eq!(list.len(), payloads.len());
        assert_eq!(list[control]["text"], CONTROL);
        assert_eq!(list.last().unwrap()["text"], PAYLOAD, "public payload query must expose retained text");
        assert_eq!(list.last().unwrap()["id"], fresh_id);
        if matches!(kind, Kind::Editorial) {
            assert_eq!(list.last().unwrap()["author"], AUTHOR);
        }
        if matches!(kind, Kind::Endnote) {
            let d = &s.doc().unwrap().doc;
            let generated = &d.story(d.endnote_story.unwrap()).unwrap().text;
            assert!(generated.contains(PAYLOAD), "the public endnote frame must still render the citation");
            assert!(generated.contains(CONTROL));
            assert_eq!(generated.matches(PAYLOAD).count(), if copy { 2 } else { 1 });
        }
    }
    s.doc().unwrap().doc.check().unwrap();
}

fn unchanged_options(s: &Session, full: &Value) {
    let now = full_doc(s);
    assert_eq!(now["endnoteOptions"], full["endnoteOptions"]);
    assert_eq!(now["footnoteOptions"], full["footnoteOptions"]);
    assert_eq!(now["styles"], full["styles"]);
}

#[test]
fn r8_physical_selected_move_and_alt_copy_keep_endnotes_editorial_and_controls() {
    for kind in [Kind::Endnote, Kind::Editorial, Kind::Footnote, Kind::Ordinary] {
        for copy in [false, true] {
            for transformed in [false, true] {
                let (mut s, fid, sid, end) = fixture(kind);
                if transformed {
                    s.execute("transform.set", &json!({"ids": [fid.0], "ref": 4, "rotation": 23.0, "shear": 9.0})).unwrap();
                    s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": end})).unwrap();
                }
                let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
                let full = full_doc(&s);
                let undo = s.doc().unwrap().history.undo.len();
                physical_drag(&mut s, fid, sid, end, copy);
                payload_oracle(&mut s, sid, kind, &before, copy);
                unchanged_options(&s, &full);
                assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one physical gesture is one history entry");
                let after = full_doc(&s);
                zip_oracle(&mut s);
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(full_doc(&s), full, "undo restores the entire original document");
                s.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(full_doc(&s), after);
                payload_oracle(&mut s, sid, kind, &before, copy);
                if copy && kind.marker().is_some() {
                    let original = kind.payloads(s.doc().unwrap().doc.story(sid).unwrap());
                    let id = original.last().unwrap()["id"].clone();
                    let command = match kind {
                        Kind::Endnote => "endnote.edit",
                        Kind::Editorial => "note.edit",
                        Kind::Footnote => "footnote.setText",
                        Kind::Ordinary => panic!("ordinary control must not enter note-marker assertions"),
                    };
                    s.execute(command, &json!({"story": sid.0, "id": id, "text": "Edited duplicate", "author": "Copy author"})).unwrap();
                    let edited = kind.payloads(s.doc().unwrap().doc.story(sid).unwrap());
                    assert_eq!(
                        edited[..2],
                        original[..2],
                        "editing a duplicate through the public command must preserve source/control payloads and ids"
                    );
                    assert_ne!(edited[2], original[2]);
                    zip_oracle(&mut s);
                    s.execute("edit.undo", &json!({})).unwrap();
                    assert_eq!(full_doc(&s), after);
                }
            }
        }
    }
}

#[test]
fn r8_formatted_cut_paste_keeps_payloads_and_public_undo_zip_oracles() {
    for kind in [Kind::Endnote, Kind::Editorial, Kind::Footnote, Kind::Ordinary] {
        let (mut s, _, sid, _) = fixture(kind);
        let before = s.doc().unwrap().doc.story(sid).unwrap().clone();
        let full = full_doc(&s);
        let cut = s.execute("edit.cut", &json!({})).unwrap();
        assert_eq!(cut["text"], "A", "system clipboard has plain text; formatted payload travels in the internal slice");
        let after_cut = full_doc(&s);
        let remaining = kind.payloads(s.doc().unwrap().doc.story(sid).unwrap());
        if kind.marker().is_some() {
            assert_eq!(remaining, vec![kind.payloads(&before)[1].clone()]);
        }
        // The reflow worker owns Cut's nested history. This oracle requires a public undo
        // to restore the whole pre-Cut document; it makes no private history repairs here.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(full_doc(&s), full);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(full_doc(&s), after_cut);
        let at = s.doc().unwrap().doc.story(sid).unwrap().len();
        s.execute("text.select", &json!({"story": sid.0, "anchor": at})).unwrap();
        let undo = s.doc().unwrap().history.undo.len();
        let pasted = s.execute("edit.paste", &json!({"text": "A"})).unwrap();
        assert_eq!(pasted["formatted"], true);
        payload_oracle(&mut s, sid, kind, &before, false);
        unchanged_options(&s, &full);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        let after_paste = full_doc(&s);
        zip_oracle(&mut s);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(full_doc(&s), after_cut);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(full_doc(&s), after_paste);
    }
}

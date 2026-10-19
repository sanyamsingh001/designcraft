//! Unicode delimiter boundaries must survive the actual Markdown placement path.
use designcraft_doc::{NO_CHAR_STYLE, StoryId};
use designcraft_engine::{Session, cmd::base64_encode};
use serde_json::json;

#[test]
fn run35_markdown_placement_preserves_unicode_bytes_and_effective_styles() {
    for (source, expected, emphasis) in [
        ("caf\u{e9}_menu_", "caf\u{e9}_menu_", None),
        ("cafe\u{301}_menu_", "cafe\u{301}_menu_", None),
        ("caf\u{e9}__menu__", "caf\u{e9}__menu__", None),
        ("cafe\u{301}__menu__", "cafe\u{301}__menu__", None),
        ("*caf\u{e9}*menu", "caf\u{e9}menu", Some(("caf\u{e9}", "Emphasis", "Italic"))),
        ("*cafe\u{301}*menu", "cafe\u{301}menu", Some(("cafe\u{301}", "Emphasis", "Italic"))),
        ("(*word*)", "(word)", Some(("word", "Emphasis", "Italic"))),
        ("\u{2014}_word_!", "\u{2014}word!", Some(("word", "Emphasis", "Italic"))),
        ("\u{ab}__word__\u{bb}", "\u{ab}word\u{bb}", Some(("word", "Strong", "Bold"))),
        ("\u{20ac}_word_\u{d7}", "\u{20ac}word\u{d7}", Some(("word", "Emphasis", "Italic"))),
        ("^_word_^", "^word^", Some(("word", "Emphasis", "Italic"))),
        ("\u{1f642}_word_\u{1f642}", "\u{1f642}word\u{1f642}", Some(("word", "Emphasis", "Italic"))),
    ] {
        let mut session = Session::new();
        session.execute("file.new", &json!({})).unwrap();
        let params = json!({"name": "unicode-flanks.md", "base64": base64_encode(source.as_bytes())});
        let info = session.execute("place.styles", &params).unwrap();
        assert_eq!(info["character"], emphasis.map_or_else(|| json!([]), |(_, style, _)| json!([style])), "{source:?}");
        let placed = session.execute("file.place", &params).unwrap();
        assert_eq!(placed["warnings"], json!([]), "{source:?}");
        let doc = &session.doc().unwrap().doc;
        let story = doc.story(StoryId(placed["story"].as_u64().unwrap())).unwrap();
        assert_eq!(story.text.as_bytes(), expected.as_bytes(), "{source:?}");
        story.check().unwrap();
        let (_, base) = doc.styles.resolve_para(&story.paras[0]);
        let span = emphasis.map(|(word, style, weight)| {
            let start = expected.find(word).unwrap();
            (start..start + word.len(), style, weight)
        });
        for (at, _) in expected.char_indices() {
            let (name, weight) = match &span {
                Some((range, style, weight)) if range.contains(&at) => (*style, *weight),
                _ => (NO_CHAR_STYLE, base.font_style.as_str()),
            };
            let format = story.format_after(at);
            assert_eq!(format.style, name, "{source:?} at {at}");
            assert_eq!(doc.styles.resolve_char(&base, format).font_style, weight, "{source:?} at {at}");
            assert!(format.over.is_empty(), "{source:?} at {at}");
        }
    }
}

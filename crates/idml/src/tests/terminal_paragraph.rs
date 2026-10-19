//! Standard IDML terminal CSR regressions; uses the existing tests.rs package helpers.
use super::*;

fn terminal_story_part(body: &str) -> String {
    format!(
        r#"<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="16.0"><Story Self="s1">{body}</Story></idPkg:Story>"#
    )
}

fn assert_terminal_csr(d: &Document, direct: Option<f64>, named: f64) {
    assert_eq!(d.stories.len(), 1);
    let story = d.stories.values().next().unwrap();
    assert_eq!(story.text, "é\n");
    assert_eq!(story.para_ranges(), vec![0..2, 3..3]);
    assert_eq!(story.paras.len(), 2);
    assert!(story.chars.iter().all(|r| r.len > 0));
    assert_eq!(story.chars.iter().map(|r| r.len).sum::<usize>(), 3);
    let previous = story.format_after(2);
    assert_eq!(previous.style, "Previous");
    assert_eq!(previous.over.size, Some(17.0));
    let terminal = story.insert_format_at(3);
    assert_eq!(terminal.style, "Tail", "the empty final CSR owns its character style independently of the preceding LF");
    assert_eq!(terminal.over.size, direct, "paragraph defaults must not be flattened into terminal direct overrides");
    assert_eq!(story.paras[1].chars.size, Some(11.0));
    let base = d.styles.resolve_para(&story.paras[1]).1;
    assert_eq!(base.size, 11.0);
    assert_eq!(d.styles.char_style("Tail").unwrap().chars.size, Some(named));
    assert_eq!(d.styles.resolve_char(&base, terminal).size, direct.unwrap_or(named));
    d.check().unwrap();
}

#[test]
fn terminal_empty_csr_keeps_defaults_style_and_direct_attrs_through_idml() {
    let styles = STYLES.replace(
        "</RootCharacterStyleGroup>",
        r#"<CharacterStyle Self="CharacterStyle/Previous" Name="Previous" PointSize="17"/>
<CharacterStyle Self="CharacterStyle/Tail" Name="Tail" PointSize="23"/>
</RootCharacterStyleGroup>"#,
    );
    let first = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Previous" PointSize="17"><Content>é</Content><Br/></CharacterStyleRange></ParagraphStyleRange>"#;
    let last = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Previous" PointSize="17"><Content>Z</Content></CharacterStyleRange></ParagraphStyleRange>"#;
    for (direct_attr, direct) in [(r#" PointSize="31""#, Some(31.0)), ("", None)] {
        let para = |content: &str| {
            format!(
                r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle" PointSize="11"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Tail"{direct_attr}>{content}</CharacterStyleRange></ParagraphStyleRange>"#
            )
        };
        // Current nonempty and interior-LF ranges retain their existing merged-run policy.
        for (content, suffix, text, mark) in [("<Content>Q</Content>", "", "é\nQ", 3), ("<Br/>", last, "é\n\nZ", 3)] {
            let part = terminal_story_part(&format!("{first}{}{suffix}", para(content)));
            let d = import_idml(&fixture_with_table_styles(&styles, &part)).unwrap();
            d.check().unwrap();
            let story = d.stories.values().next().unwrap();
            assert_eq!(story.text, text);
            assert!(story.paras[1].chars.is_empty());
            assert_eq!(story.format_after(mark).style, "Tail");
            assert_eq!(story.format_after(mark).over.size, Some(direct.unwrap_or(11.0)));
            assert_eq!(story.insert_format_at(story.len()), story.format_after(story.len() - 1));
            let bytes = export_idml(&d);
            let back = import_idml(&bytes).unwrap();
            back.check().unwrap();
            let copied = back.stories.values().next().unwrap();
            assert_eq!(copied.text, text);
            assert!(copied.paras[1].chars.is_empty());
            assert_eq!(copied.format_after(mark).style, "Tail");
            assert_eq!(copied.format_after(mark).over.size, Some(direct.unwrap_or(11.0)));
        }
        // An entirely empty story keeps its existing zero-length run.
        let empty_part = terminal_story_part(&para(""));
        let d = import_idml(&fixture_with_table_styles(&styles, &empty_part)).unwrap();
        d.check().unwrap();
        let story = d.stories.values().next().unwrap();
        assert!(story.text.is_empty());
        assert_eq!(story.chars.len(), 1);
        assert_eq!(story.chars[0].len, 0);
        assert!(story.paras[0].chars.is_empty());
        assert_eq!(story.insert_format_at(0).style, "Tail");
        assert_eq!(story.insert_format_at(0).over.size, Some(direct.unwrap_or(11.0)));
        let back = import_idml(&export_idml(&d)).unwrap();
        back.check().unwrap();
        let copied = back.stories.values().next().unwrap();
        assert!(copied.text.is_empty());
        assert_eq!(copied.chars, story.chars);
        assert!(copied.paras[0].chars.is_empty());
        // A table cell is not an outer Story: preserve its existing final-empty policy.
        let cell = format!(
            r#"<ParagraphStyleRange><CharacterStyleRange><Table><Row Name="0" MinimumHeight="24"/><Column Name="0" SingleColumnWidth="60"/><Cell Name="0:0">{first}{}</Cell></Table></CharacterStyleRange></ParagraphStyleRange>"#,
            para("")
        );
        let d = import_idml(&fixture_with_table_styles(&styles, &terminal_story_part(&cell))).unwrap();
        d.check().unwrap();
        let table = d.stories.values().flat_map(|s| s.tables.values()).next().unwrap();
        assert_eq!((table.nrows(), table.ncols()), (1, 1));
        let cell = &table.cells[0].text;
        assert_eq!(cell.text, "é\n");
        assert!(cell.paras[1].chars.is_empty());
        assert_eq!(cell.insert_format_at(3).style, "Previous");
        assert_eq!(cell.insert_format_at(3).over.size, Some(17.0));
        let back = import_idml(&export_idml(&d)).unwrap();
        back.check().unwrap();
        let copied_table = back.stories.values().flat_map(|s| s.tables.values()).next().unwrap();
        assert_eq!((copied_table.nrows(), copied_table.ncols()), (1, 1));
        let copied_cell = &copied_table.cells[0].text;
        assert_eq!(copied_cell.text, cell.text);
        assert_eq!(copied_cell.chars, cell.chars);
        assert!(copied_cell.paras[1].chars.is_empty());
        assert_eq!(copied_cell.insert_format_at(3), cell.insert_format_at(3));

        let part = terminal_story_part(&format!("{first}{}", para("")));
        let mut d = import_idml(&fixture_with_table_styles(&styles, &part)).unwrap();
        assert_terminal_csr(&d, direct, 23.0);
        let before_runs = d.stories.values().next().unwrap().chars.clone();
        let bytes = export_idml(&d);
        let xml = zip_text(&bytes, "Stories/");
        assert_eq!(xml.matches("<Br").count(), 1);
        let tail = xml.rsplit_once("<ParagraphStyleRange").unwrap().1;
        let (opening, rest) = tail.split_once('>').unwrap();
        let body = rest.split("</ParagraphStyleRange>").next().unwrap();
        assert!(opening.contains(r#"PointSize="11""#));
        assert!(body.contains(r#"AppliedCharacterStyle="CharacterStyle/Tail""#));
        assert!(!body.contains("<Br"));
        assert!(!body.contains("<Label") && !body.contains("terminalCharFormat"));
        let back = import_idml(&bytes).unwrap();
        assert_terminal_csr(&back, direct, 23.0);
        assert_eq!(back.stories.values().next().unwrap().chars, before_runs);

        d.styles_mut().char_style_mut("Tail").unwrap().chars.size = Some(29.0);
        assert_terminal_csr(&d, direct, 29.0);
        assert_eq!(d.stories.values().next().unwrap().chars, before_runs);
        let back = import_idml(&export_idml(&d)).unwrap();
        assert_terminal_csr(&back, direct, 29.0);
        assert_eq!(back.stories.values().next().unwrap().chars, before_runs);
    }
}

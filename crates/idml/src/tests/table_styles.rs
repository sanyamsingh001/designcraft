use super::*;

mod fixture {
    include!("table_styles_fixture.rs");
}

fn table(doc: &Document) -> &designcraft_doc::Table {
    doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
}

fn assert_appearance(doc: &Document, explicit: bool, local: bool) {
    use designcraft_color::swatch::{BLACK, NONE, PAPER};
    use designcraft_doc::{CellStroke, StrokeType, VerticalJustification as Vj};

    let table = table(doc);
    assert_eq!((table.nrows(), table.ncols(), table.header_rows(), table.footer_rows()), (3, 3, 1, 1));
    assert_eq!(table.style, "Catalogue");
    assert_eq!(table.options.border.weight, if local { 2.0 } else { 0.0 });
    assert_eq!(table.options.border.color, if local { "Brand" } else { BLACK });
    assert!((table.options.border.tint - if local { 0.8 } else { 0.5 }).abs() < 1e-6);
    assert_eq!(table.options.space_before, if local { 9.0 } else { 5.0 });
    assert_eq!(table.options.space_after, 6.0);
    for row in 0..3 {
        for col in 0..3 {
            let cell = table.cell(row, col).unwrap();
            let (name, fill, tint, insets, vj) = match (row, col) {
                (0, _) => ("Header", BLACK, 0.2, [6.0, 7.0, 8.0, 9.0], Vj::Center),
                (2, _) => ("Footer", BLACK, 0.6, [3.0, 4.0, 5.0, 6.0], Vj::Bottom),
                (1, 0) => ("Pastille", "Brand", 1.0, [4.0, 8.0, 4.0, 8.0], Vj::Center),
                (1, 2) => ("Right", PAPER, 0.75, [5.0, 6.0, 7.0, 8.0], Vj::Bottom),
                _ if explicit => ("Explicit", "Brand", 0.4, [11.0, 12.0, 13.0, 14.0], Vj::Center),
                _ => ("Body row", NONE, 1.0, [2.0, 3.0, 4.0, 5.0], Vj::Top),
            };
            let is_local = local && (row, col) == (1, 1);
            assert_eq!(cell.style, name);
            assert_eq!(cell.fill, if is_local { PAPER } else { fill });
            assert!((cell.fill_tint - if is_local { 0.35 } else { tint }).abs() < 1e-6);
            let mut expected_insets = insets;
            if is_local {
                expected_insets[0] = 12.0;
            }
            assert_eq!(cell.insets, expected_insets);
            assert_eq!(cell.vj, if is_local { Vj::Bottom } else { vj });
            assert_eq!(cell.text.text, format!("{row}:{col}"));
            assert_eq!(cell.text.paras[0].style, if is_local { "Local" } else { "CellText" });
            assert_eq!(cell.border_overrides, [is_local, false, true, false]);
            assert_eq!(cell.strokes[0], CellStroke { weight: 0.0, color: if is_local { NONE } else { BLACK }.into(), ..Default::default() });
            for edge in [1, 3] {
                assert_eq!(cell.strokes[edge], CellStroke { weight: 0.0, color: NONE.into(), ..Default::default() });
            }
            let is_explicit = explicit && (row, col) == (1, 1);
            assert_eq!(
                cell.strokes[2],
                CellStroke {
                    weight: if is_local {
                        2.0
                    } else if is_explicit {
                        0.75
                    } else {
                        0.5
                    },
                    color: if is_local || is_explicit { "Brand" } else { BLACK }.into(),
                    tint: if is_local { 0.6 } else { 1.0 },
                    kind: if is_explicit { StrokeType::Dotted } else { StrokeType::Solid },
                },
            );
        }
    }
    for style in &doc.styles.cell {
        if style.name != designcraft_doc::NO_CELL_STYLE {
            let edges = style.strokes.as_ref().unwrap();
            assert_eq!(edges[0].as_ref().unwrap().weight, 0.0);
            assert_eq!(edges[1].as_ref().unwrap().weight, 0.0);
            assert!(edges[2].as_ref().unwrap().weight > 0.0);
            assert_eq!(edges[3].as_ref().unwrap().weight, 0.0);
            assert!(style.stroke.is_none(), "the top edge must not become a uniform stroke");
        }
    }
}

fn import_and_roundtrip(explicit: bool, local: bool) {
    let doc = import_idml(&fixture::fixture(explicit, local)).unwrap();
    doc.check().unwrap();
    assert_appearance(&doc, explicit, local);
    let back = import_idml(&export_idml(&doc)).unwrap();
    back.check().unwrap();
    assert_appearance(&back, explicit, local);
    assert_eq!(back.styles.cell, doc.styles.cell, "named asymmetric edges survive export, not just local cell attributes");
    assert_eq!(back.styles.table, doc.styles.table);
}

#[test]
fn region_only_table_styles_import_before_local_attributes_and_roundtrip() {
    import_and_roundtrip(false, false);
}

#[test]
fn explicit_cell_style_overrides_the_table_region_and_roundtrips() {
    import_and_roundtrip(true, false);
}

#[test]
fn local_fill_insets_alignment_paragraph_and_priority_override_named_styles() {
    import_and_roundtrip(true, true);
    import_and_roundtrip(false, true);
}

#[test]
fn an_optional_imported_edge_is_not_expanded_to_all_four_edges() {
    use designcraft_doc::{Cell, CellStroke, CellStyle, StrokeType};

    let mut doc = import_idml(&fixture::fixture(false, false)).unwrap();
    let edge = CellStroke { weight: 1.25, color: "Brand".into(), tint: 0.65, kind: StrokeType::Dotted };
    doc.styles_mut().cell.push(CellStyle { name: "Partial".into(), strokes: Some([None, None, Some(edge.clone()), None]), ..Default::default() });
    let back = import_idml(&export_idml(&doc)).unwrap();
    let style = back.styles.cell.iter().find(|style| style.name == "Partial").unwrap();
    assert_eq!(style.strokes, Some([None, None, Some(edge.clone()), None]));
    let mut cell = Cell::default();
    let before = cell.strokes.clone();
    style.apply_to(&mut cell);
    assert_eq!(cell.strokes, [before[0].clone(), before[1].clone(), edge, before[3].clone()]);
}

fn assert_effective_regions(doc: &Document, same: [bool; 4], explicit: bool, local_cell: Option<(usize, usize)>) {
    use designcraft_color::swatch::{NONE, PAPER};
    use designcraft_doc::VerticalJustification;

    let baseline = import_idml(&fixture::fixture(explicit, false)).unwrap();
    assert_appearance(&baseline, explicit, false);
    let body = table(&baseline).cell(1, 1).unwrap();
    let table = table(doc);
    let style = doc.styles.table.iter().find(|s| s.name == "Catalogue").unwrap();
    for (i, (binding, separate)) in
        [(&style.header, "Header"), (&style.footer, "Footer"), (&style.left_column, "Pastille"), (&style.right_column, "Right")]
            .into_iter()
            .enumerate()
    {
        assert_eq!(binding.as_deref(), Some(if same[i] { "Body row" } else { separate }));
    }
    // Use a region-only body cell even when the fixture has an explicit middle-cell style.
    let region_only = import_idml(&fixture::fixture(false, false)).unwrap();
    let region_body = self::table(&region_only).cell(1, 1).unwrap();
    for row in 0..3 {
        for col in 0..3 {
            let inherit = match (row, col) {
                (0, _) => same[0],
                (2, _) => same[1],
                (1, 0) => same[2],
                (1, 2) => same[3],
                _ => false,
            };
            let mut expected = if !explicit && inherit { region_body.clone() } else { self::table(&baseline).cell(row, col).unwrap().clone() };
            if local_cell == Some((row, col)) {
                expected.fill = PAPER.into();
                expected.fill_tint = 0.35;
                expected.insets[0] = 12.0;
                expected.vj = VerticalJustification::Bottom;
                expected.strokes[0].color = NONE.into();
                expected.strokes[2].weight = 2.0;
                expected.strokes[2].color = "Brand".into();
                expected.strokes[2].tint = 0.6;
                expected.border_overrides[0] = true;
                expected.text.paras[0].style = "Local".into();
            }
            expected.text.text = format!("{row}:{col}");
            assert_eq!(table.cell(row, col).unwrap(), &expected, "appearance of cell ({row},{col}), flags={same:?}, explicit={explicit}");
        }
    }
    // The middle cell is unrelated to region flags and retains its explicit style.
    assert_eq!(table.cell(1, 1).unwrap().style, body.style);
}

fn same_regions_roundtrip(same: [bool; 4], explicit: bool, local_cell: Option<(usize, usize)>) {
    let doc = import_idml(&fixture::fixture_with_regions_and_local_cell(explicit, local_cell, same)).unwrap();
    assert_effective_regions(&doc, same, explicit, local_cell);
    let bytes = export_idml(&doc);
    let back = import_idml(&bytes).unwrap();
    assert_effective_regions(&back, same, explicit, local_cell);
    assert_eq!(back.styles.table, doc.styles.table);
    assert_eq!(back.styles.cell, doc.styles.cell);

    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("Resources/Styles.xml").unwrap().read_to_string(&mut xml).unwrap();
    let root = crate::xml::parse(xml.as_bytes()).unwrap();
    let style = root.find("RootTableStyleGroup").unwrap().find_all("TableStyle").find(|s| s.get("Name") == Some("Catalogue")).unwrap();
    for flag in
        ["HeaderRegionSameAsBodyRegion", "FooterRegionSameAsBodyRegion", "LeftColumnRegionSameAsBodyRegion", "RightColumnRegionSameAsBodyRegion"]
    {
        assert_eq!(style.boolean(flag), Some(false), "effective region bindings must be explicit on export");
    }
}

#[test]
fn explicit_true_same_as_body_regions_ignore_inactive_styles_and_roundtrip() {
    for same in
        [[false; 4], [true, false, false, false], [false, true, false, false], [false, false, true, false], [false, false, false, true], [true; 4]]
    {
        same_regions_roundtrip(same, false, None);
    }
}

#[test]
fn explicit_cell_styles_and_local_overrides_follow_same_as_body_regions() {
    for explicit in [false, true] {
        for local_cell in [None, Some((0, 1)), Some((2, 1)), Some((1, 0)), Some((1, 2))] {
            same_regions_roundtrip([true; 4], explicit, local_cell);
        }
    }
}

fn r6_row_fills_fixture(disabled: bool) -> Vec<u8> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(fixture::fixture(false, false))).unwrap();
    let mut files = Vec::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).unwrap();
        let name = file.name().to_string();
        if name == "mimetype" {
            continue;
        }
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();
        if name == "Resources/Styles.xml" {
            content = content.replace(
                "Name=\"Catalogue\"",
                "Name=\"Catalogue\" StartRowFillCount=\"1\" StartRowFillColor=\"Color/Black\" StartRowFillTint=\"10\" EndRowFillCount=\"1\" EndRowFillColor=\"Swatch/None\"",
            );
        }
        if disabled && name == "Stories/Story_s1.xml" {
            content = content.replace(
                "AppliedTableStyle=\"TableStyle/uCatalogue\"",
                "AppliedTableStyle=\"TableStyle/uCatalogue\" StartRowFillCount=\"0\" EndRowFillCount=\"0\"",
            );
        }
        files.push((name, content));
    }
    let refs: Vec<_> = files.iter().map(|(name, content)| (name.as_str(), content.as_str())).collect();
    super::zip_files(&refs)
}

#[test]
fn r6_disabled_row_fills_override_style_while_absent_external_attributes_inherit() {
    use designcraft_doc::AltFills;
    let expected = AltFills { first_tint: 0.1, ..Default::default() };
    for disabled in [false, true] {
        let doc = import_idml(&r6_row_fills_fixture(disabled)).unwrap();
        let style = doc.styles.table.iter().find(|style| style.name == "Catalogue").unwrap();
        assert_eq!(style.alt_rows.as_ref(), Some(&expected));
        assert_eq!(table(&doc).style, "Catalogue");
        assert_eq!(table(&doc).options.alt_rows.as_ref(), (!disabled).then_some(&expected));
        let back = import_idml(&export_idml(&doc)).unwrap();
        assert_eq!(back.styles.table, doc.styles.table);
        assert_eq!(table(&back).style, "Catalogue");
        assert_eq!(table(&back).options.alt_rows, table(&doc).options.alt_rows);
    }
}

fn row_fill_skips_region_fixture(skips: Option<(u32, u32)>) -> Vec<u8> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(r6_row_fills_fixture(false))).unwrap();
    let mut files = Vec::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).unwrap();
        let name = file.name().to_string();
        if name == "mimetype" {
            continue;
        }
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();
        if name == "Resources/Styles.xml"
            && let Some((first, last)) = skips
        {
            assert_eq!(content.matches("Name=\"Catalogue\"").count(), 1);
            content = content.replace(
                "Name=\"Catalogue\"",
                &format!("Name=\"Catalogue\" SkipFirstAlternatingFillRows=\"{first}\" SkipLastAlternatingFillRows=\"{last}\""),
            );
        }
        files.push((name, content));
    }
    let refs: Vec<_> = files.iter().map(|(name, content)| (name.as_str(), content.as_str())).collect();
    super::zip_files(&refs)
}

fn check_row_fill_skip_region(skips: Option<(u32, u32)>) {
    use designcraft_doc::AltFills;
    let (skip_first, skip_last) = skips.unwrap_or((0, 0));
    let expected = AltFills { first_tint: 0.1, skip_first, skip_last, ..Default::default() };
    let doc = import_idml(&row_fill_skips_region_fixture(skips)).unwrap();
    doc.check().unwrap();
    let style = doc.styles.table.iter().find(|style| style.name == "Catalogue").unwrap();
    assert_eq!(style.alt_rows.as_ref(), Some(&expected), "independent region-only XML supplies the named style skips");
    assert_eq!(table(&doc).options.alt_rows.as_ref(), Some(&expected), "absent local attributes inherit the style pattern");
    let bytes = export_idml(&doc);
    if skip_first > 0 || skip_last > 0 {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut xml = String::new();
        zip.by_name("Resources/Styles.xml").unwrap().read_to_string(&mut xml).unwrap();
        let root = crate::xml::parse(xml.as_bytes()).unwrap();
        let style = root.find("RootTableStyleGroup").unwrap().find_all("TableStyle").find(|style| style.get("Name") == Some("Catalogue")).unwrap();
        assert_eq!(style.num("SkipFirstAlternatingFillRows"), Some(f64::from(skip_first)));
        assert_eq!(style.num("SkipLastAlternatingFillRows"), Some(f64::from(skip_last)));
    }
    let back = import_idml(&bytes).unwrap();
    back.check().unwrap();
    assert_eq!(back.styles.table, doc.styles.table);
    assert_eq!(table(&back).options, table(&doc).options);
    for (source, cell) in table(&doc).cells.iter().zip(&table(&back).cells) {
        assert_eq!(cell, source, "source text and cell properties stay unchanged");
    }
}

#[test]
fn r7a_row_fill_skips_region_only_style_attributes_import_and_export() {
    for skips in [(1, 0), (0, 1), (1, 1)] {
        check_row_fill_skip_region(Some(skips));
    }
}

#[test]
fn r7a_row_fill_skips_region_only_zero_skip_and_absent_attributes_control() {
    for skips in [None, Some((0, 0))] {
        check_row_fill_skip_region(skips);
    }
}

//! Native-origin table fill/dash interchange regressions. No Adobe-produced fixtures.
use super::*;
use std::io::Read;

use designcraft_doc::{AltFills, CellStroke, CellStyle, RowHeightMode, StrokeType, Table, TableStyle, TextDirection};

fn table(doc: &Document) -> &Table {
    doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
}

fn native_table(rtl: bool) -> Document {
    let mut doc = Document::new(&NewDocument { pages: 1, facing_pages: false, ..Default::default() });
    let (_, sid) = doc.add_text_frame(SpreadRef::Doc(0), Rect::new(0.0, 0.0, 120.0, 200.0), doc.default_layer(), "", ParaFormat::default()).unwrap();
    let mut t = Table::new(doc.alloc(), 3, 1, 0, 0, 100.0);
    t.options.direction = if rtl { TextDirection::RightToLeft } else { TextDirection::LeftToRight };
    for row in &mut t.rows {
        row.height = 20.0;
        row.mode = RowHeightMode::Exactly;
    }
    for (r, cell) in t.cells.iter_mut().enumerate() {
        cell.insets = [0.0; 4];
        cell.text.insert(0, &format!("row {r}"));
    }
    doc.story_mut(sid).unwrap().insert_table(0, t);
    doc
}

fn edit_table(doc: &mut Document, f: impl FnOnce(&mut Table)) {
    let sid = *doc.stories.keys().next().unwrap();
    let t = doc.story_mut(sid).unwrap().tables.values_mut().next().unwrap();
    f(std::sync::Arc::make_mut(t));
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut value = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut value).unwrap();
    value
}

fn rewrite_part(bytes: &[u8], name: &str, edit: impl Fn(&str) -> String) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut files = Vec::new();
    let mut changed = false;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let path = file.name().to_string();
        if path == "mimetype" {
            continue;
        }
        let mut value = String::new();
        file.read_to_string(&mut value).unwrap();
        if path == name {
            value = edit(&value);
            changed = true;
        }
        files.push((path, value));
    }
    assert!(changed);
    let refs: Vec<_> = files.iter().map(|(path, value)| (path.as_str(), value.as_str())).collect();
    zip_files(&refs)
}

fn zero_first() -> AltFills {
    AltFills {
        first: 0,
        first_color: "[None]".into(),
        first_tint: 1.0,
        next: 1,
        next_color: "[Black]".into(),
        next_tint: 0.4,
        skip_first: 1,
        skip_last: 1,
    }
}

#[test]
fn r8_zero_first_named_resource_and_local_precedence_survive_idml() {
    for rtl in [false, true] {
        for local in [false, true] {
            let mut doc = native_table(rtl);
            let named = if local { AltFills { first_tint: 0.1, ..Default::default() } } else { zero_first() };
            doc.styles_mut().table.push(TableStyle { name: "Stripe".into(), alt_rows: Some(named.clone()), ..Default::default() });
            edit_table(&mut doc, |t| {
                t.style = "Stripe".into();
                t.options.alt_rows = Some(zero_first());
            });
            let bytes = export_idml(&doc);
            let styles = crate::xml::parse(part(&bytes, "Resources/Styles.xml").as_bytes()).unwrap();
            let resource = styles.find("RootTableStyleGroup").unwrap().find_all("TableStyle").find(|s| s.get("Name") == Some("Stripe")).unwrap();
            assert_eq!(resource.num("StartRowFillCount"), Some(f64::from(named.first)));
            assert_eq!(resource.num("EndRowFillCount"), Some(1.0));
            assert_eq!(resource.num("SkipFirstAlternatingFillRows"), Some(f64::from(named.skip_first)));
            assert_eq!(resource.num("SkipLastAlternatingFillRows"), Some(f64::from(named.skip_last)));
            let back = import_idml(&bytes).unwrap();
            let style = back.styles.table.iter().find(|s| s.name == "Stripe").unwrap();
            assert_eq!(style.alt_rows.as_ref(), Some(&named));
            assert_eq!(table(&back).options.alt_rows.as_ref(), Some(&zero_first()), "local zero/one must beat a different named pattern");
            let pattern = table(&back).options.alt_rows.as_ref().unwrap();
            assert_eq!((0..3).map(|r| pattern.fill_for(r, 3)).collect::<Vec<_>>(), [None, Some(("[Black]", 0.4)), None]);
        }
    }
}

#[test]
fn r8_zero_first_independent_region_only_and_disabled_positive_controls() {
    for (first, next) in [(0, 1), (1, 0), (1, 1), (0, 0)] {
        let story = r#"<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging"><Story Self="s1">
        <ParagraphStyleRange><CharacterStyleRange><Table AppliedTableStyle="TableStyle/Z">
        <Row Name="0" MinimumHeight="20"/><Column Name="0" SingleColumnWidth="100"/>
        <Cell Name="0:0"><ParagraphStyleRange><CharacterStyleRange><Content>row</Content></CharacterStyleRange></ParagraphStyleRange></Cell>
        </Table></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;
        let bytes = rewrite_part(&fixture_with_story(story), "Resources/Styles.xml", |xml| {
            xml.replace(
                "</idPkg:Styles>",
                &format!(
                    r#"<RootTableStyleGroup><TableStyle Self="TableStyle/Z" Name="Zero"
                StartRowFillCount="{first}" EndRowFillCount="{next}" StartRowFillColor="Color/Black" EndRowFillColor="Color/Black"
                SkipFirstAlternatingFillRows="1" SkipLastAlternatingFillRows="2"/></RootTableStyleGroup></idPkg:Styles>"#
                ),
            )
        });
        let doc = import_idml(&bytes).unwrap();
        let expected = (first + next > 0).then(|| AltFills {
            first,
            first_color: "[Black]".into(),
            first_tint: 1.0,
            next,
            next_color: "[Black]".into(),
            next_tint: 1.0,
            skip_first: 1,
            skip_last: 2,
        });
        assert_eq!(doc.styles.table.iter().find(|s| s.name == "Zero").unwrap().alt_rows, expected);
        assert_eq!(table(&doc).options.alt_rows, expected, "absent local fields inherit the named definition");
        let back = import_idml(&export_idml(&doc)).unwrap();
        assert_eq!(back.styles.table, doc.styles.table);
        assert_eq!(table(&back).options, table(&doc).options);
    }
}

#[test]
fn r8_zero_first_local_column_pattern_uses_the_same_count_contract() {
    let mut doc = native_table(false);
    let expected = AltFills { skip_first: 0, skip_last: 0, ..zero_first() };
    edit_table(&mut doc, |t| t.options.alt_cols = Some(expected.clone()));
    let back = import_idml(&export_idml(&doc)).unwrap();
    assert_eq!(table(&back).options.alt_cols.as_ref(), Some(&expected));
}

fn dashed(pattern: Vec<f64>) -> CellStroke {
    CellStroke { weight: 2.0, color: "[Black]".into(), tint: 0.6, kind: StrokeType::Dashed { pattern } }
}

#[test]
fn r8_concrete_dashes_roundtrip_table_edges_and_named_uniform_and_asymmetric_styles() {
    for pattern in [vec![6.0, 4.0], vec![12.0, 8.0], vec![1.23456789012345, 2.34567890123456, 3.0, 1.0]] {
        for rtl in [false, true] {
            let mut doc = native_table(rtl);
            let stroke = dashed(pattern.clone());
            doc.styles_mut().table.push(TableStyle { name: "Border".into(), border: Some(stroke.clone()), ..Default::default() });
            doc.styles_mut().cell.push(CellStyle { name: "Uniform".into(), stroke: Some(stroke.clone()), ..Default::default() });
            doc.styles_mut().cell.push(CellStyle {
                name: "Asymmetric".into(),
                strokes: Some([
                    Some(stroke.clone()),
                    None,
                    Some(CellStroke::none()),
                    Some(CellStroke { kind: StrokeType::Dotted, ..Default::default() }),
                ]),
                ..Default::default()
            });
            edit_table(&mut doc, |t| {
                t.style = "Border".into();
                t.options.border = stroke.clone();
                for (r, cell) in t.cells.iter_mut().enumerate() {
                    cell.style = if r == 0 { "Uniform".into() } else { "Asymmetric".into() };
                    cell.strokes[0] = stroke.clone();
                    cell.strokes[2] = stroke.clone();
                    cell.border_overrides = [true, false, true, false];
                }
            });
            let bytes = export_idml(&doc);
            let graphic = crate::xml::parse(part(&bytes, "Resources/Graphic.xml").as_bytes()).unwrap();
            let resources: Vec<_> = graphic.find_all("DashedStrokeStyle").collect();
            assert_eq!(resources.len(), 1, "equal concrete arrays share one resource across all table/style callers");
            let expected_array = pattern.iter().map(ToString::to_string).collect::<Vec<_>>().join(" ");
            assert_eq!(
                resources[0].get("DashArray"),
                Some(expected_array.as_str()),
                "the actual stored array is exported without six-digit rounding"
            );
            let back = import_idml(&bytes).unwrap();
            assert!(back.stroke_styles.is_empty(), "generated table resources must not become extra user-named styles");
            assert_eq!(table(&back).options.border, stroke);
            assert_eq!(back.styles.table.iter().find(|s| s.name == "Border").unwrap().border.as_ref(), Some(&stroke));
            for name in ["Uniform", "Asymmetric"] {
                let style = back.styles.cell.iter().find(|s| s.name == name).unwrap();
                let mut cell = designcraft_doc::Cell::default();
                style.apply_to(&mut cell);
                assert_eq!(cell.strokes[0], stroke);
                if name == "Uniform" {
                    assert!(cell.strokes.iter().all(|s| s == &stroke));
                } else {
                    assert_eq!(cell.strokes[1], CellStroke::default(), "an absent edge remains absent");
                    assert_eq!(cell.strokes[2], CellStroke::none());
                    assert_eq!(cell.strokes[3].kind, StrokeType::Dotted);
                }
            }
            for (original, reopened) in table(&doc).cells.iter().zip(&table(&back).cells) {
                assert_eq!(reopened.strokes, original.strokes);
                assert_eq!(reopened.border_overrides, original.border_overrides);
                assert_eq!(reopened.text.text, original.text.text);
            }
            let again = import_idml(&export_idml(&back)).unwrap();
            assert_eq!(table(&again).options, table(&back).options);
            assert_eq!(again.styles.cell, back.styles.cell);
            assert_eq!(again.styles.table, back.styles.table);
            assert!(again.stroke_styles.is_empty());
        }
    }
}

#[test]
fn r8_distinct_stored_arrays_at_the_same_weight_do_not_alias() {
    let mut doc = native_table(false);
    edit_table(&mut doc, |t| {
        t.options.border = dashed(vec![6.0, 4.0]);
        t.cells[0].strokes[2] = dashed(vec![3.0, 7.0]);
        t.cells[0].border_overrides[2] = true;
    });
    let bytes = export_idml(&doc);
    let graphic = crate::xml::parse(part(&bytes, "Resources/Graphic.xml").as_bytes()).unwrap();
    assert_eq!(graphic.find_all("DashedStrokeStyle").count(), 2);
    let back = import_idml(&bytes).unwrap();
    assert_eq!(table(&back).options.border.kind, StrokeType::Dashed { pattern: vec![6.0, 4.0] });
    assert_eq!(table(&back).cells[0].strokes[2].kind, StrokeType::Dashed { pattern: vec![3.0, 7.0] });
}

#[test]
fn r8_solid_dotted_zero_and_explicit_none_keep_their_strokes_and_priorities() {
    for stroke in [
        CellStroke::default(),
        dashed(vec![]),
        CellStroke { kind: StrokeType::Dotted, ..Default::default() },
        CellStroke { weight: 0.0, ..dashed(vec![3.0, 2.0]) },
        CellStroke { color: "[None]".into(), ..dashed(vec![6.0, 4.0]) },
    ] {
        let mut doc = native_table(false);
        edit_table(&mut doc, |t| {
            t.options.border = stroke.clone();
            t.cells[0].strokes = [stroke.clone(), stroke.clone(), stroke.clone(), stroke.clone()];
            t.cells[0].border_overrides = [true; 4];
        });
        let back = import_idml(&export_idml(&doc)).unwrap();
        assert_eq!(table(&back).options.border, stroke);
        assert_eq!(table(&back).cells[0].strokes, table(&doc).cells[0].strokes);
        assert_eq!(table(&back).cells[0].border_overrides, [true; 4]);
        assert_eq!(table(&back).cells[0].strokes[2].is_visible(), stroke.is_visible());
    }
}

#[test]
fn r8_user_named_custom_dash_styles_and_graphic_import_contract_remain_distinct() {
    let name = "DesignCraftTableDash/4018000000000000-4010000000000000";
    let mut doc = native_table(false);
    doc.stroke_styles.push(designcraft_doc::StrokeStyleDef { name: name.into(), kind: StrokeType::Dashed { pattern: vec![6.0, 4.0] } });
    edit_table(&mut doc, |t| {
        t.cells[0].strokes[2].kind = StrokeType::Style { name: name.into() };
        t.cells[0].border_overrides[2] = true;
    });
    let back = import_idml(&export_idml(&doc)).unwrap();
    assert_eq!(back.stroke_styles, doc.stroke_styles, "a user name resembling a generated id is still a custom resource");
    assert_eq!(table(&back).cells[0].strokes[2].kind, StrokeType::Style { name: name.into() });

    let story = r#"<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging"><Story Self="s1">
        <ParagraphStyleRange><CharacterStyleRange><Table TopBorderStrokeType="StrokeStyle/$ID/Dashed">
        <Row Name="0"/><Column Name="0"/><Cell Name="0:0" BottomEdgeStrokeType="StrokeStyle/$ID/Dashed"/>
        </Table></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;
    let bytes = rewrite_part(&fixture_with_story(story), "Spreads/Spread_sp1.xml", |xml| {
        xml.replace("StrokeWeight=\"2\"", "StrokeWeight=\"2\" StrokeType=\"StrokeStyle/$ID/Dashed\"")
    });
    let doc = import_idml(&bytes).unwrap();
    assert_eq!(
        table(&doc).options.border.kind,
        StrokeType::Dashed { pattern: vec![] },
        "unannotated external built-ins do not gain inferred patterns"
    );
    assert_eq!(table(&doc).cells[0].strokes[2].kind, StrokeType::Dashed { pattern: vec![] });
    let oval = doc.spreads.iter().flat_map(|s| &s.items).flat_map(|i| i.children()).find(|i| i.shape == Shape::Oval).unwrap();
    assert_eq!(oval.stroke.kind, StrokeType::Dashed { pattern: vec![] }, "ordinary graphic import is unchanged");
}

#[test]
fn r8_unused_named_table_and_cell_dash_definitions_register_their_own_resources() {
    let mut doc = native_table(false);
    let border = dashed(vec![9.0, 5.0]);
    let uniform = dashed(vec![7.0, 2.0]);
    let edge = dashed(vec![2.0, 9.0]);
    doc.styles_mut().table.push(TableStyle { name: "UnusedBorder".into(), border: Some(border.clone()), ..Default::default() });
    doc.styles_mut().cell.push(CellStyle { name: "UnusedUniform".into(), stroke: Some(uniform.clone()), ..Default::default() });
    doc.styles_mut().cell.push(CellStyle { name: "UnusedEdge".into(), strokes: Some([None, Some(edge.clone()), None, None]), ..Default::default() });
    let bytes = export_idml(&doc);
    let graphic = crate::xml::parse(part(&bytes, "Resources/Graphic.xml").as_bytes()).unwrap();
    assert_eq!(graphic.find_all("DashedStrokeStyle").count(), 3, "definitions register resources without any local table/cell references");
    let back = import_idml(&bytes).unwrap();
    assert!(back.stroke_styles.is_empty());
    assert_eq!(back.styles.table.iter().find(|s| s.name == "UnusedBorder").unwrap().border.as_ref(), Some(&border));
    for (name, stroke, index) in [("UnusedUniform", uniform, 0), ("UnusedEdge", edge, 1)] {
        let mut cell = designcraft_doc::Cell::default();
        back.styles.cell.iter().find(|s| s.name == name).unwrap().apply_to(&mut cell);
        assert_eq!(cell.strokes[index], stroke);
    }
    assert_eq!(table(&back).options.border, table(&doc).options.border);
    assert!(table(&back).cells.iter().all(|c| c.strokes.iter().all(|s| s.kind == StrokeType::Solid)));
}

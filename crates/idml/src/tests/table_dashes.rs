//! Native-origin table dash interchange regressions. No Adobe-produced fixtures.
use super::*;
use std::io::Read;

use designcraft_doc::{AltStrokesAttrs, CellStroke, CellStrokeAttrs, CellStyle, RowHeightMode, StrokeType, Table, TableStyle, TextDirection};

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

fn dashed(pattern: Vec<f64>) -> CellStroke {
    CellStroke { weight: 2.0, color: "[Black]".into(), tint: 0.6, kind: StrokeType::Dashed { pattern } }
}

fn edge_attrs(stroke: &CellStroke) -> CellStrokeAttrs {
    CellStrokeAttrs { weight: Some(stroke.weight), color: Some(stroke.color.clone()), tint: Some(stroke.tint), kind: Some(stroke.kind.clone()) }
}

#[test]
fn concrete_dashes_roundtrip_table_edges_and_named_uniform_and_asymmetric_styles() {
    for pattern in [vec![6.0, 4.0], vec![12.0, 8.0], vec![1.23456789012345, 2.34567890123456, 3.0, 1.0]] {
        for rtl in [false, true] {
            let mut doc = native_table(rtl);
            let stroke = dashed(pattern.clone());
            doc.styles_mut().table.push(TableStyle { name: "Border".into(), border: Some(stroke.clone()), ..Default::default() });
            doc.styles_mut().cell.push(CellStyle { name: "Uniform".into(), stroke: Some(stroke.clone()), ..Default::default() });
            doc.styles_mut().cell.push(CellStyle {
                name: "Asymmetric".into(),
                strokes: [
                    edge_attrs(&stroke),
                    Default::default(),
                    edge_attrs(&CellStroke::none()),
                    edge_attrs(&CellStroke { kind: StrokeType::Dotted, ..Default::default() }),
                ],
                ..Default::default()
            });
            let cell_styles = doc.styles.cell.clone();
            edit_table(&mut doc, |t| {
                t.style = "Border".into();
                t.options.border = stroke.clone();
                for (r, cell) in t.cells.iter_mut().enumerate() {
                    let name = if r == 0 { "Uniform" } else { "Asymmetric" };
                    cell_styles.iter().find(|s| s.name == name).unwrap().apply_to(cell);
                    cell.strokes[2] = stroke.clone();
                    cell.border_overrides[2] = true;
                    cell.stroke_defined[2] = true;
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
fn distinct_stored_arrays_at_the_same_weight_do_not_alias() {
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
fn solid_dotted_zero_and_explicit_none_keep_their_strokes_and_priorities() {
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
fn user_named_custom_dash_styles_and_graphic_import_contract_remain_distinct() {
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
fn unused_named_table_and_cell_dash_definitions_register_their_own_resources() {
    let mut doc = native_table(false);
    let border = dashed(vec![9.0, 5.0]);
    let uniform = dashed(vec![7.0, 2.0]);
    let edge = dashed(vec![2.0, 9.0]);
    doc.styles_mut().table.push(TableStyle { name: "UnusedBorder".into(), border: Some(border.clone()), ..Default::default() });
    doc.styles_mut().cell.push(CellStyle { name: "UnusedUniform".into(), stroke: Some(uniform.clone()), ..Default::default() });
    doc.styles_mut().cell.push(CellStyle {
        name: "UnusedEdge".into(),
        strokes: [Default::default(), edge_attrs(&edge), Default::default(), Default::default()],
        ..Default::default()
    });
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

#[test]
fn concrete_dashes_preserve_sparse_inheritance_and_alternating_stroke_resources() {
    let mut doc = native_table(false);
    let first = dashed(vec![1.23456789012345, 7.0]);
    let next = dashed(vec![8.0, 2.34567890123456]);
    let edge = CellStrokeAttrs { kind: Some(first.kind.clone()), ..Default::default() };
    doc.styles_mut().cell.push(CellStyle {
        name: "DashParent".into(),
        strokes: [edge.clone(), Default::default(), Default::default(), Default::default()],
        ..Default::default()
    });
    doc.styles_mut().cell.push(CellStyle {
        name: "WeightChild".into(),
        based_on: Some("DashParent".into()),
        strokes: [CellStrokeAttrs { weight: Some(5.0), ..Default::default() }, Default::default(), Default::default(), Default::default()],
        ..Default::default()
    });
    let pattern =
        AltStrokesAttrs { first: Some(1), next: Some(1), first_stroke: edge_attrs(&first), next_stroke: edge_attrs(&next), ..Default::default() };
    doc.styles_mut().table.push(TableStyle {
        name: "PatternParent".into(),
        borders: [edge, Default::default(), Default::default(), Default::default()],
        row_strokes: pattern.clone(),
        column_strokes: pattern,
        ..Default::default()
    });
    doc.styles_mut().table.push(TableStyle {
        name: "PatternChild".into(),
        based_on: Some("PatternParent".into()),
        column_strokes: AltStrokesAttrs { next_stroke: CellStrokeAttrs { weight: Some(4.0), ..Default::default() }, ..Default::default() },
        ..Default::default()
    });
    let style = doc.styles.resolve_table_style("PatternChild");
    edit_table(&mut doc, |t| style.apply_to(t, &[]));
    let bytes = export_idml(&doc);
    let graphic = crate::xml::parse(part(&bytes, "Resources/Graphic.xml").as_bytes()).unwrap();
    assert_eq!(graphic.find_all("DashedStrokeStyle").count(), 2);
    let back = import_idml(&bytes).unwrap();
    for name in ["DashParent", "WeightChild"] {
        assert_eq!(back.styles.cell.iter().find(|s| s.name == name), doc.styles.cell.iter().find(|s| s.name == name));
    }
    for name in ["PatternParent", "PatternChild"] {
        assert_eq!(back.styles.table.iter().find(|s| s.name == name), doc.styles.table.iter().find(|s| s.name == name));
    }
    let resolved = designcraft_doc::styles::resolve_cell_style(&back.styles.cell, "WeightChild");
    let mut cell = designcraft_doc::Cell::default();
    resolved.apply_to(&mut cell);
    assert_eq!(cell.strokes[0].kind, first.kind);
    assert_eq!(cell.strokes[0].weight, 5.0);
    assert_eq!(table(&back).options.row_strokes, table(&doc).options.row_strokes);
    assert_eq!(table(&back).options.column_strokes, table(&doc).options.column_strokes);
    assert_eq!(table(&back).options.border_for(0).kind, first.kind);
    assert!(back.stroke_styles.is_empty());
}

//! Native schema and public-command controls for the remaining dash interchange change.
use std::io::{Cursor, Write};

use designcraft_doc::{Cell, CellStroke, Document, StrokeType, Table};
use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let result = s.execute("frame.create", &json!({"rect": [0, 0, 200, 200], "content": "text", "text": ""})).unwrap();
    s.execute("text.select", &json!({"story": result["story"], "anchor": 0, "focus": 0})).unwrap();
    s.execute("table.insert", &json!({"rows": 2, "cols": 1, "width": 100})).unwrap();
    s
}

fn table(doc: &Document) -> &Table {
    doc.stories.values().flat_map(|story| story.tables.values()).next().unwrap()
}

fn native_zip(document: &Value) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in [
        ("mimetype", designcraft_format::MIME.as_bytes().to_vec()),
        ("meta.json", serde_json::to_vec(&json!({"format": "designcraft", "version": 1})).unwrap()),
        ("document.json", serde_json::to_vec(document).unwrap()),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn released_uniform_and_sparse_cell_styles_reopen_in_native_zip_and_legacy_json() {
    let s = session();
    let stroke = CellStroke { weight: 2.0, kind: StrokeType::Dashed { pattern: vec![6.0, 4.0] }, ..Default::default() };
    // 9e has only the optional uniform stroke; fc2 adds sparse edge objects.
    // Neither released schema uses nullable members of the strokes array.
    let fixtures = [
        (json!({"name": "OldAbsent"}), [1.0; 4], false),
        (json!({"name": "OldNull", "stroke": null}), [1.0; 4], false),
        (json!({"name": "OldUniform", "stroke": stroke}), [2.0; 4], true),
        (json!({"name": "OldPartial", "stroke": {"weight": 2, "kind": stroke.kind}}), [2.0; 4], true),
        (json!({"name": "Sparse", "stroke": stroke, "strokes": [{"weight": 0}, {}, {"tint": 0.25}, {}]}), [0.0, 2.0, 2.0, 2.0], true),
    ];
    for (fixture, weights, dashed) in fixtures {
        let mut value = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        value["styles"]["cell"] = json!([fixture]);
        let story = value["stories"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .find(|story| story["tables"].as_object().is_some_and(|tables| !tables.is_empty()))
            .unwrap();
        let table_value = story["tables"].as_object_mut().unwrap().values_mut().next().unwrap();
        // CellStroke itself already allowed missing components in 9e.
        table_value["cells"][0]["strokes"] = json!([{"weight": 0}, {}, {"color": "[None]"}, {"kind": stroke.kind}]);
        for bytes in [serde_json::to_vec(&value).unwrap(), native_zip(&value)] {
            let doc = designcraft_format::load(&bytes).unwrap();
            let edges = &table(&doc).cells[0].strokes;
            assert_eq!(edges[0], CellStroke::none());
            assert_eq!(edges[1], CellStroke::default());
            assert_eq!(edges[2].color, "[None]");
            assert_eq!(edges[3].kind, stroke.kind);
            assert_eq!(edges[3].weight, 1.0);
            let style = &doc.styles.cell[0];
            let mut cell = Cell::default();
            style.apply_to(&mut cell);
            assert_eq!(std::array::from_fn::<_, 4, _>(|i| cell.strokes[i].weight), weights);
            for edge in &cell.strokes {
                assert_eq!(edge.kind, if dashed { stroke.kind.clone() } else { StrokeType::Solid });
            }
            if style.name == "Sparse" {
                assert_eq!(cell.strokes[2].tint, 0.25);
                assert_eq!(cell.strokes[0].tint, 1.0);
                assert!(style.strokes[1].is_empty());
            }
            let saved = designcraft_format::load(&designcraft_format::save(&doc).unwrap()).unwrap();
            assert_eq!(saved, doc, "valid released fields survive another native save");
        }
    }
}

#[test]
fn weight_only_cell_edit_keeps_concrete_dashes_through_native_idml_and_undo() {
    let mut s = session();
    s.execute("table.setCell", &json!({"rows": [0, 0], "cols": [0, 0], "stroke": {"edges": "bottom", "type": "dashed", "weight": 2}})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let original = &table(&before).cells[0].strokes[2];
    assert_eq!(original.kind, StrokeType::Dashed { pattern: vec![6.0, 4.0] });
    s.execute("table.setCell", &json!({"rows": [0, 0], "cols": [0, 0], "stroke": {"edges": "bottom", "weight": 4}})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    let expected = CellStroke { weight: 4.0, ..original.clone() };
    assert_eq!(table(&after).cells[0].strokes[2], expected);
    for bytes in [designcraft_format::save(&after).unwrap(), serde_json::to_vec(&after).unwrap()] {
        let native = designcraft_format::load(&bytes).unwrap();
        assert_eq!(native, *after);
        let imported = designcraft_idml::import_idml(&designcraft_idml::export_idml(&native)).unwrap();
        assert_eq!(table(&imported).cells[0].strokes[2], expected);
        assert_eq!(table(&imported).cells[1].strokes[0], expected, "shared-edge command mirroring remains intact");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *after);
}

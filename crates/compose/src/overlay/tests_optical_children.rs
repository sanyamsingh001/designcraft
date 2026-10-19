use super::ComposedChildren;
use crate::table::{PlacedCell, TableFrag};
use crate::{ComposedStory, FrameText, PlacedNote};
use designcraft_doc::ItemId;
use designcraft_geom::{Point, Rect};
use std::cell::Cell;
use std::sync::Arc;

fn child(optical: bool) -> Arc<ComposedStory> {
    Arc::new(ComposedStory { optical_kerning: vec![Some(optical)], ..Default::default() })
}

fn cell(row: usize, col: usize, text: &Arc<ComposedStory>, repeated: bool) -> PlacedCell {
    PlacedCell {
        row,
        col,
        rect: Rect::ZERO,
        fill: None,
        origin: Point::ZERO,
        text: Arc::clone(text),
        source: Arc::from(""),
        clip: Rect::ZERO,
        overset: false,
        repeated,
        graphic: None,
    }
}

fn fragment(table: u64, cells: Vec<PlacedCell>) -> TableFrag {
    TableFrag { table, anchor: 0, column: 0, rect: Rect::ZERO, cells, strokes: vec![], line: 0, first: true, last: true }
}

fn note(id: u64, text: &Arc<ComposedStory>) -> PlacedNote {
    PlacedNote { id, index: 0, column: 0, label: "1".into(), origin: Point::ZERO, rect: Rect::ZERO, text: Arc::clone(text), source: Arc::from("") }
}

#[test]
fn child_index_visits_frames_once_and_preserves_first_placement_identity() {
    let first = child(false);
    let repeated = child(true);
    let next_row = child(false);
    let next_col = child(false);
    let other_table = child(false);
    let frames = [
        FrameText {
            frame: ItemId(0),
            tables: vec![
                fragment(7, vec![cell(0, 0, &first, false), cell(0, 1, &next_col, false)]),
                fragment(8, vec![cell(0, 0, &other_table, false)]),
            ],
            notes: vec![note(9, &first), note(10, &other_table)],
            ..Default::default()
        },
        FrameText {
            frame: ItemId(41),
            tables: vec![fragment(7, vec![cell(0, 0, &repeated, true), cell(1, 0, &next_row, false)])],
            notes: vec![note(9, &repeated), note(11, &next_row)],
            ..Default::default()
        },
        FrameText { frame: ItemId(42), notes: vec![note(11, &repeated)], ..Default::default() },
    ];
    let visited = Cell::new(0);
    let index = ComposedChildren::from_frames(frames.iter().inspect(|_| visited.set(visited.get() + 1)));
    assert_eq!(visited.get(), frames.len(), "both child kinds share one frame traversal");
    assert_eq!(index.cells.len(), 4);
    assert_eq!(index.notes.len(), 3);
    for _ in 0..8 {
        for (key, expected) in [((7, 0, 0), &first), ((7, 0, 1), &next_col), ((7, 1, 0), &next_row), ((8, 0, 0), &other_table)] {
            assert!(std::ptr::eq(index.cells.get(&key).copied().unwrap(), expected.as_ref()));
        }
        let placements = [(9, &frames[0], &frames[0].notes[0]), (10, &frames[0], &frames[0].notes[1]), (11, &frames[1], &frames[1].notes[1])];
        for (id, frame, placed) in placements {
            let (owner, actual) = index.notes.get(&id).copied().unwrap();
            assert!(std::ptr::eq(owner, frame), "retain the owning frame, including frame zero's page fallback");
            assert!(std::ptr::eq(actual, placed), "retain the first note and its composed text");
        }
        assert!(!index.cells.contains_key(&(7, 1, 1)));
        assert!(!index.cells.contains_key(&(99, 0, 0)));
        assert!(!index.notes.contains_key(&99));
    }
    assert_eq!(visited.get(), frames.len(), "hits and misses never revisit the layout");
}

#[test]
fn absent_composition_has_empty_child_indexes() {
    let index = ComposedChildren::from_frames(std::iter::empty());
    assert!(index.cells.is_empty());
    assert!(index.notes.is_empty());
}

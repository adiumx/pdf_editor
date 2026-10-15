//! Annotations: added with a look, listed, moved, re-worded, deleted and saved.

use pdfcore::{AnnotKind, Document, Error, NewAnnotation, Shape};

fn open() -> Document {
    let path = format!("{}/tests/fixtures/standard.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn with_contents(mut n: NewAnnotation, text: &str) -> NewAnnotation {
    n.contents = text.into();
    n.author = "Cesar".into();
    n
}

#[test]
fn every_kind_is_added_listed_and_undone_in_one_step() {
    let mut doc = open();
    let before = doc.save_to_bytes().unwrap();
    let run = doc.text_runs(0).unwrap().remove(0);
    let b = run.display;
    let yellow = [1.0, 0.9, 0.2];
    let shapes = vec![
        (Shape::Highlight(vec![b]), AnnotKind::Highlight),
        (Shape::Underline(vec![b]), AnnotKind::Underline),
        (Shape::StrikeOut(vec![b]), AnnotKind::StrikeOut),
        (Shape::Squiggly(vec![b]), AnnotKind::Squiggly),
        (Shape::Note { at: (300.0, 100.0) }, AnnotKind::Note),
        (Shape::FreeText { rect: [100.0, 300.0, 300.0, 360.0], size: 12.0 }, AnnotKind::FreeText),
        (Shape::Ink { strokes: vec![vec![(100.0, 400.0), (150.0, 420.0), (200.0, 400.0)]], width: 2.0 }, AnnotKind::Ink),
        (Shape::Square { rect: [100.0, 450.0, 200.0, 500.0], fill: Some([0.8, 0.9, 1.0]), width: 1.5 }, AnnotKind::Square),
        (Shape::Circle { rect: [220.0, 450.0, 320.0, 500.0], fill: None, width: 1.5 }, AnnotKind::Circle),
        (Shape::Line { from: (100.0, 520.0), to: (300.0, 560.0), width: 2.0 }, AnnotKind::Line),
    ];
    for (shape, _) in &shapes {
        let n = with_contents(NewAnnotation::new(shape.clone(), yellow), "Revisar este punto con ñ");
        doc.add_annotation(0, &n).unwrap();
    }
    let listed = doc.annotations(0).unwrap();
    assert_eq!(listed.iter().map(|a| a.kind).collect::<Vec<_>>(), shapes.iter().map(|s| s.1).collect::<Vec<_>>());
    assert!(listed.iter().all(|a| a.has_appearance && !a.hidden));
    assert_eq!(listed[0].quads.len(), 1);
    assert!((listed[0].quads[0][0] - b[0]).abs() < 0.5 && (listed[0].quads[0][3] - b[3]).abs() < 0.5, "{:?} vs {b:?}", listed[0].quads[0]);
    assert!((listed[0].display[1] - b[1]).abs() < 0.5);
    assert_eq!(listed[4].contents, "Revisar este punto con ñ");
    assert_eq!(listed[4].author, "Cesar");
    assert_eq!(listed[0].color, Some(yellow));
    // The file still opens, with the same annotations.
    let saved = doc.save_to_bytes().unwrap();
    assert_eq!(Document::open(&saved).unwrap().annotations(0).unwrap().len(), shapes.len());
    // Each undo takes one back, and the last returns the file.
    for _ in 0..shapes.len() {
        assert!(doc.undo());
    }
    assert!(doc.annotations(0).unwrap().is_empty());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn free_text_that_cannot_be_drawn_is_refused_and_nothing_changes() {
    let mut doc = open();
    let n = with_contents(NewAnnotation::new(Shape::FreeText { rect: [100.0, 300.0, 300.0, 360.0], size: 12.0 }, [0.0; 3]), "hola \u{4E16}");
    assert!(matches!(doc.add_annotation(0, &n), Err(Error::MissingGlyphs(c)) if c == vec!['\u{4E16}']));
    assert!(!doc.can_undo() && doc.annotations(0).unwrap().is_empty());
    let empty = with_contents(NewAnnotation::new(Shape::Highlight(vec![]), [0.0; 3]), "");
    assert!(matches!(doc.add_annotation(0, &empty), Err(Error::Rejected(_))));
}

#[test]
fn an_annotation_is_moved_reworded_and_deleted() {
    let mut doc = open();
    let n = with_contents(NewAnnotation::new(Shape::FreeText { rect: [100.0, 300.0, 300.0, 360.0], size: 12.0 }, [0.0, 0.0, 0.5]), "Primero");
    doc.add_annotation(0, &n).unwrap();
    doc.add_annotation(0, &with_contents(NewAnnotation::new(Shape::Note { at: (50.0, 50.0) }, [1.0, 1.0, 0.0]), "nota")).unwrap();

    doc.move_annotation(0, 0, 20.0, 30.0).unwrap();
    let moved = doc.annotations(0).unwrap().remove(0);
    assert!((moved.display[0] - 120.0).abs() < 0.01 && (moved.display[1] - (300.0 + 30.0)).abs() < 100.0);

    doc.set_annotation_contents(0, 0, "Segundo texto, mas largo que el primero").unwrap();
    assert_eq!(doc.annotations(0).unwrap()[0].contents, "Segundo texto, mas largo que el primero");
    assert!(matches!(doc.set_annotation_contents(0, 0, "\u{4E16}"), Err(Error::MissingGlyphs(_))));
    assert_eq!(doc.annotations(0).unwrap()[0].contents, "Segundo texto, mas largo que el primero");

    doc.delete_annotation(0, 0).unwrap();
    let left = doc.annotations(0).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, AnnotKind::Note);
    assert!(matches!(doc.delete_annotation(0, 5), Err(Error::Rejected(_))));
    assert!(doc.undo());
    assert_eq!(doc.annotations(0).unwrap().len(), 2);
}

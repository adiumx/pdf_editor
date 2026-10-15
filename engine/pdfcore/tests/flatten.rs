//! Flattening annotations and form fields into the page.

use pdfcore::{Document, FlattenOptions, NewAnnotation, Shape};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

#[test]
fn a_filled_form_is_drawn_into_the_page_and_the_fields_go() {
    let mut doc = open("form");
    let text_field = doc.fields().into_iter().find(|f| f.kind == pdfcore::FieldKind::Text && !f.read_only).expect("a text field");
    doc.set_field_value(&text_field.name, "Valor aplanado").unwrap();
    let before = doc.save_to_bytes().unwrap();
    let done = doc.flatten(None, FlattenOptions::default()).unwrap();
    assert!(done.drawn >= 1 && done.removed >= done.drawn, "{done:?}");
    assert!(doc.fields().into_iter().all(|f| f.kind == pdfcore::FieldKind::Signature), "{:?}", doc.fields().iter().map(|f| &f.name).collect::<Vec<_>>());
    // The value is part of the page now, drawn from a form XObject.
    assert!(doc.text_runs(0).unwrap().iter().any(|r| r.text.contains("Valor aplanado") && r.xobject.is_some()));
    // The file opens again with no fields and the text still there.
    let reopened = Document::open(&doc.save_to_bytes().unwrap()).unwrap();
    // Only the signature field, which is never flattened, is left.
    assert!(reopened.fields().iter().all(|f| f.kind == pdfcore::FieldKind::Signature));
    assert!(reopened.text_runs(0).unwrap().iter().any(|r| r.text.contains("Valor aplanado")));
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn annotations_are_flattened_without_touching_the_form_unless_asked() {
    let mut doc = open("form");
    let n = |s| NewAnnotation::new(s, [1.0, 0.9, 0.0]);
    doc.add_annotation(0, &n(Shape::Square { rect: [50.0, 50.0, 150.0, 100.0], fill: Some([0.9, 0.9, 1.0]), width: 1.0 })).unwrap();
    doc.add_annotation(0, &n(Shape::Note { at: (200.0, 60.0) })).unwrap();
    let fields_before = doc.fields().len();
    let done = doc.flatten(None, FlattenOptions { annotations: true, forms: false }).unwrap();
    assert_eq!(done.drawn, 2);
    assert!(doc.annotations(0).unwrap().iter().all(|a| a.kind == pdfcore::AnnotKind::Widget));
    assert_eq!(doc.fields().len(), fields_before);
    assert!(doc.undo());
    assert_eq!(doc.annotations(0).unwrap().iter().filter(|a| a.kind != pdfcore::AnnotKind::Widget).count(), 2);
}

#[test]
fn a_page_that_does_not_exist_is_refused() {
    let mut doc = open("form");
    assert!(matches!(doc.flatten(Some(&[7]), FlattenOptions::default()), Err(pdfcore::Error::NoSuchPage(7))));
    assert!(!doc.can_undo());
}

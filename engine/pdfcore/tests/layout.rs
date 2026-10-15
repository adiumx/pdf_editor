//! Moving, resizing and aligning form fields.

use pdfcore::{Arrange, Document, Error, FieldRef};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn reopen(doc: &mut Document) -> Document {
    Document::open(&doc.save_to_bytes().unwrap()).unwrap()
}

fn shown(doc: &Document, name: &str) -> [f32; 4] {
    doc.field(name).unwrap().widgets[0].display
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05)
}

#[test]
fn a_field_moves_and_is_redrawn_at_its_new_size_with_its_value() {
    for name in ["form", "form_rotated"] {
        let mut doc = open(name);
        doc.set_field_value("nombre", "María José").unwrap();
        let wanted = [100.0, 300.0, 260.0, 322.0];
        let got = doc.move_field("nombre", 0, wanted).unwrap();
        assert!(near(got, wanted), "{name}: {got:?}");
        let again = reopen(&mut doc);
        assert!(near(shown(&again, "nombre"), wanted), "{name}: {:?}", shown(&again, "nombre"));
        assert_eq!(again.field("nombre").unwrap().value, "María José");
        assert!(again.field("nombre").unwrap().widgets[0].has_appearance);
        // One step to undo, and the field is back where it was.
        assert!(doc.undo());
        assert!(!near(shown(&doc, "nombre"), wanted), "{name}");
        assert_eq!(doc.field("nombre").unwrap().value, "María José", "only the move was undone");
    }
}

#[test]
fn a_box_is_kept_on_the_page_and_never_collapses() {
    let mut doc = open("form");
    let size = doc.page_size(0).unwrap();
    let moved = doc.move_field("nombre", 0, [-50.0, -20.0, 40.0, 10.0]).unwrap();
    assert!(moved[0] >= 0.0 && moved[1] >= 0.0, "{moved:?}");
    let past = doc.move_field("nombre", 0, [size.width - 10.0, size.height - 5.0, size.width + 200.0, size.height + 50.0]).unwrap();
    assert!(past[2] <= size.width + 0.01 && past[3] <= size.height + 0.01, "{past:?}");
    let tiny = doc.move_field("nombre", 0, [100.0, 100.0, 100.0, 100.0]).unwrap();
    assert!(tiny[2] - tiny[0] >= 6.0 && tiny[3] - tiny[1] >= 6.0, "{tiny:?}");
    // A box drawn from the bottom-right corner up and left is the same box.
    let flipped = doc.move_field("nombre", 0, [200.0, 200.0, 100.0, 180.0]).unwrap();
    assert!(near(flipped, [100.0, 180.0, 200.0, 200.0]));
}

#[test]
fn refusals_say_why_and_change_nothing() {
    let mut doc = open("signed");
    assert!(matches!(doc.move_field("firma", 0, [10.0, 10.0, 100.0, 40.0]), Err(Error::Rejected(_))));
    assert!(matches!(doc.move_field("nada", 0, [10.0, 10.0, 100.0, 40.0]), Err(Error::NoSuchField(_))));
    assert!(matches!(doc.move_field("nombre", 3, [10.0, 10.0, 100.0, 40.0]), Err(Error::Rejected(_))));
    assert!(!doc.can_undo());
}

#[test]
fn several_fields_line_up_as_one_step() {
    let mut doc = open("form_hard");
    let names = ["rfc", "notas", "importe", "estado"];
    let items: Vec<FieldRef> = names.iter().map(|n| FieldRef { name: n.to_string(), widget: 0 }).collect();
    let before: Vec<[f32; 4]> = names.iter().map(|n| shown(&doc, n)).collect();

    doc.arrange_fields(&items, Arrange::AlignRight).unwrap();
    let right = before.iter().map(|b| b[2]).fold(f32::MIN, f32::max);
    for (n, b) in names.iter().zip(&before) {
        let after = shown(&doc, n);
        assert!((after[2] - right).abs() < 0.05, "{n}: {after:?}");
        assert!(((after[2] - after[0]) - (b[2] - b[0])).abs() < 0.05, "{n} keeps its width");
        assert!((after[1] - b[1]).abs() < 0.05, "{n} does not move up or down");
    }
    doc.arrange_fields(&items, Arrange::SameWidth).unwrap();
    let widths: Vec<f32> = names.iter().map(|n| shown(&doc, n)).map(|b| b[2] - b[0]).collect();
    assert!(widths.iter().all(|w| (w - widths[0]).abs() < 0.05), "{widths:?}");

    assert!(doc.undo() && doc.undo());
    for (n, b) in names.iter().zip(&before) {
        assert!(near(shown(&doc, n), *b), "{n}");
    }
    assert!(!doc.can_undo());
}

#[test]
fn fields_are_spread_with_equal_gaps() {
    let mut doc = open("form_hard");
    // rfc, importe and estado have different widths and heights; first stack them at odd spots.
    for (name, y) in [("rfc", 100.0), ("importe", 130.0), ("estado", 400.0)] {
        let b = shown(&doc, name);
        doc.move_field(name, 0, [b[0], y, b[2], y + (b[3] - b[1])]).unwrap();
    }
    let items: Vec<FieldRef> = ["rfc", "importe", "estado"].iter().map(|n| FieldRef { name: n.to_string(), widget: 0 }).collect();
    doc.arrange_fields(&items, Arrange::DistributeVertically).unwrap();
    let (a, b, c) = (shown(&doc, "rfc"), shown(&doc, "importe"), shown(&doc, "estado"));
    assert!(((b[1] - a[3]) - (c[1] - b[3])).abs() < 0.1, "gaps {} and {}", b[1] - a[3], c[1] - b[3]);
    assert!((a[1] - 100.0).abs() < 0.05 && (c[3] - (400.0 + (c[3] - c[1]))).abs() < 0.05, "the ends stay where they were");
    // Distributing needs three, aligning two.
    assert!(matches!(doc.arrange_fields(&items[..2], Arrange::DistributeHorizontally), Err(Error::Rejected(_))));
    assert!(matches!(doc.arrange_fields(&items[..1], Arrange::AlignLeft), Err(Error::Rejected(_))));
}

#[test]
fn a_list_box_keeps_its_selection_when_moved() {
    let mut doc = open("form_hard");
    doc.set_field_value("ramos", "Vida\nAuto").unwrap();
    doc.move_field("ramos", 0, [300.0, 340.0, 450.0, 430.0]).unwrap();
    let again = reopen(&mut doc);
    assert_eq!(again.field("ramos").unwrap().value, "Vida\nAuto");
    assert!(near(shown(&again, "ramos"), [300.0, 340.0, 450.0, 430.0]));
}

#[test]
fn field_text_aligns_left_centre_or_right_and_survives_a_save() {
    use pdfcore::Align;
    let mut doc = open("form_hard");
    doc.set_field_value("importe", "1,234").unwrap();
    doc.set_field_value("estado", "Yucatán").unwrap();
    let names = vec!["importe".to_string(), "estado".to_string()];
    for align in [Align::Center, Align::Right, Align::Left] {
        doc.set_fields_alignment(&names, align).unwrap();
        let again = reopen(&mut doc);
        for name in &names {
            assert_eq!(again.field(name).unwrap().text_align(), align, "{name}");
        }
        assert_eq!(again.field("importe").unwrap().value, "1,234");
    }
    // Three changes, three steps; nothing else was touched.
    assert!(doc.undo() && doc.undo() && doc.undo());
    assert!(matches!(doc.set_fields_alignment(&["acepto".to_string()], Align::Center), Err(Error::Rejected(_))));
    assert!(matches!(doc.set_fields_alignment(&["nada".to_string()], Align::Center), Err(Error::NoSuchField(_))));
    let mut signed = open("signed");
    assert!(matches!(signed.set_fields_alignment(&["firma".to_string()], Align::Center), Err(Error::Rejected(_))));
}

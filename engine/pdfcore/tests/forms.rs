//! Reading and filling form fields.

use pdfcore::{Document, Error, FieldKind};

fn open() -> Document {
    let path = format!("{}/tests/fixtures/form.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn reopen(doc: &mut Document) -> Document {
    Document::open(&doc.save_to_bytes().expect("save")).expect("reopen")
}

#[test]
fn every_kind_of_field_is_read() {
    let doc = open();
    let kind = |n: &str| doc.field(n).unwrap().kind;
    assert_eq!(kind("nombre"), FieldKind::Text);
    assert_eq!(kind("acepto"), FieldKind::Checkbox);
    assert_eq!(kind("respuesta"), FieldKind::Radio);
    assert_eq!(kind("pais"), FieldKind::Combo);
    assert_eq!(kind("plan"), FieldKind::List);
    assert_eq!(kind("firma"), FieldKind::Signature);

    let nombre = doc.field("nombre").unwrap();
    assert_eq!(nombre.value, "Ana");
    assert_eq!(nombre.widgets[0].page, Some(0));
    assert_eq!(doc.field("corto").unwrap().max_len, Some(4));
    assert!(doc.field("fijo").unwrap().read_only);
    assert_eq!(doc.field("respuesta").unwrap().widgets.len(), 3);
    assert_eq!(doc.field("pais").unwrap().options.len(), 3);
    assert!(doc.field("nada").is_none());
}

#[test]
fn the_display_box_is_where_a_reader_shows_the_field() {
    let w = &open().field("nombre").unwrap().widgets[0];
    // Created at (72, 100)-(300, 120) from the top of the page.
    assert!((w.display[0] - 72.0).abs() < 0.5 && (w.display[1] - 100.0).abs() < 0.5, "{:?}", w.display);
    assert!((w.display[2] - 300.0).abs() < 0.5 && (w.display[3] - 120.0).abs() < 0.5, "{:?}", w.display);
}

#[test]
fn text_is_filled_and_survives_a_save() {
    let mut doc = open();
    doc.set_field_value("nombre", "María José Núñez").unwrap();
    assert_eq!(reopen(&mut doc).field("nombre").unwrap().value, "María José Núñez");
}

#[test]
fn text_outside_latin_1_is_stored_as_utf16() {
    let mut doc = open();
    doc.set_field_value("nombre", "Precio: 5 € — «ok»").unwrap();
    assert_eq!(reopen(&mut doc).field("nombre").unwrap().value, "Precio: 5 € — «ok»");
}

#[test]
fn refusals_say_why_and_change_nothing() {
    let mut doc = open();
    assert!(matches!(doc.set_field_value("corto", "demasiado largo"), Err(Error::Rejected(m)) if m.contains('4')));
    assert!(matches!(doc.set_field_value("fijo", "x"), Err(Error::Rejected(m)) if m.contains("solo lectura")));
    assert!(matches!(doc.set_field_value("nombre", "a\nb"), Err(Error::Rejected(_))));
    assert!(matches!(doc.set_field_value("nada", "x"), Err(Error::NoSuchField(_))));
    assert!(matches!(doc.set_field_value("firma", "x"), Err(Error::Unsupported(_))));
    let again = reopen(&mut doc);
    assert_eq!(again.field("corto").unwrap().value, "ab");
    assert_eq!(again.field("fijo").unwrap().value, "no tocar");
}

#[test]
fn a_read_only_field_can_be_filled_when_the_caller_says_so() {
    let mut doc = open();
    doc.set_field_value_with("fijo", "cambiado", true).unwrap();
    assert_eq!(reopen(&mut doc).field("fijo").unwrap().value, "cambiado");
}

#[test]
fn a_checkbox_ticks_and_unticks() {
    let mut doc = open();
    doc.set_field_value("acepto", "true").unwrap();
    let ticked = reopen(&mut doc).field("acepto").unwrap();
    assert!(ticked.checked, "{ticked:?}");
    assert_eq!(ticked.value, ticked.widgets[0].on_state.clone().unwrap());

    let mut doc = reopen(&mut doc);
    doc.set_field_value("acepto", "off").unwrap();
    assert!(!reopen(&mut doc).field("acepto").unwrap().checked);
}

#[test]
fn a_radio_group_selects_one_and_only_one() {
    let mut doc = open();
    doc.set_field_value("respuesta", "No").unwrap();
    let after = reopen(&mut doc).field("respuesta").unwrap();
    assert_eq!(after.value, "No");
    assert!(matches!(doc.set_field_value("respuesta", "Quizá"), Err(Error::Rejected(_))));
    let mut doc = reopen(&mut doc);
    doc.set_field_value("respuesta", "off").unwrap();
    assert!(!reopen(&mut doc).field("respuesta").unwrap().checked);
}

#[test]
fn a_choice_field_accepts_only_its_options() {
    let mut doc = open();
    doc.set_field_value("pais", "Francia").unwrap();
    doc.set_field_value("plan", "Pro").unwrap();
    assert!(matches!(doc.set_field_value("pais", "Italia"), Err(Error::Rejected(_))));
    let again = reopen(&mut doc);
    assert_eq!(again.field("pais").unwrap().value, "Francia");
    assert_eq!(again.field("plan").unwrap().value, "Pro");
}

#[test]
fn filling_leaves_the_pages_text_alone() {
    let mut doc = open();
    let before: Vec<String> = doc.text_runs(0).unwrap().into_iter().map(|r| r.text).collect();
    doc.set_field_value("nombre", "Otra persona").unwrap();
    let after: Vec<String> = reopen(&mut doc).text_runs(0).unwrap().into_iter().map(|r| r.text).collect();
    assert_eq!(before, after);
}

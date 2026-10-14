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

#[test]
fn a_signed_document_reports_its_signature_and_an_unsigned_one_does_not() {
    assert!(open().signatures().is_empty());
    assert!(!open().field("firma").unwrap().signed);

    let path = format!("{}/tests/fixtures/signed.pdf", env!("CARGO_MANIFEST_DIR"));
    let signed = Document::open(&std::fs::read(path).unwrap()).unwrap();
    let signatures = signed.signatures();
    assert_eq!(signatures.len(), 1);
    assert_eq!(signatures[0].field, "firma");
    assert_eq!(signatures[0].signer.as_deref(), Some("Cesar Ruiz"));
    assert!(signed.field("firma").unwrap().signed);
}

#[test]
fn a_filled_text_field_ends_up_with_an_appearance_the_readers_can_draw() {
    // Many real forms ship text widgets with no appearance stream at all.
    let path = format!("{}/tests/fixtures/form_no_appearance.pdf", env!("CARGO_MANIFEST_DIR"));
    let mut doc = Document::open(&std::fs::read(path).unwrap()).unwrap();
    assert!(!doc.field("nombre").unwrap().widgets[0].has_appearance, "el fixture debería no traerla");
    doc.set_field_value("nombre", "María José").unwrap();
    let again = reopen(&mut doc);
    assert!(again.field("nombre").unwrap().widgets[0].has_appearance);
    // Every other widget in the form had one all along, and still does.
    assert!(again.field("acepto").unwrap().widgets[0].has_appearance);
}

#[test]
fn every_text_widget_filled_gets_an_appearance() {
    let mut doc = open();
    for name in ["nombre", "corto"] {
        doc.set_field_value(name, "ok").unwrap();
    }
    doc.set_field_value("pais", "Francia").unwrap();
    let after = reopen(&mut doc);
    for name in ["nombre", "corto", "pais"] {
        assert!(after.field(name).unwrap().widgets.iter().all(|w| w.has_appearance), "{name}");
    }
}

// -- harder forms ---------------------------------------------------------------

fn open_fixture(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

#[test]
fn a_checkbox_with_no_appearance_can_still_be_ticked() {
    let mut doc = open_fixture("form_hard_no_ap");
    assert!(doc.field("acepto").unwrap().widgets[0].on_state.is_none(), "el fixture no debería traer estados");
    doc.set_field_value("acepto", "true").unwrap();
    let mut again = reopen(&mut doc);
    let field = again.field("acepto").unwrap();
    assert!(field.checked);
    assert_eq!(field.widgets[0].on_state.as_deref(), Some("Yes"));
    assert!(field.widgets[0].has_appearance);
    again.set_field_value("acepto", "false").unwrap();
    assert!(!again.field("acepto").unwrap().checked);
}

#[test]
fn a_list_box_takes_several_rows_when_the_form_allows_it() {
    let mut doc = open_fixture("form_hard");
    let field = doc.field("ramos").unwrap();
    assert_eq!(field.kind, FieldKind::List);
    assert!(field.multi_select);
    doc.set_field_value("ramos", "Vida\nAuto").unwrap();
    let mut again = reopen(&mut doc);
    assert_eq!(again.field("ramos").unwrap().value, "Vida\nAuto");
    assert!(again.field("ramos").unwrap().widgets[0].has_appearance);
    // An unknown row is refused and nothing changes.
    assert!(matches!(again.set_field_value("ramos", "Vida\nMarcianos"), Err(Error::Rejected(_))));
    assert_eq!(again.field("ramos").unwrap().value, "Vida\nAuto");
    // Clearing the selection is a value too.
    again.set_field_value("ramos", "").unwrap();
    assert_eq!(again.field("ramos").unwrap().value, "");
}

/// The font size an appearance stream draws its text at.
fn drawn_size(doc: &mut Document, field: &str) -> f32 {
    let saved = doc.save_to_bytes().unwrap();
    let raw = lopdf::Document::load_mem(&saved).unwrap();
    let again = Document::open(&saved).unwrap();
    let widget = again.field(field).unwrap();
    // The widget's /AP /N stream is the only form XObject with a /Tx marked-content block.
    for (_, object) in raw.objects.iter() {
        if let Ok(stream) = object.as_stream() {
            let content = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
            let text = String::from_utf8_lossy(&content);
            if text.starts_with("/Tx BMC") && stream.dict.get(b"BBox").is_ok() {
                let bbox: Vec<f32> = stream.dict.get(b"BBox").unwrap().as_array().unwrap().iter().map(|n| n.as_float().unwrap_or(0.0)).collect();
                let w = widget.widgets[0].rect;
                if ((bbox[2] - (w[2] - w[0])).abs() < 0.01) && ((bbox[3] - (w[3] - w[1])).abs() < 0.01) {
                    let tf = text.lines().find(|l| l.ends_with(" Tf")).expect("Tf");
                    return tf.split_whitespace().rev().nth(1).unwrap().parse().unwrap();
                }
            }
        }
    }
    panic!("no appearance found for {field}");
}

#[test]
fn an_auto_sized_multiline_field_shrinks_its_text_to_fit() {
    let mut doc = open_fixture("form_hard");
    doc.set_field_value("notas", "Corto").unwrap();
    assert_eq!(drawn_size(&mut doc, "notas"), 12.0);
    let long = "palabra ".repeat(120);
    doc.set_field_value("notas", long.trim()).unwrap();
    let size = drawn_size(&mut doc, "notas");
    assert!((6.0..12.0).contains(&size), "tamaño {size}");
}

#[test]
fn fields_come_with_the_order_a_person_fills_them_in() {
    let doc = open_fixture("form_hard");
    let mut by_order: Vec<(usize, String)> = doc.fields().into_iter().map(|f| (f.order, f.name)).collect();
    by_order.sort();
    let names: Vec<&str> = by_order.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(&names[..6], ["rfc", "notas", "importe", "estado", "ramos", "acepto"]);
    let orders: Vec<usize> = by_order.iter().map(|(o, _)| *o).collect();
    assert_eq!(orders, (0..orders.len()).collect::<Vec<_>>());
}

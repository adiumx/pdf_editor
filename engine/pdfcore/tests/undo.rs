//! Undo and redo of every kind of edit.

use pdfcore::{Document, NewText};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn texts(doc: &Document) -> Vec<String> {
    doc.text_runs(0).unwrap().into_iter().map(|r| r.text).collect()
}

#[test]
fn a_fresh_document_has_nothing_to_undo() {
    let mut doc = open("standard");
    assert!(!doc.can_undo() && !doc.can_redo());
    assert!(!doc.undo() && !doc.redo());
}

#[test]
fn undoing_a_replacement_brings_the_old_text_back_and_redo_the_new() {
    let mut doc = open("standard");
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text(0, &run, "Adiós").unwrap();
    assert_eq!(texts(&doc)[0], "Adiós");
    assert!(doc.undo());
    assert_eq!(texts(&doc), ["Hola mundo", "Negrita áéíóú ñ"]);
    assert!(doc.can_redo() && !doc.can_undo());
    assert!(doc.redo());
    assert_eq!(texts(&doc)[0], "Adiós");
}

#[test]
fn undoing_added_text_removes_it_and_the_font_it_brought() {
    let mut doc = open("standard");
    let before = doc.save_to_bytes().unwrap().len();
    doc.add_text(0, &NewText::new("nuevo", 72.0, 300.0)).unwrap();
    let during = doc.save_to_bytes().unwrap().len();
    assert!(during > before + 1_000 && during < before + 40_000, "la fuente incrustada (recortada a lo usado) debería pesar unos kilobytes: {before} -> {during}");
    assert!(doc.undo());
    assert_eq!(texts(&doc), ["Hola mundo", "Negrita áéíóú ñ"]);
    let after = doc.save_to_bytes().unwrap().len();
    assert!(after < before + 2_000, "{before} -> {after}: la fuente quedó en el archivo");
    // And it can all come back.
    assert!(doc.redo());
    assert_eq!(texts(&doc).last().unwrap(), "nuevo");
}

#[test]
fn undoing_a_field_fill_restores_the_old_value() {
    let mut doc = open("form");
    doc.set_field_value("nombre", "Otra persona").unwrap();
    doc.set_field_value("acepto", "true").unwrap();
    assert!(doc.undo());
    assert!(!doc.field("acepto").unwrap().checked);
    assert_eq!(doc.field("nombre").unwrap().value, "Otra persona");
    assert!(doc.undo());
    assert_eq!(doc.field("nombre").unwrap().value, "Ana");
    assert!(doc.redo() && doc.redo());
    assert!(doc.field("acepto").unwrap().checked);
    assert_eq!(doc.field("nombre").unwrap().value, "Otra persona");
}

#[test]
fn a_failed_edit_is_not_a_step_and_changes_nothing() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    assert!(doc.replace_text(0, &run, "Póliza XQ").is_err());
    assert!(!doc.can_undo(), "un intento fallido no debe ensuciar el historial");
    assert_eq!(texts(&doc), ["Póliza número 12345"]);
}

#[test]
fn a_new_edit_after_an_undo_forgets_what_could_have_been_redone() {
    let mut doc = open("standard");
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text(0, &run, "Uno").unwrap();
    doc.undo();
    doc.add_text(0, &NewText::new("otro", 72.0, 300.0)).unwrap();
    assert!(!doc.can_redo());
}

#[test]
fn many_steps_undo_in_order_back_to_the_start() {
    let mut doc = open("form");
    let start = doc.save_to_bytes().unwrap();
    doc.set_field_value("nombre", "a").unwrap();
    doc.add_text(0, &NewText::new("x", 72.0, 500.0)).unwrap();
    doc.set_field_value("pais", "Francia").unwrap();
    doc.set_field_value("respuesta", "No").unwrap();
    doc.add_text(0, &NewText::new("Ñ y más", 72.0, 520.0)).unwrap();
    while doc.undo() {}
    let again = Document::open(&doc.save_to_bytes().unwrap()).unwrap();
    let original = Document::open(&start).unwrap();
    assert_eq!(texts(&again), texts(&original));
    for f in original.fields() {
        assert_eq!(again.field(&f.name).unwrap().value, f.value, "{}", f.name);
    }
    assert!(doc.can_redo());
    while doc.redo() {}
    assert_eq!(doc.field("nombre").unwrap().value, "a");
    assert_eq!(doc.field("pais").unwrap().value, "Francia");
    assert_eq!(texts(&doc).last().unwrap(), "Ñ y más");
}

#[test]
fn fifty_steps_all_undo_in_order() {
    let mut doc = open("embedded");
    for i in 0..50 {
        doc.add_text(0, &NewText::new(&format!("línea {i}"), 72.0, 100.0 + 10.0 * i as f32)).unwrap();
    }
    let mut undone = 0;
    while doc.undo() {
        undone += 1;
    }
    assert_eq!(undone, 50);
    assert_eq!(texts(&doc), ["Póliza número 12345"]);
}

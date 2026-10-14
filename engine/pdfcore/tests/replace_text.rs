//! Replacing a run's text and getting a PDF that still reads back right.

use pdfcore::{Document, Error};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn reopen(doc: &mut Document) -> Document {
    Document::open(&doc.save_to_bytes().expect("save")).expect("reopen")
}

#[test]
fn a_standard_font_line_can_be_replaced_and_read_back() {
    let mut doc = open("standard");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text(0, &run, "Hola Mundo cruel").unwrap();
    assert!(done.new_width > done.old_width);

    let again = reopen(&mut doc);
    let runs = again.text_runs(0).unwrap();
    assert_eq!(runs[0].text, "Hola Mundo cruel");
    assert_eq!(runs[0].base_font, "Helvetica");
    assert_eq!(runs[0].origin, run.origin, "el texto no debe moverse");
    assert_eq!(runs[0].size, run.size);
    assert_eq!(runs[1].text, "Negrita áéíóú ñ", "las demás líneas quedan como estaban");
}

#[test]
fn accented_letters_replace_through_winansi() {
    let mut doc = open("standard");
    let run = doc.text_runs(0).unwrap().remove(1);
    doc.replace_text(0, &run, "Ñandú ¿qué?").unwrap();
    assert_eq!(reopen(&mut doc).text_runs(0).unwrap()[1].text, "Ñandú ¿qué?");
}

#[test]
fn an_embedded_subset_font_accepts_characters_it_already_has() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    // Every letter and digit below appears in "Póliza número 12345".
    doc.replace_text(0, &run, "Póliza 54321 número").unwrap();
    let again = reopen(&mut doc);
    let after = &again.text_runs(0).unwrap()[0];
    assert_eq!(after.text, "Póliza 54321 número");
    assert_eq!(after.base_font, run.base_font);
}

#[test]
fn a_character_the_subset_lacks_fails_cleanly_and_changes_nothing() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    match doc.replace_text(0, &run, "Póliza XQ") {
        Err(Error::MissingGlyphs(missing)) => assert_eq!(missing, vec!['X', 'Q']),
        other => panic!("esperaba MissingGlyphs, salió {other:?}"),
    }
    assert_eq!(reopen(&mut doc).text_runs(0).unwrap()[0].text, "Póliza número 12345");
}

#[test]
fn several_edits_from_one_reading_all_land() {
    let mut doc = open("standard");
    let runs = doc.text_runs(0).unwrap();
    doc.replace_text(0, &runs[0], "Uno").unwrap();
    doc.replace_text(0, &runs[1], "Dos").unwrap();
    let again = reopen(&mut doc).text_runs(0).unwrap();
    assert_eq!((again[0].text.as_str(), again[1].text.as_str()), ("Uno", "Dos"));
}

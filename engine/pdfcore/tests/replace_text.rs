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

#[test]
fn a_simple_embedded_subset_font_does_not_get_codes_it_cannot_draw() {
    // The encoding has a code for every letter, but the subset kept only the
    // glyphs its text used. Guessing a letter's code from the encoding drew the
    // font's missing-glyph box in a real CV (a "y" in a bold label).
    let mut doc = open("simple_subset");
    let run = doc.text_runs(0).unwrap().remove(0);
    assert_eq!(run.text, "Poliza numero 12345");
    // Letters the text already used are fine...
    doc.replace_text(0, &run, "Poliza 54321 numero").unwrap();
    // ...letters it never used are not in the font.
    let run = doc.text_runs(0).unwrap().remove(0);
    match doc.replace_text(0, &run, "Poliza y") {
        Err(Error::MissingGlyphs(chars)) => assert_eq!(chars, vec!['y']),
        other => panic!("esperaba MissingGlyphs, salió {other:?}"),
    }
}

#[test]
fn the_fallback_draws_what_a_simple_subset_font_lacks() {
    let mut doc = open("simple_subset");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text_with_fallback(0, &run, "Poliza y Ñ").unwrap();
    assert_eq!(done.fallback_chars.iter().collect::<String>(), "yÑ");
    let texts: String = reopen(&mut doc).text_runs(0).unwrap().iter().filter(|r| r.origin.1 > 600.0).map(|r| r.text.clone()).collect();
    assert!(texts.contains("Poliza y Ñ") || texts.replace(' ', "").contains("PolizayÑ"), "{texts:?}");
}

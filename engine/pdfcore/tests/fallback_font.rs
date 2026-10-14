//! Characters the document's font cannot draw, drawn with the bundled one.

use pdfcore::{Document, Error};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn reopen(doc: &mut Document) -> Document {
    Document::open(&doc.save_to_bytes().expect("save")).expect("reopen")
}

fn page_text(doc: &Document) -> String {
    doc.text_runs(0).unwrap().iter().map(|r| r.text.as_str()).collect()
}

#[test]
fn strict_replace_still_refuses_what_the_font_lacks() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    assert!(matches!(doc.replace_text(0, &run, "Póliza XQ"), Err(Error::MissingGlyphs(_))));
}

#[test]
fn missing_letters_are_drawn_with_the_fallback_and_the_rest_keeps_the_original() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text_with_fallback(0, &run, "Póliza XQ").unwrap();
    assert_eq!(done.fallback_chars, vec!['X', 'Q']);
    assert!(done.reread, "se insertaron cambios de fuente: hay que volver a leer");

    let again = reopen(&mut doc);
    assert_eq!(page_text(&again), "Póliza XQ");
    let runs = again.text_runs(0).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "Póliza ");
    assert_eq!(runs[0].base_font, run.base_font, "lo que se pudo, con la fuente original");
    assert_eq!(runs[1].text, "XQ");
    assert_eq!(runs[1].base_font, "LiberationSans-Regular");
    // The fallback letters start exactly where the original ones ended.
    let gap = runs[1].origin.0 - runs[0].end.0;
    assert!(gap.abs() < 0.05, "hueco de {gap} pt entre una fuente y la otra");
    assert_eq!(runs[1].size, run.size);
}

#[test]
fn the_next_line_keeps_its_own_font_after_a_font_switch() {
    // Ā is outside WinAnsi, so the Helvetica line needs the fallback for it.
    // The font switch must be undone afterwards, or the line after it would be
    // drawn in the fallback, in the wrong face and size.
    let mut doc = open("standard");
    let runs = doc.text_runs(0).unwrap();
    let done = doc.replace_text_with_fallback(0, &runs[0], "Hola Ā").unwrap();
    assert_eq!(done.fallback_chars, vec!['Ā']);

    let again = reopen(&mut doc).text_runs(0).unwrap();
    assert_eq!(again.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["Hola ", "Ā", "Negrita áéíóú ñ"]);
    assert_eq!(again[0].base_font, "Helvetica");
    assert_eq!(again[1].base_font, "LiberationSans-Regular");
    assert_eq!(again[2].base_font, "Helvetica-Bold");
    assert_eq!(again[2].size, 14.0);
    assert_eq!(again[2].origin, runs[1].origin, "la línea siguiente no se movió");
}

#[test]
fn a_font_the_original_can_cover_needs_no_fallback_and_no_reread() {
    let mut doc = open("standard");
    let runs = doc.text_runs(0).unwrap();
    let done = doc.replace_text_with_fallback(0, &runs[0], "Hola €").unwrap();
    assert!(done.fallback_chars.is_empty() && !done.reread);
}

#[test]
fn what_neither_font_can_draw_fails_and_changes_nothing() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    match doc.replace_text_with_fallback(0, &run, "Póliza 世界") {
        Err(Error::MissingGlyphs(chars)) => assert_eq!(chars, vec!['世', '界']),
        other => panic!("esperaba MissingGlyphs, salió {other:?}"),
    }
    assert_eq!(page_text(&reopen(&mut doc)), "Póliza número 12345");
}

#[test]
fn the_fallback_font_is_embedded_once_however_often_it_is_used() {
    let size_after = |edits: &[&str]| {
        let mut doc = open("embedded");
        for text in edits {
            let run = doc.text_runs(0).unwrap().remove(0);
            doc.replace_text_with_fallback(0, &run, text).unwrap();
        }
        doc.save_to_bytes().unwrap().len()
    };
    let one = size_after(&["Póliza X"]);
    // The first run is now "Póliza "; each later edit rewrites it again.
    let three = size_after(&["Póliza X", "Póliza Y", "Póliza Z"]);
    assert!(three < one + 5_000, "{one} -> {three}: parece que se incrustó otra vez (~200 KB cada vez)");
}

#[test]
fn spanish_letters_missing_from_a_subset_are_covered() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text_with_fallback(0, &run, "Ñandú ¿qué?").unwrap();
    assert!(done.fallback_chars.contains(&'Ñ'));
    assert_eq!(page_text(&reopen(&mut doc)), "Ñandú ¿qué?");
}

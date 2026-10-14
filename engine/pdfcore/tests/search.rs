//! Lines, words, search and replace-all.

use pdfcore::{Document, ReplaceOptions, SearchOptions};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn all_text(doc: &Document) -> Vec<String> {
    (0..doc.page_count()).flat_map(|p| doc.text_lines(p).unwrap().into_iter().map(|l| l.text)).collect()
}

#[test]
fn runs_on_one_baseline_make_one_line_of_words() {
    let doc = open("line_absolute");
    let lines = doc.text_lines(0).unwrap();
    let line = lines.iter().find(|l| l.text.starts_with("El resto")).expect("the three-run line");
    assert_eq!(line.text, "El resto de las oficinas de lunes a viernes de 9 a 14 horas");
    assert_eq!(line.runs.len(), 3);
    let words: Vec<&str> = line.words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(words, ["El", "resto", "de", "las", "oficinas", "de", "lunes", "a", "viernes", "de", "9", "a", "14", "horas"]);
    // Word boxes run left to right inside the line's box.
    for pair in line.words.windows(2) {
        assert!(pair[0].display[2] <= pair[1].display[0] + 0.01, "{:?} then {:?}", pair[0], pair[1]);
    }
    assert!(line.words[0].display[0] >= line.display[0] - 0.01 && line.words[13].display[2] <= line.display[2] + 0.01);
    // Lines come top to bottom.
    let tops: Vec<f32> = lines.iter().map(|l| l.display[1]).collect();
    assert!(tops.windows(2).all(|w| w[0] <= w[1]), "{tops:?}");
}

#[test]
fn search_finds_text_and_says_where() {
    let doc = open("multi");
    let hits = doc.find_text("prudential", SearchOptions::default());
    // Per page: the title, then "Prudential y Prudential Mexico, prudential" (3).
    assert_eq!(hits.len(), 3 * 4);
    assert_eq!(hits[0].page, 0);
    assert_eq!(hits[0].text, "Prudential");
    assert!(hits[0].display[2] > hits[0].display[0] && hits[0].display[3] > hits[0].display[1]);
    // A hit later in the same line lies to the right of an earlier one.
    assert!(hits[2].display[0] > hits[1].display[0]);

    let exact = doc.find_text("prudential", SearchOptions { case_sensitive: true, ..Default::default() });
    assert_eq!(exact.len(), 3, "only the lower-case one on each page");
}

#[test]
fn accents_are_ignored_unless_asked_for() {
    let doc = open("multi");
    assert_eq!(doc.find_text("perez nunez", SearchOptions::default()).len(), 3);
    assert_eq!(doc.find_text("perez nunez", SearchOptions { accent_sensitive: true, ..Default::default() }).len(), 0);
    assert_eq!(doc.find_text("pérez núñez", SearchOptions { accent_sensitive: true, ..Default::default() }).len(), 3);
}

#[test]
fn whole_word_leaves_out_longer_words() {
    let doc = open("multi");
    assert_eq!(doc.find_text("poliza", SearchOptions::default()).len(), 3 * 2, "Polizas and poliza");
    assert_eq!(doc.find_text("poliza", SearchOptions { whole_word: true, ..Default::default() }).len(), 3);
}

#[test]
fn a_match_may_cross_from_one_string_to_the_next() {
    let doc = open("multi");
    // "Cliente: " is regular and the name is bold: two strings, one line.
    let hits = doc.find_text("cliente: perez", SearchOptions::default());
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().all(|h| h.within_run.is_none()));
    let inside = doc.find_text("nunez", SearchOptions::default());
    assert!(inside.iter().all(|h| h.within_run.is_some()));
}

#[test]
fn replace_all_changes_every_page_in_one_undoable_step() {
    let mut doc = open("multi");
    let before = all_text(&doc);
    let done = doc
        .replace_all("Prudential", "Metlife", SearchOptions { case_sensitive: true, ..Default::default() }, ReplaceOptions::default())
        .unwrap();
    // The title and the first word of line two, twice per page, none lower-case.
    assert_eq!(done.replaced, 3 * 3);
    assert_eq!(done.skipped, 0);
    let after = all_text(&doc);
    assert!(after.iter().any(|l| l == "Contrato 1 de la aseguradora Metlife"));
    assert!(after.iter().any(|l| l.starts_with("Metlife y Metlife Mexico, prudential")), "{after:?}");
    assert!(doc.find_text("Prudential", SearchOptions { case_sensitive: true, ..Default::default() }).is_empty());

    // One undo puts back all three pages.
    assert!(doc.undo());
    assert_eq!(all_text(&doc), before);
    assert!(!doc.can_undo());
    // ...and a copy saved after the edit reads the same as the live one.
    doc.redo();
    let mut saved = Document::open(&doc.save_to_bytes().unwrap()).unwrap();
    assert_eq!(all_text(&saved), after);
    let _ = saved.undo();
}

#[test]
fn replacing_what_is_not_there_changes_nothing_and_leaves_no_step() {
    let mut doc = open("multi");
    let done = doc.replace_all("Zurich", "Metlife", SearchOptions::default(), ReplaceOptions::default()).unwrap();
    assert_eq!((done.replaced, done.skipped), (0, 0));
    assert!(!doc.can_undo());
    assert!(doc.replace_all("", "x", SearchOptions::default(), ReplaceOptions::default()).is_err());
}

#[test]
fn a_match_across_two_strings_is_skipped_not_mangled() {
    let mut doc = open("multi");
    let before = all_text(&doc);
    let done = doc.replace_all("cliente: perez", "X", SearchOptions::default(), ReplaceOptions::default()).unwrap();
    assert_eq!((done.replaced, done.skipped), (0, 3));
    assert_eq!(all_text(&doc), before);
}

#[test]
fn a_replacement_that_contains_the_search_text_terminates() {
    let mut doc = open("multi");
    let done = doc
        .replace_all("Prudential", "Prudential Financial", SearchOptions { case_sensitive: true, ..Default::default() }, ReplaceOptions::default())
        .unwrap();
    assert_eq!(done.replaced, 9);
    assert!(all_text(&doc).iter().any(|l| l == "Contrato 3 de la aseguradora Prudential Financial"));
}

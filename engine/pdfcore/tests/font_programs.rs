//! Fonts that are not TrueType: the engine reads their programs to know which
//! letters they really draw, so it keeps the original typeface for the letters
//! it has and turns to the bundled one only for the rest.

use pdfcore::{Document, Error, ReplaceOptions};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

/// Letters a font lacks, from the sidecar the fixture builder wrote.
fn missing(font: &str) -> Vec<char> {
    let path = format!("{}/tests/fixtures/fonts_missing.txt", env!("CARGO_MANIFEST_DIR"));
    let all = std::fs::read_to_string(path).unwrap();
    all.lines().find_map(|l| l.strip_prefix(&format!("{font} "))).unwrap().chars().collect()
}

fn first_run(doc: &Document) -> pdfcore::TextRun {
    doc.text_runs(0).unwrap().into_iter().next().unwrap()
}

const WITHOUT: ReplaceOptions = ReplaceOptions { allow_fallback: false, reflow: pdfcore::Reflow::None };
const WITH: ReplaceOptions = ReplaceOptions { allow_fallback: true, reflow: pdfcore::Reflow::None };

/// Letters of the new text that the font has but the old text never used must
/// keep the font; a letter it lacks must say so, or use the bundled font.
fn check(fixture: &str, font: &str) {
    // Letters the page never drew, all of them in the font program.
    let mut doc = open(fixture);
    let run = first_run(&doc);
    let done = doc
        .replace_text_with(0, &run, "Adios mundo cruel", WITHOUT)
        .unwrap_or_else(|e| panic!("{fixture}: letters the font has must not need the fallback: {e}"));
    assert!(done.fallback_chars.is_empty(), "{fixture}: {:?}", done.fallback_chars);
    assert_eq!(first_run(&doc).text, "Adios mundo cruel", "{fixture}");
    assert_eq!(first_run(&doc).base_font, run.base_font, "{fixture}: still the document's own font");

    // A letter the font lacks.
    let lacking = missing(font);
    let letter = *lacking.first().unwrap_or_else(|| panic!("{font} should lack some letter"));
    let mut doc = open(fixture);
    let run = first_run(&doc);
    let text = format!("Hola {letter}");
    match doc.replace_text_with(0, &run, &text, WITHOUT) {
        Err(Error::MissingGlyphs(chars)) => assert!(chars.contains(&letter), "{fixture}: {chars:?}"),
        other => panic!("{fixture}: {letter} must be refused without the fallback, got {other:?}"),
    }
    let done = doc.replace_text_with(0, &run, &text, WITH).unwrap();
    assert_eq!(done.fallback_chars, vec![letter], "{fixture}");
}

#[test]
fn an_opentype_cff_simple_font_keeps_its_own_letters() {
    check("cff_simple", "loma");
}

#[test]
fn a_bare_cff_font_keeps_its_own_letters() {
    check("cff_bare", "loma");
}

#[test]
fn a_type1_font_keeps_its_own_letters() {
    check("type1_simple", "charter");
}

#[test]
fn a_composite_cff_font_does_not_write_an_empty_glyph_for_a_letter_it_lacks() {
    // The document's ToUnicode says an (empty) en space glyph is Ž. Writing it
    // would draw nothing, with no error.
    let mut doc = open("cff_type0");
    let run = first_run(&doc);
    match doc.replace_text_with(0, &run, "Hola Ž", WITHOUT) {
        Err(Error::MissingGlyphs(chars)) => assert_eq!(chars, vec!['Ž']),
        other => panic!("Ž must be refused, got {other:?}"),
    }
    let done = doc.replace_text_with(0, &run, "Hola Ž", WITH).unwrap();
    assert_eq!(done.fallback_chars, vec!['Ž']);
    // Letters the font does draw still use it.
    let mut doc = open("cff_type0");
    let run = first_run(&doc);
    let done = doc.replace_text_with(0, &run, "Hola nandú", WITHOUT).unwrap();
    assert!(done.fallback_chars.is_empty());
}

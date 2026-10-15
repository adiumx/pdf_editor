//! Letters a font lacks: the same face when the reader has it, the bundled font otherwise.

use pdfcore::{Document, Via};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

#[test]
fn a_standard_font_learns_the_letter_and_stays_the_same_face() {
    // Helvetica with no /Encoding is StandardEncoding, which has no n-tilde.
    let mut doc = open("font_standardenc");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text(0, &run, "mañana").unwrap();
    assert_eq!(done.substitutes.len(), 1);
    assert_eq!((done.substitutes[0].ch, done.substitutes[0].via), ('ñ', Via::Encoding));
    assert!(done.fallback_chars.is_empty());
    // Read back: the same font, the letter is text.
    let runs = doc.text_runs(0).unwrap();
    assert_eq!(runs[0].text, "mañana");
    assert_eq!(runs[0].base_font, "Helvetica");
    // Undo puts everything back.
    assert!(doc.undo());
    assert_eq!(doc.text_runs(0).unwrap()[0].base_font, "Helvetica");
    assert!(!doc.can_undo());
}

#[test]
fn a_font_with_widths_gets_a_width_for_the_new_letter() {
    let mut doc = open("font_nonstd_notembedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    // WinAnsi has n-tilde already; Euro sign is in WinAnsi at 0x80 but outside Widths (32..126).
    doc.replace_text(0, &run, "Hola ñ €").unwrap();
    let after = doc.text_runs(0).unwrap();
    assert_eq!(after[0].text, "Hola ñ €");
    // The euro sign is outside the font's /Widths: it must not be drawn with no width.
    assert!(after[0].glyphs.last().unwrap().width > 300.0, "{:?}", after[0].glyphs.last());
}

#[test]
fn an_embedded_subset_falls_to_the_bundled_font_and_says_so() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text_with_fallback(0, &run, "Ñandú").unwrap();
    assert!(done.substitutes.iter().all(|s| s.via == Via::BundledFont));
    assert!(!done.substitutes.is_empty());
}

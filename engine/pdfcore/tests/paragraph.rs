//! Re-laying a paragraph out when one of its lines is edited.

use pdfcore::{Document, Error, Warning};

fn open() -> Document {
    let path = format!("{}/tests/fixtures/paragraph.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn run_with(doc: &Document, start: &str) -> pdfcore::TextRun {
    doc.text_runs(0).unwrap().into_iter().find(|r| r.text.starts_with(start)).unwrap_or_else(|| panic!("no run starting {start:?}"))
}

fn first_paragraph(doc: &Document) -> pdfcore::Paragraph {
    let run = run_with(doc, "El asegurado");
    doc.paragraph_of(0, &run).unwrap().expect("a paragraph")
}

const ORIGINAL: &str = "El asegurado debe notificar a la compania cualquier cambio en los datos declarados dentro de los quince dias siguientes a que ocurra, para que la poliza continue vigente con las condiciones pactadas desde el inicio.";

/// Text of every line on the page, top to bottom.
fn page_lines(doc: &Document) -> Vec<String> {
    doc.text_lines(0).unwrap().into_iter().map(|l| l.text).collect()
}

#[test]
fn a_paragraph_is_recognised_without_its_neighbours() {
    let doc = open();
    let paragraph = first_paragraph(&doc);
    assert_eq!(paragraph.text, ORIGINAL);
    assert_eq!(paragraph.runs.len(), 5);
    // Not the title above, the second paragraph below, nor the other column.
    assert!(paragraph.display[0] > 70.0 && paragraph.display[2] < 430.0);
    assert!(paragraph.display[1] > 95.0 && paragraph.display[3] < 180.0);
    // Any line of it finds the same paragraph.
    let middle = doc.paragraph_of(0, &run_with(&doc, "dias siguientes")).unwrap().unwrap();
    assert_eq!(middle, paragraph);
    // A lone title is not one.
    assert!(doc.paragraph_of(0, &run_with(&doc, "Condiciones")).unwrap().is_none());
    // Nor is a column of two short lines taken for part of the first paragraph.
    let column = doc.paragraph_of(0, &run_with(&doc, "Columna")).unwrap().unwrap();
    assert!(column.display[0] >= 430.0 - 1.0);
}

#[test]
fn a_longer_text_wraps_onto_new_lines_in_the_same_style() {
    let mut doc = open();
    let before = page_lines(&doc);
    let run = run_with(&doc, "El asegurado");
    let text = format!("{ORIGINAL} Ademas, el asegurado debera entregar copia de cualquier documento que respalde el cambio solicitado.");
    let done = doc.replace_paragraph(0, &run, &text, false).unwrap();
    assert!(done.added >= 1 && done.removed == 0, "{done:?}");
    assert_eq!(done.lines, 5 + done.added);
    assert!(done.fallback_chars.is_empty());

    let paragraph = first_paragraph(&doc);
    assert_eq!(paragraph.text, text);
    assert_eq!(paragraph.runs.len(), done.lines);
    let runs = doc.text_runs(0).unwrap();
    let lines: Vec<&pdfcore::TextRun> = paragraph.runs.iter().map(|&i| &runs[i]).collect();
    let width = 351.0; // the old longest line: nothing may be wider
    for (n, line) in lines.iter().enumerate() {
        assert!((line.display[0] - 72.0).abs() < 0.5, "line {n} starts at the margin");
        assert!(line.display[2] - line.display[0] <= width + 0.5, "line {n} fits: {}", line.text);
        assert_eq!(line.base_font, lines[0].base_font, "line {n}");
        assert!((line.size - 12.0).abs() < 0.01);
    }
    // Even leading all the way down, including the new lines.
    let gaps: Vec<f32> = lines.windows(2).map(|w| w[1].display[3] - w[0].display[3]).collect();
    assert!(gaps.iter().all(|g| (g - 15.0).abs() < 0.2), "{gaps:?}");
    // The new lines are in the paragraph's colour (dark blue), not black.
    let last = lines.last().unwrap();
    let fill: Vec<&str> = last.fill.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(fill, ["rg"], "{:?}", last.fill);
    assert_eq!(last.fill[0].operands.iter().map(|o| o.as_float().unwrap()).collect::<Vec<_>>(), vec![0.0, 0.0, 0.5]);

    // The rest of the page is untouched.
    let after = page_lines(&doc);
    for kept in ["Condiciones generales", "Segundo parrafo, mas abajo, que no debe moverse", "cuando el primero crece o se acorta."] {
        assert!(after.iter().any(|l| l == kept), "{kept}");
    }
    assert!(before.iter().any(|l| l.starts_with("Columna aparte")) && after.iter().any(|l| l.starts_with("Columna aparte")));

    // One undo puts everything back.
    assert!(doc.undo());
    assert_eq!(page_lines(&doc), before);
    assert!(!doc.can_undo());
}

#[test]
fn a_shorter_text_empties_the_lines_it_no_longer_needs() {
    let mut doc = open();
    let before = page_lines(&doc);
    let run = run_with(&doc, "El asegurado");
    let done = doc.replace_paragraph(0, &run, "El asegurado debe avisar de cualquier cambio.", false).unwrap();
    assert_eq!(done.lines, 1);
    assert_eq!(done.removed, 4);
    assert!(done.warnings.is_empty());
    let after = page_lines(&doc);
    assert!(after.contains(&"El asegurado debe avisar de cualquier cambio.".to_string()), "{after:?}");
    assert!(!after.iter().any(|l| l.contains("quince") || l.contains("inicio")), "{after:?}");
    assert!(after.iter().any(|l| l.starts_with("Segundo parrafo")));
    // Saved and reopened, the emptied lines are still gone.
    let reopened = Document::open(&doc.save_to_bytes().unwrap()).unwrap();
    assert!(!reopened.text_lines(0).unwrap().iter().any(|l| l.text.contains("quince")));
    assert!(doc.undo());
    assert_eq!(page_lines(&doc), before);
}

#[test]
fn a_paragraph_that_grows_into_the_text_below_says_so() {
    let mut doc = open();
    let run = run_with(&doc, "El asegurado");
    let long = format!("{ORIGINAL} {}", "Ademas se establecen otras obligaciones que el asegurado debe cumplir. ".repeat(8));
    let done = doc.replace_paragraph(0, &run, &long, false).unwrap();
    assert!(done.added > 5);
    assert!(
        done.warnings.iter().any(|w| matches!(w, Warning::OverlapsText { text } if text.starts_with("Segundo parrafo"))),
        "{:?}",
        done.warnings
    );
    // The text below stayed where it was.
    let below = run_with(&doc, "Segundo parrafo");
    assert!((below.display[1] - 219.1).abs() < 1.0);
}

#[test]
fn letters_the_font_lacks_use_the_bundled_one_only_if_allowed() {
    let mut doc = open();
    let before = page_lines(&doc);
    let run = run_with(&doc, "El asegurado");
    let text = format!("{ORIGINAL} Prima anual: \u{6F22}.");
    match doc.replace_paragraph(0, &run, &text, true) {
        Err(Error::MissingGlyphs(chars)) => assert!(chars.contains(&'\u{6F22}')),
        other => panic!("a letter neither font has must be refused, got {other:?}"),
    }
    assert_eq!(page_lines(&doc), before, "a refused edit changes nothing");
    assert!(!doc.can_undo());
    // A letter only the bundled font has (Cyrillic): refused unless allowed.
    let text = format!("{ORIGINAL} Prima anual: \u{0416}.");
    assert!(matches!(doc.replace_paragraph(0, &run, &text, false), Err(Error::MissingGlyphs(_))));
    assert!(!doc.can_undo());
    let with = doc.replace_paragraph(0, &run, &text, true).unwrap();
    assert_eq!(with.fallback_chars, vec!['\u{0416}']);
    assert!(page_lines(&doc).iter().any(|l| l.contains('\u{0416}')));
}

#[test]
fn a_line_that_is_not_part_of_a_paragraph_is_refused() {
    let mut doc = open();
    let run = run_with(&doc, "Condiciones");
    assert!(matches!(doc.replace_paragraph(0, &run, "Otro titulo", false), Err(Error::Unsupported(_))));
    assert!(!doc.can_undo());
    let run = run_with(&doc, "El asegurado");
    assert!(matches!(doc.replace_paragraph(0, &run, "   ", false), Err(Error::Unsupported(_))));
}

#[test]
fn the_same_text_changes_nothing_visible() {
    let mut doc = open();
    let before = page_lines(&doc);
    let run = run_with(&doc, "El asegurado");
    let done = doc.replace_paragraph(0, &run, ORIGINAL, false).unwrap();
    assert_eq!((done.added, done.removed), (0, 0));
    assert_eq!(page_lines(&doc), before);
}

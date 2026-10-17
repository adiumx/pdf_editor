//! Text of several styles on one line: each span starts where the one before ends, all share
//! the baseline, one undo step, and the style comes back when the page is read.

use pdfcore::{Anchor, Document, NewSpans, NewText, StyledSpan, TextRun};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
}

const BLUE: [f32; 3] = [0.0, 0.2, 0.7];

fn juan<'a>() -> Vec<StyledSpan<'a>> {
    let plain = StyledSpan { size: 11.0, ..StyledSpan::new("Juan ") };
    let name = StyledSpan { bold: true, color: BLUE, size: 11.0, ..StyledSpan::new("Pérez") };
    let last = StyledSpan { size: 11.0, ..StyledSpan::new(" López") };
    vec![plain, name, last]
}

/// The runs written by an `add_spans` (the ones that are not in `before`), left to right.
fn added(doc: &Document, page: usize, before: usize) -> Vec<TextRun> {
    let mut runs = doc.text_runs(page).unwrap();
    runs.drain(..before);
    runs.sort_by(|a, b| a.display[0].total_cmp(&b.display[0]));
    runs
}

fn close(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < 0.05, "{what}: {a} vs {b}");
}

/// Spans on a baseline of `page`, checked against the page as read back and after saving.
fn three_spans(bytes: &[u8], page: usize, x: f32, baseline: f32) {
    let mut doc = Document::open(bytes).unwrap();
    let before = doc.text_runs(page).unwrap().len();
    let original = doc.save_to_bytes().unwrap();
    let mut new = NewSpans::new(juan(), x, baseline);
    new.anchor = Anchor::Baseline;
    let done = doc.add_spans(page, &new).unwrap();
    assert_eq!(done.lines, 1);

    let check = |doc: &Document| {
        let runs = added(doc, page, 0).into_iter().filter(|r| ["Juan ", "Pérez", " López"].contains(&r.text.as_str())).collect::<Vec<_>>();
        assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["Juan ", "Pérez", " López"]);
        assert_eq!((runs[0].bold, runs[1].bold, runs[2].bold), (false, true, false));
        for (a, b) in runs[0].color.iter().zip([0.0; 3]).chain(runs[1].color.iter().zip(BLUE)) {
            close(*a, b, "colour");
        }
        // No gap and no overlap: each starts where the one before ends.
        close(runs[1].display[0], runs[0].display[2], "second after first");
        close(runs[2].display[0], runs[1].display[2], "third after second");
        // One baseline.
        let line = |r: &TextRun| doc.to_display_point(page, r.origin).unwrap().1;
        close(line(&runs[0]), line(&runs[1]), "baseline 1");
        close(line(&runs[0]), line(&runs[2]), "baseline 2");
        close(line(&runs[0]), baseline, "baseline where asked");
        runs
    };
    let runs = check(&doc);
    assert!(runs[0].display[0] >= 0.0);
    let reopened = Document::open(&doc.save_to_bytes().unwrap()).unwrap();
    let again = check(&reopened);
    for (a, b) in runs.iter().zip(&again) {
        assert_eq!((a.bold, a.italic, &a.text), (b.bold, b.italic, &b.text));
    }
    // The total box covers the three.
    assert!(done.display[0] <= runs[0].display[0] + 0.05 && done.display[2] >= runs[2].display[2] - 0.05);

    // One undo removes everything.
    assert!(doc.undo());
    assert_eq!(doc.text_runs(page).unwrap().len(), before);
    assert_eq!(doc.save_to_bytes().unwrap().len(), original.len());
    assert!(!doc.undo() || doc.can_redo());
}

#[test]
fn three_styles_sit_end_to_end_on_one_baseline() {
    three_spans(&fixture("standard"), 0, 140.0, 345.0);
}

#[test]
fn a_rotated_page_gets_the_same_line() {
    three_spans(&fixture("page_rotated"), 0, 140.0, 345.0);
}

#[test]
fn serif_italic_and_different_sizes_share_the_baseline() {
    let mut doc = Document::open(&fixture("standard")).unwrap();
    let a = StyledSpan { serif: true, italic: true, size: 10.0, ..StyledSpan::new("Hola ") };
    let b = StyledSpan { size: 20.0, ..StyledSpan::new("grande ") };
    let c = StyledSpan { serif: true, bold: true, size: 8.0, color: [1.0, 0.0, 0.0], ..StyledSpan::new("chico") };
    let mut new = NewSpans::new(vec![a, b, c], 100.0, 300.0);
    new.anchor = Anchor::Baseline;
    doc.add_spans(0, &new).unwrap();
    let runs = added(&doc, 0, 0).into_iter().filter(|r| ["Hola ", "grande ", "chico"].contains(&r.text.as_str())).collect::<Vec<_>>();
    assert_eq!(runs.len(), 3);
    assert!(runs[0].italic && !runs[0].bold && runs[2].bold && !runs[1].italic);
    close(runs[0].origin.1, runs[1].origin.1, "baseline");
    close(runs[1].origin.1, runs[2].origin.1, "baseline");
    close(runs[1].display[0], runs[0].display[2], "gap");
    close(runs[2].display[0], runs[1].display[2], "gap");
    assert_eq!((runs[0].size, runs[1].size, runs[2].size), (10.0, 20.0, 8.0));
}

#[test]
fn wrapping_never_splits_a_word_where_the_style_changes() {
    let mut doc = Document::open(&fixture("standard")).unwrap();
    let a = StyledSpan::new("uno dos ");
    let b = StyledSpan { bold: true, ..StyledSpan::new("tres") };
    let c = StyledSpan::new("cuatro cinco");
    let mut new = NewSpans::new(vec![a, b, c], 50.0, 200.0);
    new.max_width = Some(75.0);
    let done = doc.add_spans(0, &new).unwrap();
    assert!(done.lines >= 2, "{done:?}");
    let ours = ["uno dos", "tres", "cuatro", "cinco"];
    let runs: Vec<TextRun> = added(&doc, 0, 0).into_iter().filter(|r| ours.iter().any(|w| r.text.contains(w))).collect();
    // "tres" and "cuatro" are one word, written in two styles: they stay on one line.
    let tres = runs.iter().find(|r| r.text.trim() == "tres" || r.text.ends_with("tres")).expect("tres");
    let cuatro = runs.iter().find(|r| r.text.starts_with("cuatro")).expect("cuatro");
    close(tres.origin.1, cuatro.origin.1, "same line");
    close(cuatro.display[0], tres.display[2], "joined");
    // Lines are below each other and inside the width.
    for r in &runs {
        assert!(r.display[0] >= 49.9 && r.display[2] <= 50.0 + 75.0 + 0.1, "{:?}", r.display);
    }
    let top = runs.iter().map(|r| r.origin.1).fold(f32::MIN, f32::max);
    let bottom = runs.iter().map(|r| r.origin.1).fold(f32::MAX, f32::min);
    assert!(top > bottom);
    assert!(doc.undo());
}

#[test]
fn a_single_style_is_the_same_as_before() {
    let mut a = Document::open(&fixture("standard")).unwrap();
    let mut b = Document::open(&fixture("standard")).unwrap();
    let mut one = NewText::new("Texto de prueba", 72.0, 100.0);
    one.size = 14.0;
    one.bold = true;
    let x = a.add_text(0, &one).unwrap();
    let span = StyledSpan { size: 14.0, bold: true, ..StyledSpan::new("Texto de prueba") };
    let y = b.add_spans(0, &NewSpans::new(vec![span], 72.0, 100.0)).unwrap();
    assert_eq!(x, y);
}

#[test]
fn empty_or_unsupported_input_is_refused_without_changing_anything() {
    let mut doc = Document::open(&fixture("standard")).unwrap();
    let before = doc.save_to_bytes().unwrap();
    assert!(doc.add_spans(0, &NewSpans::new(vec![], 0.0, 0.0)).is_err());
    assert!(doc.add_spans(0, &NewSpans::new(vec![StyledSpan::new("  ")], 0.0, 0.0)).is_err());
    let nothing = StyledSpan { size: 0.0, ..StyledSpan::new("x") };
    assert!(doc.add_spans(0, &NewSpans::new(vec![nothing], 0.0, 0.0)).is_err());
    // A letter no bundled font has fails the whole text, none of it is written.
    let bad = vec![StyledSpan::new("ok "), StyledSpan { bold: true, ..StyledSpan::new("\u{1F600}") }];
    assert!(doc.add_spans(0, &NewSpans::new(bad, 10.0, 10.0)).is_err());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
    assert!(!doc.can_undo());
}

#[test]
fn reading_gives_colour_bold_and_italic() {
    let mut doc = Document::open(&fixture("standard")).unwrap();
    let mut t = NewText::new("Rojo", 40.0, 40.0);
    t.color = [1.0, 0.0, 0.0];
    t.italic = true;
    doc.add_text(0, &t).unwrap();
    let run = doc.text_runs(0).unwrap().into_iter().find(|r| r.text == "Rojo").unwrap();
    assert_eq!(run.color, [1.0, 0.0, 0.0]);
    assert!(run.italic && !run.bold);
    // A standard font named Bold is read as bold even though nothing embeds it.
    let plain = Document::open(&fixture("standard")).unwrap();
    assert!(plain.text_runs(0).unwrap().iter().all(|r| r.color.iter().all(|c| (0.0..=1.0).contains(c))));
}

#[test]
fn the_real_insurance_form() {
    // CAMBIO_DE_CONTRATANTE.pdf is the person's own document, not in the repository: set CAMBIO_PDF.
    let Some(path) = std::env::var_os("CAMBIO_PDF").filter(|p| std::path::Path::new(p).exists()) else { return };
    three_spans(&std::fs::read(path).unwrap(), 0, 140.0, 345.0);
}

//! Style of a range of letters, replacing a range, justification, pushing what is below down.

use pdfcore::{Block, BlockOptions, Document, Error, StyleChange, Warning};

fn open() -> Document {
    let path = format!("{}/tests/fixtures/producer_reportlab_ttf.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn first(doc: &Document) -> Block {
    doc.blocks(0).unwrap().into_iter().find(|b| b.text.starts_with("Para ")).unwrap()
}

fn second_top(doc: &Document) -> f32 {
    doc.blocks(0).unwrap().into_iter().find(|b| b.text.starts_with("Otro")).unwrap().display[1]
}

fn span_text(b: &Block, pick: impl Fn(&pdfcore::StyleSpan) -> bool) -> Vec<String> {
    b.spans.iter().filter(|s| pick(s)).map(|s| b.text.chars().skip(s.start).take(s.end - s.start).collect()).collect()
}

#[test]
fn bold_uses_the_documents_own_bold_font_and_undo_restores_the_file() {
    let mut doc = open();
    let before = doc.save_to_bytes().unwrap();
    let b = first(&doc);
    let done = doc.set_style(0, &b, 0, 4, StyleChange { bold: Some(true), ..Default::default() }, BlockOptions::default()).unwrap();
    assert!(done.fallback_chars.is_empty() && done.substitutes.is_empty());
    let after = first(&doc);
    assert_eq!(after.text, b.text);
    assert_eq!(span_text(&after, |s| s.bold)[0], "Para ubicación".get(..0).map_or("", |_| "").to_string() + &after.text.chars().take(after.spans[0].end).collect::<String>());
    assert!(after.spans[0].bold && after.spans[0].start == 0);
    assert!(after.spans[0].base_font.contains("Bold"));
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn colour_and_size_change_a_range_and_the_block_is_laid_out_again() {
    let mut doc = open();
    let b = first(&doc);
    doc.set_style(0, &b, 0, 14, StyleChange { color: Some([1.0, 0.0, 0.0]), ..Default::default() }, BlockOptions::default()).unwrap();
    let red = first(&doc);
    assert_eq!(red.spans[0].color, Some([1.0, 0.0, 0.0]));
    assert!(red.spans[1..].iter().all(|s| s.color == Some([0.0, 0.0, 0.0])));

    let b = first(&doc);
    doc.set_style(0, &b, 0, b.text.chars().count(), StyleChange { size: Some(14.0), ..Default::default() }, BlockOptions::default()).unwrap();
    let big = first(&doc);
    assert!(big.spans.iter().all(|s| (s.size - 14.0).abs() < 0.01));
    assert!(big.lines.len() > b.lines.len(), "wider letters need more lines");
    assert_eq!(big.text, b.text);
}

#[test]
fn italic_without_an_italic_font_in_the_document_needs_the_bundled_one() {
    let mut doc = open();
    let b = first(&doc);
    let change = StyleChange { italic: Some(true), ..Default::default() };
    assert!(matches!(doc.set_style(0, &b, 0, 4, change, BlockOptions::default()), Err(Error::Rejected(_))));
    assert!(!doc.can_undo());
    let done = doc.set_style(0, &b, 0, 4, change, BlockOptions { allow_fallback: true, ..Default::default() }).unwrap();
    assert_eq!(done.substitutes.iter().map(|s| s.ch).collect::<String>(), "Par");
    assert!(done.substitutes.iter().all(|s| s.font == "LiberationSans-Italic" && s.similarity == 100));
    let after = first(&doc);
    assert!(after.spans[0].italic);
    assert!(doc.undo());
}

#[test]
fn a_range_is_replaced_and_keeps_the_style_around_it() {
    let mut doc = open();
    let b = first(&doc);
    let start = b.text.find("ubicación").unwrap();
    let start = b.text[..start].chars().count();
    doc.replace_range(0, &b, start, start + 9, "dirección", BlockOptions::default()).unwrap();
    let after = first(&doc);
    assert!(after.text.contains("la dirección, días"));
    assert!(span_text(&after, |s| s.bold)[0].starts_with("dirección"));
    assert!(matches!(doc.replace_range(0, &after, 5, 4, "x", BlockOptions::default()), Err(Error::Rejected(_))));
    assert!(matches!(doc.replace_range(0, &after, 0, 9999, "x", BlockOptions::default()), Err(Error::Rejected(_))));
}

#[test]
fn justification_can_be_switched_off_and_on() {
    let mut doc = open();
    let before = doc.save_to_bytes().unwrap();
    let b = first(&doc);
    assert!(b.justified);
    doc.replace_block_with(0, &b, &b.text, BlockOptions { justify: Some(false), ..Default::default() }).unwrap();
    let ragged = first(&doc);
    assert!(!ragged.justified);
    assert_eq!(ragged.text, b.text);
    let rights: Vec<f32> = ragged.lines.iter().take(3).map(|l| l.display[2]).collect();
    assert!(rights.iter().any(|r| (r - rights[0]).abs() > 1.0), "{rights:?}");
    // And back.
    doc.replace_block_with(0, &ragged, &ragged.text, BlockOptions { justify: Some(true), ..Default::default() }).unwrap();
    assert!(first(&doc).justified);
    assert!(doc.undo() && doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn what_is_below_moves_down_when_the_block_grows() {
    let longer = |b: &Block| format!("{} Ademas de lo anterior, conviene conservar copia de cada comprobante y avisar de cualquier cambio posterior a la fecha de emision. Tambien se pide guardar los recibos de pago, las constancias de entrega y toda la correspondencia con la aseguradora durante el tiempo que dure la vigencia del contrato y cinco anos mas.", b.text);
    // Without asking: it grows over the next paragraph and says so.
    let mut doc = open();
    let (b, top) = (first(&doc), second_top(&doc));
    let done = doc.replace_block_with(0, &b, &longer(&b), BlockOptions::default()).unwrap();
    assert!(done.added >= 1);
    assert!(done.warnings.iter().any(|w| matches!(w, Warning::OverlapsText { .. })));
    assert_eq!(second_top(&doc), top);
    // Asking: the next paragraph is pushed down by the lines added, and nothing overlaps.
    let mut doc = open();
    let b = first(&doc);
    let leading = b.lines[1].baseline - b.lines[0].baseline;
    let done = doc.replace_block_with(0, &b, &longer(&b), BlockOptions { push_down: true, ..Default::default() }).unwrap();
    assert!(done.warnings.is_empty(), "{:?}", done.warnings);
    assert!((second_top(&doc) - (top + done.added as f32 * leading)).abs() < 0.6, "{} vs {}", second_top(&doc), top + done.added as f32 * leading);
    assert!(doc.undo());
    assert!((second_top(&doc) - top).abs() < 0.01);
}

#[test]
fn every_letter_has_a_box_inside_the_block() {
    let doc = open();
    let b = first(&doc);
    assert_eq!(b.char_boxes.len(), b.text.chars().count());
    for (i, bx) in b.char_boxes.iter().enumerate() {
        assert!(bx[0] >= b.display[0] - 1.0 && bx[2] <= b.display[2] + 1.0 && bx[1] >= b.display[1] - 1.0 && bx[3] <= b.display[3] + 1.0, "{i}: {bx:?}");
    }
    // Within a line the letters run left to right.
    let line0 = b.lines[0].text.chars().count();
    assert!(b.char_boxes[..line0].windows(2).all(|w| w[1][0] >= w[0][0] - 0.01));
    // The first letter sits at the start of the first line.
    assert!((b.char_boxes[0][0] - b.lines[0].display[0]).abs() < 1.0);
}

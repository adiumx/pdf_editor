//! Paragraphs as one block with a style per stretch of text.

use pdfcore::{Block, Document, Error};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn block_with(doc: &Document, start: &str) -> Block {
    doc.blocks(0).unwrap().into_iter().find(|b| b.text.starts_with(start)).unwrap_or_else(|| panic!("no block {start:?}"))
}

const MIXED: &str = "Para consultar";

#[test]
fn a_paragraph_with_bold_words_is_one_block() {
    let doc = open("producer_reportlab_ttf");
    let b = block_with(&doc, MIXED);
    assert_eq!(b.lines.len(), 4);
    assert!(b.justified);
    assert!(b.spans.len() >= 5);
    let bold: Vec<String> = b.spans.iter().filter(|s| s.bold).map(|s| b.text.chars().skip(s.start).take(s.end - s.start).collect()).collect();
    assert_eq!(bold, vec!["ubicación, días y horarios de atención; costos del servicio del pasaporte y plazos de entrega", "55 893 24827"]);
    // The spans tile the text.
    assert_eq!(b.spans[0].start, 0);
    assert_eq!(b.spans.last().unwrap().end, b.text.chars().count());
    // The next paragraph is its own block.
    assert!(doc.blocks(0).unwrap().iter().any(|b| b.text.starts_with("Otro párrafo")));
}

#[test]
fn a_first_line_indent_is_seen() {
    let doc = open("producer_reportlab_indent");
    let b = doc.blocks(0).unwrap().remove(0);
    assert!(b.first_line_indent > 10.0, "{}", b.first_line_indent);
}

#[test]
fn a_list_of_labels_is_not_merged() {
    let doc = open("blocks_list");
    assert!(doc.blocks(0).unwrap().iter().all(|b| b.lines.len() == 1));
}

#[test]
fn editing_keeps_the_style_of_each_word_and_undo_restores_the_file() {
    let mut doc = open("producer_reportlab_ttf");
    let before = doc.save_to_bytes().unwrap();
    let b = block_with(&doc, MIXED);
    let text = b.text.replace("55 893 24827", "55 111 22233").replace("Para consultar", "Para saber");
    let done = doc.replace_block(0, &b, &text, false).unwrap();
    assert!(done.rewritten >= 1);

    let after = block_with(&doc, "Para saber");
    assert_eq!(after.text, text);
    assert_eq!(after.lines.len(), 4);
    let bold: Vec<String> = after.spans.iter().filter(|s| s.bold).map(|s| after.text.chars().skip(s.start).take(s.end - s.start).collect()).collect();
    assert_eq!(bold.last().unwrap(), "55 111 22233");
    assert!(bold[0].starts_with("ubicación"));
    // Still justified: every line but the last ends at the same place.
    let right: Vec<f32> = after.lines.iter().map(|l| l.display[2]).collect();
    for r in &right[..right.len() - 1] {
        assert!((r - right[0]).abs() < 1.0, "{right:?}");
    }

    assert!(doc.undo());
    assert!(!doc.can_undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn typed_letters_take_the_style_of_the_letter_before() {
    let mut doc = open("producer_reportlab_ttf");
    let b = block_with(&doc, MIXED);
    let text = b.text.replace("55 893 24827", "55 893 24827 y 9");
    doc.replace_block(0, &b, &text, false).unwrap();
    let after = block_with(&doc, MIXED);
    let last_bold = after.spans.iter().filter(|s| s.bold).last().unwrap();
    let t: String = after.text.chars().skip(last_bold.start).take(last_bold.end - last_bold.start).collect();
    assert_eq!(t, "55 893 24827 y 9");
}

#[test]
fn lines_that_do_not_change_are_not_touched() {
    let mut doc = open("producer_reportlab_ttf");
    let b = block_with(&doc, MIXED);
    // Same words, the last one changed: the first lines are exactly as they were.
    let text = b.text.replace("un generador de PDF.", "un generador de PDF!");
    let done = doc.replace_block(0, &b, &text, false).unwrap();
    assert_eq!(done.kept, b.lines.len() - 1);
    assert_eq!(done.rewritten, 1);
}

#[test]
fn identical_text_is_not_a_step() {
    let mut doc = open("producer_reportlab_ttf");
    let b = block_with(&doc, MIXED);
    let done = doc.replace_block(0, &b, &b.text, false).unwrap();
    assert_eq!(done.rewritten, 0);
    assert!(!doc.can_undo());
}

#[test]
fn a_letter_the_font_lacks_needs_permission() {
    let mut doc = open("producer_reportlab_ttf");
    let b = block_with(&doc, "Otro párrafo");
    let text = b.text.replace("medio", "mediø");
    match doc.replace_block(0, &b, &text, false) {
        Err(Error::MissingGlyphs(chars)) => assert!(chars.contains(&'ø') || chars.iter().any(|c| *c == 'ø')),
        other => panic!("{other:?}"),
    }
    assert!(!doc.can_undo());
}

#[test]
fn a_missing_letter_can_be_drawn_with_the_bundled_font() {
    let mut doc = open("producer_reportlab_ttf");
    let b = block_with(&doc, "Otro párrafo");
    let text = b.text.replace("medio", "mediø");
    let done = doc.replace_block(0, &b, &text, true).unwrap();
    assert_eq!(done.fallback_chars, vec!['ø']);
    assert!(doc.undo());
}

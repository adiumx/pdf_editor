//! `cargo run --example write_on -- in.pdf page "texto de la raya" "texto nuevo" out.pdf [size]`
//! Writes new text starting on the baseline of an existing run (a drawn line of underscores, say).
use pdfcore::{Anchor, Document, NewText};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut doc = Document::open(&std::fs::read(&a[1]).unwrap()).unwrap();
    let page: usize = a[2].parse().unwrap();
    let run = doc.text_runs(page).unwrap().into_iter().find(|r| r.text.trim().starts_with(&a[3])).expect("no hay un tramo así");
    let (x, y) = doc.to_display_point(page, run.origin).unwrap();
    let mut text = NewText::new(&a[4], x + 3.0, y - 1.5);
    text.anchor = Anchor::Baseline;
    text.size = a.get(6).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    doc.add_text(page, &text).unwrap();
    std::fs::write(&a[5], doc.save_to_bytes().unwrap()).unwrap();
}

use pdfcore::*;
use std::time::Instant;
fn t<T>(name: &str, f: impl FnOnce() -> T) -> T { let s = Instant::now(); let r = f(); let e = s.elapsed(); if e.as_millis() > 200 { println!("{name}: {e:?}"); } r }
fn main() {
    let bytes = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let mut doc = match Document::open(&bytes) { Ok(d) => d, Err(e) => { println!("open: {e}"); return } };
    t("fields", || doc.fields());
    for page in 0..doc.page_count().min(5) {
        let runs = t("text_runs", || doc.text_runs(page)).unwrap_or_default();
        println!("page {page}: {} runs", runs.len());
        if let Some(run) = runs.first() {
            t("paragraph_of", || doc.paragraph_of(page, run));
            t("replace_paragraph", || doc.replace_paragraph(page, run, "Texto nuevo para el parrafo, bastante mas largo que el que habia antes en esas lineas.", true));
            t("replace_text_with", || doc.replace_text_with(page, run, "Prueba", ReplaceOptions { allow_fallback: true, ..Default::default() }));
        }
    }
    t("undo", || doc.undo());
    t("find", || doc.find_text("a", SearchOptions::default()));
    t("replace_all", || doc.replace_all("a", "bb", SearchOptions::default(), ReplaceOptions::default()));
    for page in 0..doc.page_count().min(3) {
        t("text_lines", || doc.text_lines(page));
        t("blocks", || doc.blocks(page));
        t("page_items", || doc.page_items(page));
        t("annotations", || doc.annotations(page));
    }
    t("ocr_report", || doc.ocr_report());
    t("required_fonts", || doc.required_fonts());
    println!("done");
}

//! `cargo run --example replace -- in.pdf page "texto viejo" "texto nuevo" out.pdf`
use pdfcore::Document;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut doc = Document::open(&std::fs::read(&a[1]).unwrap()).unwrap();
    let page: usize = a[2].parse().unwrap();
    let run = doc.text_runs(page).unwrap().into_iter().find(|r| r.text.trim() == a[3]).expect("no hay un tramo con ese texto");
    match doc.replace_text_with_fallback(page, &run, &a[4]) {
        Ok(done) => println!("ok: {:.1}pt -> {:.1}pt, respaldo para {:?}", done.old_width, done.new_width, done.fallback_chars),
        Err(e) => { println!("rechazado: {e}"); std::process::exit(2) }
    }
    std::fs::write(&a[5], doc.save_to_bytes().unwrap()).unwrap();
}

//! `cargo run --example paragraph -- in.pdf out.pdf "Texto" "texto nuevo del párrafo"`:
//! lay a paragraph out again; `Texto` starts one of its lines.
use pdfcore::Document;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut doc = Document::open(&std::fs::read(&a[1]).unwrap()).unwrap();
    let run = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.starts_with(&a[3])).expect("line");
    match doc.replace_paragraph(0, &run, &a[4], true) {
        Ok(done) => println!("{done:?}"),
        Err(e) => println!("error: {e}"),
    }
    std::fs::write(&a[2], doc.save_to_bytes().unwrap()).unwrap();
}

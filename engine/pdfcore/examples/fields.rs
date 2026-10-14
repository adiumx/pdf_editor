//! `cargo run --example fields -- file.pdf`: list a form's fields.
use pdfcore::Document;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = Document::open(&std::fs::read(&a[1]).unwrap()).unwrap();
    for f in doc.fields() {
        let w = &f.widgets[0];
        println!("{:<28} {:?}  valor={:?} widgets={} pág={:?} flags[ro={} multi={} comb={} max={:?}] opciones={}",
            f.name, f.kind, f.value, f.widgets.len(), w.page, f.read_only, f.multiline, f.comb, f.max_len, f.options.len());
    }
}

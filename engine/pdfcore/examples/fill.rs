//! `cargo run --example fill -- in.pdf out.pdf campo=valor [campo=valor ...]`
//! (a trailing `!` after the value, as in `campo=valor!`, fills even if read-only)
use pdfcore::Document;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut doc = Document::open(&std::fs::read(&a[1]).unwrap()).unwrap();
    for pair in &a[3..] {
        let (name, value) = pair.split_once('=').expect("campo=valor");
        let (value, force) = value.strip_suffix('!').map(|v| (v, true)).unwrap_or((value, false));
        match doc.set_field_value_with(name, value, force) {
            Ok(()) => println!("ok       {name} = {value:?}"),
            Err(e) => println!("rechazado {name}: {e}"),
        }
    }
    std::fs::write(&a[2], doc.save_to_bytes().unwrap()).unwrap();
}

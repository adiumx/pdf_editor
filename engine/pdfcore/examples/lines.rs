//! `cargo run --example lines -- file.pdf [page]`: print a page's lines.
use pdfcore::Document;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = Document::open(&std::fs::read(&a[1]).unwrap()).unwrap();
    let page: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    for line in doc.text_lines(page).unwrap() {
        println!("{:>6.1} {:>6.1} {:>6.1} {:>6.1}  runs={:?}  {}", line.display[0], line.display[1], line.display[2], line.display[3], line.runs, line.text);
    }
}

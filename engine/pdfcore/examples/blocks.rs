//! `cargo run --example blocks -- file.pdf [page]`: print a page's blocks with their style ranges.
use pdfcore::Document;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = Document::open(&std::fs::read(&a[1]).unwrap()).unwrap();
    let page: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    for (n, b) in doc.blocks(page).unwrap().iter().enumerate() {
        println!("#{n} líneas={} justificado={} sangría={:.1} editable={:?}", b.lines.len(), b.justified, b.first_line_indent, b.not_editable);
        println!("   {:?}", b.text);
        let chars: Vec<char> = b.text.chars().collect();
        for s in &b.spans {
            println!("   [{:>3},{:>3}) {:<24} {:>4.1}pt bold={} italic={} color={:?}  {:?}", s.start, s.end, s.base_font, s.size, s.bold, s.italic, s.color, chars[s.start..s.end].iter().collect::<String>());
        }
    }
}

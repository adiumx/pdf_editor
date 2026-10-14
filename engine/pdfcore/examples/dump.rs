//! `cargo run --example dump -- file.pdf [page]`: print every text run as TSV.
use pdfcore::Document;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("read file");
    let doc = Document::open(&bytes).expect("open");
    let only: Option<usize> = args.get(2).and_then(|s| s.parse().ok());
    for page in 0..doc.page_count() {
        if only.is_some_and(|p| p != page) {
            continue;
        }
        for run in doc.text_runs(page).unwrap() {
            println!(
                "{page}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{}\t{}\t{}",
                run.display[0], run.display[1], run.display[2], run.display[3],
                run.size, run.base_font, run.text.replace('\t', " "), run.xobject.as_deref().unwrap_or("-")
            );
        }
    }
}

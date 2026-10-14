//! `cargo run --release --example bench -- file.pdf`: time open, read, edit, save.
use std::time::Instant;
use pdfcore::Document;

/// Peak resident memory of this process (Linux), in MB.
fn peak_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()))
        .map_or(0, |kb| kb / 1024)
}

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let bytes = std::fs::read(&path).unwrap();
    let t = Instant::now();
    let mut doc = Document::open(&bytes).unwrap();
    println!("peak after open {} MB", peak_mb());
    println!("open            {:>8.0?}  ({} pages, {} MB)", t.elapsed(), doc.page_count(), bytes.len() / 1_000_000);
    let t = Instant::now();
    let runs = doc.text_runs(0).unwrap();
    println!("read page 1     {:>8.0?}  ({} runs)", t.elapsed(), runs.len());
    let t = Instant::now();
    let mut total = 0;
    for p in 0..doc.page_count() {
        total += doc.text_runs(p).unwrap().len();
    }
    println!("read all pages  {:>8.0?}  ({total} runs)", t.elapsed());
    let t = Instant::now();
    doc.replace_text(0, &runs[runs.len() - 1], "Texto cambiado").unwrap();
    println!("one edit        {:>8.0?}", t.elapsed());
    let last = doc.page_count() - 1;
    let runs = doc.text_runs(last).unwrap();
    let t = Instant::now();
    doc.replace_text(last, &runs[runs.len() - 1], "Otro cambio").unwrap();
    println!("edit last page  {:>8.0?}", t.elapsed());
    let t = Instant::now();
    doc.undo();
    println!("undo            {:>8.0?}", t.elapsed());
    let t = Instant::now();
    let out = doc.save_to_bytes().unwrap();
    println!("save            {:>8.0?}  ({} MB)", t.elapsed(), out.len() / 1_000_000);
    println!("peak memory    {:>5} MB", peak_mb());
}

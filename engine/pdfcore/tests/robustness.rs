//! Damaged files must come back as errors (or be read as far as possible),
//! never as a panic or a hang.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use pdfcore::{Document, Reflow, ReplaceOptions};

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "pdf") {
            out.push((path.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&path).unwrap()));
        }
    }
    out.sort();
    out
}

/// Touch every read and write path of a document that opened.
fn exercise(doc: &mut Document) {
    let _ = doc.fields();
    let _ = doc.signatures();
    for page in 0..doc.page_count().min(5) {
        let _ = doc.page_size(page);
        if let Ok(runs) = doc.text_runs(page) {
            if let Some(run) = runs.first() {
                let _ = doc.replace_text_with(page, run, "Prueba", ReplaceOptions { allow_fallback: true, reflow: Reflow::Following });
            }
        }
        let _ = doc.text_runs(page);
    }
    let _ = doc.undo();
    let _ = doc.save_to_bytes();
}

fn survive(name: &str, bytes: &[u8]) {
    eprintln!("-> {name}");
    if let Ok(path) = std::env::var("DUMP_CASE") { let _ = std::fs::write(path, bytes); }
    let started = Instant::now();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut doc) = Document::open(bytes) {
            exercise(&mut doc);
        }
    }));
    assert!(outcome.is_ok(), "{name}: panicked");
    assert!(started.elapsed() < Duration::from_secs(10), "{name}: took {:?}", started.elapsed());
}

#[test]
fn truncated_files_do_not_panic() {
    for (name, bytes) in fixtures() {
        for cut in [0, 1, 8, 20, bytes.len() / 4, bytes.len() / 2, bytes.len() * 3 / 4, bytes.len() - 20, bytes.len() - 1] {
            survive(&format!("{name} cut at {cut}"), &bytes[..cut.min(bytes.len())]);
        }
    }
}

#[test]
fn flipped_bytes_do_not_panic() {
    // A fixed pseudo-random sequence keeps failures reproducible.
    let mut state = 0x2545F4914F6CDD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for (name, bytes) in fixtures() {
        let rounds: usize = std::env::var("FUZZ_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
        for round in 0..rounds {
            let mut copy = bytes.clone();
            for _ in 0..1 + round % 8 {
                let at = (next() as usize) % copy.len();
                copy[at] = next() as u8;
            }
            survive(&format!("{name} round {round}"), &copy);
        }
    }
}

#[test]
fn garbage_is_an_error() {
    for bytes in [&b""[..], b"hola", b"%PDF-1.7\n", b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n<</Root 1 0 R>>\n%%EOF"] {
        survive("garbage", bytes);
    }
    assert!(Document::open(b"not a pdf at all").is_err());
}

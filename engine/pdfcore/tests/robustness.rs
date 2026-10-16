//! Damaged files must come back as errors (or be read as far as possible),
//! never as a panic or a hang.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use pdfcore::{Document, Reflow, ReplaceOptions, SearchOptions};

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
                let _ = doc.paragraph_of(page, run);
                let _ = doc.replace_paragraph(page, run, "Texto nuevo para el parrafo, bastante mas largo que el que habia antes en esas lineas.", true);
                let _ = doc.replace_text_with(page, run, "Prueba", ReplaceOptions { allow_fallback: true, reflow: Reflow::Following, substitution: pdfcore::Substitution::PerChar });
            }
        }
        let _ = doc.text_runs(page);
    }
    for page in 0..doc.page_count().min(3) {
        let _ = doc.page_graphics(page);
        let _ = doc.delete_runs(page, &[0, 2]);
    }
    let _ = doc.undo();
    let _ = doc.find_text("a", SearchOptions::default());
    let _ = doc.replace_all("a", "bb", SearchOptions::default(), ReplaceOptions::default());
    for page in 0..doc.page_count().min(3) {
        let _ = doc.text_lines(page);
    }
    if let Ok(items) = doc.page_items(0) {
        let moves: Vec<_> = items.iter().take(3).cloned().map(|i| (i, (7.0, 3.0))).collect();
        let _ = doc.move_items(0, &moves);
        let again = doc.page_items(0).unwrap_or_default();
        let _ = doc.arrange_items(0, &again.iter().take(3).cloned().collect::<Vec<_>>(), pdfcore::Arrange::AlignLeft);
        let _ = doc.space_lines(0, &again.iter().take(3).cloned().collect::<Vec<_>>(), None);
        if let Some(run) = doc.text_runs(0).ok().and_then(|r| r.into_iter().next()) {
            let _ = doc.align_paragraph(0, &run, pdfcore::Align::Center);
        }
    }
    let _ = doc.rotate_page(0, 90);
    let _ = doc.move_page(0, doc.page_count().saturating_sub(1));
    let _ = doc.extract_pages(&[0]);
    if let Ok(own) = doc.save_to_bytes() {
        let _ = doc.insert_pages(&own, "", None, 0);
    }
    let _ = doc.delete_page(0);
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
    if started.elapsed() > Duration::from_secs(5) {
        if let Ok(dir) = std::env::var("SLOW_DIR") {
            let _ = std::fs::write(format!("{dir}/slow_{}.pdf", name.replace(' ', "_")), bytes);
        }
    }
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

#[test]
fn damaged_pictures_are_refused_not_fatal() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut state = 0x9E3779B97F4A7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for name in ["img_rgb.jpg", "img_exif6.jpg", "img_alpha.png"] {
        let bytes = std::fs::read(format!("{dir}/{name}")).unwrap();
        for round in 0..300 {
            let mut copy = bytes.clone();
            if round % 3 == 0 {
                copy.truncate((next() as usize) % bytes.len());
            }
            for _ in 0..1 + round % 5 {
                let at = (next() as usize) % copy.len().max(1);
                if let Some(b) = copy.get_mut(at) {
                    *b = next() as u8;
                }
            }
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let mut doc = Document::open(&std::fs::read(format!("{dir}/standard.pdf")).unwrap()).unwrap();
                let _ = doc.add_image(0, &pdfcore::NewImage::new(&copy, 10.0, 10.0, 50.0));
                let _ = doc.save_to_bytes();
            }));
            assert!(outcome.is_ok(), "{name} round {round}: panicked");
        }
    }
}

//! The OCR report, and saving changes as an update appended to the original file.

use pdfcore::{Document, PageKind};

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
}

#[test]
fn the_report_tells_scans_from_real_text() {
    let kind = |name: &str| Document::open(&bytes(name)).unwrap().ocr_report().unwrap().remove(0);
    let scan = kind("scan_no_text");
    assert_eq!((scan.kind, scan.needs_ocr, scan.images), (PageKind::ScannedNoText, true, 1));
    assert!(scan.image_coverage > 0.95);
    let ocr = kind("scan_with_text");
    assert_eq!((ocr.kind, ocr.needs_ocr), (PageKind::ScannedWithTextLayer, false));
    assert!(ocr.invisible_chars > 20 && ocr.visible_chars == 0);
    let native = kind("paragraph");
    assert_eq!((native.kind, native.needs_ocr, native.images), (PageKind::Native, false, 0));
    assert_eq!(kind("blank").kind, PageKind::Blank);
    // Several pages: one entry each.
    assert_eq!(Document::open(&bytes("multi")).unwrap().ocr_report().unwrap().len(), Document::open(&bytes("multi")).unwrap().page_count());
}

#[test]
fn an_incremental_save_keeps_every_byte_of_the_original_and_adds_the_change() {
    let original = bytes("standard");
    let mut doc = Document::open(&original).unwrap();
    // Nothing changed: the same bytes.
    assert_eq!(doc.save_incremental().unwrap(), original);
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text(0, &run, "Cambiado de forma incremental").unwrap();
    let saved = doc.save_incremental().unwrap();
    assert!(saved.len() > original.len());
    assert_eq!(&saved[..original.len()], &original[..], "the original must stay untouched at the start");
    let reopened = Document::open(&saved).unwrap();
    assert_eq!(reopened.text_runs(0).unwrap()[0].text, "Cambiado de forma incremental");
    // And again on top of that: an update of an updated file.
    let mut again = Document::open(&saved).unwrap();
    let run = again.text_runs(0).unwrap().remove(1);
    again.replace_text(0, &run, "Segunda actualizacion").unwrap();
    let twice = again.save_incremental().unwrap();
    assert_eq!(&twice[..saved.len()], &saved[..]);
    assert_eq!(Document::open(&twice).unwrap().text_runs(0).unwrap()[1].text, "Segunda actualizacion");
}

#[test]
fn an_incremental_save_of_a_protected_file_stays_protected() {
    let original = bytes("encrypted_user");
    let mut doc = Document::open_with_password(&original, "secreto").unwrap();
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text(0, &run, "Cifrado y cambiado").unwrap();
    let saved = doc.save_incremental().unwrap();
    assert_eq!(&saved[..original.len()], &original[..]);
    assert!(matches!(Document::open(&saved), Err(pdfcore::Error::PasswordRequired)));
    let reopened = Document::open_with_password(&saved, "secreto").unwrap();
    assert_eq!(reopened.text_runs(0).unwrap()[0].text, "Cifrado y cambiado");
}

#[test]
fn an_original_with_a_damaged_xref_is_not_given_a_broken_update() {
    // The same file with its line endings converted (what a text-mode checkout does): it still opens,
    // by scanning, but an update appended to it would not.
    let damaged: Vec<u8> = bytes("standard").iter().flat_map(|b| if *b == b'\n' { vec![b'\r', b'\n'] } else { vec![*b] }).collect();
    let mut doc = Document::open(&damaged).unwrap();
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text(0, &run, "Cambiado").unwrap();
    assert!(matches!(doc.save_incremental(), Err(pdfcore::Error::Unsupported(_))));
    // The ordinary save still works.
    assert_eq!(Document::open(&doc.save_to_bytes().unwrap()).unwrap().text_runs(0).unwrap()[0].text, "Cambiado");
}

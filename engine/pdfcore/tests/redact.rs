//! Redaction: the text under a box leaves the file; the rest does not move.

use pdfcore::{Document, Error, RedactOptions};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn all_text(doc: &Document) -> String {
    doc.text_lines(0).unwrap().into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_word_is_removed_from_the_file_and_its_neighbours_stay_where_they_were() {
    for fixture in ["paragraph", "kerned"] {
        let mut doc = open(fixture);
        let before_bytes = doc.save_to_bytes().unwrap();
        let before_runs = doc.text_runs(0).unwrap();
        let lines = doc.text_lines(0).unwrap();
        let line = lines.iter().find(|l| l.words.len() >= 4).expect("a line with words");
        let target = line.words[2].clone();
        let mut inset = target.display;
        inset[0] += 0.3;
        inset[2] -= 0.3;
        let done = doc.redact(0, &[inset], &RedactOptions::default()).unwrap();
        assert_eq!(done.chars, target.text.chars().count(), "{fixture}: {done:?}");

        // The text is gone from the file, not only covered.
        let saved = doc.save_to_bytes().unwrap();
        let again = Document::open(&saved).unwrap();
        let text = all_text(&again);
        let still = again.text_lines(0).unwrap().iter().flat_map(|l| l.words.clone()).any(|w| w.text == target.text && (w.display[1] - target.display[1]).abs() < 1.0 && (w.display[0] - target.display[0]).abs() < 3.0);
        assert!(!still, "{fixture}: {} still there in {text}", target.text);
        // Every string keeps its place and its total advance: what follows the gap did not move.
        let after_runs = again.text_runs(0).unwrap();
        for (old, new) in before_runs.iter().zip(&after_runs) {
            assert!((old.origin.0 - new.origin.0).abs() < 0.01 && (old.end.0 - new.end.0).abs() < 0.05, "{fixture}: {:?} -> {:?}", old.end, new.end);
        }
        assert!(doc.undo());
        assert_eq!(doc.save_to_bytes().unwrap(), before_bytes, "{fixture}");
    }
}

#[test]
fn an_empty_list_of_boxes_and_text_in_a_form_xobject_are_refused() {
    let mut doc = open("line_xobject");
    assert!(matches!(doc.redact(0, &[], &RedactOptions::default()), Err(Error::Rejected(_))));
    let inside = doc.text_runs(0).unwrap().into_iter().find(|r| r.xobject.is_some()).expect("text in an XObject");
    let before = doc.save_to_bytes().unwrap();
    assert!(matches!(doc.redact(0, &[inside.display], &RedactOptions::default()), Err(Error::Unsupported(_))));
    assert!(!doc.can_undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn pictures_fields_and_notes_under_a_box_are_dealt_with() {
    let mut doc = open("canvas");
    let items = doc.page_items(0).unwrap();
    let picture = items.iter().find(|i| i.kind == pdfcore::ItemKind::Image).unwrap();
    // Kept (covered), and the report says its data is still there.
    let done = doc.redact(0, &[picture.display], &RedactOptions::default()).unwrap();
    assert_eq!((done.images_kept, done.images_removed), (1, 0));
    assert!(doc.undo());
    // Removed on request.
    let done = doc.redact(0, &[picture.display], &RedactOptions { remove_images: true, ..Default::default() }).unwrap();
    assert_eq!(done.images_removed, 1);
    assert_eq!(doc.page_items(0).unwrap().iter().filter(|i| i.kind == pdfcore::ItemKind::Image).count(), 1);
    // The field under another box goes.
    let field = items.iter().find(|i| i.kind == pdfcore::ItemKind::Field).unwrap();
    let done = doc.redact(0, &[field.display], &RedactOptions::default()).unwrap();
    assert!(done.annotations_removed >= 1);
    assert!(doc.fields().is_empty());
}

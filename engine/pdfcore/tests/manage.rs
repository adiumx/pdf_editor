//! Batches, deleting, hit testing, resizing and rebuilding a form's look.

use pdfcore::{Document, Error, Item, ItemKind};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn of_kind(doc: &Document, kind: ItemKind) -> Vec<Item> {
    doc.page_items(0).unwrap().into_iter().filter(|i| i.kind == kind).collect()
}

#[test]
fn a_batch_is_one_undo_step_and_a_failure_undoes_all_of_it() {
    let mut doc = open("standard");
    let before = doc.save_to_bytes().unwrap();
    let runs = doc.text_runs(0).unwrap();
    doc.batch(|d| {
        d.replace_text(0, &runs[0], "Uno")?;
        // Replacing in place keeps the other runs where they were read.
        d.replace_text(0, &runs[1], "Dos")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(doc.text_runs(0).unwrap().iter().map(|r| r.text.as_str()).collect::<Vec<_>>()[..2], ["Uno", "Dos"]);
    assert!(doc.undo());
    assert!(!doc.can_undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);

    let runs = doc.text_runs(0).unwrap();
    let failed = doc.batch(|d| {
        d.replace_text(0, &runs[0], "Tres")?;
        d.replace_text(0, &runs[1], "x\u{4E16}") // no font has it
    });
    assert!(matches!(failed, Err(Error::MissingGlyphs(_))));
    assert!(!doc.can_undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn deleting_text_a_picture_and_a_field_each_undo_in_one_step() {
    let mut doc = open("canvas");
    let before = doc.save_to_bytes().unwrap();
    let items = doc.page_items(0).unwrap();
    doc.delete_items(0, &items).unwrap();
    assert!(doc.text_runs(0).unwrap().iter().all(|r| r.text.is_empty()));
    assert!(doc.page_items(0).unwrap().is_empty(), "{:?}", doc.page_items(0).unwrap().iter().map(|i| &i.kind).collect::<Vec<_>>());
    assert!(doc.fields().is_empty());
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
    assert_eq!(doc.page_items(0).unwrap().len(), 7);
}

#[test]
fn deleting_one_item_leaves_the_rest() {
    let mut doc = open("canvas");
    let pictures = of_kind(&doc, ItemKind::Image);
    doc.delete_items(0, &pictures[..1]).unwrap();
    assert_eq!(of_kind(&doc, ItemKind::Image).len(), 1);
    assert_eq!(of_kind(&doc, ItemKind::Text).len(), 4);
    let field = of_kind(&doc, ItemKind::Field);
    doc.delete_items(0, &field).unwrap();
    assert_eq!(of_kind(&doc, ItemKind::Field).len(), 0);
    assert!(doc.undo() && doc.undo());
    assert_eq!(doc.page_items(0).unwrap().len(), 7);
}

#[test]
fn a_click_finds_the_letter_the_field_or_nothing() {
    let doc = open("canvas");
    let items = doc.page_items(0).unwrap();
    let text = items.iter().find(|i| i.kind == ItemKind::Text && i.text.starts_with("Primera")).unwrap();
    let (x, y) = ((text.display[0] + text.display[2]) / 2.0, (text.display[1] + text.display[3]) / 2.0);
    let hit = doc.hit_test(0, x, y).unwrap().unwrap();
    assert_eq!(hit.kind, ItemKind::Text);
    let block = &doc.blocks(0).unwrap()[hit.block.unwrap()];
    assert!(block.text.starts_with("Primera"));
    let at = hit.char_index.unwrap();
    let b = block.char_boxes[at];
    assert!(x >= b[0] - 6.0 && x <= b[2] + 6.0, "{x} not near {b:?}");
    assert!(hit.caret.unwrap() == at || hit.caret.unwrap() == at + 1);
    // The start of the text puts the caret before the first letter.
    let start = doc.hit_test(0, text.display[0] + 0.1, text.display[1] + 1.0).unwrap().unwrap();
    assert_eq!(start.caret, Some(0));
    // A field is hit as a field, and nothing is hit where nothing is.
    let field = items.iter().find(|i| i.kind == ItemKind::Field).unwrap();
    let hit = doc.hit_test(0, (field.display[0] + field.display[2]) / 2.0, (field.display[1] + field.display[3]) / 2.0).unwrap().unwrap();
    assert_eq!((hit.kind, hit.field.is_some()), (ItemKind::Field, true));
    assert!(doc.hit_test(0, 590.0, 835.0).unwrap().is_none());
}

#[test]
fn a_picture_is_resized_to_its_new_box_and_undone() {
    let mut doc = open("canvas");
    let before = doc.save_to_bytes().unwrap();
    let picture = of_kind(&doc, ItemKind::Image).remove(0);
    let new = [picture.display[0] + 10.0, picture.display[1] + 5.0, picture.display[0] + 70.0, picture.display[1] + 45.0];
    doc.resize_item(0, &picture, new).unwrap();
    let now = doc.page_items(0).unwrap().into_iter().filter(|i| i.kind == ItemKind::Image).find(|i| (i.display[0] - new[0]).abs() < 1.0).expect("moved picture");
    for (a, b) in now.display.iter().zip(new) {
        assert!((a - b).abs() < 0.5, "{:?} vs {:?}", now.display, new);
    }
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
}

#[test]
fn a_field_and_a_text_are_resized() {
    let mut doc = open("canvas");
    let field = of_kind(&doc, ItemKind::Field).remove(0);
    let new = [field.display[0], field.display[1], field.display[0] + 200.0, field.display[1] + 30.0];
    doc.resize_item(0, &field, new).unwrap();
    let after = of_kind(&doc, ItemKind::Field).remove(0);
    assert!((after.display[2] - after.display[0] - 200.0).abs() < 0.5);

    let text = of_kind(&doc, ItemKind::Text).into_iter().find(|i| i.text.starts_with("Primera")).unwrap();
    let width = text.display[2] - text.display[0];
    let new = [text.display[0] + 5.0, text.display[1] + 8.0, text.display[0] + 5.0 + width * 0.6, text.display[3]];
    doc.resize_item(0, &text, new).unwrap();
    let after = of_kind(&doc, ItemKind::Text).into_iter().find(|i| i.text.starts_with("Primera")).unwrap();
    assert!(after.display[2] - after.display[0] <= width * 0.6 + 0.5, "{:?}", after.display);
    assert!(after.runs.len() > text.runs.len(), "narrower text needs more lines");
    assert!((after.display[0] - new[0]).abs() < 0.6 && (after.display[1] - new[1]).abs() < 0.6, "{:?} vs {:?}", after.display, new);
    assert_eq!(after.text, text.text);
}

#[test]
fn appearances_are_rebuilt_for_fields_that_have_none() {
    let mut doc = open("form_no_appearance");
    let before = doc.fields();
    assert!(before.iter().any(|f| f.widgets.iter().any(|w| !w.has_appearance)));
    let done = doc.refresh_appearances(true).unwrap();
    assert!(done.rebuilt >= 1);
    assert!(doc.fields().iter().filter(|f| !matches!(f.kind, pdfcore::FieldKind::PushButton | pdfcore::FieldKind::Signature)).all(|f| f.widgets.iter().all(|w| w.has_appearance)));
    // Values are what they were.
    for (a, b) in before.iter().zip(doc.fields()) {
        assert_eq!((&a.name, &a.value), (&b.name, &b.value));
    }
    // Nothing left to do the second time, and no step recorded for it beyond the first.
    let again = doc.refresh_appearances(true).unwrap();
    assert_eq!(again.rebuilt, 0);
}

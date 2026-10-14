//! Rotating, deleting, moving, extracting and bringing in pages.

use pdfcore::{Document, Error};

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
}

fn open(name: &str) -> Document {
    Document::open(&bytes(name)).expect("open")
}

/// First line of every page.
fn titles(doc: &Document) -> Vec<String> {
    (0..doc.page_count())
        .map(|p| doc.text_lines(p).unwrap().first().map(|l| l.text.clone()).unwrap_or_default())
        .collect()
}

fn reopen(doc: &mut Document) -> Document {
    Document::open(&doc.save_to_bytes().unwrap()).unwrap()
}

#[test]
fn a_page_turns_and_turns_back() {
    let mut doc = open("multi");
    let before = doc.page_size(0).unwrap();
    assert_eq!(before.rotation, 0);
    doc.rotate_page(0, 90).unwrap();
    let after = doc.page_size(0).unwrap();
    assert_eq!((after.rotation, after.width, after.height), (90, before.height, before.width));
    // The text is still found, now in the turned page's coordinates.
    assert!(doc.text_runs(0).unwrap().iter().all(|r| r.display[2] <= after.width + 1.0 && r.display[3] <= after.height + 1.0));
    assert_eq!(reopen(&mut doc).page_size(0).unwrap().rotation, 90);
    doc.rotate_page(0, -180).unwrap();
    assert_eq!(doc.page_size(0).unwrap().rotation, 270);
    assert!(doc.undo() && doc.undo());
    assert_eq!(doc.page_size(0).unwrap().rotation, 0);
    assert!(matches!(doc.rotate_page(0, 45), Err(Error::Rejected(_))));
}

#[test]
fn deleting_a_page_is_one_undo() {
    let mut doc = open("multi");
    doc.delete_page(1).unwrap();
    assert_eq!(titles(&doc), ["Contrato 1 de la aseguradora Prudential", "Contrato 3 de la aseguradora Prudential"]);
    assert_eq!(titles(&reopen(&mut doc)).len(), 2);
    assert!(doc.undo());
    assert_eq!(doc.page_count(), 3);
    assert!(titles(&doc)[1].starts_with("Contrato 2"));
    assert!(doc.redo());
    assert_eq!(doc.page_count(), 2);
    // Edits still land on the right page after a deletion.
    let run = doc.text_runs(1).unwrap().into_iter().next().unwrap();
    doc.replace_text(1, &run, "Contrato tres").unwrap();
    assert!(titles(&doc)[1].starts_with("Contrato tres"));
}

#[test]
fn the_last_page_cannot_be_deleted() {
    let mut doc = open("standard");
    assert!(matches!(doc.delete_page(0), Err(Error::Rejected(_))));
    assert_eq!(doc.page_count(), 1);
    assert!(matches!(doc.delete_page(5), Err(Error::NoSuchPage(5))));
}

#[test]
fn a_page_moves_to_where_it_is_wanted() {
    let mut doc = open("multi");
    doc.move_page(0, 2).unwrap();
    let order: Vec<String> = titles(&doc).iter().map(|t| t[..10].to_string()).collect();
    assert_eq!(order, ["Contrato 2", "Contrato 3", "Contrato 1"]);
    assert!(doc.undo());
    assert!(titles(&doc)[0].starts_with("Contrato 1"));
}

#[test]
fn pages_can_be_extracted_in_any_order() {
    let doc = open("multi");
    let out = doc.extract_pages(&[2, 0]).unwrap();
    let part = Document::open(&out).unwrap();
    assert_eq!(part.page_count(), 2);
    let order: Vec<String> = titles(&part).iter().map(|t| t[..10].to_string()).collect();
    assert_eq!(order, ["Contrato 3", "Contrato 1"]);
    assert!(out.len() < bytes("multi").len(), "the left-out pages are not carried");
    assert!(doc.extract_pages(&[]).is_err());
    assert!(matches!(doc.extract_pages(&[7]), Err(Error::NoSuchPage(7))));
}

#[test]
fn pages_keep_what_they_inherit_from_the_page_tree() {
    let mut doc = open("inherited");
    let (w, h) = {
        let s = doc.page_size(0).unwrap();
        (s.width, s.height)
    };
    assert_eq!(doc.page_size(0).unwrap().rotation, 90);
    doc.move_page(0, 1).unwrap();
    let moved = doc.page_size(1).unwrap();
    assert_eq!((moved.width, moved.height, moved.rotation), (w, h, 90));
    assert!(titles(&doc)[0].starts_with("Hereda 2"));
    // An extracted page is whole on its own: size, rotation and its font.
    let alone = Document::open(&doc.extract_pages(&[1]).unwrap()).unwrap();
    assert_eq!((alone.page_size(0).unwrap().width, alone.page_size(0).unwrap().rotation), (w, 90));
    assert!(titles(&alone)[0].starts_with("Hereda 1"));
    assert!(alone.text_runs(0).unwrap()[0].base_font.contains("Helvetica"));
}

#[test]
fn pages_come_in_from_another_document_with_their_fields() {
    let mut doc = open("multi");
    let added = doc.insert_pages(&bytes("form"), "", None, 1).unwrap();
    assert_eq!(added, 1);
    assert_eq!(doc.page_count(), 4);
    assert!(titles(&doc)[0].starts_with("Contrato 1") && titles(&doc)[2].starts_with("Contrato 2"));
    // The form's fields are the document's now, on the page that came in.
    let nombre = doc.field("nombre").expect("the incoming field");
    assert_eq!(nombre.widgets[0].page, Some(1));
    doc.set_field_value("nombre", "María").unwrap();
    let mut again = reopen(&mut doc);
    assert_eq!(again.field("nombre").unwrap().value, "María");
    assert_eq!(again.page_count(), 4);
    // And all of it is a single step.
    assert!(again.undo() || doc.undo());
    assert!(doc.undo());
    assert_eq!(doc.page_count(), 3);
    assert!(doc.field("nombre").is_none());
}

#[test]
fn a_chosen_few_pages_can_come_in_at_the_end() {
    let mut doc = open("standard");
    let added = doc.insert_pages(&bytes("multi"), "", Some(&[2, 1]), 1).unwrap();
    assert_eq!(added, 2);
    let order: Vec<String> = titles(&doc).iter().map(|t| t.chars().take(10).collect()).collect();
    assert_eq!(order, ["Hola mundo", "Contrato 3", "Contrato 2"]);
    assert!(matches!(doc.insert_pages(&bytes("multi"), "", None, 9), Err(Error::NoSuchPage(9))));
    // Pages from a protected document need its password.
    assert!(matches!(doc.insert_pages(&bytes("encrypted_user"), "", None, 0), Err(Error::PasswordRequired)));
    assert_eq!(doc.insert_pages(&bytes("encrypted_user"), "secreto", None, 0).unwrap(), 1);
}

#[test]
fn deleting_a_page_takes_its_fields_with_it() {
    let mut doc = open("multi");
    doc.insert_pages(&bytes("form"), "", None, 3).unwrap();
    assert!(doc.field("nombre").is_some());
    doc.delete_page(3).unwrap();
    assert!(doc.field("nombre").is_none());
    assert!(doc.fields().is_empty());
    assert!(doc.undo());
    assert!(doc.field("nombre").is_some());
}

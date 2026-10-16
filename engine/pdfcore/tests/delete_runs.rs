//! Deleting strings for real: the letters leave the file, nothing around them moves, one undo step.

use pdfcore::{Document, Error, GraphicKind, NewText};

fn open_bytes(bytes: &[u8]) -> Document {
    Document::open(bytes).expect("open")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
}

/// Text and where it starts, for every string of the page.
fn layout(doc: &Document, page: usize) -> Vec<(String, (f32, f32))> {
    doc.text_runs(page).unwrap().into_iter().map(|r| (r.text, r.origin)).collect()
}

fn same_layout(a: &[(String, (f32, f32))], b: &[(String, (f32, f32))]) {
    assert_eq!(a.len(), b.len(), "{a:?}\n{b:?}");
    for (x, y) in a.iter().zip(b) {
        assert_eq!(x.0, y.0);
        assert!((x.1 .0 - y.1 .0).abs() < 0.01 && (x.1 .1 - y.1 .1).abs() < 0.01, "{} moved: {:?} -> {:?}", x.0, x.1, y.1);
    }
}

fn index_of(doc: &Document, page: usize, text: &str) -> usize {
    doc.text_runs(page).unwrap().iter().position(|r| r.text == text).unwrap_or_else(|| panic!("no {text:?}"))
}

/// The check box left of a label, and an "X" written in it, deleted again.
fn x_next_to(bytes: &[u8], page: usize, label: &str) {
    let mut doc = open_bytes(bytes);
    let original = doc.save_to_bytes().unwrap();
    let before = layout(&doc, page);
    let graphics_before = doc.page_graphics(page).unwrap();
    let label_run = doc.text_runs(page).unwrap().into_iter().find(|r| r.text.trim_start().starts_with(label)).expect("label");
    let check_box = graphics_before
        .iter()
        .filter(|g| g.kind == GraphicKind::Rect && g.display[2] <= label_run.display[0] + 0.5 && g.display[3] - g.display[1] < 14.0)
        .filter(|g| g.display[1] < label_run.display[3] && g.display[3] > label_run.display[1])
        .max_by(|a, b| a.display[2].total_cmp(&b.display[2]))
        .expect("the box left of the label")
        .display;
    let mut x = NewText::new("X", check_box[0] + 1.5, check_box[1] + 0.2);
    x.size = 9.0;
    x.bold = true;
    doc.add_text(page, &x).unwrap();
    let with_x = doc.save_to_bytes().unwrap();
    assert_eq!(layout(&doc, page).iter().filter(|r| r.0 == "X").count(), 1);

    let done = doc.delete_runs(page, &[index_of(&doc, page, "X")]).unwrap();
    assert_eq!((done.runs, done.chars), (1, 1));
    // The X is gone from the file, and the line (labels, boxes) is exactly where it was.
    let after = layout(&doc, page);
    same_layout(&before, &after);
    assert_eq!(doc.page_graphics(page).unwrap(), graphics_before);
    let reopened = open_bytes(&doc.save_to_bytes().unwrap());
    assert!(reopened.text_runs(page).unwrap().iter().all(|r| r.text != "X"));
    // One undo brings it back, a second takes the X away as it came.
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), with_x);
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), original);
}

#[test]
fn an_x_in_a_check_box_is_written_and_deleted_without_touching_the_line() {
    x_next_to(&fixture("checkbox_line"), 0, "Cónyuge");
}

#[test]
fn the_real_insurance_form() {
    // CAMBIO_DE_CONTRATANTE.pdf is the person's own document, not part of the repository: set
    // CAMBIO_PDF to its path to run this.
    let Some(path) = std::env::var_os("CAMBIO_PDF").filter(|p| std::path::Path::new(p).exists()) else { return };
    let bytes = std::fs::read(path).unwrap();
    for label in ["Cónyuge", "Concubina", "Otra"] {
        x_next_to(&bytes, 0, label);
    }
}

#[test]
fn eighteen_letters_of_a_comb_come_and_go_in_one_step_each() {
    let mut doc = open_bytes(&fixture("checkbox_line"));
    let original = doc.save_to_bytes().unwrap();
    let before = layout(&doc, 0);
    let letters = "RUFC800101HDFZLS09";
    assert_eq!(letters.chars().count(), 18);
    doc.batch(|d| {
        for (i, c) in letters.chars().enumerate() {
            let s = c.to_string();
            let mut t = NewText::new(&s, 60.0 + 14.0 * i as f32, 400.0);
            t.size = 10.0;
            d.add_text(0, &t)?;
        }
        Ok(())
    })
    .unwrap();
    let with_comb = doc.save_to_bytes().unwrap();
    let ids: Vec<usize> = doc
        .text_runs(0)
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, r)| (r.display[1] - 400.0).abs() < 2.0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(ids.len(), 18);
    let done = doc.delete_runs(0, &ids).unwrap();
    assert_eq!((done.runs, done.chars), (18, 18));
    same_layout(&before, &layout(&doc, 0));
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), with_comb);
    assert!(doc.undo());
    assert_eq!(doc.save_to_bytes().unwrap(), original);
    assert!(!doc.can_undo());
}

#[test]
fn on_a_turned_page_nothing_else_moves() {
    let mut doc = open_bytes(&fixture("page_rotated"));
    let before = layout(&doc, 0);
    let display_before: Vec<[f32; 4]> = doc.text_runs(0).unwrap().iter().map(|r| r.display).collect();
    doc.add_text(0, &NewText::new("X", 100.0, 100.0)).unwrap();
    let x = index_of(&doc, 0, "X");
    doc.delete_runs(0, &[x]).unwrap();
    same_layout(&before, &layout(&doc, 0));
    let display_after: Vec<[f32; 4]> = doc.text_runs(0).unwrap().iter().map(|r| r.display).collect();
    assert_eq!(display_before, display_after);
}

#[test]
fn text_that_follows_by_the_pen_stays_where_it_was() {
    // `line_pen`: each string continues where the previous one ended (no Td between them).
    let mut doc = open_bytes(&fixture("line_pen"));
    let before = layout(&doc, 0);
    doc.delete_runs(0, &[index_of(&doc, 0, "lunes a viernes")]).unwrap();
    let mut expected = before.clone();
    expected.retain(|r| r.0 != "lunes a viernes");
    same_layout(&expected, &layout(&doc, 0));
}

#[test]
fn a_kerned_tj_keeps_its_followers_in_place() {
    let mut doc = open_bytes(&fixture("kerned"));
    let before = layout(&doc, 0);
    doc.delete_runs(0, &[0]).unwrap();
    same_layout(&before[1..], &layout(&doc, 0));
}

#[test]
fn quote_operators_keep_their_line_move_and_spacing() {
    // `'` and `"` move to the next line (and `"` sets the spacing) as well as showing a string.
    let mut doc = open_bytes(&fixture("show_operators"));
    let before = layout(&doc, 0);
    let done = doc.delete_runs(0, &[index_of(&doc, 0, "Dos"), index_of(&doc, 0, "Tres"), index_of(&doc, 0, "Cinco")]).unwrap();
    assert_eq!((done.runs, done.chars), (3, 12));
    let mut expected = before.clone();
    expected.retain(|r| !matches!(r.0.as_str(), "Dos" | "Tres" | "Cinco"));
    same_layout(&expected, &layout(&doc, 0));
    assert!(doc.undo());
    same_layout(&before, &layout(&doc, 0));
}

#[test]
fn stale_ids_text_in_a_form_xobject_and_nothing_to_do() {
    let mut doc = open_bytes(&fixture("line_xobject"));
    let before = doc.save_to_bytes().unwrap();
    let count = doc.text_runs(0).unwrap().len();
    assert!(matches!(doc.delete_runs(0, &[count + 3]), Err(Error::StaleRun(_))));
    let inside = doc.text_runs(0).unwrap().iter().position(|r| r.xobject.is_some()).unwrap();
    assert!(matches!(doc.delete_runs(0, &[0, inside]), Err(Error::Unsupported(_))));
    assert_eq!(doc.delete_runs(0, &[]).unwrap().runs, 0);
    assert!(!doc.can_undo());
    assert_eq!(doc.save_to_bytes().unwrap(), before);
    assert!(matches!(doc.delete_runs(9, &[0]), Err(Error::NoSuchPage(9))));
}

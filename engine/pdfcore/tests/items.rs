//! Moving, aligning and spacing the things on a page.

use pdfcore::{Align, Arrange, Document, Item, ItemKind};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn item_with(items: &[Item], start: &str) -> Item {
    items.iter().find(|i| i.text.starts_with(start)).unwrap_or_else(|| panic!("no item starting {start:?}")).clone()
}

fn texts(doc: &Document) -> Vec<(String, [f32; 4])> {
    doc.page_items(0).unwrap().into_iter().filter(|i| i.kind == ItemKind::Text).map(|i| (i.text, i.display)).collect()
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.06
}

#[test]
fn a_page_lists_its_texts_pictures_and_fields() {
    let doc = open("canvas");
    let items = doc.page_items(0).unwrap();
    let kinds = |k: ItemKind| items.iter().filter(|i| i.kind == k).count();
    assert_eq!((kinds(ItemKind::Text), kinds(ItemKind::Image), kinds(ItemKind::Field)), (4, 2, 1));
    // The paragraph is one item of three lines.
    let block = item_with(&items, "Primera linea");
    assert_eq!(block.runs.len(), 3);
    assert_eq!(block.text, "Primera linea del bloque segunda linea mas larga que la primera tercera");
    assert!(items.iter().filter(|i| i.kind == ItemKind::Image).all(|i| i.picture.is_some()));
    let tops: Vec<f32> = items.iter().map(|i| i.display[1]).collect();
    assert!(tops.windows(2).all(|w| w[0] <= w[1]), "{tops:?}");
}

#[test]
fn moving_a_text_moves_only_that_text_and_undoes_in_one_step() {
    let mut doc = open("canvas");
    let before = texts(&doc);
    let items = doc.page_items(0).unwrap();
    let dos = item_with(&items, "Dos mas");
    doc.move_items(0, &[(dos, (25.0, -10.0))]).unwrap();
    let after = texts(&doc);
    for ((ta, ba), (tb, bb)) in before.iter().zip(&after) {
        assert_eq!(ta, tb);
        if ta.starts_with("Dos") {
            assert!(near(bb[0] - ba[0], 25.0) && near(bb[1] - ba[1], -10.0), "{ba:?} -> {bb:?}");
        } else {
            assert!(ba.iter().zip(bb).all(|(x, y)| near(*x, *y)), "{ta} moved: {ba:?} -> {bb:?}");
        }
    }
    let saved = Document::open(&doc.save_to_bytes().unwrap()).unwrap();
    assert_eq!(texts(&saved).iter().map(|t| t.0.clone()).collect::<Vec<_>>(), before.iter().map(|t| t.0.clone()).collect::<Vec<_>>());
    assert!(doc.undo());
    assert_eq!(texts(&doc).iter().map(|t| t.1).collect::<Vec<_>>(), before.iter().map(|t| t.1).collect::<Vec<_>>());
    assert!(!doc.can_undo());
}

#[test]
fn a_run_moves_on_its_own_however_the_line_is_laid_out() {
    // The same three-run line written four ways: only the first run may move.
    for name in ["line_absolute", "line_pen", "line_relative", "line_xobject"] {
        let mut doc = open(name);
        let runs = doc.text_runs(0).unwrap();
        let line: Vec<_> = runs.iter().filter(|r| (r.origin.1 - runs[0].origin.1).abs() < 1.0 && r.xobject.is_none()).collect();
        let first = line[0].clone();
        let others: Vec<_> = runs.iter().filter(|r| r.op_index != first.op_index).map(|r| (r.text.clone(), r.origin)).collect();
        let item = item_of_run(first.clone());
        doc.move_items(0, &[(item, (0.0, 30.0))]).unwrap();
        let now = doc.text_runs(0).unwrap();
        let moved = now.iter().find(|r| r.text == first.text).unwrap();
        let down = doc.to_display_point(0, moved.origin).unwrap().1 - doc.to_display_point(0, first.origin).unwrap().1;
        assert!(near(down, 30.0) && near(moved.origin.0, first.origin.0), "{name}: moved by {down}");
        for (text, origin) in others {
            let r = now.iter().find(|r| r.text == text).unwrap_or_else(|| panic!("{name}: lost {text:?}"));
            assert!(near(r.origin.0, origin.0) && near(r.origin.1, origin.1), "{name}: {text:?} moved from {origin:?} to {:?}", r.origin);
        }
    }
}

/// An item for one run alone (its line may hold others, or text a page item cannot be made of).
fn item_of_run(run: pdfcore::TextRun) -> Item {
    Item { kind: ItemKind::Text, display: run.display, text: run.text.clone(), runs: vec![run], picture: None, field: None }
}

#[test]
fn a_picture_moves_exactly() {
    let mut doc = open("canvas");
    let pictures = doc.page_pictures(0).unwrap();
    let items = doc.page_items(0).unwrap();
    let first = items.iter().find(|i| i.kind == ItemKind::Image).unwrap().clone();
    let before = first.display;
    doc.move_items(0, &[(first, (15.0, 40.0))]).unwrap();
    let after = doc.page_pictures(0).unwrap();
    assert_eq!(after.len(), pictures.len());
    let moved = after.iter().find(|p| near(p.display[0], before[0] + 15.0)).expect("the moved picture");
    assert!(near(moved.display[1], before[1] + 40.0) && near(moved.display[2] - moved.display[0], before[2] - before[0]));
    let other = pictures.iter().find(|p| !near(p.display[0], before[0])).unwrap();
    assert!(after.iter().any(|p| other.display.iter().zip(p.display).all(|(a, b)| near(*a, b))), "the other picture stayed");
    assert!(doc.undo());
    assert_eq!(doc.page_pictures(0).unwrap().iter().map(|p| p.display[0]).fold(0.0, f32::max), pictures.iter().map(|p| p.display[0]).fold(0.0, f32::max));
}

#[test]
fn texts_line_up_like_in_a_design_tool() {
    type Check = fn(&[f32; 4], &[f32; 4]) -> bool;
    let cases: Vec<(Arrange, Check)> = vec![
        (Arrange::AlignLeft, |a, b| near(a[0], b[0])),
        (Arrange::AlignRight, |a, b| near(a[2], b[2])),
        (Arrange::CenterHorizontally, |a, b| near(a[0] + a[2], b[0] + b[2])),
        (Arrange::AlignTop, |a, b| near(a[1], b[1])),
        (Arrange::AlignBottom, |a, b| near(a[3], b[3])),
        (Arrange::CenterVertically, |a, b| near(a[1] + a[3], b[1] + b[3])),
    ];
    for (how, check) in cases {
        let mut doc = open("canvas");
        let items = doc.page_items(0).unwrap();
        let chosen: Vec<Item> = ["Uno", "Dos mas", "Tres"].iter().map(|s| item_with(&items, s)).collect();
        doc.arrange_items(0, &chosen, how).unwrap();
        // Measured by runs: aligned on one baseline, the three become a single line item.
        let runs = doc.text_runs(0).unwrap();
        let boxes: Vec<[f32; 4]> = ["Uno", "Dos mas", "Tres"].iter().map(|s| runs.iter().find(|r| r.text.starts_with(s)).unwrap().display).collect();
        assert!(check(&boxes[0], &boxes[1]) && check(&boxes[1], &boxes[2]), "{how:?}: {boxes:?}");
        // The paragraph was not in the selection and did not move.
        assert!(near(runs.iter().find(|r| r.text.starts_with("Primera")).unwrap().display[0], 60.0));
    }
}

#[test]
fn different_kinds_of_item_line_up_together() {
    let mut doc = open("canvas");
    let items = doc.page_items(0).unwrap();
    let chosen = vec![
        item_with(&items, "Dos mas"),
        items.iter().find(|i| i.kind == ItemKind::Image).unwrap().clone(),
        items.iter().find(|i| i.kind == ItemKind::Field).unwrap().clone(),
    ];
    doc.arrange_items(0, &chosen, Arrange::AlignTop).unwrap();
    let now = doc.page_items(0).unwrap();
    let tops: Vec<f32> = [
        now.iter().find(|i| i.text.starts_with("Dos mas")).unwrap().display[1],
        now.iter().filter(|i| i.kind == ItemKind::Image).map(|i| i.display[1]).fold(f32::MAX, f32::min),
        now.iter().find(|i| i.kind == ItemKind::Field).unwrap().display[1],
    ]
    .to_vec();
    assert!(tops.iter().all(|t| near(*t, tops[0])), "{tops:?}");
    assert!(doc.undo());
    assert!(!doc.can_undo());
}

#[test]
fn items_are_spread_with_equal_gaps() {
    let mut doc = open("canvas");
    let items = doc.page_items(0).unwrap();
    let chosen: Vec<Item> = ["Uno", "Dos mas", "Tres"].iter().map(|s| item_with(&items, s)).collect();
    doc.arrange_items(0, &chosen, Arrange::DistributeVertically).unwrap();
    let after = texts(&doc);
    let b = |s: &str| after.iter().find(|t| t.0.starts_with(s)).unwrap().1;
    assert!(near(b("Dos mas")[1] - b("Uno")[3], b("Tres")[1] - b("Dos mas")[3]), "{:?}", after);
    assert!(matches!(doc.arrange_items(0, &chosen[..2], Arrange::DistributeVertically), Err(pdfcore::Error::Rejected(_))));
    assert!(matches!(doc.arrange_items(0, &chosen, Arrange::SameWidth), Err(pdfcore::Error::Rejected(_))));
}

#[test]
fn lines_get_an_even_leading() {
    let mut doc = open("canvas");
    let items = doc.page_items(0).unwrap();
    let chosen: Vec<Item> = ["Uno", "Dos mas", "Tres"].iter().map(|s| item_with(&items, s)).collect();
    let baseline = |doc: &Document, s: &str| {
        let run = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.starts_with(s)).unwrap();
        doc.to_display_point(0, run.origin).unwrap().1
    };
    let first = baseline(&doc, "Uno");
    doc.space_lines(0, &chosen, Some(24.0)).unwrap();
    assert!(near(baseline(&doc, "Uno"), first), "the first stays");
    assert!(near(baseline(&doc, "Dos mas") - first, 24.0) && near(baseline(&doc, "Tres") - first, 48.0));
    // Without a figure, the average of what there was: from y=110 to y=190, so 40 apart.
    let mut doc = open("canvas");
    doc.space_lines(0, &chosen, None).unwrap();
    assert!(near(baseline(&doc, "Dos mas") - baseline(&doc, "Uno"), 40.0) && near(baseline(&doc, "Tres") - baseline(&doc, "Dos mas"), 40.0));
    assert!(matches!(doc.space_lines(0, &chosen[..1], None), Err(pdfcore::Error::Rejected(_))));
    assert!(matches!(doc.space_lines(0, &chosen, Some(0.0)), Err(pdfcore::Error::Rejected(_))));
}

#[test]
fn a_paragraph_aligns_its_lines_left_centre_or_right() {
    type Check = fn(&[f32; 4], &[f32; 4]) -> bool;
    let cases: Vec<(Align, Check)> = vec![
        (Align::Left, |a, b| near(a[0], b[0])),
        (Align::Center, |a, b| near(a[0] + a[2], b[0] + b[2])),
        (Align::Right, |a, b| near(a[2], b[2])),
    ];
    for (align, check) in cases {
        let mut doc = open("canvas");
        let run = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.starts_with("Primera")).unwrap();
        let [_, _, right, _] = doc.paragraph_of(0, &run).unwrap().unwrap().display;
        doc.align_paragraph(0, &run, align).unwrap();
        let runs = doc.text_runs(0).unwrap();
        let lines: Vec<[f32; 4]> = ["Primera", "segunda", "tercera"].iter().map(|s| runs.iter().find(|r| r.text.starts_with(s)).unwrap().display).collect();
        assert!(check(&lines[0], &lines[1]) && check(&lines[1], &lines[2]), "{align:?}: {lines:?}");
        // The block keeps its extent: the longest line stays where it was.
        assert!(lines.iter().map(|l| l[2]).fold(f32::MIN, f32::max) <= right + 0.1);
        assert!(doc.undo() && !doc.can_undo());
    }
    let mut doc = open("canvas");
    let lone = doc.text_runs(0).unwrap().into_iter().find(|r| r.text == "Uno").unwrap();
    assert!(matches!(doc.align_paragraph(0, &lone, Align::Center), Err(pdfcore::Error::Unsupported(_))));
}

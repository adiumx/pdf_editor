//! What happens to the rest of a line when an edit changes the length of one of
//! its runs, for each way a producer may lay a line out.

use pdfcore::{Alignment, Document, Reflow, ReplaceOptions, Replaced, TextRun, Warning};

const LAYOUTS: [&str; 3] = ["line_absolute", "line_pen", "line_relative"];
const R0: &str = "El resto de las oficinas de";
const R1: &str = "lunes a viernes";
const R2: &str = "de 9 a 14 horas";
const NEXT: &str = "Segunda linea del documento";

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn run(doc: &Document, text: &str) -> TextRun {
    doc.text_runs(0).unwrap().into_iter().find(|r| r.text.trim() == text).unwrap_or_else(|| panic!("no hay «{text}»"))
}

fn origin(doc: &Document, text: &str) -> (f32, f32) {
    run(doc, text).origin
}

fn near(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() <= 0.05, "{what}: {a} no está cerca de {b}");
}

fn same(a: (f32, f32), b: (f32, f32), what: &str) {
    near(a.0, b.0, &format!("{what} (x)"));
    near(a.1, b.1, &format!("{what} (y)"));
}

fn edit(doc: &mut Document, from: &str, to: &str) -> Replaced {
    let r = run(doc, from);
    doc.replace_text(0, &r, to).unwrap()
}

fn delta(done: &Replaced) -> f32 {
    done.new_width - done.old_width
}

#[test]
fn a_longer_middle_run_pushes_what_follows_and_nothing_else() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let before: Vec<_> = [R0, R1, R2, NEXT].iter().map(|t| origin(&doc, t)).collect();
        let done = edit(&mut doc, R1, "de lunes a viernes inclusive");
        assert!(delta(&done) > 10.0, "{layout}");
        assert_eq!(done.moved, 1, "{layout}: solo el tramo siguiente");
        assert!(done.warnings.is_empty(), "{layout}: {:?}", done.warnings);

        same(origin(&doc, R0), before[0], &format!("{layout}: el anterior"));
        same(origin(&doc, "de lunes a viernes inclusive"), before[1], &format!("{layout}: el editado"));
        same(origin(&doc, R2), (before[2].0 + delta(&done), before[2].1), &format!("{layout}: el siguiente"));
        same(origin(&doc, NEXT), before[3], &format!("{layout}: la línea de abajo"));
    }
}

#[test]
fn a_shorter_middle_run_pulls_what_follows_in() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let before = origin(&doc, R2);
        let done = edit(&mut doc, R1, "lunes");
        assert!(delta(&done) < -10.0);
        same(origin(&doc, R2), (before.0 + delta(&done), before.1), layout);
        same(origin(&doc, NEXT), (72.0, 680.0), &format!("{layout}: la línea de abajo"));
    }
}

#[test]
fn an_equal_width_edit_moves_nothing_and_touches_nothing_else() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let before = origin(&doc, R2);
        let done = edit(&mut doc, R1, "lunes a viernes");
        near(delta(&done), 0.0, layout);
        assert!(!done.reread && done.moved == 0 && done.warnings.is_empty(), "{layout}: {done:?}");
        same(origin(&doc, R2), before, layout);
    }
}

#[test]
fn editing_the_first_run_moves_every_run_after_it() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let (b1, b2, bn) = (origin(&doc, R1), origin(&doc, R2), origin(&doc, NEXT));
        let done = edit(&mut doc, R0, "El resto de todas las oficinas de pasaportes de");
        let d = delta(&done);
        assert!(d > 20.0);
        assert_eq!(done.moved, 2, "{layout}");
        same(origin(&doc, R1), (b1.0 + d, b1.1), &format!("{layout}: negrita"));
        same(origin(&doc, R2), (b2.0 + d, b2.1), &format!("{layout}: último"));
        same(origin(&doc, NEXT), bn, &format!("{layout}: la línea de abajo"));
    }
}

#[test]
fn editing_the_last_run_has_nothing_to_move() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let before: Vec<_> = [R0, R1, NEXT].iter().map(|t| origin(&doc, t)).collect();
        let done = edit(&mut doc, R2, "de 8 a 20 horas todos los dias");
        assert_eq!(done.moved, 0, "{layout}");
        for (text, was) in [R0, R1, NEXT].iter().zip(before) {
            same(origin(&doc, text), was, &format!("{layout}: {text}"));
        }
    }
}

#[test]
fn undo_puts_everything_back_and_redo_does_it_again() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let all = |d: &Document| d.text_runs(0).unwrap().into_iter().map(|r| (r.text, r.origin)).collect::<Vec<_>>();
        let original = all(&doc);
        edit(&mut doc, R1, "de lunes a viernes inclusive");
        let edited = all(&doc);
        assert_ne!(edited, original);
        assert!(doc.undo());
        for ((t1, o1), (t2, o2)) in all(&doc).iter().zip(&original) {
            assert_eq!(t1, t2, "{layout}");
            same(*o1, *o2, &format!("{layout}: {t1}"));
        }
        assert!(doc.redo());
        for ((t1, o1), (t2, o2)) in all(&doc).iter().zip(&edited) {
            assert_eq!(t1, t2);
            same(*o1, *o2, &format!("{layout}: {t1}"));
        }
    }
}

#[test]
fn the_move_survives_saving_and_reopening() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let before = origin(&doc, R2);
        let done = edit(&mut doc, R1, "de lunes a viernes inclusive");
        let again = Document::open(&doc.save_to_bytes().unwrap()).unwrap();
        same(origin(&again, R2), (before.0 + delta(&done), before.1), layout);
        same(origin(&again, NEXT), (72.0, 680.0), layout);
    }
}

#[test]
fn run_ids_expire_exactly_when_the_page_was_renumbered() {
    // Each run has a position of its own: a move had to be inserted.
    let mut doc = open("line_absolute");
    assert!(edit(&mut doc, R1, "de lunes a viernes inclusive").reread, "se insertaron operaciones: hay que volver a leer");
    // The pen carries the followers along by itself: nothing was inserted, and
    // the ids still mean what they meant.
    let mut doc = open("line_pen");
    let done = edit(&mut doc, R1, "de lunes a viernes inclusive");
    assert!(!done.reread && done.moved == 1);
}

#[test]
fn reflow_none_leaves_the_rest_alone_even_if_they_overlap() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let before = origin(&doc, R2);
        let r = run(&doc, R1);
        let done = doc
            .replace_text_with(0, &r, "de lunes a viernes inclusive", ReplaceOptions { reflow: Reflow::None, ..Default::default() })
            .unwrap();
        // A pen-following layout moves its followers by itself; the others do not.
        let moved_by_itself = origin(&doc, R2).0 - before.0;
        assert!(done.moved == 0 && done.warnings.is_empty());
        assert!(moved_by_itself.abs() < 0.05 || (moved_by_itself - delta(&done)).abs() < 0.05, "{layout}");
    }
}

#[test]
fn centred_keeps_the_middle_of_the_line() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let extent = |d: &Document| {
            let runs = d.text_runs(0).unwrap();
            let line: Vec<_> = runs.iter().filter(|r| (r.origin.1 - 700.0).abs() < 1.0).collect();
            (line.iter().map(|r| r.display[0]).fold(f32::MAX, f32::min), line.iter().map(|r| r.display[2]).fold(f32::MIN, f32::max))
        };
        let (l0, r0) = extent(&doc);
        let r = run(&doc, R1);
        doc.replace_text_with(0, &r, "de lunes a viernes inclusive", ReplaceOptions { reflow: Reflow::Centered, ..Default::default() }).unwrap();
        let (l1, r1) = extent(&doc);
        near((l1 + r1) / 2.0, (l0 + r0) / 2.0, &format!("{layout}: el centro"));
        assert!(r1 - l1 > r0 - l0 + 10.0, "{layout}");
        same(origin(&doc, NEXT), (72.0, 680.0), &format!("{layout}: la línea de abajo"));
    }
}

#[test]
fn right_aligned_keeps_the_right_edge() {
    for layout in LAYOUTS {
        let mut doc = open(layout);
        let right = |d: &Document| {
            d.text_runs(0).unwrap().iter().filter(|r| (r.origin.1 - 700.0).abs() < 1.0).map(|r| r.display[2]).fold(f32::MIN, f32::max)
        };
        let before = right(&doc);
        let r = run(&doc, R1);
        doc.replace_text_with(0, &r, "de lunes a viernes inclusive", ReplaceOptions { reflow: Reflow::RightAligned, ..Default::default() }).unwrap();
        near(right(&doc), before, &format!("{layout}: el borde derecho"));
        same(origin(&doc, NEXT), (72.0, 680.0), &format!("{layout}: la línea de abajo"));
    }
}

#[test]
fn a_line_pushed_off_the_page_says_so() {
    let mut doc = open("line_absolute");
    let done = edit(&mut doc, R1, &"lunes a viernes y tambien ".repeat(6));
    assert!(done.warnings.iter().any(|w| matches!(w, Warning::ExceedsPage { by } if *by > 0.0)), "{:?}", done.warnings);
}

#[test]
fn a_line_pushed_past_every_other_line_says_so_but_a_small_change_does_not() {
    let mut doc = open("line_absolute");
    // The longest line ends at about 480; this one ends near 385.
    let small = edit(&mut doc, R1, "lunes a viernes y sabado");
    assert!(small.warnings.is_empty(), "{:?}", small.warnings);
    let mut doc = open("line_absolute");
    let big = edit(&mut doc, R1, "lunes a viernes y tambien los sabados y domingos");
    assert!(big.warnings.iter().any(|w| matches!(w, Warning::ExceedsMargin { .. })), "{:?}", big.warnings);
}

#[test]
fn text_inside_a_form_xobject_is_not_moved_and_the_edit_says_so() {
    let mut doc = open("line_xobject");
    let before = run(&doc, R1);
    assert!(before.xobject.is_some());
    let done = edit(&mut doc, R0, "El resto de todas las oficinas de pasaportes de");
    assert_eq!(done.moved, 0);
    assert!(done.warnings.contains(&Warning::FollowerNotMoved { text: R1.to_string() }), "{:?}", done.warnings);
    same(run(&doc, R1).origin, before.origin, "el tramo del XObject se queda donde estaba");
    same(origin(&doc, NEXT), (72.0, 680.0), "la línea de abajo");
}

#[test]
fn a_fallback_font_split_still_moves_what_follows() {
    let mut doc = open("line_pen");
    let before = origin(&doc, R1);
    let r = run(&doc, R0);
    let done = doc.replace_text_with_fallback(0, &r, "El resto de las oficinas Ā de").unwrap();
    assert_eq!(done.fallback_chars, vec!['Ā']);
    assert!(done.reread);
    same(origin(&doc, R1), (before.0 + delta(&done), before.1), "el siguiente");
    same(origin(&doc, NEXT), (72.0, 680.0), "la línea de abajo");
}

#[test]
fn only_nearby_runs_move_not_a_column_further_along_the_line() {
    // R2 sits one space from R1; a run a column away is another thing.
    let mut doc = open("aligned");
    let before = doc.text_runs(0).unwrap();
    let far = before.iter().find(|r| r.text.starts_with("Fecha")).unwrap().origin;
    let r = before.iter().find(|r| r.text.starts_with("Nombre")).unwrap().clone();
    doc.replace_text(0, &r, "Nombre completo: ").unwrap();
    let again = doc.text_runs(0).unwrap();
    same(again.iter().find(|r| r.text.starts_with("Fecha")).unwrap().origin, far, "otra línea, no se toca");
}

#[test]
fn a_centred_or_right_aligned_line_is_flagged_but_a_left_one_is_not() {
    let hint = |starts: &str, new: &str| {
        let mut doc = open("aligned");
        let r = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.starts_with(starts)).unwrap();
        doc.replace_text(0, &r, new).unwrap().warnings
    };
    assert!(hint("Resumen", "Resumen largo ").contains(&Warning::LineMayBeAligned { alignment: Alignment::Centered }));
    assert!(hint("Fecha", "Fecha de hoy: ").contains(&Warning::LineMayBeAligned { alignment: Alignment::Right }));
    assert!(!hint("Nombre", "Nombre completo: ").iter().any(|w| matches!(w, Warning::LineMayBeAligned { .. })));
}

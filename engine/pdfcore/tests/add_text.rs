//! Adding new text to a page.

use pdfcore::{Align, Anchor, Document, Error, NewText};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn reopen(doc: &mut Document) -> Document {
    Document::open(&doc.save_to_bytes().expect("save")).expect("reopen")
}

fn near(a: f32, b: f32, tolerance: f32) {
    assert!((a - b).abs() <= tolerance, "{a} no está cerca de {b} (±{tolerance})");
}

#[test]
fn text_lands_at_the_top_left_it_was_asked_for() {
    let mut doc = open("standard");
    let added = doc.add_text(0, &NewText::new("Nuevo texto ñ", 72.0, 300.0)).unwrap();
    assert_eq!(added.lines, 1);

    let runs = reopen(&mut doc).text_runs(0).unwrap();
    let run = runs.last().unwrap();
    assert_eq!(run.text, "Nuevo texto ñ");
    assert_eq!(run.base_font, "LiberationSans-Regular");
    near(run.display[0], 72.0, 0.2);
    near(run.display[1], 300.0, 3.5);
    near(run.size, 12.0, 0.01);
    // The page's own text is untouched.
    assert_eq!(runs[0].text, "Hola mundo");
    assert_eq!(runs[1].text, "Negrita áéíóú ñ");
}

#[test]
fn a_baseline_anchor_puts_the_line_on_the_given_height() {
    let mut doc = open("standard");
    let mut text = NewText::new("sobre la raya", 100.0, 500.0);
    text.anchor = Anchor::Baseline;
    doc.add_text(0, &text).unwrap();
    let run = reopen(&mut doc).text_runs(0).unwrap().pop().unwrap();
    near(run.origin.0, 100.0, 0.05);
    near(run.origin.1, 842.0 - 500.0, 0.05);
}

#[test]
fn text_is_upright_on_a_rotated_page() {
    let mut doc = open("page_rotated"); // 400x600 turned to 600x400
    doc.add_text(0, &NewText::new("Derecho", 300.0, 200.0)).unwrap();
    let runs = reopen(&mut doc).text_runs(0).unwrap();
    let [x0, y0, x1, y1] = runs.last().unwrap().display;
    assert!(x1 - x0 > 2.0 * (y1 - y0), "debería ser ancho, no alto: {:?}", [x0, y0, x1, y1]);
    near(x0, 300.0, 0.5);
    near(y0, 200.0, 3.5);
}

#[test]
fn what_the_page_left_scaled_does_not_reach_the_new_text() {
    let mut doc = open("leaky");
    doc.add_text(0, &NewText::new("fuera de la escala", 72.0, 400.0)).unwrap();
    let runs = reopen(&mut doc).text_runs(0).unwrap();
    let run = runs.last().unwrap();
    near(run.size, 12.0, 0.01);
    near(run.display[0], 72.0, 0.2);
    near(run.display[1], 400.0, 3.5);
}

#[test]
fn long_text_wraps_to_the_width_it_was_given() {
    let mut doc = open("standard");
    let mut text = NewText::new("Solicito el cambio de contratante por motivo de fallecimiento del titular", 72.0, 200.0);
    text.max_width = Some(150.0);
    let added = doc.add_text(0, &text).unwrap();
    assert!(added.lines >= 3, "{} líneas", added.lines);
    let runs = reopen(&mut doc).text_runs(0).unwrap();
    let new: Vec<_> = runs.iter().skip(2).collect();
    assert_eq!(new.len(), added.lines);
    for run in &new {
        assert!(run.end.0 - run.origin.0 <= 150.5, "«{}» mide {}", run.text, run.end.0 - run.origin.0);
    }
    near(new[0].origin.1 - new[1].origin.1, 12.0 * 1.2, 0.05);
    assert_eq!(new.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join(" "),
        "Solicito el cambio de contratante por motivo de fallecimiento del titular");
}

#[test]
fn lines_align_inside_the_box() {
    let width_of = |align| {
        let mut doc = open("standard");
        let mut text = NewText::new("largo largo largo\ncorto", 72.0, 200.0);
        text.align = align;
        doc.add_text(0, &text).unwrap();
        let runs = reopen(&mut doc).text_runs(0).unwrap();
        let (long, short) = (&runs[2], &runs[3]);
        (short.origin.0 - long.origin.0, long.end.0 - long.origin.0, short.end.0 - short.origin.0)
    };
    let (left, ..) = width_of(Align::Left);
    near(left, 0.0, 0.05);
    let (centre, long, short) = width_of(Align::Center);
    near(centre, (long - short) / 2.0, 0.05);
    let (right, long, short) = width_of(Align::Right);
    near(right, long - short, 0.05);
}

#[test]
fn style_picks_the_matching_bundled_font() {
    let mut doc = open("standard");
    let mut text = NewText::new("Titulo", 72.0, 200.0);
    text.serif = true;
    text.bold = true;
    doc.add_text(0, &text).unwrap();
    assert_eq!(reopen(&mut doc).text_runs(0).unwrap().last().unwrap().base_font, "LiberationSerif-Bold");
}

#[test]
fn what_the_font_cannot_draw_fails_and_changes_nothing() {
    let mut doc = open("standard");
    match doc.add_text(0, &NewText::new("hola 世界", 72.0, 200.0)) {
        Err(Error::MissingGlyphs(chars)) => assert_eq!(chars, vec!['世', '界']),
        other => panic!("esperaba MissingGlyphs, salió {other:?}"),
    }
    assert_eq!(reopen(&mut doc).text_runs(0).unwrap().len(), 2);
}

#[test]
fn nonsense_is_refused() {
    let mut doc = open("standard");
    assert!(doc.add_text(0, &NewText::new("   ", 0.0, 0.0)).is_err());
    let mut zero = NewText::new("x", 0.0, 0.0);
    zero.size = 0.0;
    assert!(doc.add_text(0, &zero).is_err());
    assert!(doc.add_text(9, &NewText::new("x", 0.0, 0.0)).is_err());
}

#[test]
fn several_additions_and_an_edit_of_the_original_all_hold() {
    let mut doc = open("standard");
    doc.add_text(0, &NewText::new("uno", 72.0, 300.0)).unwrap();
    doc.add_text(0, &NewText::new("dos", 72.0, 330.0)).unwrap();
    let runs = doc.text_runs(0).unwrap();
    // Read again after adding, then edit a line the page already had.
    doc.replace_text(0, &runs[0], "Hola Mundo").unwrap();
    let texts: Vec<String> = reopen(&mut doc).text_runs(0).unwrap().into_iter().map(|r| r.text).collect();
    assert_eq!(texts, ["Hola Mundo", "Negrita áéíóú ñ", "uno", "dos"]);
}

#[test]
fn text_added_in_an_earlier_session_keeps_its_spacing() {
    // Each session used to embed its own copy of the bundled font under the
    // same name, replacing the first in the page's resources. The copy only
    // knew the widths of the glyphs it had been used for, so text written
    // earlier came out spaced as if every other glyph were a full em wide.
    let mut doc = open("standard");
    doc.add_text(0, &NewText::new("POL-2026-00431", 72.0, 300.0)).unwrap();
    let mut doc = reopen(&mut doc);
    let width = |d: &Document| {
        let run = d.text_runs(0).unwrap().into_iter().find(|r| r.text == "POL-2026-00431").unwrap();
        run.end.0 - run.origin.0
    };
    let before = width(&doc);
    assert!(before < 100.0, "{before}");
    let after_first = doc.save_to_bytes().unwrap().len();

    doc.add_text(0, &NewText::new("Ñandú ZW", 72.0, 340.0)).unwrap();
    let mut doc = reopen(&mut doc);
    near(width(&doc), before, 0.01);
    let again = doc.save_to_bytes().unwrap().len();
    assert!(again < after_first + 5_000, "{after_first} -> {again}: se incrustó otra copia de la fuente");

    // And a third session, for good measure.
    doc.add_text(0, &NewText::new("más", 72.0, 380.0)).unwrap();
    near(width(&reopen(&mut doc)), before, 0.01);
}

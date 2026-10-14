//! Reading text out of small PDFs built by `tools/make_fixtures.py`.

use pdfcore::Document;

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn near(a: f32, b: f32, tolerance: f32) {
    assert!((a - b).abs() <= tolerance, "{a} no está cerca de {b} (±{tolerance})");
}

#[test]
fn standard_fonts_read_without_a_widths_array() {
    let doc = open("standard");
    let runs = doc.text_runs(0).unwrap();
    assert_eq!(runs.len(), 2);

    assert_eq!(runs[0].text, "Hola mundo");
    assert_eq!(runs[0].base_font, "Helvetica");
    near(runs[0].size, 12.0, 0.01);
    // Placed at (72, 100) from the top of an 842pt page, so 742 in user space.
    near(runs[0].origin.0, 72.0, 0.01);
    near(runs[0].origin.1, 742.0, 0.01);
    // Helvetica: H722 o556 l222 a556 ' '278 m833 u556 n556 d556 o556 = 5391/1000 em.
    near(runs[0].end.0 - runs[0].origin.0, 5.391 * 12.0, 0.05);

    assert_eq!(runs[1].text, "Negrita áéíóú ñ");
    assert_eq!(runs[1].base_font, "Helvetica-Bold");
    near(runs[1].size, 14.0, 0.01);
}

#[test]
fn the_display_box_is_top_left_with_y_down() {
    let runs = open("standard").text_runs(0).unwrap();
    let [x0, y0, x1, y1] = runs[0].display;
    near(x0, 72.0, 0.01);
    assert!(y0 < 100.0 && y1 > 100.0, "la línea base (y=100) debe caer dentro de la caja: {y0}..{y1}");
    assert!(x1 > x0 && y1 > y0);
}

#[test]
fn an_embedded_composite_font_is_read_through_its_to_unicode() {
    let runs = open("embedded").text_runs(0).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "Póliza número 12345");
    near(runs[0].size, 11.0, 0.01);
    assert_eq!(runs[0].glyphs.len(), "Póliza número 12345".chars().count());
    // The width comes from the font's own /W table, not a guess.
    let width = runs[0].end.0 - runs[0].origin.0;
    assert!(width > 80.0 && width < 130.0, "ancho {width}");
}

#[test]
fn rotated_text_gets_a_tall_narrow_box() {
    let runs = open("rotated").text_runs(0).unwrap();
    assert_eq!(runs[0].text, "Vertical");
    let [x0, y0, x1, y1] = runs[0].display;
    assert!(y1 - y0 > 30.0, "alto {}", y1 - y0);
    assert!(x1 - x0 < 20.0, "ancho {}", x1 - x0);
}

#[test]
fn a_rotated_page_swaps_its_size_and_turns_the_text_with_it() {
    let doc = open("page_rotated");
    let size = doc.page_size(0).unwrap();
    assert_eq!((size.width, size.height, size.rotation), (600.0, 400.0, 90));
    let run = &doc.text_runs(0).unwrap()[0];
    assert_eq!(run.text, "Pagina girada");
    let [x0, y0, x1, y1] = run.display;
    // Written near the top-left of a 400x600 page, a quarter turn puts it
    // near the top-right of the 600x400 page it is shown on, running down.
    assert!(x0 > 500.0 && x1 < 540.0, "x {x0}..{x1}");
    near(y0, 50.0, 0.5);
    near(y1, 124.0, 0.5);
}

#[test]
fn a_page_that_does_not_exist_is_an_error_not_a_panic() {
    assert!(open("standard").text_runs(5).is_err());
}

//! The vector graphics a page paints.

use pdfcore::{Document, GraphicKind};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn every_painted_path_is_listed_where_the_page_shows_it() {
    let doc = open("graphics");
    let g = doc.page_graphics(0).unwrap();
    // Five shapes, the form's rectangle and the fill under a clip (the clip itself paints nothing).
    assert_eq!(g.len(), 7, "{g:#?}");
    // A rectangle, filled and stroked.
    assert_eq!(g[0].kind, GraphicKind::Rect);
    assert!(near(g[0].display, [100.0, 112.0, 150.0, 142.0]), "{:?}", g[0].display);
    assert_eq!((g[0].filled, g[0].stroked), (true, true));
    assert_eq!((g[0].fill, g[0].stroke), (Some([1.0, 0.0, 0.0]), Some([0.0, 0.0, 1.0])));
    assert!((g[0].line_width - 2.0).abs() < 0.01);
    // A line.
    assert_eq!(g[1].kind, GraphicKind::Line);
    assert!(near(g[1].display, [200.0, 92.0, 300.0, 142.0]));
    assert_eq!(g[1].ends, Some([(200.0, 142.0), (300.0, 92.0)]));
    assert_eq!((g[1].filled, g[1].stroked), (false, true));
    // A rectangle under `cm`: the matrix is applied (and so is the width).
    assert_eq!(g[2].kind, GraphicKind::Rect);
    assert!(near(g[2].display, [100.0, 222.0, 140.0, 242.0]), "{:?}", g[2].display);
    // A curve is a path; a box drawn with lines is still a rectangle.
    assert_eq!(g[3].kind, GraphicKind::Path);
    assert!(near(g[3].display, [100.0, 292.0, 250.0, 392.0]), "{:?}", g[3].display);
    assert_eq!(g[4].kind, GraphicKind::Rect);
    assert!(near(g[4].display, [300.0, 502.0, 340.0, 542.0]), "{:?}", g[4].display);
    // Painted by a form XObject, with the form's matrix.
    assert_eq!(g[5].xobject.as_deref(), Some("Fm1"));
    assert!(near(g[5].display, [10.0, 842.0 - 60.0, 50.0, 842.0 - 20.0]), "{:?}", g[5].display);
    assert_eq!(g[5].fill, Some([0.0, 1.0, 0.0]));
    assert!(g.iter().filter(|x| x.xobject.is_none()).count() == 6);
}

#[test]
fn a_turned_page_gives_the_same_graphics_as_the_page_is_shown() {
    let doc = open("graphics_rotated");
    let size = doc.page_size(0).unwrap();
    assert_eq!((size.width, size.height), (842.0, 595.0));
    let g = doc.page_graphics(0).unwrap();
    // (x, y) of the page become (y, x) once turned 90 degrees.
    assert!(near(g[0].display, [700.0, 100.0, 730.0, 150.0]), "{:?}", g[0].display);
    assert_eq!(g[0].kind, GraphicKind::Rect);
    assert!(near(g[1].display, [700.0, 200.0, 750.0, 300.0]), "{:?}", g[1].display);
    assert_eq!(g[1].kind, GraphicKind::Line);
    assert_eq!(g[4].kind, GraphicKind::Rect);
}

#[test]
fn a_page_with_no_graphics_and_a_missing_page() {
    assert!(open("blank").page_graphics(0).unwrap().is_empty());
    assert!(matches!(open("blank").page_graphics(3), Err(pdfcore::Error::NoSuchPage(3))));
}

#[test]
fn the_check_boxes_of_the_real_form_are_found() {
    // CAMBIO_DE_CONTRATANTE.pdf (a form of an insurer) sits in the uploads of the session, not in
    // the repository: this test is a no-op where the file is not.
    let Some(path) = std::env::var_os("CAMBIO_PDF").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return };
    let doc = Document::open(&std::fs::read(path).unwrap()).unwrap();
    let boxes: Vec<_> = doc
        .page_graphics(0)
        .unwrap()
        .into_iter()
        .filter(|g| g.kind == GraphicKind::Rect && (g.display[2] - g.display[0] - 9.4).abs() < 0.3 && (g.display[3] - g.display[1] - 9.4).abs() < 0.3)
        .filter(|g| (g.display[1] - 266.2).abs() < 0.5)
        .collect();
    let lefts: Vec<f32> = boxes.iter().map(|g| g.display[0]).collect();
    assert_eq!(lefts.len(), 3, "{lefts:?}");
    for (found, expected) in lefts.iter().zip([29.6, 87.9, 174.5]) {
        assert!((found - expected).abs() < 0.2, "{lefts:?}");
    }
}

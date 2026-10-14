//! Placing pictures. How they *look* is checked against a renderer in
//! pdfcore-ffi/tests/test_images.py; here, what the engine records.

use pdfcore::{Document, Error, NewImage};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).expect("fixture")
}

fn open(name: &str) -> Document {
    Document::open(&fixture(name)).expect("open")
}

#[test]
fn a_picture_is_placed_in_the_box_asked_for_and_keeps_its_proportions() {
    let mut doc = open("standard.pdf");
    let jpeg = fixture("img_rgb.jpg");
    let added = doc.add_image(0, &NewImage::new(&jpeg, 100.0, 200.0, 160.0)).unwrap();
    assert_eq!(added.pixels, (80, 40));
    assert_eq!(added.display, [100.0, 200.0, 260.0, 280.0], "80×40 at 160 wide is 80 tall");
    let sized = doc.add_image(0, &NewImage { height: Some(30.0), ..NewImage::new(&jpeg, 10.0, 10.0, 30.0) }).unwrap();
    assert_eq!(sized.display, [10.0, 10.0, 40.0, 40.0]);
}

#[test]
fn an_exif_turned_photo_swaps_its_proportions() {
    let mut doc = open("standard.pdf");
    let upright = doc.add_image(0, &NewImage::new(&fixture("img_exif6.jpg"), 0.0, 0.0, 40.0)).unwrap();
    assert_eq!(upright.pixels, (40, 80), "a photo stored sideways is shown the other way round");
    assert_eq!(upright.display[3], 80.0);
    let half_turn = doc.add_image(0, &NewImage::new(&fixture("img_exif3.jpg"), 0.0, 0.0, 40.0)).unwrap();
    assert_eq!(half_turn.pixels, (80, 40));
}

#[test]
fn a_picture_survives_a_save_and_adds_no_second_copy() {
    let mut doc = open("standard.pdf");
    doc.add_image(0, &NewImage::new(&fixture("img_rgb.jpg"), 50.0, 50.0, 80.0)).unwrap();
    let saved = doc.save_to_bytes().unwrap();
    let again = Document::open(&saved).unwrap();
    // The page's text is untouched by the picture.
    assert_eq!(again.text_runs(0).unwrap()[0].text, "Hola mundo");
    // The JPEG went in as it was: its bytes are in the file, unchanged.
    let jpeg = fixture("img_rgb.jpg");
    assert!(saved.windows(jpeg.len()).any(|w| w == &jpeg[..]));
}

#[test]
fn a_png_with_transparency_gets_a_soft_mask() {
    let mut doc = open("standard.pdf");
    doc.add_image(0, &NewImage::new(&fixture("img_alpha.png"), 50.0, 50.0, 40.0)).unwrap();
    let raw = lopdf::Document::load_mem(&doc.save_to_bytes().unwrap()).unwrap();
    let image = raw
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| s.dict.get(b"Subtype").and_then(|o| o.as_name()).is_ok_and(|n| n == b"Image" ) && s.dict.has(b"SMask"))
        .expect("an image with a soft mask");
    assert_eq!(image.dict.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceRGB");
}

#[test]
fn a_picture_is_one_undo_step() {
    let mut doc = open("standard.pdf");
    let before = doc.save_to_bytes().unwrap().len();
    doc.add_image(0, &NewImage::new(&fixture("img_rgb.jpg"), 50.0, 50.0, 80.0)).unwrap();
    assert!(doc.undo());
    assert!(!doc.can_undo());
    assert!(doc.save_to_bytes().unwrap().len() <= before + 16, "nothing of the picture is left");
}

#[test]
fn what_is_not_a_usable_picture_is_refused_and_changes_nothing() {
    let mut doc = open("standard.pdf");
    for (bytes, what) in [(&b"GIF89a....."[..], "gif"), (&b""[..], "empty"), (&[0xFF, 0xD8, 0xFF, 0xD9][..], "no frame")] {
        let error = doc.add_image(0, &NewImage::new(bytes, 0.0, 0.0, 10.0)).unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)), "{what}: {error}");
    }
    let png = fixture("img_alpha.png");
    assert!(matches!(doc.add_image(0, &NewImage::new(&png[..60], 0.0, 0.0, 10.0)), Err(Error::Unsupported(_))), "truncated png");
    assert!(matches!(doc.add_image(0, &NewImage::new(&png, 0.0, 0.0, 0.0)), Err(Error::Rejected(_))));
    assert!(matches!(doc.add_image(9, &NewImage::new(&png, 0.0, 0.0, 10.0)), Err(Error::NoSuchPage(9))));
    assert!(!doc.can_undo());
}

#[test]
fn a_huge_picture_is_refused_before_it_is_decoded() {
    // A header claiming 20000×20000 pixels, with no data behind it.
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x4E, 0x20, 0x4E, 0x20, 0x03];
    jpeg.extend([1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    let mut doc = open("standard.pdf");
    assert!(matches!(doc.add_image(0, &NewImage::new(&jpeg, 0.0, 0.0, 10.0)), Err(Error::Rejected(_))));
}

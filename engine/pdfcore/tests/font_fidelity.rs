//! Reading and writing text the way the document's own encoding says, and refusing
//! to edit what cannot be reproduced (never changing the format in silence).

use pdfcore::{Document, Error, FontClass, ReplaceOptions};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

fn first_run(doc: &Document) -> pdfcore::TextRun {
    doc.text_runs(0).unwrap().into_iter().find(|r| !r.text.trim().is_empty() || r.not_editable.is_some()).expect("a run")
}

const NO_FALLBACK: ReplaceOptions = ReplaceOptions { allow_fallback: false, reflow: pdfcore::Reflow::None };

#[test]
fn macroman_text_is_read_and_written_with_the_documents_own_codes() {
    let mut doc = open("font_macroman");
    let run = first_run(&doc);
    assert_eq!(run.text, "café ñandú");
    // Every letter has a MacRoman code, so nothing goes to another font.
    let done = doc.replace_text_with(0, &run, "Camión ñandú ÁÉÍÓÚ", NO_FALLBACK).unwrap();
    assert!(done.fallback_chars.is_empty());
    assert_eq!(first_run(&doc).text, "Camión ñandú ÁÉÍÓÚ");
    // A letter MacRoman lacks is refused, not written as some other letter.
    let run = first_run(&doc);
    assert!(matches!(doc.replace_text_with(0, &run, "Žluťoučký", NO_FALLBACK), Err(Error::MissingGlyphs(c)) if c.contains(&'Ž')));
}

#[test]
fn standard_encoding_reads_ligatures_and_keeps_them_as_one_glyph() {
    let mut doc = open("font_standardenc");
    let run = first_run(&doc);
    assert_eq!(run.text, "o\u{FB01}ce \u{FB02}or ¡hola");
    // The ligature is one code of this font: typing it back keeps the font's own glyph.
    let done = doc.replace_text_with(0, &run, "\u{FB01}nal \u{FB02}or", NO_FALLBACK).unwrap();
    assert!(done.fallback_chars.is_empty());
    assert_eq!(first_run(&doc).text, "\u{FB01}nal \u{FB02}or");
    // StandardEncoding has no accented letters (they are composed): they are not guessed.
    let run = first_run(&doc);
    assert!(matches!(doc.replace_text_with(0, &run, "camión", NO_FALLBACK), Err(Error::MissingGlyphs(c)) if c.contains(&'ó')));
}

#[test]
fn symbol_text_is_greek_and_latin_letters_are_not_silently_turned_into_greek() {
    let mut doc = open("font_symbol");
    let run = first_run(&doc);
    assert_eq!(run.text, "αβγ");
    assert_eq!(run.font_kind, FontClass::Standard);
    let done = doc.replace_text_with(0, &run, "αβ π", NO_FALLBACK).unwrap();
    assert!(done.fallback_chars.is_empty());
    let run = first_run(&doc);
    match doc.replace_text_with(0, &run, "Hola", NO_FALLBACK) {
        Err(Error::MissingGlyphs(chars)) => assert_eq!(chars, vec!['H', 'o', 'l', 'a']),
        other => panic!("Latin letters must not become Symbol's Greek: {other:?}"),
    }
}

#[test]
fn text_that_cannot_be_reproduced_is_read_and_refused_with_a_reason() {
    for (fixture, reason) in [
        ("font_type3", "type3"),
        ("font_type0_identity_v", "vertical_writing"),
        ("font_zapf", "symbolic_font"),
        ("font_rtl_arabic", "rtl_text"),
    ] {
        let mut doc = open(fixture);
        let run = first_run(&doc);
        assert_eq!(run.not_editable, Some(reason), "{fixture}");
        let before = doc.save_to_bytes().unwrap();
        match doc.replace_text_with(0, &run, "Hola", ReplaceOptions { allow_fallback: true, reflow: pdfcore::Reflow::Following }) {
            Err(Error::NotEditable(r)) => assert_eq!(r, reason, "{fixture}"),
            other => panic!("{fixture}: expected NotEditable({reason}), got {other:?}"),
        }
        assert!(!doc.can_undo(), "{fixture}: a refusal is not a step");
        assert_eq!(doc.save_to_bytes().unwrap(), before, "{fixture}: nothing changed");
    }
    // Reading still works (the Type 3 text is the codes' letters, as other readers show it)...
    assert_eq!(first_run(&open("font_type3")).text, "ABAB");
    // ...and so does moving: only editing the letters is refused.
    let mut doc = open("font_type3");
    let item = doc.page_items(0).unwrap().into_iter().find(|i| i.kind == pdfcore::ItemKind::Text).unwrap();
    doc.move_items(0, &[(item, (10.0, 5.0))]).unwrap();
}

#[test]
fn right_to_left_text_is_refused_even_in_a_font_that_could_draw_it() {
    // The new text, not only the old, decides: Arabic typed into a Latin run needs shaping.
    let mut doc = open("font_tt_full_type0");
    let run = first_run(&doc);
    assert_eq!(run.not_editable, None);
    assert!(matches!(
        doc.replace_text_with(0, &run, "\u{645}\u{631}\u{62D}\u{628}\u{627}", NO_FALLBACK),
        Err(Error::NotEditable(r)) if r == "rtl_text"
    ));
    assert!(!doc.can_undo());
}

#[test]
fn a_paragraph_with_text_that_cannot_be_edited_is_refused_whole() {
    let mut doc = open("font_type3");
    let run = first_run(&doc);
    assert!(matches!(doc.replace_paragraph(0, &run, "x y z", false), Err(Error::NotEditable(_)) | Err(Error::Unsupported(_))));
    assert!(!doc.can_undo());
}

#[test]
fn every_run_says_what_kind_of_font_it_is_in() {
    for (fixture, kind, embedded) in [
        ("standard", FontClass::Standard, false),
        ("font_nonstd_notembedded", FontClass::NotEmbedded, false),
        ("font_truetype_notembedded", FontClass::NotEmbedded, false),
        ("font_mmtype1", FontClass::NotEmbedded, false),
        ("font_type3", FontClass::Type3, false),
        ("simple_subset", FontClass::EmbeddedSubset, true),
        ("embedded", FontClass::EmbeddedSubset, true),
        ("font_tt_full_simple", FontClass::EmbeddedFull, true),
        ("font_tt_full_type0", FontClass::EmbeddedFull, true),
        ("cff_type0", FontClass::EmbeddedFull, true),
        ("type1_simple", FontClass::EmbeddedFull, true),
        ("font_cjk", FontClass::EmbeddedSubset, true),
    ] {
        let run = first_run(&open(fixture));
        assert_eq!((run.font_kind, run.font_embedded), (kind, embedded), "{fixture}");
    }
    assert_eq!(FontClass::EmbeddedSubset.as_str(), "embedded_subset");
}

#[test]
fn an_embedded_font_with_its_own_encoding_is_read_by_it() {
    // No /Encoding: the program's built-in StandardEncoding says 0xF1 is "ae", 0xFA "oe".
    for fixture in ["font_type1_noenc", "font_cff_noenc", "font_otf_noenc"] {
        assert_eq!(first_run(&open(fixture)).text, "Hola æandœ", "{fixture}");
    }
}

#[test]
fn glyph_names_follow_the_adobe_glyph_list() {
    use pdfcore::glyph_name_text as text;
    assert_eq!(text("A").as_deref(), Some("A"));
    assert_eq!(text("ntilde").as_deref(), Some("ñ"));
    assert_eq!(text("Zcaron").as_deref(), Some("Ž"));
    assert_eq!(text("f_i").as_deref(), Some("fi"));
    assert_eq!(text("fi").as_deref(), Some("\u{FB01}"));
    assert_eq!(text("uni00E9").as_deref(), Some("é"));
    assert_eq!(text("uni00660069").as_deref(), Some("fi"));
    assert_eq!(text("u1F600").as_deref(), Some("\u{1F600}"));
    assert_eq!(text("a.sc").as_deref(), Some("a"));
    assert_eq!(text("alpha").as_deref(), Some("α"));
    assert_eq!(text(".notdef"), None);
    assert_eq!(text("g123"), None);
}

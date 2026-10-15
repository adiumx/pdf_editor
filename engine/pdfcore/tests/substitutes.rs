//! Letters a font lacks: the same face when the reader has it, the bundled font otherwise.

use pdfcore::{Document, Via};

fn open(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"));
    Document::open(&std::fs::read(path).expect("fixture")).expect("open")
}

#[test]
fn a_standard_font_learns_the_letter_and_stays_the_same_face() {
    // Helvetica with no /Encoding is StandardEncoding, which has no n-tilde.
    let mut doc = open("font_standardenc");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text(0, &run, "mañana").unwrap();
    assert_eq!(done.substitutes.len(), 1);
    assert_eq!((done.substitutes[0].ch, done.substitutes[0].via), ('ñ', Via::Encoding));
    assert!(done.fallback_chars.is_empty());
    // Read back: the same font, the letter is text.
    let runs = doc.text_runs(0).unwrap();
    assert_eq!(runs[0].text, "mañana");
    assert_eq!(runs[0].base_font, "Helvetica");
    // Undo puts everything back.
    assert!(doc.undo());
    assert_eq!(doc.text_runs(0).unwrap()[0].base_font, "Helvetica");
    assert!(!doc.can_undo());
}

#[test]
fn a_font_with_widths_gets_a_width_for_the_new_letter() {
    let mut doc = open("font_nonstd_notembedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    // WinAnsi has n-tilde already; Euro sign is in WinAnsi at 0x80 but outside Widths (32..126).
    doc.replace_text(0, &run, "Hola ñ €").unwrap();
    let after = doc.text_runs(0).unwrap();
    assert_eq!(after[0].text, "Hola ñ €");
    // The euro sign is outside the font's /Widths: it must not be drawn with no width.
    assert!(after[0].glyphs.last().unwrap().width > 300.0, "{:?}", after[0].glyphs.last());
}

#[test]
fn an_embedded_subset_falls_to_the_bundled_font_and_says_so() {
    let mut doc = open("embedded");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text_with_fallback(0, &run, "Ñandú").unwrap();
    assert!(done.substitutes.iter().all(|s| s.via == Via::BundledFont));
    assert!(!done.substitutes.is_empty());
}

mod policy {
    use super::*;
    use pdfcore::{ReplaceOptions, Substitution};

    fn options(substitution: Substitution) -> ReplaceOptions {
        ReplaceOptions { allow_fallback: true, substitution, ..Default::default() }
    }

    #[test]
    fn per_char_changes_only_the_missing_letter() {
        let mut doc = open("font_subset_as_verdana");
        let run = doc.text_runs(0).unwrap().remove(0);
        let done = doc.replace_text_with(0, &run, "Hola mundo cruel de Ñandú", options(Substitution::PerChar)).unwrap();
        let chars: String = done.substitutes.iter().map(|s| s.ch).collect();
        assert!(chars.contains('Ñ') && !chars.contains('H'), "{chars}");
        // The closest bundled face to a Verdana is a sans: only a guess, and it says so.
        assert!(done.substitutes.iter().all(|s| s.similarity < 90 && s.font.starts_with("LiberationSans")));
    }

    #[test]
    fn whole_run_draws_everything_in_one_face_and_squeezes_it_to_the_original_pace() {
        let mut doc = open("font_subset_as_verdana");
        let run = doc.text_runs(0).unwrap().remove(0);
        let text = "Hola mundo cruel de Ñandú";
        let done = doc.replace_text_with(0, &run, text, options(Substitution::WholeRun)).unwrap();
        assert_eq!(done.substitutes.len(), text.chars().count());
        let runs = doc.text_runs(0).unwrap();
        assert_eq!(runs[0].text, text);
        assert!(runs[0].base_font.contains("LiberationSans"));
        // The sans runs wider than the serif it replaces: Tz makes it keep the serif's pace.
        assert!(runs[0].hscale < 0.97 && runs[0].hscale > 0.8, "{}", runs[0].hscale);
        // Undone, nothing of it is left.
        assert!(doc.undo());
        assert_eq!(doc.text_runs(0).unwrap()[0].hscale, 1.0);
    }

    #[test]
    fn auto_keeps_a_metric_clone_per_char() {
        let mut doc = open("producer_reportlab_ttf");
        let b = doc.blocks(0).unwrap().into_iter().find(|b| b.text.starts_with("Otro")).unwrap();
        let done = doc.replace_block(0, &b, &b.text.replace("medio", "mediø"), true).unwrap();
        assert_eq!(done.substitutes.len(), 1);
        assert_eq!(done.substitutes[0].similarity, 100);
    }
}

#[test]
fn a_full_composite_font_draws_letters_its_text_map_never_listed() {
    // The program holds every Latin letter; the ToUnicode map only the ones the page used.
    let mut doc = open("font_tt_full_type0_tight");
    let run = doc.text_runs(0).unwrap().remove(0);
    let done = doc.replace_text_with(0, &run, "Póliza ñandú €", pdfcore::ReplaceOptions::default()).unwrap();
    assert!(done.fallback_chars.is_empty(), "{:?}", done.fallback_chars);
    // Drawn by the original font, and said so.
    assert!(done.substitutes.iter().any(|s| s.ch == 'ñ' && s.via == Via::FontProgram), "{:?}", done.substitutes);
    let after = doc.text_runs(0).unwrap();
    assert_eq!(after[0].text, "Póliza ñandú €");
    assert!(after[0].glyphs.iter().all(|g| g.width > 0.0));
    assert!(doc.undo());
}

mod font_book {
    use super::*;
    use pdfcore::{Error, FontClass, ReplaceOptions, Substitution};

    fn verdana() -> Vec<u8> {
        std::fs::read(format!("{}/tests/fixtures/registered_verdana.ttf", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn required_fonts_say_what_is_missing_and_where_to_get_it() {
        let doc = open("font_subset_as_verdana");
        let fonts = doc.required_fonts().unwrap();
        assert_eq!(fonts.len(), 1);
        let f = &fonts[0];
        assert_eq!((f.base_font.as_str(), f.class, f.needed, f.registered), ("ABCDEF+Verdana", FontClass::EmbeddedSubset, true, false));
        assert_eq!(f.pages, vec![0]);
        assert!(f.similarity < 90);
        assert_eq!(f.source.as_ref().unwrap().name, "Open Sans");
        // A standard font needs nothing.
        let standard = open("standard").required_fonts().unwrap();
        assert!(standard.iter().all(|f| !f.needed));
    }

    #[test]
    fn a_registered_font_is_used_for_its_family_at_full_similarity() {
        let mut doc = open("font_subset_as_verdana");
        let info = doc.register_font(&verdana()).unwrap();
        assert_eq!((info.family.as_str(), info.bold, info.italic), ("Verdana", false, false));
        assert!(doc.required_fonts().unwrap()[0].registered);
        let run = doc.text_runs(0).unwrap().remove(0);
        let opts = ReplaceOptions { allow_fallback: true, substitution: Substitution::WholeRun, ..Default::default() };
        let done = doc.replace_text_with(0, &run, "Hola mundo Ñandú", opts).unwrap();
        assert!(done.substitutes.iter().all(|s| s.similarity == 100 && s.font == "Verdana-Regular"), "{:?}", done.substitutes);
        assert_eq!(doc.text_runs(0).unwrap()[0].text, "Hola mundo Ñandú");
        assert!(doc.undo());
    }

    #[test]
    fn junk_and_oversized_fonts_are_refused_with_a_reason() {
        let mut doc = open("standard");
        assert!(matches!(doc.register_font(b"not a font"), Err(Error::Unsupported(_))));
        assert!(matches!(doc.register_font(&vec![0u8; 21 * 1024 * 1024]), Err(Error::Rejected(_))));
    }
}

#[test]
fn search_finds_words_written_with_ligature_glyphs() {
    // "oﬁce ﬂor ¡hola": the ligatures are one glyph each.
    let doc = open("font_standardenc");
    let found = doc.find_text("ofice", Default::default());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].text, "o\u{FB01}ce");
    assert_eq!(doc.find_text("flor", Default::default()).len(), 1);
    // Typing the ligature itself finds it too.
    assert_eq!(doc.find_text("\u{FB01}ce", Default::default()).len(), 1);
}

#[test]
fn the_bundled_font_is_cut_down_to_the_letters_used() {
    let mut doc = open("embedded");
    let before = doc.save_to_bytes().unwrap().len();
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text_with_fallback(0, &run, "Ñandú ЖЖ").unwrap();
    let grown = doc.save_to_bytes().unwrap().len() - before;
    // A whole Liberation Sans is ~410 KB (~190 KB compressed); a few letters of it are a few KB.
    assert!(grown < 30_000, "grew by {grown} bytes");
    // More letters later: the same font, grown a little.
    let run = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("Ñ") || r.text.contains("Ж")).unwrap_or_else(|| doc.text_runs(0).unwrap().remove(0));
    doc.replace_text_with_fallback(0, &run, "Ñandú ЖЖ Щ").unwrap();
    assert!(doc.save_to_bytes().unwrap().len() - before < 40_000);
}

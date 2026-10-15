//! Protected documents: opened when the user password is empty, a clear
//! error when it is needed.

use pdfcore::{Document, Error};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}.pdf", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn an_owner_password_does_not_stop_reading_or_editing() {
    for name in ["encrypted_owner", "encrypted_rc4"] {
        let mut doc = Document::open(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(doc.page_count(), 1, "{name}");
        let runs = doc.text_runs(0).unwrap();
        assert_eq!(runs[0].text, "Hola mundo", "{name}");
        doc.replace_text(0, &runs[0], "Adiós").unwrap();
        // The saved copy is readable without any password.
        let saved = doc.save_to_bytes().unwrap();
        let again = Document::open(&saved).unwrap();
        assert_eq!(again.text_runs(0).unwrap()[0].text, "Adiós", "{name}");
    }
}

#[test]
fn a_user_password_is_asked_for() {
    let bytes = fixture("encrypted_user");
    assert!(matches!(Document::open(&bytes), Err(Error::PasswordRequired)));
    assert!(matches!(Document::open_with_password(&bytes, "mal"), Err(Error::WrongPassword)));
    let doc = Document::open_with_password(&bytes, "secreto").unwrap();
    assert_eq!(doc.text_runs(0).unwrap()[0].text, "Hola mundo");
}

#[test]
fn a_dictionary_written_inline_in_the_trailer_is_read() {
    // MuPDF writes /Encrypt as a dictionary in the trailer; the engine makes it an object first.
    let original = fixture("encrypted_inline");
    let mut doc = Document::open(&original).expect("opens");
    assert_eq!(doc.text_runs(0).unwrap()[0].text, "Hola mundo");
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text(0, &run, "Editado").unwrap();
    assert_eq!(Document::open(&doc.save_to_bytes().unwrap()).unwrap().text_runs(0).unwrap()[0].text, "Editado");
    // And the password is still asked for when there is one.
    let protected = fixture("encrypted_inline_user");
    assert!(matches!(Document::open(&protected), Err(Error::PasswordRequired)));
    assert!(matches!(Document::open_with_password(&protected, "mal"), Err(Error::WrongPassword)));
    let mut doc = Document::open_with_password(&protected, "secreto").unwrap();
    assert_eq!(doc.text_runs(0).unwrap()[0].text, "Hola mundo");
    // An incremental save keeps working on top of the rewritten file.
    let run = doc.text_runs(0).unwrap().remove(0);
    doc.replace_text(0, &run, "Cambiado").unwrap();
    let saved = doc.save_incremental().unwrap();
    assert_eq!(Document::open_with_password(&saved, "secreto").unwrap().text_runs(0).unwrap()[0].text, "Cambiado");
}

#[test]
fn a_file_with_an_xref_stream_and_inline_protection_still_says_it_cannot() {
    // No classic trailer to rewrite: refuse in plain words rather than fail obscurely.
    let mut bytes = fixture("encrypted_inline");
    let at = bytes.windows(7).rposition(|w| w == b"trailer").unwrap();
    bytes[at..at + 7].copy_from_slice(b"trailor");
    let error = Document::open(&bytes).err().expect("must fail");
    assert!(matches!(error, Error::Unsupported(_) | Error::Pdf(_)), "{error}");
}

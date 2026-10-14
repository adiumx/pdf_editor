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
fn an_unreadable_form_of_protection_says_so() {
    let error = Document::open(&fixture("encrypted_inline")).err().expect("must fail");
    assert!(matches!(error, Error::Unsupported(_)), "{error}");
}

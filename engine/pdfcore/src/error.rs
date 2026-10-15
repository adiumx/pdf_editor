use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no se pudo leer el PDF: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("el documento está protegido: hace falta la contraseña")]
    PasswordRequired,
    #[error("la contraseña no es correcta")]
    WrongPassword,
    #[error("la página {0} no existe")]
    NoSuchPage(usize),
    #[error("la fuente de ese texto no trae estos caracteres: {}", .0.iter().collect::<String>())]
    MissingGlyphs(Vec<char>),
    #[error("no hay un campo llamado «{0}»")]
    NoSuchField(String),
    #[error("este texto no se puede editar: {}", not_editable_reason(.0))]
    NotEditable(String),
    #[error("{0}")]
    Rejected(String),
    #[error("{0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What a `NotEditable` reason code means, in words.
fn not_editable_reason(reason: &str) -> &str {
    match reason {
        "type3" => "su fuente es de tipo Type 3 (los glifos son dibujos, no letras)",
        "vertical_writing" => "está escrito en vertical",
        "symbolic_font" => "su fuente es de símbolos",
        "rtl_text" => "se escribe de derecha a izquierda (árabe, hebreo)",
        other => other,
    }
}

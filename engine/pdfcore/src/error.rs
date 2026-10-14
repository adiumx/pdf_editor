use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no se pudo leer el PDF: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("la página {0} no existe")]
    NoSuchPage(usize),
    #[error("{0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

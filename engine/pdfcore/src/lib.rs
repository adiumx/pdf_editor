//! The PDF engine behind the Flutter editor.
//!
//! Reading comes first: open a document, list its pages, and get each page's
//! text as runs that know their font and where they sit.

mod add;
mod cmap;
mod edit;
mod error;
mod fallback;
mod font;
mod forms;
mod journal;
mod reflow;
mod standard_widths;
mod text;

use lopdf::{Dictionary, Document as Raw, Object, ObjectId};

use journal::{Patch, Tracked};

pub use error::{Error, Result};
pub use font::{Font, FontKind, Glyph};
pub use add::{Added, Align, Anchor, NewText};
pub use edit::{ReplaceOptions, Replaced};
pub use reflow::{Alignment, Reflow, Warning};
pub use forms::{Field, FieldKind, Signature, Widget};
pub use text::{Matrix, TextRun};

/// A page's size as a reader shows it: rotation applied, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
}

/// `/Encrypt` written as a dictionary in the trailer instead of a reference to
/// an object (the PDF spec allows both, MuPDF writes the first); lopdf cannot
/// read that form.
fn has_inline_encrypt(bytes: &[u8]) -> bool {
    let key = b"/Encrypt";
    // The last one belongs to the newest trailer, the one that counts.
    // (`/EncryptMetadata` starts the same way, so the name must end there.)
    let Some(at) = (0..bytes.len().saturating_sub(key.len()))
        .rev()
        .find(|&at| &bytes[at..at + key.len()] == key && !bytes[at + key.len()].is_ascii_alphanumeric())
    else {
        return false;
    };
    bytes[at + key.len()..]
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| *b == b'<')
}

pub struct Document {
    pdf: Tracked,
    pages: Vec<ObjectId>,
    /// Fallback fonts already written into this document, by style.
    fallbacks: std::collections::HashMap<&'static str, fallback::Embedded>,
    /// Pages whose own content is already wrapped in `q`/`Q` for added text.
    wrapped: std::collections::HashSet<ObjectId>,
    undo: Vec<Patch>,
    redo: Vec<Patch>,
}

/// How many steps can be undone.
const UNDO_LIMIT: usize = 200;

impl Document {
    /// Open a document. One protected only against editing or printing (an
    /// owner password, with an empty user password) opens like any other; one
    /// that needs a password to be read fails with `Error::PasswordRequired`.
    pub fn open(bytes: &[u8]) -> Result<Document> {
        Self::open_with_password(bytes, "")
    }

    /// Open a document protected by `password`. Saving writes it unprotected.
    pub fn open_with_password(bytes: &[u8], password: &str) -> Result<Document> {
        let pdf = match Raw::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(password)) {
            Ok(pdf) => pdf,
            Err(lopdf::Error::InvalidPassword) if has_inline_encrypt(bytes) => {
                return Err(Error::Unsupported(
                    "este PDF protegido guarda su cifrado de una forma que el motor no lee todavía".into(),
                ))
            }
            Err(lopdf::Error::InvalidPassword) if password.is_empty() => return Err(Error::PasswordRequired),
            Err(lopdf::Error::InvalidPassword) => return Err(Error::WrongPassword),
            Err(other) => return Err(other.into()),
        };
        let pages = pdf.get_pages().values().copied().collect();
        Ok(Document {
            pdf: Tracked::new(pdf),
            pages,
            fallbacks: Default::default(),
            wrapped: Default::default(),
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_id(&self, page: usize) -> Result<ObjectId> {
        self.pages.get(page).copied().ok_or(Error::NoSuchPage(page))
    }

    pub fn page_size(&self, page: usize) -> Result<PageSize> {
        let id = self.page_id(page)?;
        let boxed = self.page_box(id);
        let (w, h) = (boxed[2] - boxed[0], boxed[3] - boxed[1]);
        let rotation = self
            .inherited(id, b"Rotate")
            .and_then(|o| o.as_i64().ok())
            .map(|r| (r.rem_euclid(360)) as i32)
            .unwrap_or(0);
        let (width, height) = if rotation % 180 == 0 { (w, h) } else { (h, w) };
        Ok(PageSize { width, height, rotation })
    }

    fn page_box(&self, id: ObjectId) -> [f32; 4] {
        self.inherited(id, b"CropBox")
            .or_else(|| self.inherited(id, b"MediaBox"))
            .and_then(|o| o.as_array().ok().cloned())
            .map(|a| a.iter().map(|o| font::num(&self.pdf, o).unwrap_or(0.0)).collect::<Vec<f32>>())
            .filter(|v| v.len() == 4)
            .map(|v| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
            .unwrap_or([0.0, 0.0, 612.0, 792.0])
    }

    /// Maps default user space (origin bottom-left of the page box, y up) to
    /// what a reader shows: page rotation applied, origin top-left, y down.
    pub fn display_matrix(&self, page: usize) -> Result<Matrix> {
        let id = self.page_id(page)?;
        let [x0, y0, x1, y1] = self.page_box(id);
        let rotation = self.page_size(page)?.rotation;
        Ok(match rotation {
            90 => [0.0, 1.0, 1.0, 0.0, -y0, -x0],
            180 => [-1.0, 0.0, 0.0, 1.0, x1, -y0],
            270 => [0.0, -1.0, -1.0, 0.0, y1, x1],
            _ => [1.0, 0.0, 0.0, -1.0, -x0, y1],
        })
    }

    /// A point of default user space as a reader shows it (top-left origin).
    pub fn to_display_point(&self, page: usize, point: (f32, f32)) -> Result<(f32, f32)> {
        Ok(text::apply(self.display_matrix(page)?, point.0, point.1))
    }

    /// Run one edit as a single undoable step: if it fails nothing is changed,
    /// if it succeeds it can be undone as a whole.
    pub(crate) fn transaction<T>(&mut self, edit: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.pdf.begin();
        match edit(self) {
            Ok(value) => {
                self.undo.push(self.pdf.commit());
                if self.undo.len() > UNDO_LIMIT {
                    self.undo.remove(0);
                }
                self.redo.clear();
                Ok(value)
            }
            Err(error) => {
                self.pdf.rollback();
                self.forget_caches();
                Err(error)
            }
        }
    }

    /// What the engine remembers about fonts it wrote and pages it wrapped can
    /// no longer be trusted once objects come or go; it is rebuilt from the file.
    fn forget_caches(&mut self) {
        self.fallbacks.clear();
        self.wrapped.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Take back the last edit. Page runs read before it are stale afterwards.
    pub fn undo(&mut self) -> bool {
        let Some(patch) = self.undo.pop() else { return false };
        let inverse = self.pdf.apply(patch);
        self.redo.push(inverse);
        self.forget_caches();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(patch) = self.redo.pop() else { return false };
        let inverse = self.pdf.apply(patch);
        self.undo.push(inverse);
        self.forget_caches();
        true
    }

    /// Make the bundled font of this style available to write with: the copy
    /// an earlier session left in the file, or a fresh one.
    pub(crate) fn ensure_fallback(&mut self, key: &'static str, data: &'static [u8]) -> Result<()> {
        if !self.fallbacks.contains_key(key) {
            let embedded = match fallback::Embedded::adopt(&self.pdf, key, data) {
                Some(found) => found,
                None => fallback::Embedded::embed(&mut self.pdf, key, data)?,
            };
            self.fallbacks.insert(key, embedded);
        }
        Ok(())
    }

    /// A page attribute, looked up the page tree the way the spec inherits it.
    fn inherited(&self, mut id: ObjectId, key: &[u8]) -> Option<Object> {
        for _ in 0..32 {
            let dict = self.pdf.get_dictionary(id).ok()?;
            if let Ok(value) = dict.get(key) {
                return Some(font::resolve(&self.pdf, value).clone());
            }
            id = dict.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    fn resources(&self, id: ObjectId) -> Dictionary {
        self.inherited(id, b"Resources")
            .and_then(|o| o.as_dict().ok().cloned())
            .unwrap_or_default()
    }

    /// Every string the page shows, with the font and position it is drawn at.
    pub fn text_runs(&self, page: usize) -> Result<Vec<TextRun>> {
        let id = self.page_id(page)?;
        let content = self.pdf.get_page_content(id);
        let to_display = self.display_matrix(page)?;
        let mut runs = text::run_page_text(&self.pdf, &content, &self.resources(id));
        for run in &mut runs {
            run.display = text::transform_box(to_display, run.bbox);
        }
        Ok(runs)
    }
}

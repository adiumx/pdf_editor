//! The PDF engine behind the Flutter editor.
//!
//! Reading comes first: open a document, list its pages, and get each page's
//! text as runs that know their font and where they sit.

mod cmap;
mod error;
mod font;
mod standard_widths;
mod text;

use lopdf::{Dictionary, Document as Pdf, Object, ObjectId};

pub use error::{Error, Result};
pub use font::{Font, FontKind, Glyph};
pub use text::{Matrix, TextRun};

/// A page's size as a reader shows it: rotation applied, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
}

pub struct Document {
    pdf: Pdf,
    pages: Vec<ObjectId>,
}

impl Document {
    pub fn open(bytes: &[u8]) -> Result<Document> {
        let pdf = Pdf::load_mem(bytes)?;
        let pages = pdf.get_pages().values().copied().collect();
        Ok(Document { pdf, pages })
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

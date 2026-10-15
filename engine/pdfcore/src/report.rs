//! What kind of page is this: real text, a picture of text, a picture with a text layer, or blank.
//! The engine does not recognise characters (no OCR engine is bundled); the report says which
//! pages the app should offer to run one on.

use crate::error::Result;
use crate::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    /// Text the document draws itself: searchable, copyable, editable.
    Native,
    /// A picture of a page and nothing readable: needs OCR to be searched or edited.
    ScannedNoText,
    /// A picture of a page with an invisible text layer on top (already run through OCR).
    ScannedWithTextLayer,
    /// Pictures and little or no text, not covering the page.
    ImageOnly,
    /// Nothing drawn.
    Blank,
}

impl PageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PageKind::Native => "native",
            PageKind::ScannedNoText => "scanned_no_text",
            PageKind::ScannedWithTextLayer => "scanned_with_text_layer",
            PageKind::ImageOnly => "image_only",
            PageKind::Blank => "blank",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageReport {
    pub page: usize,
    pub kind: PageKind,
    /// Letters drawn so that a reader sees them, and letters of the invisible text layer.
    pub visible_chars: usize,
    pub invisible_chars: usize,
    pub images: usize,
    /// Part of the page the pictures cover, 0..=1 (overlaps counted once).
    pub image_coverage: f32,
    /// Running OCR on this page would make it searchable.
    pub needs_ocr: bool,
}

impl Document {
    /// One entry per page: what it holds and whether it needs OCR.
    pub fn ocr_report(&self) -> Result<Vec<PageReport>> {
        let mut out = Vec::new();
        for page in 0..self.page_count() {
            let size = self.page_size(page)?;
            let runs = self.text_runs(page)?;
            let count = |invisible: bool| -> usize {
                runs.iter().filter(|r| (r.render_mode == 3) == invisible).map(|r| r.text.chars().filter(|c| !c.is_whitespace()).count()).sum()
            };
            let (visible_chars, invisible_chars) = (count(false), count(true));
            let pictures = self.page_pictures(page)?;
            // Union of the pictures on a coarse grid: scans are one big picture or a few strips.
            const N: usize = 64;
            let mut covered = vec![false; N * N];
            for p in &pictures {
                let [x0, y0, x1, y1] = p.display;
                let (cx0, cx1) = (((x0 / size.width) * N as f32).floor().max(0.0) as usize, ((x1 / size.width) * N as f32).ceil().min(N as f32) as usize);
                let (cy0, cy1) = (((y0 / size.height) * N as f32).floor().max(0.0) as usize, ((y1 / size.height) * N as f32).ceil().min(N as f32) as usize);
                for y in cy0..cy1 {
                    for x in cx0..cx1 {
                        covered[y * N + x] = true;
                    }
                }
            }
            let image_coverage = covered.iter().filter(|c| **c).count() as f32 / (N * N) as f32;
            let kind = if visible_chars >= 20 {
                PageKind::Native
            } else if image_coverage >= 0.6 {
                if invisible_chars >= 20 { PageKind::ScannedWithTextLayer } else { PageKind::ScannedNoText }
            } else if !pictures.is_empty() {
                PageKind::ImageOnly
            } else if visible_chars + invisible_chars == 0 {
                PageKind::Blank
            } else {
                PageKind::Native
            };
            out.push(PageReport {
                page,
                kind,
                visible_chars,
                invisible_chars,
                images: pictures.len(),
                image_coverage,
                needs_ocr: matches!(kind, PageKind::ScannedNoText),
            });
        }
        Ok(out)
    }
}

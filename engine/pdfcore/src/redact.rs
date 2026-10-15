//! Redaction: take the text under some boxes out of the file (not just cover it), cover the
//! place, and report what was and was not removed.
//!
//! The glyphs whose centre lies in a box are removed from the string that draws them, and the
//! gap they leave is kept with a `TJ` number of exactly their advance, so every other letter
//! stays where it was. Annotations and form fields under a box are removed too. Pictures are
//! removed whole on request; otherwise they stay (covered) and the report says so: a picture's
//! pixels are not edited.
//!
//! Not done, and said: text drawn inside a form XObject (the redaction is refused, so nothing is
//! left readable by mistake), the document's metadata, and an earlier revision kept by an
//! incremental save of the original file.

use lopdf::content::{Content, Operation};
use lopdf::Object;

use crate::error::{Error, Result};
use crate::items::ItemKind;
use crate::text::{apply, invert, TextRun};
use crate::Document;

#[derive(Debug, Clone, PartialEq)]
pub struct RedactOptions {
    /// Colour of the cover drawn over each box, red, green, blue in 0..=1.
    pub fill: [f32; 3],
    /// Remove the pictures that touch a box (whole). Otherwise they are only covered.
    pub remove_images: bool,
}

impl Default for RedactOptions {
    fn default() -> Self {
        RedactOptions { fill: [0.0, 0.0, 0.0], remove_images: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Redacted {
    /// Letters removed from the file.
    pub chars: usize,
    /// Strings that lost some.
    pub runs: usize,
    pub images_removed: usize,
    /// Pictures that touch a box and were kept: their data is still in the file.
    pub images_kept: usize,
    /// Annotations and fields taken off the page.
    pub annotations_removed: usize,
}

fn inside(b: [f32; 4], x: f32, y: f32) -> bool {
    x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3]
}

fn touches(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

impl Document {
    /// Redact `boxes` (`[x0, y0, x1, y1]` as a reader shows the page). One step.
    pub fn redact(&mut self, page: usize, boxes: &[[f32; 4]], options: &RedactOptions) -> Result<Redacted> {
        if boxes.is_empty() {
            return Err(Error::Rejected("hace falta al menos una zona".into()));
        }
        self.transaction(|d| d.redact_page(page, boxes, options))
    }

    fn redact_page(&mut self, page: usize, boxes: &[[f32; 4]], options: &RedactOptions) -> Result<Redacted> {
        let id = self.page_id(page)?;
        let to_display = self.display_matrix(page)?;
        let mut report = Redacted::default();

        // Text first: which glyph of which string falls in a box.
        let runs = self.text_runs(page)?;
        let mut jobs: Vec<(&TextRun, Vec<bool>)> = Vec::new();
        for run in &runs {
            if !boxes.iter().any(|b| touches(run.display, *b)) {
                continue;
            }
            let spans = self.glyph_spans(page, run, to_display)?;
            let gone: Vec<bool> = spans.iter().map(|s| boxes.iter().any(|b| inside(*b, (s[0] + s[2]) / 2.0, (s[1] + s[3]) / 2.0))).collect();
            if gone.iter().any(|g| *g) {
                if run.xobject.is_some() {
                    return Err(Error::Unsupported("hay texto dentro de un formulario XObject en esa zona: no se puede censurar sin dejarlo legible".into()));
                }
                if run.not_editable == Some("vertical_writing") {
                    return Err(Error::Unsupported("hay texto en vertical en esa zona: no se puede censurar".into()));
                }
                jobs.push((run, gone));
            }
        }
        if !jobs.is_empty() {
            let fonts = crate::text::fonts_of(&self.pdf, &self.resources(id));
            let mut content = Content::decode(&self.pdf.get_page_content(id))?;
            for (run, gone) in &jobs {
                let font = fonts.get(&run.font).ok_or_else(|| Error::Unsupported(format!("no se encontró la fuente {}", run.font)))?;
                let original = content.operations.get(run.op_index).ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó".into()))?.clone();
                if !matches!(original.operator.as_str(), "Tj" | "TJ") {
                    return Err(Error::Unsupported(format!("no se puede censurar texto dibujado con la operación {}", original.operator)));
                }
                let items: Vec<Object> = match original.operator.as_str() {
                    "TJ" => original.operands.first().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default(),
                    _ => original.operands.clone(),
                };
                let mut out: Vec<Object> = Vec::new();
                let mut bytes: Vec<u8> = Vec::new();
                let mut gap = 0.0f32;
                let mut glyph_index = 0usize;
                let size = run.tf_size.max(0.01);
                let flush_gap = |out: &mut Vec<Object>, gap: &mut f32| {
                    if gap.abs() > 1e-6 {
                        out.push(Object::Real(*gap));
                        *gap = 0.0;
                    }
                };
                for item in &items {
                    match item {
                        Object::String(s, _) => {
                            for glyph in font.decode(s) {
                                let removed = gone.get(glyph_index).copied().unwrap_or(false);
                                glyph_index += 1;
                                if removed {
                                    if !bytes.is_empty() {
                                        out.push(crate::fallback::Embedded::hex(std::mem::take(&mut bytes)));
                                    }
                                    let spacing = run.char_space + if glyph.is_space { run.word_space } else { 0.0 };
                                    gap -= glyph.width + spacing * 1000.0 / size;
                                    report.chars += glyph.text.chars().count().max(1);
                                } else {
                                    flush_gap(&mut out, &mut gap);
                                    bytes.extend((0..font.code_bytes().max(1)).rev().map(|k| (glyph.code >> (8 * k)) as u8));
                                }
                            }
                        }
                        other => {
                            if let Ok(n) = other.as_float() {
                                if !bytes.is_empty() {
                                    out.push(crate::fallback::Embedded::hex(std::mem::take(&mut bytes)));
                                }
                                gap += n;
                            }
                        }
                    }
                }
                if !bytes.is_empty() {
                    out.push(crate::fallback::Embedded::hex(std::mem::take(&mut bytes)));
                }
                flush_gap(&mut out, &mut gap);
                content.operations[run.op_index] = Operation::new("TJ", vec![Object::Array(out)]);
                report.runs += 1;
            }
            self.set_page_content(id, &content)?;
        }

        // Pictures, annotations and fields under a box.
        let items = self.page_items(page)?;
        let mut doomed = Vec::new();
        for item in items {
            if !boxes.iter().any(|b| touches(item.display, *b)) {
                continue;
            }
            match item.kind {
                ItemKind::Image if options.remove_images => {
                    report.images_removed += 1;
                    doomed.push(item);
                }
                ItemKind::Image => report.images_kept += 1,
                ItemKind::Field => {
                    report.annotations_removed += 1;
                    doomed.push(item);
                }
                ItemKind::Text => {}
            }
        }
        let doomed_pictures: Vec<_> = doomed.iter().filter(|i| i.kind == ItemKind::Image).cloned().collect();
        let doomed_fields: Vec<_> = doomed.iter().filter(|i| i.kind == ItemKind::Field).cloned().collect();
        self.remove_items(page, &doomed_pictures)?;
        self.remove_items(page, &doomed_fields)?;
        for note in self.annotations(page)?.into_iter().rev() {
            if note.kind != crate::annots::AnnotKind::Widget && boxes.iter().any(|b| touches(note.display, *b)) {
                self.delete_annotation(page, note.index)?;
                report.annotations_removed += 1;
            }
        }

        // The cover.
        let to_user = invert(to_display).ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        let mut ops = vec![
            Operation::new("q", vec![]),
            Operation::new("rg", options.fill.iter().map(|v| Object::Real(*v)).collect()),
        ];
        for b in boxes {
            let (p, q) = (apply(to_user, b[0], b[1]), apply(to_user, b[2], b[3]));
            let (x, y) = (p.0.min(q.0), p.1.min(q.1));
            ops.push(Operation::new("re", [x, y, (p.0 - q.0).abs(), (p.1 - q.1).abs()].iter().map(|v| Object::Real(*v)).collect()));
            ops.push(Operation::new("f", vec![]));
        }
        ops.push(Operation::new("Q", vec![]));
        self.append_to_page(id, &Content { operations: ops })?;

        // Nothing under a box may still be readable.
        for left in self.text_runs(page)? {
            if left.xobject.is_some() || left.text.trim().is_empty() {
                continue;
            }
            let spans = self.glyph_spans(page, &left, to_display)?;
            if spans.iter().any(|s| boxes.iter().any(|b| inside(*b, (s[0] + s[2]) / 2.0, (s[1] + s[3]) / 2.0))) {
                return Err(Error::Unsupported("quedó texto legible bajo la zona: se deshizo la censura".into()));
            }
        }
        Ok(report)
    }

    /// The box each glyph of a run takes, as a reader shows the page. The run's advance is shared
    /// out exactly: every glyph's advance and every `TJ` number counts.
    fn glyph_spans(&self, _page: usize, run: &TextRun, to_display: crate::text::Matrix) -> Result<Vec<[f32; 4]>> {
        let size = run.tf_size.max(0.01);
        // Pieces of the string with their advance in text-space units, in order.
        let id = self.page_id(_page)?;
        let content = Content::decode(&self.pdf.get_page_content(id))?;
        let op = content.operations.get(run.op_index);
        let mut advances: Vec<f32> = Vec::new();
        let mut shifts_before: Vec<f32> = Vec::new();
        let mut pending = 0.0f32;
        let items: Vec<Object> = match op {
            Some(o) if o.operator == "TJ" => o.operands.first().and_then(|x| x.as_array().ok()).cloned().unwrap_or_default(),
            Some(o) => o.operands.clone(),
            None => Vec::new(),
        };
        let fonts = crate::text::fonts_of(&self.pdf, &self.resources(id));
        let font = fonts.get(&run.font);
        for item in &items {
            match item {
                Object::String(s, _) => {
                    if let Some(font) = font {
                        for g in font.decode(s) {
                            let spacing = run.char_space + if g.is_space { run.word_space } else { 0.0 };
                            shifts_before.push(pending);
                            pending = 0.0;
                            advances.push((g.width / 1000.0 * size + spacing) * run.hscale);
                        }
                    }
                }
                other => {
                    if let Ok(n) = other.as_float() {
                        pending += -n / 1000.0 * size * run.hscale;
                    }
                }
            }
        }
        if advances.is_empty() {
            return Ok(Vec::new());
        }
        // Offsets along the run, then scaled so that they end exactly where the run ends.
        let mut offsets = Vec::with_capacity(advances.len() + 1);
        let mut at = 0.0f32;
        for (a, s) in advances.iter().zip(&shifts_before) {
            at += s;
            offsets.push((at, at + a));
            at += a;
        }
        let total = (at + pending).max(1e-6);
        let (dx, dy) = (run.end.0 - run.origin.0, run.end.1 - run.origin.1);
        let [_, y0, _, y1] = run.display;
        let spans = offsets
            .into_iter()
            .map(|(a, b)| {
                let p = apply(to_display, run.origin.0 + dx * a / total, run.origin.1 + dy * a / total);
                let q = apply(to_display, run.origin.0 + dx * b / total, run.origin.1 + dy * b / total);
                [p.0.min(q.0), y0, p.0.max(q.0), y1]
            })
            .collect();
        Ok(spans)
    }
}

//! Changing what a page says without changing how it looks.
//!
//! An edit swaps the operand of the one drawing operation that showed a run,
//! in place: the operation keeps its place in the stream, so every other run's
//! `op_index` stays valid and several edits can be made one after another from
//! a single reading of the page.

use lopdf::content::Content;
use lopdf::{Object, Stream, StringFormat};

use crate::error::{Error, Result};
use crate::text::{fonts_of, TextRun};
use crate::Document;

/// What an edit did to the line's length, so a caller can decide what to do
/// about whatever follows on the same line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Replaced {
    /// Advance of the old string, in points.
    pub old_width: f32,
    /// Advance of the new one.
    pub new_width: f32,
}

impl Document {
    /// Draw `new_text` where `run` was, with the font it already used.
    ///
    /// Fails with [`Error::MissingGlyphs`] when the font — typically a subset
    /// that kept only the glyphs the original text used — has no glyph for
    /// some character, and with nothing changed.
    pub fn replace_text(&mut self, page: usize, run: &TextRun, new_text: &str) -> Result<Replaced> {
        if run.xobject.is_some() {
            return Err(Error::Unsupported(
                "editar texto dentro de un formulario XObject aún no está soportado".into(),
            ));
        }
        let id = self.page_id(page)?;
        let resources = self.resources(id);
        let fonts = fonts_of(&self.pdf, &resources);
        let font = fonts
            .get(&run.font)
            .ok_or_else(|| Error::Unsupported(format!("no se encontró la fuente {}", run.font)))?;

        let bytes = font.encode(new_text).map_err(Error::MissingGlyphs)?;
        let old: Vec<u8> = run
            .glyphs
            .iter()
            .flat_map(|g| (0..font.code_bytes()).rev().map(move |b| (g.code >> (8 * b)) as u8))
            .collect();
        let replaced = Replaced {
            old_width: font.advance_of(&old, run.size),
            new_width: font.advance_of(&bytes, run.size),
        };

        let mut content = Content::decode(&self.pdf.get_page_content(id))?;
        let op = content
            .operations
            .get_mut(run.op_index)
            .ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó".into()))?;
        let shown = Object::String(bytes, StringFormat::Hexadecimal);
        match op.operator.as_str() {
            "Tj" | "'" => op.operands = vec![shown],
            "\"" => {
                op.operands.truncate(2);
                op.operands.push(shown);
            }
            // Kerning goes: the new string has different letters to kern.
            "TJ" => op.operands = vec![Object::Array(vec![shown])],
            other => return Err(Error::Unsupported(format!("operación inesperada: {other}"))),
        }
        self.set_page_content(id, &content)?;
        Ok(replaced)
    }

    fn set_page_content(&mut self, page: lopdf::ObjectId, content: &Content) -> Result<()> {
        let mut stream = Stream::new(lopdf::Dictionary::new(), content.encode()?);
        let _ = stream.compress();
        let stream_id = self.pdf.add_object(stream);
        self.pdf
            .get_dictionary_mut(page)?
            .set("Contents", Object::Reference(stream_id));
        Ok(())
    }

    /// The document as a PDF file, with the replaced content streams gone.
    pub fn save_to_bytes(&mut self) -> Result<Vec<u8>> {
        self.pdf.prune_objects();
        let mut out = Vec::new();
        self.pdf.save_to(&mut out).map_err(lopdf::Error::from)?;
        Ok(out)
    }
}

//! Changing what a page says without changing how it looks.
//!
//! An edit swaps the operand of the one drawing operation that showed a run.
//! When the document's font can draw every character that is all it does, in
//! place: the operation keeps its place in the stream, so every other run's
//! `op_index` stays valid and several edits can follow from one reading.
//!
//! Characters the font cannot draw (a subset keeps only what its document
//! used) can be drawn with a bundled font instead, see
//! [`Document::replace_text_with_fallback`]. That inserts font switches, so the
//! page has to be read again afterwards; [`Replaced::reread`] says so.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, Stream};

use crate::error::{Error, Result};
use crate::fallback::{self, Embedded};
use crate::reflow::{Reflow, Warning};
use crate::text::{fonts_of, TextRun};
use crate::Document;

/// What an edit did, so a caller can decide what to do next.
#[derive(Debug, Clone, PartialEq)]
pub struct Replaced {
    /// Advance of the old string, in points.
    pub old_width: f32,
    /// Advance of the new one, in points. A different length does not move
    /// whatever follows on the same line.
    pub new_width: f32,
    /// Characters drawn with the bundled font because the document's own could
    /// not. Empty when the original font did it all; worth telling the person.
    pub fallback_chars: Vec<char>,
    /// The page's operations were renumbered: read its runs again before the
    /// next edit on it.
    pub reread: bool,
    /// How many runs after (or before) the edited one were moved so the line
    /// still reads, see [`Reflow`].
    pub moved: usize,
    /// Things worth telling the person: the line now runs past the margin, a
    /// run that could not be moved, a line that looks centred...
    pub warnings: Vec<Warning>,
}

/// What an edit may do besides replacing the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaceOptions {
    /// Draw characters the document's font lacks with a bundled one.
    pub allow_fallback: bool,
    /// What happens to the rest of the line when the length changes.
    pub reflow: Reflow,
}

impl Default for ReplaceOptions {
    fn default() -> Self {
        ReplaceOptions { allow_fallback: false, reflow: Reflow::Following }
    }
}

impl Document {
    /// Draw `new_text` where `run` was, with the font it already used.
    ///
    /// Fails with [`Error::MissingGlyphs`] when that font has no glyph for some
    /// character, and with nothing changed.
    pub fn replace_text(&mut self, page: usize, run: &TextRun, new_text: &str) -> Result<Replaced> {
        self.replace_text_with(page, run, new_text, ReplaceOptions::default())
    }

    /// As [`replace_text`](Self::replace_text), but characters the original
    /// font lacks are drawn with a bundled font of the closest style. Fails
    /// only for characters neither font has, again with nothing changed.
    pub fn replace_text_with_fallback(&mut self, page: usize, run: &TextRun, new_text: &str) -> Result<Replaced> {
        self.replace_text_with(page, run, new_text, ReplaceOptions { allow_fallback: true, ..Default::default() })
    }

    /// The general form: [`replace_text`](Self::replace_text) and
    /// [`replace_text_with_fallback`](Self::replace_text_with_fallback) with the
    /// options spelled out, including [`Reflow`].
    pub fn replace_text_with(&mut self, page: usize, run: &TextRun, new_text: &str, options: ReplaceOptions) -> Result<Replaced> {
        self.transaction(|d| d.replace(page, run, new_text, options))
    }

    fn replace(&mut self, page: usize, run: &TextRun, new_text: &str, options: ReplaceOptions) -> Result<Replaced> {
        let allow_fallback = options.allow_fallback;
        if run.xobject.is_some() {
            return Err(Error::Unsupported(
                "editar texto dentro de un formulario XObject aún no está soportado".into(),
            ));
        }
        let id = self.page_id(page)?;
        let mut fonts = fonts_of(&self.pdf, &self.resources(id));
        let page_runs = self.text_runs(page).ok();
        let font = fonts
            .get_mut(&run.font)
            .ok_or_else(|| Error::Unsupported(format!("no se encontró la fuente {}", run.font)))?;
        // What this font already draws on the page is certainly in the font.
        font.allow_seen(
            run.glyphs
                .iter()
                .map(|g| g.code)
                .chain(page_runs.iter().flatten().filter(|r| r.font == run.font).flat_map(|r| r.glyphs.iter().map(|g| g.code))),
        );
        let font = &*font;

        let segments = font.segments(new_text);
        let (key, data) = fallback::choose(&run.base_font);

        // Everything that can fail is decided before anything is changed.
        let unfit: Vec<char> = segments.iter().filter(|(_, b)| b.is_none()).flat_map(|(t, _)| t.chars()).collect();
        if !unfit.is_empty() {
            let lacking = if allow_fallback { fallback::lacks(data, &unfit.iter().collect::<String>()) } else { unfit.clone() };
            if !lacking.is_empty() {
                return Err(Error::MissingGlyphs(lacking));
            }
        }

        let old: Vec<u8> = run
            .glyphs
            .iter()
            .flat_map(|g| (0..font.code_bytes()).rev().map(move |b| (g.code >> (8 * b)) as u8))
            .collect();
        let old_width = font.advance_of(&old, run.size);

        let before_edit = if options.reflow != Reflow::None { page_runs } else { None };
        let mut content = Content::decode(&self.pdf.get_page_content(id))?;
        let original = content
            .operations
            .get(run.op_index)
            .ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó".into()))?
            .clone();
        if !matches!(original.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
            return Err(Error::Unsupported(format!("operación inesperada: {}", original.operator)));
        }

        // Build the replacement operations.
        let mut new_width = 0.0;
        let mut ops: Vec<Operation> = Vec::new();
        let mut in_fallback = false;
        let mut used_fallback: Vec<char> = Vec::new();
        let switch = |name: &str, size: f32| {
            Operation::new("Tf", vec![Object::Name(name.as_bytes().to_vec()), Object::Real(size)])
        };
        let first_shows = |ops: &Vec<Operation>| !ops.iter().any(|o| o.operator != "Tf");

        for (piece, bytes) in &segments {
            let (shown, wants_fallback) = match bytes {
                Some(b) => {
                    new_width += font.advance_of(b, run.size);
                    (Embedded::hex(b.clone()), false)
                }
                None => {
                    self.ensure_fallback(key, data)?;
                    let embedded = self.fallbacks.get_mut(key).expect("just ensured");
                    let (b, advance) = embedded.write(&mut self.pdf, piece, run.size)?;
                    new_width += advance;
                    used_fallback.extend(piece.chars());
                    (Embedded::hex(b), true)
                }
            };
            if wants_fallback != in_fallback {
                let name = if wants_fallback { self.fallbacks[key].resource.clone() } else { run.font.clone() };
                ops.push(switch(&name, run.tf_size));
                in_fallback = wants_fallback;
            }
            ops.push(if first_shows(&ops) { show_like(&original, shown) } else { Operation::new("Tj", vec![shown]) });
        }
        if in_fallback {
            ops.push(switch(&run.font, run.tf_size)); // what follows expects the original font
        }
        if ops.iter().all(|o| o.operator == "Tf") {
            return Err(Error::Unsupported("el texto nuevo está vacío".into()));
        }

        let reread = ops.len() != 1;
        if !used_fallback.is_empty() {
            let resource = self.fallbacks[key].resource.clone();
            let font_id = self.fallbacks[key].font_id;
            self.register_font(id, &resource, font_id)?;
        }
        let pieces = ops.len();
        content.operations.splice(run.op_index..=run.op_index, ops);

        // What follows on the line keeps pace with the new length.
        let delta = new_width - old_width;
        let (mut moved, mut corrections, mut warnings) = (0, 0, Vec::new());
        if let (Some(before), true) = (&before_edit, delta.abs() > 0.01 && options.reflow != Reflow::None) {
            let outcome = self.reflow_line(page, id, before, run.op_index, pieces, delta, options.reflow, &mut content)?;
            (moved, corrections, warnings) = (outcome.moved, outcome.corrections, outcome.warnings);
        }
        self.set_page_content(id, &content)?;
        Ok(Replaced {
            old_width,
            new_width,
            fallback_chars: used_fallback,
            reread: reread || corrections > 0,
            moved,
            warnings,
        })
    }

    /// Make a font usable by name in one page's content.
    pub(crate) fn register_font(&mut self, page: lopdf::ObjectId, resource: &str, font: lopdf::ObjectId) -> Result<()> {
        let mut resources = self.resources(page);
        let mut fonts = resources
            .get(b"Font")
            .ok()
            .map(|o| crate::font::resolve(&self.pdf, o))
            .and_then(|o| o.as_dict().ok().cloned())
            .unwrap_or_default();
        fonts.set(resource, Object::Reference(font));
        resources.set("Font", Object::Dictionary(fonts));
        self.pdf.get_dictionary_mut(page)?.set("Resources", Object::Dictionary(resources));
        Ok(())
    }

    pub(crate) fn set_page_content(&mut self, page: lopdf::ObjectId, content: &Content) -> Result<()> {
        let mut stream = Stream::new(Dictionary::new(), content.encode()?);
        let _ = stream.compress();
        let stream_id = self.pdf.add_object(stream);
        self.pdf.get_dictionary_mut(page)?.set("Contents", Object::Reference(stream_id));
        Ok(())
    }

    /// The document as a PDF file, without the objects nothing points to.
    pub fn save_to_bytes(&mut self) -> Result<Vec<u8>> {
        self.pdf.to_bytes()
    }
}

/// The operation that shows `string` the way `original` did (`'` and `"` also
/// start a new line; `TJ` loses its kerning, which belonged to the old letters).
fn show_like(original: &Operation, string: Object) -> Operation {
    match original.operator.as_str() {
        "TJ" => Operation::new("TJ", vec![Object::Array(vec![string])]),
        "\"" => {
            let mut operands: Vec<Object> = original.operands.iter().take(2).cloned().collect();
            operands.push(string);
            Operation::new("\"", operands)
        }
        other => Operation::new(other, vec![string]),
    }
}

//! Deleting text strings for real: the letters leave the file, and nothing around them moves.
//!
//! A string is drawn by one show operation (`Tj`, `TJ`, `'`, `"`). Emptying it would draw nothing,
//! but the pen would no longer advance past it, and text drawn after it in the same text object
//! without its own position (a `Tj` that follows on) would slide left. So the string becomes a
//! `TJ` with a single number that moves the pen exactly as far as the letters did; `'` and `"` keep
//! their move to the next line (and `"` its spacing). Nothing is drawn, nothing else moves.

use lopdf::content::{Content, Operation};
use lopdf::Object;

use crate::error::{Error, Result};
use crate::text::{fonts_of, TextRun};
use crate::Document;

/// What [`Document::delete_runs`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeletedRuns {
    /// Strings taken off the page.
    pub runs: usize,
    /// Letters they had.
    pub chars: usize,
}

impl Document {
    /// Delete the strings `run_ids` (indexes into [`text_runs`](Self::text_runs) of `page` as it is
    /// now) as one step for undo. Text inside a form XObject or written top to bottom is refused
    /// (`Unsupported`) and an id the page does not have is `StaleRun`; either way nothing changes.
    pub fn delete_runs(&mut self, page: usize, run_ids: &[usize]) -> Result<DeletedRuns> {
        if run_ids.is_empty() {
            return Ok(DeletedRuns::default());
        }
        self.transaction(|d| d.remove_runs(page, run_ids))
    }

    fn remove_runs(&mut self, page: usize, run_ids: &[usize]) -> Result<DeletedRuns> {
        let id = self.page_id(page)?;
        let runs = self.text_runs(page)?;
        let mut chosen: Vec<&TextRun> = Vec::new();
        for &i in run_ids {
            let run = runs.get(i).ok_or_else(|| Error::StaleRun(format!("la página {page} no tiene el texto {i}")))?;
            if run.xobject.is_some() {
                return Err(Error::Unsupported("ese texto está dentro de un formulario XObject: no se puede borrar desde la página".into()));
            }
            if run.not_editable == Some("vertical_writing") {
                return Err(Error::Unsupported("ese texto está escrito en vertical: no se puede borrar".into()));
            }
            if !chosen.iter().any(|c| c.op_index == run.op_index) {
                chosen.push(run);
            }
        }
        let fonts = fonts_of(&self.pdf, &self.resources(id));
        let mut content = Content::decode(&self.pdf.get_page_content(id))?;
        // From the last to the first: `'` and `"` become several operations, which renumbers what follows.
        chosen.sort_by_key(|r| std::cmp::Reverse(r.op_index));
        let mut done = DeletedRuns::default();
        for run in chosen {
            let font = fonts.get(&run.font).ok_or_else(|| Error::Unsupported(format!("no se encontró la fuente {}", run.font)))?;
            let original = content
                .operations
                .get(run.op_index)
                .ok_or_else(|| Error::StaleRun("el texto ya no está donde se leyó: vuelve a leer la página".into()))?
                .clone();
            let size = run.tf_size.abs().max(1e-3);
            // How far the pen moved over the string, in thousandths of the font size (TJ's unit).
            let mut advance = 0.0f32;
            let mut chars = 0usize;
            let mut measure = |item: &Object| match item {
                Object::String(bytes, _) => {
                    for g in font.decode(bytes) {
                        let spacing = run.char_space + if g.is_space { run.word_space } else { 0.0 };
                        advance += g.width + spacing * 1000.0 / size;
                        chars += g.text.chars().count().max(1);
                    }
                }
                other => {
                    if let Ok(n) = other.as_float() {
                        advance -= n;
                    }
                }
            };
            let gap = |advance: f32| Operation::new("TJ", vec![Object::Array(if advance.abs() > 1e-6 { vec![Object::Real(-advance)] } else { Vec::new() })]);
            let replacement: Vec<Operation> = match original.operator.as_str() {
                "Tj" => {
                    original.operands.iter().take(1).for_each(&mut measure);
                    vec![gap(advance)]
                }
                "TJ" => {
                    if let Some(Object::Array(items)) = original.operands.first() {
                        items.iter().for_each(&mut measure);
                    }
                    vec![gap(advance)]
                }
                "'" => {
                    original.operands.iter().take(1).for_each(&mut measure);
                    vec![Operation::new("T*", vec![]), gap(advance)]
                }
                "\"" => {
                    // `aw ac string "`: word spacing, character spacing, next line, show.
                    original.operands.get(2).into_iter().for_each(&mut measure);
                    let (aw, ac) = (original.operands.first().cloned().unwrap_or(Object::Real(0.0)), original.operands.get(1).cloned().unwrap_or(Object::Real(0.0)));
                    vec![Operation::new("Tw", vec![aw]), Operation::new("Tc", vec![ac]), Operation::new("T*", vec![]), gap(advance)]
                }
                other => return Err(Error::Unsupported(format!("operación inesperada: {other}"))),
            };
            content.operations.splice(run.op_index..=run.op_index, replacement);
            done.runs += 1;
            done.chars += chars;
        }
        self.set_page_content(id, &content)?;
        Ok(done)
    }
}

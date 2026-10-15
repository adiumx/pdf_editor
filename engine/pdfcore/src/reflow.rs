//! Moving the rest of a line when an edit changes the length of one of its runs.
//!
//! Every change of style inside a line is a run of its own, and nothing ties a
//! run to the one before it: a producer may place each with its own matrix, let
//! the pen carry on from the last, or give each a relative `Td`. So the width a
//! replacement gained or lost cannot simply be added to what follows. It would
//! move twice the runs that follow the pen on their own.
//!
//! Instead the edit is made first, the page is read again, and each run is
//! compared with where it *should* be (where it was, plus the shift its place on
//! the line calls for). A run that is somewhere else gets a `Td` before its
//! show operation. `Td` moves everything after it, which is what a run on the
//! same line wants and what the next line does not, so the next line is checked
//! the same way and put back. Whatever the producer did, what is on the page at
//! the end is what was asked for.

use lopdf::content::{Content, Operation};
use lopdf::{Object, ObjectId};

use crate::error::Result;
use crate::text::{self, TextRun};
use crate::Document;

/// How far apart two runs may be and still count as one stretch of text: a few
/// spaces. Columns of a table sit further apart than this, and are left alone.
const MAX_GAP_EM: f32 = 1.5;
/// How far off the baseline a run may be and still be on the same line.
const LINE_TOLERANCE_EM: f32 = 0.4;
/// Smaller errors (in points) are not worth a correction.
const EPSILON: f32 = 0.02;
const MAX_CORRECTIONS: usize = 64;

/// What happens to the rest of the line when a replacement changes its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reflow {
    /// Nothing moves: a longer text may overlap what follows.
    None,
    /// What follows on the line moves by the difference. For left-aligned text.
    #[default]
    Following,
    /// The line keeps its centre: what comes before moves back half the
    /// difference and what follows moves forward half of it.
    Centered,
    /// The line keeps its right edge: what comes before (and the edited run)
    /// moves back by the whole difference, what follows stays.
    RightAligned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Centered,
    Right,
}

/// Something about the result a person may want to know.
#[derive(Debug, Clone, PartialEq)]
pub enum Warning {
    /// The line now runs past the right edge of the page by this many points.
    ExceedsPage { by: f32 },
    /// The line now runs past where every other line on the page ends.
    ExceedsMargin { by: f32 },
    /// Text that would have moved is inside a form XObject, which the engine
    /// can read but not edit: it stayed where it was.
    FollowerNotMoved { text: String },
    /// The line looks centred or right-aligned, so moving only what follows may
    /// leave it misaligned; `Reflow::Centered` or `RightAligned` may suit it.
    LineMayBeAligned { alignment: Alignment },
    /// The runs could not be put where they belong; they were left as written.
    ReflowFailed,
    /// A paragraph grew by lines that now overlap text below it, which stayed
    /// where it was.
    OverlapsText { text: String },
    /// A paragraph grew past the bottom of the page by this many points.
    BelowPage { by: f32 },
    /// The text was inside a form XObject used by this many pages: they all show the change.
    SharedForm { pages: usize },
}

pub(crate) struct Outcome {
    pub moved: usize,
    pub corrections: usize,
    pub warnings: Vec<Warning>,
}

fn unit(v: (f32, f32)) -> Option<(f32, f32)> {
    let len = (v.0 * v.0 + v.1 * v.1).sqrt();
    (len > 1e-4).then(|| (v.0 / len, v.1 / len))
}

fn dot(a: (f32, f32), b: (f32, f32)) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

/// The direction a run's baseline runs in, on the page.
fn direction(run: &TextRun) -> (f32, f32) {
    unit((run.end.0 - run.origin.0, run.end.1 - run.origin.1))
        .or_else(|| unit((run.text_linear[0], run.text_linear[1])))
        .unwrap_or((1.0, 0.0))
}

struct Tracked {
    /// Where its show operation is now.
    index: usize,
    desired: (f32, f32),
    on_line: bool,
    /// Its place on the line calls for it to move (as opposed to being watched).
    shifts: bool,
}

impl Document {
    /// The runs of a page as `content` would draw them, before it is saved.
    pub(crate) fn runs_in(&self, page: usize, id: ObjectId, content: &Content) -> Result<Vec<TextRun>> {
        let to_display = self.display_matrix(page)?;
        let bytes = content.encode()?;
        let mut runs = text::run_page_text(&self.pdf, &bytes, &self.resources(id));
        for run in &mut runs {
            run.display = text::transform_box(to_display, run.bbox);
        }
        Ok(runs)
    }

    /// Put the rest of the line where an edit of the run at `r_index` calls
    /// for. `before` is the page as it was read before the edit, `content` the
    /// page after it, `r_pieces` how many operations the run now takes.
    pub(crate) fn reflow_line(
        &self,
        page: usize,
        id: ObjectId,
        before: &[TextRun],
        r_index: usize,
        r_pieces: usize,
        delta: f32,
        mode: Reflow,
        content: &mut Content,
    ) -> Result<Outcome> {
        let mut outcome = Outcome { moved: 0, corrections: 0, warnings: Vec::new() };
        let Some(r) = before.iter().find(|x| x.op_index == r_index && x.xobject.is_none()) else {
            outcome.warnings.push(Warning::ReflowFailed);
            return Ok(outcome);
        };
        let u = direction(r);
        let n = (-u.1, u.0);
        let size = r.size.max(1.0);
        let r_width = dot((r.end.0 - r.origin.0, r.end.1 - r.origin.1), u).max(0.0);

        // Which runs share the line, and on which side of the edited one.
        let mut following: Vec<(f32, f32, &TextRun)> = Vec::new(); // (start, end) along the line
        let mut preceding: Vec<(f32, f32, &TextRun)> = Vec::new();
        for x in before.iter().filter(|x| !std::ptr::eq(*x, r) && !x.glyphs.is_empty()) {
            let rel = (x.origin.0 - r.origin.0, x.origin.1 - r.origin.1);
            if dot(rel, n).abs() > LINE_TOLERANCE_EM * size {
                continue;
            }
            if dot(direction(x), u) < 0.9 {
                continue;
            }
            let start = dot(rel, u);
            let end = start + dot((x.end.0 - x.origin.0, x.end.1 - x.origin.1), u);
            if start >= r_width - 0.5 {
                following.push((start, end, x));
            } else if end <= 0.5 {
                preceding.push((start, end, x));
            }
        }
        following.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut reach = r_width;
        let mut chain_after: Vec<&TextRun> = Vec::new();
        for (start, end, x) in &following {
            if *start - reach > MAX_GAP_EM * size {
                break;
            }
            reach = reach.max(*end);
            chain_after.push(x);
        }
        preceding.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut reach = 0.0f32;
        let mut chain_before: Vec<&TextRun> = Vec::new();
        for (start, end, x) in &preceding {
            if reach - *end > MAX_GAP_EM * size {
                break;
            }
            reach = reach.min(*start);
            chain_before.push(x);
        }

        let (off_before, off_r, off_after) = match mode {
            Reflow::None => return Ok(outcome),
            Reflow::Following => (0.0, 0.0, delta),
            Reflow::Centered => (-delta / 2.0, -delta / 2.0, delta / 2.0),
            Reflow::RightAligned => (-delta, -delta, 0.0),
        };

        // Who would move, and which of them cannot be moved.
        let at = |run: &TextRun, by: f32| (run.origin.0 + u.0 * by, run.origin.1 + u.1 * by);
        let mut items: Vec<Tracked> = Vec::new();
        let mut line_indices = Vec::new();
        for (runs, by) in [(&chain_before, off_before), (&chain_after, off_after)] {
            for x in runs.iter() {
                if x.xobject.is_some() {
                    if by.abs() > EPSILON {
                        outcome.warnings.push(Warning::FollowerNotMoved { text: x.text.trim().to_string() });
                    }
                    continue;
                }
                items.push(Tracked { index: x.op_index, desired: at(x, by), on_line: true, shifts: by.abs() > EPSILON });
                line_indices.push(x.op_index);
            }
        }
        items.push(Tracked { index: r.op_index, desired: at(r, off_r), on_line: true, shifts: off_r.abs() > EPSILON });

        // Anything after the first change that is not on the line is held in
        // place: `Td` moves everything after it, the next line included.
        let first = items.iter().filter(|t| (t.desired.0 - 0.0).is_finite()).map(|t| t.index).min().unwrap_or(r.op_index);
        let moves_something = items.iter().any(|t| {
            before
                .iter()
                .find(|x| x.op_index == t.index && x.xobject.is_none())
                .map(|x| ((t.desired.0 - x.origin.0).powi(2) + (t.desired.1 - x.origin.1).powi(2)).sqrt() > EPSILON)
                .unwrap_or(false)
        });
        if !moves_something {
            return Ok(outcome);
        }
        for x in before.iter().filter(|x| x.xobject.is_none() && x.op_index > first && !line_indices.contains(&x.op_index) && x.op_index != r.op_index) {
            items.push(Tracked { index: x.op_index, desired: x.origin, on_line: false, shifts: false });
        }

        let moved_ids: Vec<usize> = items.iter().filter(|t| t.shifts && t.index != r.op_index).map(|t| t.index).collect();

        // Numbering after the edit: runs after the edited one moved down by the
        // extra operations the edit itself added.
        let grown = r_pieces.saturating_sub(1);
        for t in &mut items {
            if t.index > r.op_index {
                t.index += grown;
            }
        }
        items.sort_by_key(|t| t.index);

        // Put each run where it belongs, in stream order, reading the page again
        // after every correction (one `Td` can fix or break several runs).
        let original = content.operations.clone();
        let mut i = 0;
        while i < items.len() {
            let runs = self.runs_in(page, id, content)?;
            let Some(run) = runs.iter().find(|x| x.op_index == items[i].index && x.xobject.is_none()) else {
                i += 1;
                continue;
            };
            let error = (items[i].desired.0 - run.origin.0, items[i].desired.1 - run.origin.1);
            if (error.0 * error.0 + error.1 * error.1).sqrt() <= EPSILON {
                i += 1;
                continue;
            }
            let [a, b, c, d] = run.text_linear;
            let det = a * d - b * c;
            if det.abs() < 1e-9 {
                content.operations = original;
                outcome.warnings.push(Warning::ReflowFailed);
                return Ok(outcome);
            }
            // The page-space error, as a movement of the text line matrix.
            let (tx, ty) = ((error.0 * d - error.1 * c) / det, (-error.0 * b + error.1 * a) / det);
            if outcome.corrections >= MAX_CORRECTIONS {
                content.operations = original;
                outcome.corrections = 0;
                outcome.warnings.push(Warning::ReflowFailed);
                return Ok(outcome);
            }
            content
                .operations
                .insert(items[i].index, Operation::new("Td", vec![Object::Real(tx), Object::Real(ty)]));
            outcome.corrections += 1;
            for t in &mut items[i..] {
                t.index += 1;
            }
        }
        outcome.moved = moved_ids.len();

        // What the person should know about the result.
        let after = self.runs_in(page, id, content)?;
        let size_page = self.page_size(page)?;
        let boxes = |indices: &mut dyn Iterator<Item = usize>| -> Vec<[f32; 4]> {
            indices.filter_map(|i| after.iter().find(|x| x.op_index == i && x.xobject.is_none()).map(|x| x.display)).collect()
        };
        let r_current = items.iter().find(|t| t.on_line && line_indices.iter().all(|i| *i != t.index) && t.index >= r.op_index).map(|t| t.index);
        let mut live: Vec<usize> = items.iter().filter(|t| t.on_line).map(|t| t.index).collect();
        if let Some(start) = r_current {
            live.extend(start..start + r_pieces);
        }
        let new_right = boxes(&mut live.into_iter()).iter().map(|b| b[2]).fold(f32::MIN, f32::max);
        if new_right > size_page.width + 0.5 {
            outcome.warnings.push(Warning::ExceedsPage { by: new_right - size_page.width });
        } else {
            let on_line_old: Vec<[f32; 4]> = chain_before
                .iter()
                .chain(chain_after.iter())
                .copied()
                .chain(std::iter::once(r))
                .map(|x| x.display)
                .collect();
            let old_right = on_line_old.iter().map(|b| b[2]).fold(f32::MIN, f32::max);
            let others: Vec<&TextRun> = before
                .iter()
                .filter(|x| {
                    !x.glyphs.is_empty()
                        && dot((x.origin.0 - r.origin.0, x.origin.1 - r.origin.1), n).abs() > LINE_TOLERANCE_EM * size
                })
                .collect();
            if others.len() >= 3 {
                let margin = others.iter().map(|x| x.display[2]).fold(f32::MIN, f32::max).max(old_right);
                if new_right > margin + 1.0 {
                    outcome.warnings.push(Warning::ExceedsMargin { by: new_right - margin });
                }
            }
        }
        if mode == Reflow::Following {
            let all: Vec<&TextRun> = before.iter().filter(|x| !x.glyphs.is_empty() && x.render_mode != 3).collect();
            if all.len() >= 4 {
                let (left, right) = (
                    all.iter().map(|x| x.display[0]).fold(f32::MAX, f32::min),
                    all.iter().map(|x| x.display[2]).fold(f32::MIN, f32::max),
                );
                let line: Vec<[f32; 4]> = chain_before.iter().chain(chain_after.iter()).copied().chain(std::iter::once(r)).map(|x| x.display).collect();
                let (lx0, lx1) = (
                    line.iter().map(|b| b[0]).fold(f32::MAX, f32::min),
                    line.iter().map(|b| b[2]).fold(f32::MIN, f32::max),
                );
                if lx0 - left > 4.0 * size && right - lx1 < 1.5 {
                    outcome.warnings.push(Warning::LineMayBeAligned { alignment: Alignment::Right });
                } else if lx0 - left > 4.0 * size && right - lx1 > 4.0 * size && ((lx0 + lx1) / 2.0 - (left + right) / 2.0).abs() <= 1.5 {
                    outcome.warnings.push(Warning::LineMayBeAligned { alignment: Alignment::Centered });
                }
            }
        }
        Ok(outcome)
    }
}

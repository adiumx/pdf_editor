//! Paragraphs: several lines of one font that read as one block of text.
//!
//! Editing one line of a paragraph rarely leaves the paragraph as it was: a
//! longer sentence no longer fits its line and has to wrap, a shorter one leaves
//! the lines below it short. [`Document::replace_paragraph`] takes the new text
//! of the whole paragraph and lays it out again within the width the paragraph
//! already had, using the lines that are there and adding or emptying some at
//! the end.
//!
//! It handles what word processors produce: left-aligned lines of one font and
//! size at an even leading, each drawn by one string. Justified text, first-line
//! indents and lines with mixed fonts are not recognised as paragraphs.

use lopdf::content::{Content, Operation};
use lopdf::{Object, StringFormat};

use crate::edit::ReplaceOptions;
use crate::error::{Error, Result};
use crate::fallback;
use crate::lines::{build_lines, is_upright, Line};
use crate::reflow::{Reflow, Warning};
use crate::text::{fonts_of, TextRun};
use crate::Document;

/// Lines whose left edges differ by less than this (points) share a margin.
const SAME_LEFT: f32 = 1.0;
/// How much the distance between lines may vary within a paragraph.
const LEADING_SLACK: f32 = 0.25;

#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    /// The lines' text joined by single spaces.
    pub text: String,
    /// Indexes (in `Document::text_runs(page)`) of the run of each line, top to bottom.
    pub runs: Vec<usize>,
    /// `[x0, y0, x1, y1]` as a reader shows the page.
    pub display: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphEdit {
    /// Lines the paragraph has now.
    pub lines: usize,
    /// Lines added after the old last one.
    pub added: usize,
    /// Lines at the end emptied because the text got shorter.
    pub removed: usize,
    /// Letters drawn with the bundled font because the document's font lacks them.
    pub fallback_chars: Vec<char>,
    pub warnings: Vec<Warning>,
}

/// A line the paragraph logic can work with: one visible, upright string.
fn plain_line(runs: &[TextRun], line: &Line) -> Option<usize> {
    let [only] = line.runs[..] else { return None };
    let run = &runs[only];
    (run.xobject.is_none() && run.render_mode != 3 && is_upright(run)).then_some(only)
}

fn baseline(run: &TextRun) -> f32 {
    run.display[3]
}

/// Indexes of the runs of the paragraph that `start` belongs to, top to bottom.
pub(crate) fn paragraph_runs(runs: &[TextRun], lines: &[Line], start: usize) -> Option<Vec<usize>> {
    let mut chain = vec![start];
    let like = |a: &TextRun, b: &TextRun| {
        a.font == b.font && (a.size - b.size).abs() < 0.05 && (a.display[0] - b.display[0]).abs() <= SAME_LEFT
    };
    let candidates: Vec<usize> = lines.iter().filter_map(|l| plain_line(runs, l)).collect();
    let mut leading: Option<f32> = None;
    let fits = |gap: f32, size: f32, leading: Option<f32>| {
        gap >= 0.9 * size && gap <= 2.5 * size && leading.map_or(true, |l| (gap - l).abs() <= LEADING_SLACK * l)
    };
    // Downwards: the nearest line below that continues the block.
    loop {
        let last = &runs[*chain.last().unwrap()];
        let next = candidates
            .iter()
            .copied()
            .filter(|&c| !chain.contains(&c) && like(last, &runs[c]) && baseline(&runs[c]) > baseline(last))
            .min_by(|&a, &b| baseline(&runs[a]).total_cmp(&baseline(&runs[b])));
        let Some(next) = next else { break };
        let gap = baseline(&runs[next]) - baseline(last);
        if !fits(gap, last.size, leading) {
            break;
        }
        leading.get_or_insert(gap);
        chain.push(next);
    }
    // Upwards.
    loop {
        let first = &runs[chain[0]];
        let previous = candidates
            .iter()
            .copied()
            .filter(|&c| !chain.contains(&c) && like(first, &runs[c]) && baseline(&runs[c]) < baseline(first))
            .max_by(|&a, &b| baseline(&runs[a]).total_cmp(&baseline(&runs[b])));
        let Some(previous) = previous else { break };
        let gap = baseline(first) - baseline(&runs[previous]);
        if !fits(gap, first.size, leading) {
            break;
        }
        leading.get_or_insert(gap);
        chain.insert(0, previous);
    }
    (chain.len() >= 2).then_some(chain)
}

impl Document {
    fn run_index(runs: &[TextRun], run: &TextRun) -> Option<usize> {
        runs.iter().position(|r| r.op_index == run.op_index && r.xobject == run.xobject)
    }

    /// The paragraph `run` is a line of, if it is one (two lines or more).
    pub fn paragraph_of(&self, page: usize, run: &TextRun) -> Result<Option<Paragraph>> {
        let runs = self.text_runs(page)?;
        let Some(start) = Self::run_index(&runs, run) else { return Ok(None) };
        let lines = build_lines(&runs);
        let Some(chain) = paragraph_runs(&runs, &lines, start) else { return Ok(None) };
        let text = chain.iter().map(|&i| runs[i].text.trim_end()).collect::<Vec<_>>().join(" ");
        let display = chain.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, &i| {
            let d = runs[i].display;
            [b[0].min(d[0]), b[1].min(d[1]), b[2].max(d[2]), b[3].max(d[3])]
        });
        Ok(Some(Paragraph { text, runs: chain, display }))
    }

    /// Lay `new_text` out as the paragraph `run` belongs to, within the width the
    /// paragraph already has. Fails if `run` is not part of a paragraph (use
    /// `replace_text` for a single line).
    pub fn replace_paragraph(&mut self, page: usize, run: &TextRun, new_text: &str, allow_fallback: bool) -> Result<ParagraphEdit> {
        self.transaction(|d| d.rewrap(page, run, new_text, allow_fallback))
    }

    fn rewrap(&mut self, page: usize, run: &TextRun, new_text: &str, allow_fallback: bool) -> Result<ParagraphEdit> {
        let id = self.page_id(page)?;
        let runs = self.text_runs(page)?;
        let start = Self::run_index(&runs, run).ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó".into()))?;
        let lines = build_lines(&runs);
        let chain = paragraph_runs(&runs, &lines, start)
            .ok_or_else(|| Error::Unsupported("ese texto no es un párrafo de varias líneas parecidas".into()))?;
        let members: Vec<TextRun> = chain.iter().map(|&i| runs[i].clone()).collect();
        let first = &members[0];

        let mut fonts = fonts_of(&self.pdf, &self.resources(id));
        let font = fonts
            .get_mut(&first.font)
            .ok_or_else(|| Error::Unsupported(format!("no se encontró la fuente {}", first.font)))?;
        font.allow_seen(runs.iter().filter(|r| r.font == first.font).flat_map(|r| r.glyphs.iter().map(|g| g.code)));
        let font = &*font;
        let (key, data) = fallback::choose(&first.base_font);

        // The new text has to be drawable before anything changes.
        let everything: String = new_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if everything.is_empty() {
            return Err(Error::Unsupported("el texto nuevo está vacío".into()));
        }
        let unfit: Vec<char> = font.segments(&everything).iter().filter(|(_, b)| b.is_none()).flat_map(|(t, _)| t.chars()).collect();
        if !unfit.is_empty() {
            let lacking = if allow_fallback { fallback::lacks(data, &unfit.iter().collect::<String>()) } else { unfit };
            if !lacking.is_empty() {
                return Err(Error::MissingGlyphs(lacking));
            }
        }

        // Greedy wrap within the width of the paragraph's longest line.
        let limit = members.iter().map(|m| m.display[2] - m.display[0]).fold(0.0, f32::max);
        let size = first.size;
        let measure = |text: &str| -> f32 {
            font.segments(text)
                .iter()
                .map(|(piece, bytes)| match bytes {
                    Some(b) => font.advance_of(b, size),
                    None => fallback::advance(data, piece, size),
                })
                .sum()
        };
        let mut wrapped: Vec<String> = Vec::new();
        let mut current = String::new();
        for word in everything.split(' ') {
            let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
            // A hair of tolerance: the width measured here and the one a reader drew agree only to a fraction of a point.
            if !current.is_empty() && measure(&candidate) > limit + 0.5 {
                wrapped.push(std::mem::take(&mut current));
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        wrapped.push(current);

        let (old_count, new_count) = (members.len(), wrapped.len());
        let mut used_fallback: Vec<char> = Vec::new();
        let options = ReplaceOptions { allow_fallback, reflow: Reflow::None };

        // Existing lines, last drawn first: an edit changes only the operations
        // from its own on, so the runs read earlier stay valid for the ones before.
        let mut jobs: Vec<usize> = (0..old_count).collect();
        jobs.sort_by_key(|&i| std::cmp::Reverse(members[i].op_index));
        for i in jobs {
            if i < new_count {
                if wrapped[i] != members[i].text.trim_end() {
                    let done = self.replace(page, &members[i], &wrapped[i], options)?;
                    used_fallback.extend(done.fallback_chars);
                }
            } else {
                self.blank_run(id, &members[i])?;
            }
        }

        // Lines beyond the old last one, drawn the way the last one is, further down.
        if new_count > old_count {
            let last = &members[old_count - 1];
            let before = &members[old_count - 2];
            let step = (last.origin.0 - before.origin.0, last.origin.1 - before.origin.1);
            let mut ops: Vec<Operation> = Vec::new();
            for (n, text) in wrapped.iter().enumerate().skip(old_count) {
                let k = (n + 1 - old_count) as f32;
                let origin = (last.origin.0 + step.0 * k, last.origin.1 + step.1 * k);
                let segments = font.segments(text);
                let (shown, _, fell_back) = self.build_show_ops(font, last, &segments, key, data, None)?;
                used_fallback.extend(fell_back);
                ops.push(Operation::new("q", vec![]));
                ops.extend(last.fill.iter().cloned());
                ops.push(Operation::new("BT", vec![]));
                ops.push(Operation::new("Tf", vec![Object::Name(last.font.as_bytes().to_vec()), Object::Real(last.tf_size)]));
                let [a, b, c, d] = last.text_linear;
                ops.push(Operation::new("Tm", [a, b, c, d, origin.0, origin.1].iter().map(|v| Object::Real(*v)).collect()));
                ops.extend(shown);
                ops.push(Operation::new("ET", vec![]));
                ops.push(Operation::new("Q", vec![]));
            }
            if self.fallbacks.contains_key(key) && !used_fallback.is_empty() {
                let (resource, font_id) = (self.fallbacks[key].resource.clone(), self.fallbacks[key].font_id);
                self.register_font(id, &resource, font_id)?;
            }
            self.append_to_page(id, &Content { operations: ops })?;
        }

        // What the longer paragraph now covers, and whether that is free.
        let mut warnings = Vec::new();
        let new_bottom = members[old_count - 1].display[3] + (new_count.saturating_sub(old_count)) as f32 * (baseline(&members[old_count - 1]) - baseline(&members[old_count - 2]));
        if new_count > old_count {
            let area = [first.display[0], baseline(&members[old_count - 1]), first.display[0] + limit, new_bottom];
            let page_height = self.page_size(page)?.height;
            for other in runs.iter().filter(|r| !chain.iter().any(|&i| runs[i].op_index == r.op_index && runs[i].xobject == r.xobject)) {
                let d = other.display;
                let overlaps = d[0] < area[2] && d[2] > area[0] && d[1] < area[3] - 1.0 && d[3] > area[1] + 1.0 && other.render_mode != 3;
                if overlaps && !other.text.trim().is_empty() {
                    warnings.push(Warning::OverlapsText { text: other.text.clone() });
                }
            }
            if new_bottom > page_height {
                warnings.push(Warning::BelowPage { by: new_bottom - page_height });
            }
        }
        used_fallback.sort_unstable();
        used_fallback.dedup();
        Ok(ParagraphEdit {
            lines: new_count,
            added: new_count.saturating_sub(old_count),
            removed: old_count.saturating_sub(new_count),
            fallback_chars: used_fallback,
            warnings,
        })
    }

    /// Make a run draw nothing, keeping the operation it is (so no other
    /// operation's index moves).
    fn blank_run(&mut self, page: lopdf::ObjectId, run: &TextRun) -> Result<()> {
        let mut content = Content::decode(&self.pdf.get_page_content(page))?;
        let original = content
            .operations
            .get(run.op_index)
            .ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó".into()))?
            .clone();
        let empty = Object::String(Vec::new(), StringFormat::Literal);
        content.operations[run.op_index] = crate::edit::show_like(&original, empty);
        self.set_page_content(page, &content)
    }
}

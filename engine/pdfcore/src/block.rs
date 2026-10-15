//! Blocks: the unit a person edits, a paragraph (or a lone line) as one piece of text
//! with its styles, however many strings and fonts the producer used to draw it.
//!
//! A word processor draws a paragraph with bold words in the middle as several strings
//! in several fonts per line. Showing one editing box per string splits the paragraph
//! at every change of style; a [`Block`] has the whole text, and a list of ranges that
//! say which font, size and colour each stretch of it has.
//!
//! [`Document::replace_block`] takes the new text, lets every letter keep the style it
//! had (what is typed takes the style of the letter before it, as in a word processor),
//! lays it out again within the width the block had (justified if it was) and writes
//! only the lines that change. Lines whose text and styles did not change are not
//! touched at all, so their kerning and spacing stay exactly as the producer wrote them.

use std::collections::HashMap;

use lopdf::content::{Content, Operation};
use lopdf::Object;

use crate::error::{Error, Result};
use crate::fallback::{self, Embedded};
use crate::font::Font;
use crate::lines::{build_lines, is_upright, Line};
use crate::reflow::Warning;
use crate::substitute::{Substitute, Via};
use crate::text::{apply, fonts_of, invert, TextRun};
use crate::Document;

/// A stretch of a block's text in one style.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleSpan {
    /// Character range `[start, end)` of [`Block::text`].
    pub start: usize,
    pub end: usize,
    /// Name the page's resources give the font.
    pub font: String,
    pub base_font: String,
    /// Size on the page, in points.
    pub size: f32,
    /// Fill colour as red, green, blue in 0..=1, when it is a plain one.
    pub color: Option<[f32; 3]>,
    /// From the font's name (`Bold`, `Black`, `Semibold`...).
    pub bold: bool,
    /// From the font's name (`Italic`, `Oblique`).
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockLine {
    pub text: String,
    /// `[x0, y0, x1, y1]` as a reader shows the page.
    pub display: [f32; 4],
    /// Where the letters sit, as a reader shows the page (y down).
    pub baseline: f32,
}

#[derive(Debug, Clone)]
pub struct Block {
    /// The lines' text joined by single spaces.
    pub text: String,
    pub display: [f32; 4],
    pub lines: Vec<BlockLine>,
    pub spans: Vec<StyleSpan>,
    /// Every line but the last ends at the same place: re-laid text is justified too.
    pub justified: bool,
    /// How far the first line starts from the others (negative: hanging).
    pub first_line_indent: f32,
    /// Why the text cannot be edited, if it cannot (`type3`, `rtl_text`...).
    pub not_editable: Option<&'static str>,
    /// The strings it is drawn with, line by line.
    pub(crate) runs: Vec<TextRun>,
    /// Which line (index in `lines`) each run belongs to.
    run_line: Vec<usize>,
    styles: Vec<RunStyle>,
    /// The style of every character of `text`.
    char_style: Vec<usize>,
}

#[derive(Debug, Clone)]
struct RunStyle {
    font: String,
    base_font: String,
    tf_size: f32,
    size: f32,
    fill: Vec<Operation>,
}

impl RunStyle {
    fn same(&self, other: &RunStyle) -> bool {
        self.font == other.font
            && (self.tf_size - other.tf_size).abs() < 0.01
            && self.fill.len() == other.fill.len()
            && self.fill.iter().zip(&other.fill).all(|(a, b)| a.operator == b.operator && a.operands == b.operands)
    }
}

fn plain_line(runs: &[TextRun], line: &Line) -> bool {
    !line.runs.is_empty()
        && line.runs.iter().all(|&i| runs[i].xobject.is_none() && runs[i].render_mode != 3 && is_upright(&runs[i]))
}

fn dominant_size(runs: &[TextRun], line: &Line) -> f32 {
    line.runs.iter().map(|&i| runs[i].size).fold(0.0, f32::max)
}

fn color_of(fill: &[Operation]) -> Option<[f32; 3]> {
    let op = fill.last()?;
    let v: Vec<f32> = op.operands.iter().filter_map(|o| o.as_float().ok()).collect();
    match (op.operator.as_str(), v.len()) {
        ("g", 1) => Some([v[0]; 3]),
        ("rg", 3) => Some([v[0], v[1], v[2]]),
        ("k", 4) => Some([(1.0 - v[0]) * (1.0 - v[3]), (1.0 - v[1]) * (1.0 - v[3]), (1.0 - v[2]) * (1.0 - v[3])]),
        _ => None,
    }
}

/// `Nombre:`, `Teléfono celular:` at the start of a line: the line opens a field, not a continuation.
fn opens_with_label(text: &str) -> bool {
    text.split_once(':').is_some_and(|(label, _)| !label.is_empty() && label.chars().count() <= 30 && !label.contains(['.', ',', ';']))
}

/// `•`, `●`, `-`, `1.`, `a)` at the start of a line: an item of a list.
fn opens_with_bullet(text: &str) -> bool {
    let t = text.trim_start();
    let mut chars = t.chars();
    match chars.next() {
        Some('•' | '●' | '·' | '▪' | '■' | '○' | '-' | '–' | '*') => true,
        Some(c) if c.is_ascii_alphanumeric() => {
            let rest: String = t.chars().skip(1).take(2).collect();
            (c.is_ascii_digit() && rest.starts_with(['.', ')'])) || (c.is_ascii_lowercase() && rest.starts_with(')'))
        }
        _ => false,
    }
}

fn rest_left_of(lines: &[Line], chain: &[usize]) -> f32 {
    lines[chain[1]].display[0]
}

fn weight_words(base_font: &str) -> (bool, bool) {
    let name = base_font.rsplit('+').next().unwrap_or(base_font).to_lowercase();
    let bold = ["bold", "black", "heavy", "semibold", "demibold", "extrabold"].iter().any(|w| name.contains(w));
    let italic = name.contains("italic") || name.contains("oblique");
    (bold, italic)
}

impl Document {
    /// The page's text as blocks, top to bottom: each paragraph as one block, each
    /// other line as a block of its own.
    pub fn blocks(&self, page: usize) -> Result<Vec<Block>> {
        let runs = self.text_runs(page)?;
        let lines = build_lines(&runs);
        let baseline = |l: &Line| self.to_display_point(page, runs[l.runs[0]].origin).map(|p| p.1).unwrap_or(l.display[3]);
        let mut taken = vec![false; lines.len()];
        let mut out = Vec::new();
        for first in 0..lines.len() {
            if taken[first] || !plain_line(&runs, &lines[first]) {
                continue;
            }
            taken[first] = true;
            let mut chain = vec![first];
            let mut leading: Option<f32> = None;
            let mut rest_left: Option<f32> = None;
            loop {
                let last = *chain.last().unwrap();
                let size_last = dominant_size(&runs, &lines[last]);
                let next = (0..lines.len())
                    .filter(|&j| !taken[j] && plain_line(&runs, &lines[j]) && baseline(&lines[j]) > baseline(&lines[last]) + 0.5)
                    .min_by(|&a, &b| baseline(&lines[a]).total_cmp(&baseline(&lines[b])));
                let Some(next) = next else { break };
                let gap = baseline(&lines[next]) - baseline(&lines[last]);
                let (left_next, left_first) = (lines[next].display[0], lines[first].display[0]);
                let same_size = (dominant_size(&runs, &lines[next]) - size_last).abs() < 0.1 * size_last.max(1.0);
                let aligned = match rest_left {
                    Some(l) => (left_next - l).abs() <= 1.0,
                    // The second line fixes the margin; the first may be indented (a hanging
                    // first line is a list item, not a paragraph).
                    None => left_next <= left_first + 0.5 && left_first - left_next <= 3.0 * size_last,
                };
                let even = leading.map_or(true, |l| (gap - l).abs() <= 0.25 * l);
                let close = gap >= 0.9 * size_last && gap <= 2.5 * size_last;
                // The line above must be full: a paragraph's lines reach the margin, a list's do not.
                let width = |l: &Line| l.display[2] - l.display[0];
                let reach = chain.iter().chain(std::iter::once(&next)).map(|&i| lines[i].display[2]).fold(f32::MIN, f32::max);
                let word = lines[next].text.split(' ').next().map_or(0, |w| w.chars().count()) as f32
                    * (width(&lines[next]) / lines[next].text.chars().count().max(1) as f32);
                let full = lines[last].display[2] + word + 0.3 * size_last >= reach - 0.5;
                // A line that opens with `Label:` starts something of its own (a form's fields).
                // The sentence goes on: the next line starts in lower case (or, after a full stop, once
                // the lines have already shown they flow). Labels and list items start in capitals.
                let first_letter = lines[next].text.chars().next().unwrap_or(' ');
                let after_stop = lines[last].text.trim_end().ends_with(['.', '!', '?']);
                let continues = !first_letter.is_uppercase() || (chain.len() > 1 && after_stop);
                if !(same_size && aligned && even && close && full && continues)
                    || opens_with_label(&lines[next].text)
                    || opens_with_bullet(&lines[next].text)
                {
                    break;
                }
                leading.get_or_insert(gap);
                rest_left.get_or_insert(left_next);
                taken[next] = true;
                chain.push(next);
            }
            // Two lines are a paragraph only if the first reaches the margin of its column: lines
            // of a list can look like one otherwise (a long one followed by a short one).
            if chain.len() == 2 {
                let margin = rest_left_of(&lines, &chain);
                let column_right = (0..lines.len())
                    .filter(|&k| plain_line(&runs, &lines[k]) && (lines[k].display[0] - margin).abs() <= 1.0)
                    .map(|k| lines[k].display[2])
                    .fold(f32::MIN, f32::max);
                let size = dominant_size(&runs, &lines[chain[0]]);
                let word = lines[chain[1]].text.split(' ').next().map_or(0, |w| w.chars().count()) as f32
                    * (lines[chain[1]].display[2] - lines[chain[1]].display[0]) / lines[chain[1]].text.chars().count().max(1) as f32;
                if column_right - lines[chain[0]].display[2] > word + 0.3 * size {
                    taken[chain[1]] = false;
                    chain.truncate(1);
                }
            }
            out.push(self.make_block(page, &runs, &lines, &chain));
        }
        Ok(out)
    }

    fn make_block(&self, page: usize, runs: &[TextRun], lines: &[Line], chain: &[usize]) -> Block {
        let mut block_runs: Vec<TextRun> = Vec::new();
        let mut run_line: Vec<usize> = Vec::new();
        let mut styles: Vec<RunStyle> = Vec::new();
        let mut char_style: Vec<usize> = Vec::new();
        let mut text = String::new();
        let mut block_lines: Vec<BlockLine> = Vec::new();
        let mut display = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut style_of_run: HashMap<usize, usize> = HashMap::new();
        for (n, &li) in chain.iter().enumerate() {
            let line = &lines[li];
            if n > 0 {
                text.push(' ');
                char_style.push(*char_style.last().unwrap_or(&0));
            }
            for &ri in &line.runs {
                let run = &runs[ri];
                let style = RunStyle {
                    font: run.font.clone(),
                    base_font: run.base_font.clone(),
                    tf_size: run.tf_size,
                    size: run.size,
                    fill: run.fill.clone(),
                };
                let id = match styles.iter().position(|s| s.same(&style)) {
                    Some(id) => id,
                    None => {
                        styles.push(style);
                        styles.len() - 1
                    }
                };
                style_of_run.insert(ri, id);
                block_runs.push(run.clone());
                run_line.push(n);
            }
            for c in &line.chars {
                text.push(c.ch);
                char_style.push(style_of_run.get(&c.run).copied().unwrap_or(0));
            }
            let base = self.to_display_point(page, runs[line.runs[0]].origin).map(|p| p.1).unwrap_or(line.display[3]);
            block_lines.push(BlockLine { text: line.text.clone(), display: line.display, baseline: base });
            display = [display[0].min(line.display[0]), display[1].min(line.display[1]), display[2].max(line.display[2]), display[3].max(line.display[3])];
        }
        // Merge neighbouring letters of one style into ranges.
        let mut spans: Vec<StyleSpan> = Vec::new();
        for (i, &s) in char_style.iter().enumerate() {
            match spans.last_mut() {
                Some(last) if last.end == i && styles[s].same(&styles[char_style[last.start]]) => last.end = i + 1,
                _ => {
                    let st = &styles[s];
                    let (bold, italic) = weight_words(&st.base_font);
                    spans.push(StyleSpan {
                        start: i, end: i + 1, font: st.font.clone(), base_font: st.base_font.clone(), size: st.size,
                        color: color_of(&st.fill), bold, italic,
                    });
                }
            }
        }
        let rights: Vec<f32> = block_lines.iter().take(block_lines.len().saturating_sub(1)).map(|l| l.display[2]).collect();
        let justified = rights.len() >= 2 && rights.iter().all(|r| (r - rights[0]).abs() <= 0.6);
        let first_line_indent = if block_lines.len() > 1 { block_lines[0].display[0] - block_lines[1].display[0] } else { 0.0 };
        let not_editable = block_runs.iter().find_map(|r| r.not_editable);
        Block { text, display, lines: block_lines, spans, justified, first_line_indent, not_editable, runs: block_runs, run_line, styles, char_style }
    }
}

/// What [`Document::replace_block`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockEdit {
    /// Lines the block has now.
    pub lines: usize,
    /// Lines left exactly as they were (same text and styles: not a byte of them is rewritten).
    pub kept: usize,
    /// Lines drawn again.
    pub rewritten: usize,
    /// Lines added after the old last one, and old lines at the end that are no longer needed.
    pub added: usize,
    pub removed: usize,
    /// Letters drawn with the bundled font because the document's font lacks them.
    pub fallback_chars: Vec<char>,
    /// How every letter the fonts lacked was drawn (see [`Substitute`]).
    pub substitutes: Vec<Substitute>,
    pub warnings: Vec<Warning>,
}

/// Collects the operations of one line: strings and the numbers between them as a `TJ`.
#[derive(Default)]
struct LineWriter {
    ops: Vec<Operation>,
    array: Vec<Object>,
    bytes: Vec<u8>,
}

impl LineWriter {
    fn flush_string(&mut self) {
        if !self.bytes.is_empty() {
            self.array.push(Embedded::hex(std::mem::take(&mut self.bytes)));
        }
    }
    fn flush_show(&mut self) {
        self.flush_string();
        if !self.array.is_empty() {
            self.ops.push(Operation::new("TJ", vec![Object::Array(std::mem::take(&mut self.array))]));
        }
    }
    fn op(&mut self, op: Operation) {
        self.flush_show();
        self.ops.push(op);
    }
    fn shift(&mut self, n: f32) {
        self.flush_string();
        self.array.push(Object::Real(n));
    }
}

/// Longest common subsequence of two slices, as the matched index pairs. Past `LIMIT` cells of
/// table it only matches the common start and end, which is all a big rewrite needs.
fn common<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    const LIMIT: usize = 4_000_000;
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut pairs: Vec<(usize, usize)> = (0..head).map(|i| (i, i)).collect();
    if am.len() * bm.len() <= LIMIT && !am.is_empty() && !bm.is_empty() {
        let w = bm.len() + 1;
        let mut t = vec![0u32; (am.len() + 1) * w];
        for i in (0..am.len()).rev() {
            for j in (0..bm.len()).rev() {
                t[i * w + j] = if am[i] == bm[j] { t[(i + 1) * w + j + 1] + 1 } else { t[(i + 1) * w + j].max(t[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < am.len() && j < bm.len() {
            if am[i] == bm[j] {
                pairs.push((head + i, head + j));
                i += 1;
                j += 1;
            } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    pairs.extend((0..tail).map(|k| (a.len() - tail + k, b.len() - tail + k)));
    pairs
}

/// Words and single spaces of a text, as `[start, end)` character ranges.
fn tokens(text: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let start = i;
        if text[i] == ' ' {
            i += 1;
        } else {
            while i < text.len() && text[i] != ' ' {
                i += 1;
            }
        }
        out.push((start, i));
    }
    out
}

/// The style of every letter of `new`: a letter that stays keeps the style it had, one that is
/// typed takes the style of the letter before it. Words are matched first, then the letters
/// inside the words that changed, so a fix in the middle of a bold word stays bold.
fn carry_styles(old: &[char], new: &[char], old_style: &[usize]) -> Vec<usize> {
    // Only words are matched: any space equals any other, which would pair the wrong ones.
    let spaceless = |text: &[char]| tokens(text).into_iter().filter(|r| text[r.0] != ' ').collect::<Vec<_>>();
    let (ot, nt) = (spaceless(old), spaceless(new));
    let owords: Vec<&[char]> = ot.iter().map(|r| &old[r.0..r.1]).collect();
    let nwords: Vec<&[char]> = nt.iter().map(|r| &new[r.0..r.1]).collect();
    let mut matched: Vec<Option<usize>> = vec![None; new.len()];
    // Matched words, and the gaps between them compared letter by letter.
    let mut pairs = common(&owords, &nwords);
    pairs.push((ot.len(), nt.len()));
    let (mut po, mut pn) = (0usize, 0usize);
    for (wo, wn) in pairs {
        let (o0, o1) = (ot.get(po).map_or(old.len(), |r| r.0), ot.get(wo).map_or(old.len(), |r| r.0));
        let (n0, n1) = (nt.get(pn).map_or(new.len(), |r| r.0), nt.get(wn).map_or(new.len(), |r| r.0));
        for (i, j) in common(&old[o0..o1], &new[n0..n1]) {
            matched[n0 + j] = Some(o0 + i);
        }
        if wo < ot.len() {
            for k in 0..(ot[wo].1 - ot[wo].0) {
                matched[nt[wn].0 + k] = Some(ot[wo].0 + k);
            }
        }
        po = wo + 1;
        pn = wn + 1;
    }
    // A space beside a letter that stayed is the space that was beside it before.
    for i in 0..new.len() {
        if new[i] != ' ' || matched[i].is_some() {
            continue;
        }
        let after_prev = i.checked_sub(1).and_then(|k| matched[k]).filter(|o| old.get(o + 1) == Some(&' ')).map(|o| o + 1);
        let before_next = matched.get(i + 1).copied().flatten().filter(|o| *o > 0 && old[o - 1] == ' ').map(|o| o - 1);
        matched[i] = after_prev.or(before_next);
    }
    let mut out: Vec<usize> = Vec::with_capacity(new.len());
    for i in 0..new.len() {
        out.push(match matched[i] {
            Some(o) => old_style[o],
            None if i > 0 => out[i - 1],
            // Typed at the very start: the style of the first letter that stayed.
            None => matched.iter().flatten().next().map_or(old_style[0], |o| old_style[*o]),
        });
    }
    out
}

fn tf(name: &str, size: f32) -> Operation {
    Operation::new("Tf", vec![Object::Name(name.as_bytes().to_vec()), Object::Real(size)])
}

impl Document {
    /// Replace the whole text of a block, keeping the style of every letter that stays and
    /// laying the text out again within the width the block had. Fails with
    /// `MissingGlyphs` when a font cannot draw a letter and no fallback is allowed (or
    /// neither font has it), with `NotEditable` for text that cannot be reproduced; in both
    /// cases nothing changes. One step to undo.
    pub fn replace_block(&mut self, page: usize, block: &Block, new_text: &str, allow_fallback: bool) -> Result<BlockEdit> {
        let wanted: String = new_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if wanted == block.text {
            return Ok(BlockEdit {
                lines: block.lines.len(), kept: block.lines.len(), rewritten: 0, added: 0, removed: 0,
                fallback_chars: Vec::new(), substitutes: Vec::new(), warnings: Vec::new(),
            });
        }
        self.transaction(|d| d.relay_block(page, block, &wanted, allow_fallback))
    }

    fn relay_block(&mut self, page: usize, block: &Block, wanted: &str, allow_fallback: bool) -> Result<BlockEdit> {
        if let Some(reason) = block.not_editable {
            return Err(Error::NotEditable(reason.into()));
        }
        if wanted.chars().any(crate::text::is_rtl_char) {
            return Err(Error::NotEditable("rtl_text".into()));
        }
        if wanted.is_empty() {
            return Err(Error::Unsupported("el texto nuevo está vacío".into()));
        }
        let id = self.page_id(page)?;
        // Every letter keeps its style; what is typed takes the style of the letter before it.
        let old: Vec<char> = block.text.chars().collect();
        let new: Vec<char> = wanted.chars().collect();
        let carried = carry_styles(&old, &new, &block.char_style);
        let letters: Vec<(char, usize)> = (0..new.len()).map(|i| (new[i], carried[i])).collect();

        // Fonts the reader draws itself learn the letters they lack (same face, no bundled font).
        let wanted: Vec<(String, String)> = block
            .styles
            .iter()
            .enumerate()
            .map(|(sid, st)| (st.font.clone(), letters.iter().filter(|(_, s)| *s == sid).map(|(c, _)| *c).collect::<String>()))
            .collect();
        let taught = self.teach_fonts(id, &wanted)?;
        let page_runs = self.text_runs(page)?;
        let mut fonts = fonts_of(&self.pdf, &self.resources(id));
        for style in &block.styles {
            let font = fonts.get_mut(&style.font).ok_or_else(|| Error::Unsupported(format!("no se encontró la fuente {}", style.font)))?;
            font.allow_seen(page_runs.iter().filter(|r| r.font == style.font).flat_map(|r| r.glyphs.iter().map(|g| g.code)));
        }

        // The new text has to be drawable before anything changes.
        let mut lacking: Vec<char> = Vec::new();
        for (sid, style) in block.styles.iter().enumerate() {
            let text: String = letters.iter().filter(|(_, s)| *s == sid).map(|(c, _)| *c).collect();
            let font = &fonts[&style.font];
            let unfit: Vec<char> = font.segments(&text).iter().filter(|(_, b)| b.is_none()).flat_map(|(t, _)| t.chars()).collect();
            if unfit.is_empty() {
                continue;
            }
            let (_, data) = self.pick_font(&style.base_font, font.flags()).into_pair();
            let missing = if allow_fallback { fallback::lacks(data, &unfit.iter().collect::<String>()) } else { unfit };
            lacking.extend(missing);
        }
        if !lacking.is_empty() {
            lacking.sort_unstable();
            lacking.dedup();
            return Err(Error::MissingGlyphs(lacking));
        }

        // Width of a letter in its style (kerning ignored), cached.
        let mut cache: HashMap<(usize, char), f32> = HashMap::new();
        // The bundled (or registered) font each style falls back to.
        let choices: Vec<fallback::Choice> =
            block.styles.iter().map(|st| self.pick_font(&st.base_font, fonts[&st.font].flags())).collect();
        let mut advance = |sid: usize, ch: char| -> f32 {
            *cache.entry((sid, ch)).or_insert_with(|| {
                let style = &block.styles[sid];
                let font = &fonts[&style.font];
                let piece = ch.to_string();
                font.segments(&piece)
                    .iter()
                    .map(|(t, b)| match b {
                        Some(bytes) => font.advance_of(bytes, style.size),
                        None => fallback::advance(choices[sid].data, t, style.size),
                    })
                    .sum()
            })
        };

        // Where lines start and how far they may reach.
        let old_count = block.lines.len();
        let rest_left = if old_count > 1 { block.lines[1].display[0] } else { block.lines[0].display[0] };
        let first_left = block.lines[0].display[0];
        let right = block.lines.iter().map(|l| l.display[2]).fold(f32::MIN, f32::max);
        let flowing = old_count > 1;
        let left_of = |i: usize| if i == 0 { first_left } else { rest_left };

        // Greedy wrap by words.
        let mut lines: Vec<Vec<(char, usize)>> = vec![Vec::new()];
        let mut width = 0.0f32;
        // Each word with the style of the space before it (the spaces keep their own style).
        let mut words: Vec<(usize, Vec<(char, usize)>)> = Vec::new();
        let mut space_before = letters.first().map_or(0, |l| l.1);
        let mut current: Vec<(char, usize)> = Vec::new();
        for &(c, st) in &letters {
            if c == ' ' {
                words.push((space_before, std::mem::take(&mut current)));
                space_before = st;
            } else {
                current.push((c, st));
            }
        }
        words.push((space_before, current));
        for (space_style, word) in &words {
            let word_width: f32 = word.iter().map(|&(c, s)| advance(s, c)).sum();
            let space_style = *space_style;
            let space = advance(space_style, ' ');
            let line = lines.len() - 1;
            let avail = right - left_of(line);
            let needed = if lines[line].is_empty() { word_width } else { width + space + word_width };
            if flowing && !lines[line].is_empty() && needed > avail + 0.01 {
                lines.push(word.clone());
                width = word_width;
            } else {
                if !lines[line].is_empty() {
                    lines[line].push((' ', space_style));
                }
                lines[line].extend(word.iter().copied());
                width = needed;
            }
        }

        // Which lines stay untouched: same text and same styles as the old line at that place.
        let mut offsets = Vec::new();
        let mut at = 0usize;
        for l in &block.lines {
            offsets.push(at);
            at += l.text.chars().count() + 1;
        }
        let kept: Vec<bool> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                i < old_count && {
                    let old_line: Vec<char> = block.lines[i].text.chars().collect();
                    let text_same = old_line.len() == l.len() && old_line.iter().zip(l).all(|(a, (b, _))| a == b);
                    text_same && (0..l.len()).all(|k| block.char_style.get(offsets[i] + k) == Some(&l[k].1))
                }
            })
            .collect();
        let new_count = lines.len();

        // Leading and baselines.
        let leading = if old_count > 1 {
            (block.lines[old_count - 1].baseline - block.lines[0].baseline) / (old_count - 1) as f32
        } else {
            block.styles[block.char_style[0]].size * 1.2
        };
        let baseline_of = |i: usize| -> f32 {
            if i < old_count {
                block.lines[i].baseline
            } else {
                block.lines[old_count - 1].baseline + (i + 1 - old_count) as f32 * leading
            }
        };
        let to_user = invert(self.display_matrix(page)?).ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        let [a, b, c, d] = block.runs[0].text_linear;

        // Draw the lines that changed.
        let mut ops: Vec<Operation> = Vec::new();
        let mut used_fallback: Vec<char> = Vec::new();
        let mut fallback_key: Option<&'static str> = None;
        let mut fallback_similarity = 100u8;
        for (i, line) in lines.iter().enumerate().filter(|(i, _)| !kept[*i]) {
            let mut trimmed: &[(char, usize)] = line;
            while trimmed.last().is_some_and(|(c, _)| *c == ' ') {
                trimmed = &trimmed[..trimmed.len() - 1];
            }
            if trimmed.is_empty() {
                continue;
            }
            let avail = right - left_of(i);
            let natural: f32 = trimmed.iter().map(|&(ch, s)| advance(s, ch)).sum();
            let spaces = trimmed.iter().filter(|(ch, _)| *ch == ' ').count();
            let stretch = flowing && block.justified && i + 1 < new_count && spaces > 0 && natural <= avail && natural >= 0.6 * avail;
            let extra = if stretch { (avail - natural) / spaces as f32 } else { 0.0 };

            let (ox, oy) = apply(to_user, left_of(i), baseline_of(i));
            let mut w = LineWriter::default();
            w.op(Operation::new("q", vec![]));
            w.op(Operation::new("BT", vec![]));
            w.op(Operation::new("Tm", [a, b, c, d, ox, oy].iter().map(|v| Object::Real(*v)).collect()));
            let (mut current_fill, mut current_font): (Option<usize>, Option<(String, f32)>) = (None, None);
            // Group neighbouring letters of one style.
            let mut start = 0;
            while start < trimmed.len() {
                let sid = trimmed[start].1;
                let end = trimmed[start..].iter().position(|(_, s)| *s != sid).map_or(trimmed.len(), |n| start + n);
                let text: String = trimmed[start..end].iter().map(|(c, _)| *c).collect();
                let style = &block.styles[sid];
                let font: &Font = &fonts[&style.font];
                if current_fill.map_or(true, |f| !block.styles[f].same(style) || block.styles[f].fill.len() != style.fill.len()) {
                    for op in &style.fill {
                        w.op(op.clone());
                    }
                    current_fill = Some(sid);
                }
                let choice = choices[sid];
                let (key, data) = (choice.key, choice.data);
                for (piece, bytes) in font.segments(&text) {
                    let (name, size) = match &bytes {
                        Some(_) => (style.font.clone(), style.tf_size),
                        None => {
                            self.ensure_fallback(key, data)?;
                            fallback_key = Some(key);
                            fallback_similarity = choice.similarity;
                            (self.fallbacks[key].resource.clone(), style.tf_size)
                        }
                    };
                    if current_font.as_ref().map_or(true, |(n, s)| *n != name || (*s - size).abs() > 0.001) {
                        w.op(tf(&name, size));
                        current_font = Some((name, size));
                    }
                    let shift = -extra / style.size.max(0.1) * 1000.0;
                    match bytes {
                        Some(bytes) => {
                            for g in font.decode(&bytes) {
                                let code: Vec<u8> = (0..font.code_bytes().max(1)).rev().map(|k| (g.code >> (8 * k)) as u8).collect();
                                w.bytes.extend(code);
                                if g.text == " " && extra > 0.0 {
                                    w.shift(shift);
                                }
                            }
                        }
                        None => {
                            let embedded = self.fallbacks.get_mut(key).expect("just ensured");
                            let (written, _) = embedded.write(&mut self.pdf, &piece, style.size)?;
                            used_fallback.extend(piece.chars());
                            for (ch, glyph) in piece.chars().zip(written.chunks(2)) {
                                w.bytes.extend_from_slice(glyph);
                                if ch == ' ' && extra > 0.0 {
                                    w.shift(shift);
                                }
                            }
                        }
                    }
                }
                start = end;
            }
            w.op(Operation::new("ET", vec![]));
            w.op(Operation::new("Q", vec![]));
            ops.extend(w.ops);
        }
        if let Some(key) = fallback_key {
            let (resource, font_id) = (self.fallbacks[key].resource.clone(), self.fallbacks[key].font_id);
            self.add_font_resource(id, &resource, font_id)?;
        }

        // The old lines that are no longer there as they were stop drawing.
        let dead: Vec<&TextRun> = block
            .runs
            .iter()
            .zip(&block.run_line)
            .filter(|(_, &line)| line >= new_count || !kept[line])
            .map(|(r, _)| r)
            .collect();
        self.blank_runs(id, &dead)?;
        if !ops.is_empty() {
            self.append_to_page(id, &Content { operations: ops })?;
        }

        // Does the block, grown, run over what is below it?
        let mut warnings = Vec::new();
        if new_count > old_count {
            let bottom = baseline_of(new_count - 1) + 0.25 * block.styles[block.char_style[0]].size;
            let area = [rest_left.min(first_left), block.lines[old_count - 1].baseline, right, bottom];
            let page_height = self.page_size(page)?.height;
            let ours = |r: &TextRun| block.runs.iter().any(|m| m.op_index == r.op_index && m.xobject == r.xobject);
            for other in page_runs.iter().filter(|r| !ours(r) && r.render_mode != 3 && !r.text.trim().is_empty()) {
                let d = other.display;
                if d[0] < area[2] && d[2] > area[0] && d[1] < area[3] - 1.0 && d[3] > area[1] + 1.0 {
                    warnings.push(Warning::OverlapsText { text: other.text.clone() });
                }
            }
            if bottom > page_height {
                warnings.push(Warning::BelowPage { by: bottom - page_height });
            }
        }
        used_fallback.sort_unstable();
        used_fallback.dedup();
        let kept_count = kept.iter().filter(|k| **k).count();
        Ok(BlockEdit {
            lines: new_count,
            kept: kept_count,
            rewritten: new_count - kept_count,
            added: new_count.saturating_sub(old_count),
            removed: old_count.saturating_sub(new_count),
            substitutes: taught
                .iter()
                .map(|&(ch, via)| Substitute { ch, via, font: block.styles[0].base_font.clone(), similarity: 100 })
                .chain(used_fallback.iter().map(|&ch| Substitute {
                ch,
                via: Via::BundledFont,
                font: fallback_key.unwrap_or_default().to_string(),
                similarity: fallback_similarity,
            }))
                .collect(),
            fallback_chars: used_fallback,
            warnings,
        })
    }

    /// Make runs draw nothing, keeping each operation (so no operation's index moves).
    fn blank_runs(&mut self, page: lopdf::ObjectId, runs: &[&TextRun]) -> Result<()> {
        if runs.is_empty() {
            return Ok(());
        }
        let mut content = Content::decode(&self.pdf.get_page_content(page))?;
        let empty = Object::String(Vec::new(), lopdf::StringFormat::Literal);
        for run in runs {
            let original = content
                .operations
                .get(run.op_index)
                .ok_or_else(|| Error::Unsupported("el texto ya no está donde se leyó: vuelve a leer la página".into()))?
                .clone();
            content.operations[run.op_index] = crate::edit::show_like(&original, empty.clone());
        }
        self.set_page_content(page, &content)
    }
}

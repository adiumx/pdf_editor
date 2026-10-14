//! Text as a person reads it: lines made of words, rather than the strings a
//! producer happened to show.
//!
//! A line is the runs that sit on one baseline with no more than a short gap
//! between them (the same rule the reflow uses), so a label in bold and the
//! value after it in regular weight are one line, and two table cells a column
//! apart are two. Words are found across run boundaries, so a word a producer
//! split in two runs is still one word.

use crate::text::TextRun;
use crate::{Document, Result};

/// Gap, in ems, up to which the next run continues the line.
const MAX_GAP_EM: f32 = 1.5;
/// A gap wider than this (in ems) is a space even if neither run has one.
const SPACE_EM: f32 = 0.18;

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    /// `[x0, y0, x1, y1]` as a reader shows it (top-left origin, y down).
    pub display: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct Line {
    pub text: String,
    pub display: [f32; 4],
    /// Indexes into the runs `Document::text_runs` returns for the page.
    pub runs: Vec<usize>,
    pub words: Vec<Word>,
    /// Every run of the line is invisible: the text layer of a scanned page.
    pub invisible: bool,
    pub(crate) chars: Vec<CharBox>,
}

/// One character of a line with the horizontal span it occupies.
#[derive(Debug, Clone)]
pub(crate) struct CharBox {
    pub ch: char,
    pub x0: f32,
    pub x1: f32,
    pub run: usize,
    /// Index of the character within its run's text.
    pub at: usize,
    /// Vertical extent of its run.
    pub y: (f32, f32),
}

impl Document {
    /// The page's text as lines of words, top to bottom, each line left to right.
    pub fn text_lines(&self, page: usize) -> Result<Vec<Line>> {
        Ok(build_lines(&self.text_runs(page)?))
    }
}

/// Does the run read horizontally as a reader shows the page? Judged from its
/// box on screen, so text turned by the page's `/Rotate` (a scan turned upright)
/// counts, and text running sideways on screen does not.
pub(crate) fn is_upright(run: &TextRun) -> bool {
    let (w, h) = (run.display[2] - run.display[0], run.display[3] - run.display[1]);
    run.text.chars().count() < 2 || w >= 0.9 * h
}

/// Where each character of a run falls: its width shared out in proportion to
/// the glyph advances (spacing the file adds is spread along).
fn char_boxes(run: &TextRun, index: usize) -> Vec<CharBox> {
    let [x0, y0, x1, y1] = run.display;
    let mut pieces: Vec<(char, f32)> = Vec::new();
    if run.glyphs.is_empty() {
        pieces.extend(run.text.chars().map(|c| (c, 1.0)));
    } else {
        for glyph in &run.glyphs {
            let count = glyph.text.chars().count().max(1) as f32;
            pieces.extend(glyph.text.chars().map(|c| (c, glyph.width.max(0.0) / count)));
        }
    }
    let total: f32 = pieces.iter().map(|(_, w)| w).sum();
    let equal = total <= 0.0;
    let count = pieces.len().max(1) as f32;
    let share = |w: f32| if equal { 1.0 / count } else { w / total };
    let mut at_x = x0;
    pieces
        .into_iter()
        .enumerate()
        .map(|(at, (ch, w))| {
            let next = at_x + (x1 - x0) * share(w);
            let boxed = CharBox { ch, x0: at_x, x1: next, run: index, at, y: (y0, y1) };
            at_x = next;
            boxed
        })
        .collect()
}

/// Lines of words from runs already read (what `Document::text_lines` does
/// after reading), for a caller that keeps the runs to edit them.
pub fn text_lines_of(runs: &[TextRun]) -> Vec<Line> {
    build_lines(runs)
}

pub(crate) fn build_lines(runs: &[TextRun]) -> Vec<Line> {
    // Cluster by vertical centre: scan in order of centre and start a new
    // cluster when one is clearly lower than the cluster's first. (Pairwise
    // comparisons would not be a consistent order to sort by.)
    let centre = |r: &TextRun| (r.display[1] + r.display[3]) / 2.0;
    let mut order: Vec<usize> = (0..runs.len()).filter(|&i| !runs[i].text.trim().is_empty()).collect();
    order.sort_by(|&a, &b| centre(&runs[a]).total_cmp(&centre(&runs[b])).then(a.cmp(&b)));

    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut first_centre = f32::MIN;
    for i in order {
        let tolerance = 0.4 * runs[i].size.max(1.0);
        let upright = is_upright(&runs[i]);
        let joins = upright && clusters.last().is_some_and(|c| is_upright(&runs[c[0]]) && centre(&runs[i]) - first_centre <= tolerance);
        if joins {
            clusters.last_mut().unwrap().push(i);
        } else {
            first_centre = centre(&runs[i]);
            clusters.push(vec![i]);
        }
    }

    let mut lines = Vec::new();
    for mut cluster in clusters {
        cluster.sort_by(|&a, &b| runs[a].display[0].total_cmp(&runs[b].display[0]).then(a.cmp(&b)));
        // Split where the gap is too wide to be the same line.
        let mut group: Vec<usize> = Vec::new();
        for i in cluster {
            if let Some(&last) = group.last() {
                let gap = runs[i].display[0] - runs[last].display[2];
                if gap > MAX_GAP_EM * runs[i].size.max(1.0) {
                    lines.push(make_line(runs, std::mem::take(&mut group)));
                }
            }
            group.push(i);
        }
        if !group.is_empty() {
            lines.push(make_line(runs, group));
        }
    }
    lines.sort_by(|a, b| {
        let ya = (a.display[1] + a.display[3]) / 2.0;
        let yb = (b.display[1] + b.display[3]) / 2.0;
        ya.total_cmp(&yb).then(a.display[0].total_cmp(&b.display[0]))
    });
    lines
}

fn make_line(runs: &[TextRun], members: Vec<usize>) -> Line {
    let mut chars: Vec<CharBox> = Vec::new();
    let mut display = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for &i in &members {
        let run = &runs[i];
        // A space is added where the gap says there is one and the text has none.
        if let Some(prev) = chars.last() {
            let gap = run.display[0] - prev.x1;
            if gap > SPACE_EM * run.size.max(1.0) && !prev.ch.is_whitespace() && !run.text.starts_with(char::is_whitespace) {
                let at = prev.at + 1;
                let (x0, y, run_index) = (prev.x1, prev.y, prev.run);
                chars.push(CharBox { ch: ' ', x0, x1: run.display[0], run: run_index, at, y });
            }
        }
        chars.extend(char_boxes(run, i));
        display = [
            display[0].min(run.display[0]),
            display[1].min(run.display[1]),
            display[2].max(run.display[2]),
            display[3].max(run.display[3]),
        ];
    }

    let mut words = Vec::new();
    let mut current: Vec<&CharBox> = Vec::new();
    let flush = |current: &mut Vec<&CharBox>, words: &mut Vec<Word>| {
        if let (Some(first), Some(last)) = (current.first(), current.last()) {
            let (top, bottom) = current.iter().fold((f32::MAX, f32::MIN), |(t, b), c| (t.min(c.y.0), b.max(c.y.1)));
            words.push(Word { text: current.iter().map(|c| c.ch).collect(), display: [first.x0, top, last.x1, bottom] });
        }
        current.clear();
    };
    for c in &chars {
        if c.ch.is_whitespace() {
            flush(&mut current, &mut words);
        } else {
            current.push(c);
        }
    }
    flush(&mut current, &mut words);

    Line {
        text: chars.iter().map(|c| c.ch).collect(),
        display,
        invisible: members.iter().all(|&i| runs[i].render_mode == 3),
        runs: members,
        words,
        chars,
    }
}

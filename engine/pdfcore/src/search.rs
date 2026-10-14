//! Finding text across the document, and replacing every occurrence in one
//! undoable step.

use crate::error::{Error, Result};
use crate::lines::{build_lines, Line};
use crate::{Document, ReplaceOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    /// `Póliza` and `póliza` are different.
    pub case_sensitive: bool,
    /// Only whole words: `poliza` does not match inside `polizas`.
    pub whole_word: bool,
    /// `á` and `a` are different. By default accents are ignored, which is how
    /// people type a name they cannot spell the accents of.
    pub accent_sensitive: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions { case_sensitive: false, whole_word: false, accent_sensitive: false }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub page: usize,
    /// The text that matched, as the page has it.
    pub text: String,
    /// The line it was found in.
    pub line: String,
    /// `[x0, y0, x1, y1]` as a reader shows it, for a highlight.
    pub display: [f32; 4],
    /// Index (in `Document::text_runs(page)`) of the run it starts in, and the
    /// character range of that run it covers; `None` when it runs on into the
    /// next run (the match crosses a boundary between two strings).
    pub run: usize,
    pub within_run: Option<(usize, usize)>,
}

/// What a replace-all did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplacedAll {
    pub replaced: usize,
    /// Matches left as they were: the text crosses two strings, lies inside a
    /// form XObject, or the font lacks a letter of the replacement.
    pub skipped: usize,
}

/// One character as it is compared: lower-cased and without accents when the
/// options say so, always as a single character so positions stay aligned.
fn fold(c: char, options: &SearchOptions) -> char {
    let c = if options.case_sensitive { c } else { c.to_lowercase().next().unwrap_or(c) };
    if options.accent_sensitive {
        return c;
    }
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'A',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'O',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ç' => 'c',
        'Ç' => 'C',
        other => other,
    }
}

/// Character ranges `(start, end)` of the occurrences of `query` in `haystack`.
fn occurrences(haystack: &[char], query: &[char], options: &SearchOptions) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    if query.is_empty() || haystack.len() < query.len() {
        return found;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = 0;
    while at + query.len() <= haystack.len() {
        if haystack[at..at + query.len()] == *query {
            let end = at + query.len();
            let bounded = !options.whole_word
                || ((at == 0 || !word(haystack[at - 1])) && (end == haystack.len() || !word(haystack[end])));
            if bounded {
                found.push((at, end));
                at = end;
                continue;
            }
        }
        at += 1;
    }
    found
}

fn matches_in_line(page: usize, line: &Line, query: &[char], options: &SearchOptions) -> Vec<Match> {
    let folded: Vec<char> = line.chars.iter().map(|c| fold(c.ch, options)).collect();
    occurrences(&folded, query, options)
        .into_iter()
        .map(|(start, end)| {
            let part = &line.chars[start..end];
            let (first, last) = (&part[0], &part[part.len() - 1]);
            let (top, bottom) = part.iter().fold((f32::MAX, f32::MIN), |(t, b), c| (t.min(c.y.0), b.max(c.y.1)));
            let one_run = part.iter().all(|c| c.run == first.run);
            Match {
                page,
                text: part.iter().map(|c| c.ch).collect(),
                line: line.text.clone(),
                display: [first.x0, top, last.x1, bottom],
                run: first.run,
                within_run: one_run.then_some((first.at, last.at + 1)),
            }
        })
        .collect()
}

impl Document {
    /// Every occurrence of `query` in the document, page by page, top to bottom.
    pub fn find_text(&self, query: &str, options: SearchOptions) -> Vec<Match> {
        let query: Vec<char> = query.chars().map(|c| fold(c, &options)).collect();
        let mut out = Vec::new();
        for page in 0..self.page_count() {
            let Ok(runs) = self.text_runs(page) else { continue };
            for line in build_lines(&runs) {
                out.extend(matches_in_line(page, &line, &query, &options));
            }
        }
        out
    }

    /// Replace every occurrence of `find` with `replacement`, as one step that
    /// `undo` takes back whole. Each string keeps its own font, size and place;
    /// what follows it on the line moves as `replace.reflow` says.
    pub fn replace_all(
        &mut self,
        find: &str,
        replacement: &str,
        search: SearchOptions,
        replace: ReplaceOptions,
    ) -> Result<ReplacedAll> {
        let query: Vec<char> = find.chars().map(|c| fold(c, &search)).collect();
        if query.is_empty() {
            return Err(Error::Rejected("no hay nada que buscar".into()));
        }
        let mut skipped_only = 0;
        let outcome = self.transaction(|d| {
            let mut total = ReplacedAll::default();
            for page in 0..d.page_count() {
                let mut next = 0usize;
                loop {
                    let runs = d.text_runs(page)?;
                    let lines = build_lines(&runs);
                    // First run, from `next` on, that holds a whole match.
                    let mut target: Option<(usize, Vec<(usize, usize)>)> = None;
                    let mut crossing = 0;
                    let mut per_run: std::collections::BTreeMap<usize, Vec<(usize, usize)>> = Default::default();
                    for line in &lines {
                        for m in matches_in_line(page, line, &query, &search) {
                            match m.within_run {
                                Some(range) if m.run >= next => per_run.entry(m.run).or_default().push(range),
                                Some(_) => {}
                                None => crossing += 1,
                            }
                        }
                    }
                    if next == 0 {
                        total.skipped += crossing;
                    }
                    if let Some((run, ranges)) = per_run.into_iter().next() {
                        target = Some((run, ranges));
                    }
                    let Some((index, mut ranges)) = target else { break };
                    ranges.sort_unstable();
                    ranges.dedup();
                    let run = &runs[index];
                    let original: Vec<char> = run.text.chars().collect();
                    let mut rebuilt = String::new();
                    let mut from = 0;
                    for &(start, end) in &ranges {
                        rebuilt.extend(&original[from..start.min(original.len())]);
                        rebuilt.push_str(replacement);
                        from = end.min(original.len());
                    }
                    rebuilt.extend(&original[from..]);
                    match d.replace(page, run, &rebuilt, replace) {
                        Ok(_) => total.replaced += ranges.len(),
                        Err(_) => total.skipped += ranges.len(),
                    }
                    // The edit may have split a run in two (a fallback font): carry on
                    // after everything the old run became.
                    let after = d.text_runs(page)?.len();
                    next = index + 1 + after.saturating_sub(runs.len());
                }
            }
            if total.replaced == 0 {
                // Nothing changed, so there is nothing to undo either.
                skipped_only = total.skipped;
                return Err(Error::Rejected(String::new()));
            }
            Ok(total)
        });
        match outcome {
            Err(Error::Rejected(m)) if m.is_empty() => Ok(ReplacedAll { replaced: 0, skipped: skipped_only }),
            other => other,
        }
    }
}

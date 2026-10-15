//! Keeping the spacing a producer wrote into a `TJ` when text inside it is edited.
//!
//! A `TJ` interleaves strings with numbers that move the pen (kerning, or the spaces
//! a justified line stretches). Writing the new text as one plain string would throw
//! those numbers away and move every letter a little, even where the text did not
//! change. So the letters that stay (the same ones at the start and at the end) keep
//! the numbers that were between them; only the middle, which is new, has none.

use lopdf::content::Operation;
use lopdf::Object;

use crate::fallback::Embedded;
use crate::font::Font;
use crate::text::TextRun;

/// One element of a `TJ` array.
enum Piece {
    Text(Vec<u8>),
    /// Thousandths of the font size the pen moves back by (negative: forward).
    Shift(f32),
}

fn pieces(op: &Operation) -> Option<Vec<Piece>> {
    let Some(Object::Array(items)) = op.operands.first() else { return None };
    items
        .iter()
        .map(|item| match item {
            Object::String(bytes, _) => Some(Piece::Text(bytes.clone())),
            other => other.as_float().ok().map(Piece::Shift),
        })
        .collect()
}

/// How far the numbers of a `TJ` move the pen in all, in points.
fn points(shift: f32, size: f32) -> f32 {
    -shift / 1000.0 * size
}

/// The pen movement of the numbers in the original operation (zero for anything but a `TJ`).
pub(crate) fn old_kerning(run: &TextRun, original: &Operation) -> f32 {
    if original.operator != "TJ" {
        return 0.0;
    }
    pieces(original).map_or(0.0, |all| {
        all.iter().map(|p| if let Piece::Shift(s) = p { points(*s, run.size) } else { 0.0 }).sum()
    })
}

/// A `TJ` that draws `new_text` where the letters that did not change at the start and at the
/// end keep their own codes and the numbers between them, with how wide it comes out.
/// `None` when nothing is kept, the text needs another font, or `original` is not a `TJ`.
pub(crate) fn keep_kerning(font: &Font, run: &TextRun, original: &Operation, new_text: &str) -> Option<(Operation, f32)> {
    if original.operator != "TJ" {
        return None;
    }
    let all = pieces(original)?;
    let step = font.code_bytes().max(1);

    // The glyphs in order, and the number (if any) in front of each (and after the last).
    let mut glyphs: Vec<(Vec<u8>, String)> = Vec::new();
    let mut shifts: Vec<Option<f32>> = vec![None];
    for piece in &all {
        match piece {
            Piece::Text(bytes) => {
                for g in font.decode(bytes) {
                    let code: Vec<u8> = (0..step).rev().map(|b| (g.code >> (8 * b)) as u8).collect();
                    glyphs.push((code, g.text));
                    shifts.push(None);
                }
            }
            Piece::Shift(s) => {
                let slot = shifts.last_mut().expect("never empty");
                *slot = Some(slot.unwrap_or(0.0) + s);
            }
        }
    }
    let n = glyphs.len();

    // Whole glyphs that start the new text, then whole glyphs that end it.
    let (mut kept_start, mut prefix) = (0, String::new());
    while kept_start < n && new_text.starts_with(&format!("{prefix}{}", glyphs[kept_start].1)) {
        prefix.push_str(&glyphs[kept_start].1);
        kept_start += 1;
    }
    let rest = &new_text[prefix.len()..];
    let (mut kept_end, mut suffix) = (0, String::new());
    while kept_start + kept_end < n && rest.ends_with(&format!("{}{suffix}", glyphs[n - 1 - kept_end].1)) {
        suffix = format!("{}{suffix}", glyphs[n - 1 - kept_end].1);
        kept_end += 1;
    }
    if kept_start == 0 && kept_end == 0 {
        return None;
    }
    let middle = &rest[..rest.len() - suffix.len()];
    let middle_bytes = if middle.is_empty() { Vec::new() } else { font.encode(middle).ok()? };

    // Assemble: the start with its numbers, the new middle, the end with its numbers.
    let mut array: Vec<Object> = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    let mut width = 0.0;
    let flush = |text: &mut Vec<u8>, array: &mut Vec<Object>, width: &mut f32| {
        if !text.is_empty() {
            *width += font.advance_of(text, run.size);
            array.push(Embedded::hex(std::mem::take(text)));
        }
    };
    let shift = |at: Option<f32>, text: &mut Vec<u8>, array: &mut Vec<Object>, width: &mut f32| {
        if let Some(s) = at {
            flush(text, array, width);
            array.push(Object::Real(s));
            *width += points(s, run.size);
        }
    };
    for (i, (code, _)) in glyphs.iter().enumerate().take(kept_start) {
        shift(shifts[i], &mut text, &mut array, &mut width); // in front of glyph i
        text.extend_from_slice(code);
    }
    text.extend_from_slice(&middle_bytes);
    for i in n - kept_end..n {
        // The number between the last new letter and the first kept one belonged to the old pair.
        if i > n - kept_end {
            shift(shifts[i], &mut text, &mut array, &mut width);
        }
        text.extend_from_slice(&glyphs[i].0);
    }
    if kept_end > 0 {
        shift(shifts[n], &mut text, &mut array, &mut width);
    }
    flush(&mut text, &mut array, &mut width);
    Some((Operation::new("TJ", vec![Object::Array(array)]), width))
}

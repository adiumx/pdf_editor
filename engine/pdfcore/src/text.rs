//! Reading a page's text: a small interpreter for the text-showing part of a
//! content stream.
//!
//! It tracks only what places and sizes glyphs — the transformation matrix and
//! its stack, the text matrix, the font and the spacing parameters — and
//! reports each shown string as a [`TextRun`] with where it starts, how far it
//! goes, and which operation of the stream drew it, so an edit can find its
//! way back to exactly that operation.

use std::collections::HashMap;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object};

use crate::font::{num, resolve, Font, Glyph};

/// `[a b c d e f]`, applied to a row vector `[x y 1]` as in the PDF spec.
pub type Matrix = [f32; 6];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` first, then `b`.
pub fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub fn apply(m: Matrix, x: f32, y: f32) -> (f32, f32) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

#[derive(Debug, Clone)]
pub struct TextRun {
    pub text: String,
    /// Name the page's resources give the font (`F1`, `TT0`...).
    pub font: String,
    pub base_font: String,
    /// Size on the page, in points, after every scaling in between.
    pub size: f32,
    /// Where the first glyph's baseline starts, in default user space.
    pub origin: (f32, f32),
    /// Where the baseline ends.
    pub end: (f32, f32),
    /// Axis-aligned box, `[x0, y0, x1, y1]`, in default user space (y up).
    pub bbox: [f32; 4],
    /// The same box as a reader shows it: page rotation applied, origin at the
    /// top-left, y down. This is the one to draw overlays with.
    pub display: [f32; 4],
    /// 0 fill, 1 stroke, 2 both, 3 invisible (the text layer of a scan).
    pub render_mode: i64,
    /// Index of the drawing operation within the page's content, and, for text
    /// inside a form XObject, which one (`None` for the page itself).
    pub op_index: usize,
    pub xobject: Option<String>,
    pub glyphs: Vec<Glyph>,
}

struct State {
    ctm: Matrix,
    tc: f32,
    tw: f32,
    th: f32,
    tl: f32,
    rise: f32,
    mode: i64,
    size: f32,
    font: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        State { ctm: IDENTITY, tc: 0.0, tw: 0.0, th: 1.0, tl: 0.0, rise: 0.0, mode: 0, size: 0.0, font: None }
    }
}

pub(crate) fn fonts_of(doc: &Document, resources: &Dictionary) -> HashMap<String, Font> {
    let mut fonts = HashMap::new();
    let Some(Object::Dictionary(table)) = resources.get(b"Font").ok().map(|o| resolve(doc, o)) else {
        return fonts;
    };
    for (name, entry) in table.iter() {
        if let Ok(dict) = resolve(doc, entry).as_dict() {
            let name = String::from_utf8_lossy(name).into_owned();
            fonts.insert(name.clone(), Font::from_dict(doc, &name, dict));
        }
    }
    fonts
}

pub(crate) fn run_page_text(doc: &Document, content: &[u8], resources: &Dictionary) -> Vec<TextRun> {
    let mut runs = Vec::new();
    interpret(doc, content, resources, IDENTITY, None, 0, &mut runs);
    runs
}

fn interpret(
    doc: &Document,
    bytes: &[u8],
    resources: &Dictionary,
    base: Matrix,
    xobject: Option<&str>,
    depth: usize,
    runs: &mut Vec<TextRun>,
) {
    let Ok(content) = Content::decode(bytes) else { return };
    let fonts = fonts_of(doc, resources);

    let mut state = State { ctm: base, ..State::default() };
    let mut stack: Vec<State> = Vec::new();
    let mut tm = IDENTITY;
    let mut tlm = IDENTITY;

    for (index, op) in content.operations.iter().enumerate() {
        let n = |i: usize| op.operands.get(i).and_then(|o| num(doc, o)).unwrap_or(0.0);
        match op.operator.as_str() {
            "q" => stack.push(State { font: state.font.clone(), ..copy_of(&state) }),
            "Q" => {
                if let Some(previous) = stack.pop() {
                    state = previous;
                }
            }
            "cm" => state.ctm = multiply([n(0), n(1), n(2), n(3), n(4), n(5)], state.ctm),
            "BT" => {
                tm = IDENTITY;
                tlm = IDENTITY;
            }
            "Tf" => {
                state.font = op.operands.first().and_then(|o| o.as_name().ok()).map(|b| String::from_utf8_lossy(b).into_owned());
                state.size = n(1);
            }
            "Tc" => state.tc = n(0),
            "Tw" => state.tw = n(0),
            "Tz" => state.th = n(0) / 100.0,
            "TL" => state.tl = n(0),
            "Ts" => state.rise = n(0),
            "Tr" => state.mode = n(0) as i64,
            "Td" => {
                tlm = multiply([1.0, 0.0, 0.0, 1.0, n(0), n(1)], tlm);
                tm = tlm;
            }
            "TD" => {
                state.tl = -n(1);
                tlm = multiply([1.0, 0.0, 0.0, 1.0, n(0), n(1)], tlm);
                tm = tlm;
            }
            "Tm" => {
                tm = [n(0), n(1), n(2), n(3), n(4), n(5)];
                tlm = tm;
            }
            "T*" => next_line(&state, &mut tm, &mut tlm),
            "Tj" => show(&state, &fonts, &mut tm, &[Piece::Text(first_string(op))], index, xobject, runs),
            "TJ" => {
                let pieces = pieces_of(doc, op);
                show(&state, &fonts, &mut tm, &pieces, index, xobject, runs);
            }
            "'" => {
                next_line(&state, &mut tm, &mut tlm);
                show(&state, &fonts, &mut tm, &[Piece::Text(first_string(op))], index, xobject, runs);
            }
            "\"" => {
                state.tw = n(0);
                state.tc = n(1);
                next_line(&state, &mut tm, &mut tlm);
                let text = op.operands.get(2).and_then(|o| o.as_str().ok()).unwrap_or_default().to_vec();
                show(&state, &fonts, &mut tm, &[Piece::Text(text)], index, xobject, runs);
            }
            "Do" if depth < 8 => {
                if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                    form_xobject(doc, resources, name, state.ctm, depth, runs);
                }
            }
            _ => {}
        }
    }
}

fn copy_of(s: &State) -> State {
    State { ctm: s.ctm, tc: s.tc, tw: s.tw, th: s.th, tl: s.tl, rise: s.rise, mode: s.mode, size: s.size, font: None }
}

fn next_line(state: &State, tm: &mut Matrix, tlm: &mut Matrix) {
    *tlm = multiply([1.0, 0.0, 0.0, 1.0, 0.0, -state.tl], *tlm);
    *tm = *tlm;
}

fn form_xobject(doc: &Document, resources: &Dictionary, name: &[u8], ctm: Matrix, depth: usize, runs: &mut Vec<TextRun>) {
    let Some(Object::Dictionary(table)) = resources.get(b"XObject").ok().map(|o| resolve(doc, o)) else { return };
    let Ok(entry) = table.get(name) else { return };
    let Ok(stream) = resolve(doc, entry).as_stream() else { return };
    if stream.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) != Some(b"Form") {
        return;
    }
    let matrix = stream
        .dict
        .get(b"Matrix")
        .ok()
        .and_then(|o| resolve(doc, o).as_array().ok())
        .map(|a| {
            let mut m = IDENTITY;
            for (i, o) in a.iter().take(6).enumerate() {
                m[i] = num(doc, o).unwrap_or(m[i]);
            }
            m
        })
        .unwrap_or(IDENTITY);
    let inner = stream
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|o| resolve(doc, o).as_dict().ok())
        .unwrap_or(resources);
    let Ok(data) = stream.decompressed_content() else { return };
    let label = String::from_utf8_lossy(name).into_owned();
    interpret(doc, &data, inner, multiply(matrix, ctm), Some(&label), depth + 1, runs);
}

enum Piece {
    Text(Vec<u8>),
    /// A TJ adjustment, in thousandths of the font size.
    Shift(f32),
}

fn first_string(op: &Operation) -> Vec<u8> {
    op.operands.first().and_then(|o| o.as_str().ok()).unwrap_or_default().to_vec()
}

fn pieces_of(doc: &Document, op: &Operation) -> Vec<Piece> {
    let Some(Ok(array)) = op.operands.first().map(|o| resolve(doc, o).as_array()) else { return Vec::new() };
    array
        .iter()
        .filter_map(|o| match o {
            Object::String(bytes, _) => Some(Piece::Text(bytes.clone())),
            other => num(doc, other).map(Piece::Shift),
        })
        .collect()
}

fn show(
    state: &State,
    fonts: &HashMap<String, Font>,
    tm: &mut Matrix,
    pieces: &[Piece],
    op_index: usize,
    xobject: Option<&str>,
    runs: &mut Vec<TextRun>,
) {
    let Some(name) = state.font.as_deref() else { return };
    let Some(font) = fonts.get(name) else { return };

    let to_page = |tm: Matrix| multiply([state.size * state.th, 0.0, 0.0, state.size, 0.0, state.rise], multiply(tm, state.ctm));
    let start = to_page(*tm);
    let mut text = String::new();
    let mut glyphs = Vec::new();

    for piece in pieces {
        match piece {
            Piece::Shift(amount) => {
                let tx = -amount / 1000.0 * state.size * state.th;
                *tm = multiply([1.0, 0.0, 0.0, 1.0, tx, 0.0], *tm);
            }
            Piece::Text(bytes) => {
                for glyph in font.decode(bytes) {
                    let spacing = state.tc + if glyph.is_space { state.tw } else { 0.0 };
                    let tx = (glyph.width / 1000.0 * state.size + spacing) * state.th;
                    *tm = multiply([1.0, 0.0, 0.0, 1.0, tx, 0.0], *tm);
                    text.push_str(&glyph.text);
                    glyphs.push(glyph);
                }
            }
        }
    }
    if glyphs.is_empty() {
        return;
    }

    let end = to_page(*tm);
    let scale = (start[2] * start[2] + start[3] * start[3]).sqrt();
    let size = scale;
    let (ox, oy) = (start[4], start[5]);
    let (ex, ey) = (end[4], end[5]);

    // Walk the four corners of the em box so a rotated run still gets a box.
    let up = (start[2] / scale.max(1e-6), start[3] / scale.max(1e-6));
    let (top, bottom) = (font.ascent / 1000.0 * size, font.descent / 1000.0 * size);
    let corners = [
        (ox + up.0 * bottom, oy + up.1 * bottom),
        (ox + up.0 * top, oy + up.1 * top),
        (ex + up.0 * bottom, ey + up.1 * bottom),
        (ex + up.0 * top, ey + up.1 * top),
    ];
    let bbox = [
        corners.iter().map(|c| c.0).fold(f32::MAX, f32::min),
        corners.iter().map(|c| c.1).fold(f32::MAX, f32::min),
        corners.iter().map(|c| c.0).fold(f32::MIN, f32::max),
        corners.iter().map(|c| c.1).fold(f32::MIN, f32::max),
    ];

    runs.push(TextRun {
        text,
        font: name.to_string(),
        base_font: font.base_font.clone(),
        size,
        origin: (ox, oy),
        end: (ex, ey),
        bbox,
        display: bbox,
        render_mode: state.mode,
        op_index,
        xobject: xobject.map(str::to_string),
        glyphs,
    });
}

/// The box that holds the four corners of `b` after `m`.
pub fn transform_box(m: Matrix, b: [f32; 4]) -> [f32; 4] {
    let corners = [apply(m, b[0], b[1]), apply(m, b[2], b[1]), apply(m, b[0], b[3]), apply(m, b[2], b[3])];
    [
        corners.iter().map(|c| c.0).fold(f32::MAX, f32::min),
        corners.iter().map(|c| c.1).fold(f32::MAX, f32::min),
        corners.iter().map(|c| c.0).fold(f32::MIN, f32::max),
        corners.iter().map(|c| c.1).fold(f32::MIN, f32::max),
    ]
}

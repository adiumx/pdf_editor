//! The vector graphics a page paints: rectangles (check boxes, table cells, frames), lines (rules,
//! underlines) and any other path, with where they are as a reader shows the page, whether they are
//! filled or stroked, and in what colour. Form XObjects are followed.
//!
//! Only painting counts: a path that ends in `n` (clipping) or is never painted is not a graphic.
//! The page's rotation and every `cm` are applied. Letters drawn as outlines are graphics too (a
//! logo converted to curves is a pile of small paths); filter by size when only boxes matter.

use lopdf::content::Content;
use lopdf::Dictionary;

use crate::error::Result;
use crate::font::{num, resolve};
use crate::text::{apply, multiply, transform_box, Matrix, IDENTITY};
use crate::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicKind {
    /// One axis-aligned rectangle (as a reader shows the page).
    Rect,
    /// One straight segment.
    Line,
    /// Anything else: curves, polygons, several pieces.
    Path,
}

impl GraphicKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GraphicKind::Rect => "rect",
            GraphicKind::Line => "line",
            GraphicKind::Path => "path",
        }
    }
}

/// A painted path.
#[derive(Debug, Clone, PartialEq)]
pub struct Graphic {
    pub kind: GraphicKind,
    /// `[x0, y0, x1, y1]` as a reader shows the page (top-left origin, y down): the box of the
    /// path itself, without the stroke's width (a curve's box is that of its control points).
    pub display: [f32; 4],
    pub filled: bool,
    pub stroked: bool,
    /// Red, green, blue in 0..=1 when the colour is a plain gray, RGB or CMYK one.
    pub fill: Option<[f32; 3]>,
    pub stroke: Option<[f32; 3]>,
    /// Width of the stroke in points as the page is shown.
    pub line_width: f32,
    /// The form XObject that paints it, if any.
    pub xobject: Option<String>,
    /// For a line: its two ends as the page is shown.
    pub ends: Option<[(f32, f32); 2]>,
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    fill: Option<[f32; 3]>,
    stroke: Option<[f32; 3]>,
    width: f32,
}

#[derive(Default)]
struct Subpath {
    points: Vec<(f32, f32)>,
    closed: bool,
    from_re: bool,
    curved: bool,
}

fn color(operator: &str, v: &[f32]) -> Option<[f32; 3]> {
    match (operator, v.len()) {
        ("g" | "G", 1) | ("sc" | "scn" | "SC" | "SCN", 1) => Some([v[0]; 3]),
        ("rg" | "RG", 3) | ("sc" | "scn" | "SC" | "SCN", 3) => Some([v[0], v[1], v[2]]),
        ("k" | "K", 4) | ("sc" | "scn" | "SC" | "SCN", 4) => {
            Some([(1.0 - v[0]) * (1.0 - v[3]), (1.0 - v[1]) * (1.0 - v[3]), (1.0 - v[2]) * (1.0 - v[3])])
        }
        _ => None,
    }
}

impl Document {
    /// Every path the page paints, in painting order, form XObjects included.
    pub fn page_graphics(&self, page: usize) -> Result<Vec<Graphic>> {
        let id = self.page_id(page)?;
        let base = self.display_matrix(page)?;
        let content = Content::decode(&self.pdf.get_page_content(id))?;
        let state = State { ctm: base, fill: Some([0.0; 3]), stroke: Some([0.0; 3]), width: 1.0 };
        let mut out = Vec::new();
        self.paint_walk(&content, &self.resources(id), state, None, 0, &mut out);
        Ok(out)
    }

    fn paint_walk(&self, content: &Content, resources: &Dictionary, mut state: State, form: Option<&str>, depth: usize, out: &mut Vec<Graphic>) {
        let mut stack: Vec<State> = Vec::new();
        let mut path: Vec<Subpath> = Vec::new();
        let mut current: Option<(f32, f32)> = None;
        for op in &content.operations {
            let v: Vec<f32> = op.operands.iter().filter_map(|o| num(&self.pdf, o)).collect();
            let user = |x: f32, y: f32| apply(state.ctm, x, y);
            match op.operator.as_str() {
                "q" => stack.push(state.clone()),
                "Q" => {
                    if let Some(s) = stack.pop() {
                        state = s;
                    }
                }
                "cm" if v.len() == 6 => state.ctm = multiply([v[0], v[1], v[2], v[3], v[4], v[5]], state.ctm),
                "w" if v.len() == 1 => state.width = v[0],
                "g" | "rg" | "k" | "sc" | "scn" => state.fill = color(&op.operator, &v),
                "G" | "RG" | "K" | "SC" | "SCN" => state.stroke = color(&op.operator, &v),
                "m" if v.len() == 2 => {
                    let p = user(v[0], v[1]);
                    path.push(Subpath { points: vec![p], ..Default::default() });
                    current = Some(p);
                }
                "l" if v.len() == 2 => {
                    let p = user(v[0], v[1]);
                    match path.last_mut() {
                        Some(s) => s.points.push(p),
                        None => path.push(Subpath { points: vec![p], ..Default::default() }),
                    }
                    current = Some(p);
                }
                "c" | "v" | "y" if !v.is_empty() => {
                    let pts: Vec<(f32, f32)> = v.chunks_exact(2).map(|c| user(c[0], c[1])).collect();
                    if let Some(s) = path.last_mut() {
                        s.points.extend(pts.iter().copied());
                        s.curved = true;
                        current = pts.last().copied();
                    }
                }
                "h" => {
                    if let Some(s) = path.last_mut() {
                        s.closed = true;
                    }
                }
                "re" if v.len() == 4 => {
                    let (x, y, w, h) = (v[0], v[1], v[2], v[3]);
                    let corners = [user(x, y), user(x + w, y), user(x + w, y + h), user(x, y + h)];
                    path.push(Subpath { points: corners.to_vec(), closed: true, from_re: true, curved: false });
                    current = Some(corners[0]);
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                    let o = op.operator.as_str();
                    let closes = matches!(o, "s" | "b" | "b*");
                    let filled = matches!(o, "f" | "F" | "f*" | "B" | "B*" | "b" | "b*");
                    let stroked = matches!(o, "S" | "s" | "B" | "B*" | "b" | "b*");
                    if closes {
                        if let Some(s) = path.last_mut() {
                            s.closed = true;
                        }
                    }
                    if let Some(g) = self.graphic_of(&path, &state, filled, stroked, form) {
                        out.push(g);
                    }
                    path.clear();
                    current = None;
                }
                "n" => {
                    path.clear();
                    current = None;
                }
                "Do" if depth < 8 => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        self.paint_form(resources, name, &state, depth, out);
                    }
                }
                _ => {}
            }
        }
        let _ = current;
    }

    fn paint_form(&self, resources: &Dictionary, name: &[u8], state: &State, depth: usize, out: &mut Vec<Graphic>) {
        let Some(table) = resources.get(b"XObject").ok().map(|o| resolve(&self.pdf, o)).and_then(|o| o.as_dict().ok()) else { return };
        let Some(stream) = table.get(name).ok().map(|o| resolve(&self.pdf, o)).and_then(|o| o.as_stream().ok()) else { return };
        if stream.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) != Some(b"Form") {
            return;
        }
        let matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|o| resolve(&self.pdf, o).as_array().ok())
            .map(|a| {
                let mut m = IDENTITY;
                for (i, o) in a.iter().take(6).enumerate() {
                    m[i] = num(&self.pdf, o).unwrap_or(m[i]);
                }
                m
            })
            .unwrap_or(IDENTITY);
        let inner = stream.dict.get(b"Resources").ok().and_then(|o| resolve(&self.pdf, o).as_dict().ok()).unwrap_or(resources);
        let Ok(data) = stream.decompressed_content() else { return };
        let Ok(content) = Content::decode(&data) else { return };
        let mut inside = state.clone();
        inside.ctm = multiply(matrix, state.ctm);
        let label = String::from_utf8_lossy(name).into_owned();
        self.paint_walk(&content, inner, inside, Some(&label), depth + 1, out);
    }

    /// The graphic a painted path makes. Points are already in display space.
    fn graphic_of(&self, path: &[Subpath], state: &State, filled: bool, stroked: bool, form: Option<&str>) -> Option<Graphic> {
        let all: Vec<(f32, f32)> = path.iter().flat_map(|s| s.points.iter().copied()).collect();
        if all.is_empty() {
            return None;
        }
        let display = transform_box(IDENTITY, [
            all.iter().map(|p| p.0).fold(f32::MAX, f32::min),
            all.iter().map(|p| p.1).fold(f32::MAX, f32::min),
            all.iter().map(|p| p.0).fold(f32::MIN, f32::max),
            all.iter().map(|p| p.1).fold(f32::MIN, f32::max),
        ]);
        let eps = 0.01;
        let mut kind = GraphicKind::Path;
        let mut ends = None;
        if let [only] = path {
            let mut pts = only.points.clone();
            if pts.len() == 5 && (pts[0].0 - pts[4].0).abs() < eps && (pts[0].1 - pts[4].1).abs() < eps {
                pts.pop();
            }
            let axis_aligned = pts.len() == 4
                && !only.curved
                && (0..4).all(|i| {
                    let (a, b) = (pts[i], pts[(i + 1) % 4]);
                    (a.0 - b.0).abs() < eps || (a.1 - b.1).abs() < eps
                });
            if axis_aligned && (only.closed || only.from_re) {
                kind = GraphicKind::Rect;
            } else if only.points.len() == 2 && !only.curved {
                kind = GraphicKind::Line;
                ends = Some([only.points[0], only.points[1]]);
            }
        }
        // Width as the page shows it: the stroke's width through the matrix's scale.
        let scale = (state.ctm[0] * state.ctm[3] - state.ctm[1] * state.ctm[2]).abs().sqrt();
        Some(Graphic {
            kind,
            display,
            filled,
            stroked,
            fill: if filled { state.fill } else { None },
            stroke: if stroked { state.stroke } else { None },
            line_width: if stroked { state.width * scale } else { 0.0 },
            xobject: form.map(str::to_string),
            ends,
        })
    }
}


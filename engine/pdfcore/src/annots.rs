//! Annotations: marks laid over a page that are not part of its content (highlights, notes, free
//! text, drawings...). Each one is added with an appearance stream, so every reader draws it, and
//! can be listed, moved, re-worded and removed.

use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::error::{Error, Result};
use crate::font::{resolve, Font};
use crate::forms::{encode_text_string, text_string};
use crate::text::{apply, invert, transform_box};
use crate::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotKind {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Note,
    FreeText,
    Ink,
    Square,
    Circle,
    Line,
    Link,
    Stamp,
    Popup,
    Widget,
    Other,
}

impl AnnotKind {
    fn from_subtype(s: &str) -> AnnotKind {
        match s {
            "Highlight" => AnnotKind::Highlight,
            "Underline" => AnnotKind::Underline,
            "StrikeOut" => AnnotKind::StrikeOut,
            "Squiggly" => AnnotKind::Squiggly,
            "Text" => AnnotKind::Note,
            "FreeText" => AnnotKind::FreeText,
            "Ink" => AnnotKind::Ink,
            "Square" => AnnotKind::Square,
            "Circle" => AnnotKind::Circle,
            "Line" => AnnotKind::Line,
            "Link" => AnnotKind::Link,
            "Stamp" => AnnotKind::Stamp,
            "Popup" => AnnotKind::Popup,
            "Widget" => AnnotKind::Widget,
            _ => AnnotKind::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AnnotKind::Highlight => "highlight",
            AnnotKind::Underline => "underline",
            AnnotKind::StrikeOut => "strike_out",
            AnnotKind::Squiggly => "squiggly",
            AnnotKind::Note => "note",
            AnnotKind::FreeText => "free_text",
            AnnotKind::Ink => "ink",
            AnnotKind::Square => "square",
            AnnotKind::Circle => "circle",
            AnnotKind::Line => "line",
            AnnotKind::Link => "link",
            AnnotKind::Stamp => "stamp",
            AnnotKind::Popup => "popup",
            AnnotKind::Widget => "widget",
            AnnotKind::Other => "other",
        }
    }
}

/// An annotation of a page. Read the page's annotations again after any edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// Position in the page's annotation list.
    pub index: usize,
    pub kind: AnnotKind,
    /// The PDF subtype (`Highlight`, `Text`...).
    pub subtype: String,
    /// `[x0, y0, x1, y1]` as a reader shows the page.
    pub display: [f32; 4],
    pub contents: String,
    pub author: String,
    pub color: Option<[f32; 3]>,
    /// For text markup: the boxes it covers, as a reader shows the page.
    pub quads: Vec<[f32; 4]>,
    /// It carries a drawing, which is what a reader shows.
    pub has_appearance: bool,
    /// Flagged hidden or not for viewing: a reader does not show it.
    pub hidden: bool,
}

/// What to add.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Text markup over these boxes (as a reader shows the page).
    Highlight(Vec<[f32; 4]>),
    Underline(Vec<[f32; 4]>),
    StrikeOut(Vec<[f32; 4]>),
    Squiggly(Vec<[f32; 4]>),
    /// A sticky note whose icon has its top-left corner here.
    Note { at: (f32, f32) },
    /// Text in a box, in Helvetica at `size` points.
    FreeText { rect: [f32; 4], size: f32 },
    /// Freehand strokes, each a list of points.
    Ink { strokes: Vec<Vec<(f32, f32)>>, width: f32 },
    Square { rect: [f32; 4], fill: Option<[f32; 3]>, width: f32 },
    Circle { rect: [f32; 4], fill: Option<[f32; 3]>, width: f32 },
    Line { from: (f32, f32), to: (f32, f32), width: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewAnnotation {
    pub shape: Shape,
    /// Stroke, markup or text colour, red, green, blue in 0..=1.
    pub color: [f32; 3],
    /// The note's text, or the free text itself.
    pub contents: String,
    pub author: String,
    /// 0..=1.
    pub opacity: f32,
}

impl NewAnnotation {
    pub fn new(shape: Shape, color: [f32; 3]) -> NewAnnotation {
        NewAnnotation { shape, color, contents: String::new(), author: String::new(), opacity: 1.0 }
    }
}

fn reals(values: &[f32]) -> Object {
    Object::Array(values.iter().map(|v| Object::Real(*v)).collect())
}

fn name(n: &str) -> Object {
    Object::Name(n.as_bytes().to_vec())
}

fn boxed(points: &[(f32, f32)]) -> [f32; 4] {
    points.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)])
}

fn rgb(c: [f32; 3]) -> String {
    format!("{:.3} {:.3} {:.3}", c[0], c[1], c[2])
}

impl Document {
    /// The annotation objects a page lists, with their position in the list.
    pub(crate) fn annotation_refs(&self, page: ObjectId) -> Vec<(usize, ObjectId)> {
        let Ok(dict) = self.pdf.get_dictionary(page) else { return Vec::new() };
        let Some(Object::Array(list)) = dict.get(b"Annots").ok().map(|o| resolve(&self.pdf, o)) else { return Vec::new() };
        list.iter().enumerate().filter_map(|(i, o)| o.as_reference().ok().map(|r| (i, r))).collect()
    }

    /// The annotations of a page, in the order the page lists them.
    pub fn annotations(&self, page: usize) -> Result<Vec<Annotation>> {
        let id = self.page_id(page)?;
        let to_display = self.display_matrix(page)?;
        let mut out = Vec::new();
        for (index, object) in self.annotation_refs(id) {
            let Ok(dict) = self.pdf.get_dictionary(object) else { continue };
            let subtype = dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default();
            let numbers = |key: &[u8]| -> Vec<f32> {
                dict.get(key)
                    .ok()
                    .map(|o| resolve(&self.pdf, o))
                    .and_then(|o| o.as_array().ok())
                    .map(|a| a.iter().filter_map(|n| crate::font::num(&self.pdf, n)).collect())
                    .unwrap_or_default()
            };
            let rect = numbers(b"Rect");
            if rect.len() != 4 {
                continue;
            }
            let display = transform_box(to_display, [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])]);
            let quads: Vec<[f32; 4]> = numbers(b"QuadPoints")
                .chunks_exact(8)
                .map(|q| {
                    let pts: Vec<(f32, f32)> = (0..4).map(|k| apply(to_display, q[2 * k], q[2 * k + 1])).collect();
                    boxed(&pts)
                })
                .collect();
            let color = match numbers(b"C").as_slice() {
                [g] => Some([*g; 3]),
                [r, g, b] => Some([*r, *g, *b]),
                [c, m, y, k] => Some([(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]),
                _ => None,
            };
            let text_of = |key: &[u8]| dict.get(key).ok().map(|o| resolve(&self.pdf, o)).and_then(|o| o.as_str().ok()).map(text_string).unwrap_or_default();
            let flags = dict.get(b"F").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
            let has_appearance = dict
                .get(b"AP")
                .ok()
                .map(|o| resolve(&self.pdf, o))
                .and_then(|o| o.as_dict().ok())
                .is_some_and(|ap| ap.has(b"N"));
            out.push(Annotation {
                index,
                kind: AnnotKind::from_subtype(&subtype),
                subtype,
                display,
                contents: text_of(b"Contents"),
                author: text_of(b"T"),
                color,
                quads,
                has_appearance,
                hidden: flags & (2 | 32) != 0,
            });
        }
        Ok(out)
    }

    /// Add an annotation with its appearance and return its position in the page's list. One step.
    pub fn add_annotation(&mut self, page: usize, new: &NewAnnotation) -> Result<usize> {
        self.transaction(|d| d.create_annotation(page, new))
    }

    fn create_annotation(&mut self, page: usize, new: &NewAnnotation) -> Result<usize> {
        let id = self.page_id(page)?;
        let to_user = invert(self.display_matrix(page)?).ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        let user = |p: (f32, f32)| apply(to_user, p.0, p.1);
        let ubox = |b: [f32; 4]| transform_box(to_user, b);
        let opacity = new.opacity.clamp(0.0, 1.0);
        let mut dict = Dictionary::new();
        dict.set("Type", name("Annot"));
        dict.set("F", 4);
        dict.set("C", reals(&new.color));
        if opacity < 1.0 {
            dict.set("CA", Object::Real(opacity));
        }
        if !new.contents.is_empty() {
            dict.set("Contents", encode_text_string(&new.contents));
        }
        if !new.author.is_empty() {
            dict.set("T", encode_text_string(&new.author));
        }
        dict.set("P", Object::Reference(id));
        dict.set("NM", Object::string_literal(format!("pdfcore-{}", self.pdf.max_id + 1)));

        let mut resources = Dictionary::new();
        let mut gs = Dictionary::new();
        gs.set("Type", name("ExtGState"));
        gs.set("CA", Object::Real(opacity));
        gs.set("ca", Object::Real(opacity));
        let colour = rgb(new.color);
        let (rect, content): ([f32; 4], String) = match &new.shape {
            Shape::Highlight(boxes) | Shape::Underline(boxes) | Shape::StrikeOut(boxes) | Shape::Squiggly(boxes) => {
                if boxes.is_empty() {
                    return Err(Error::Rejected("hace falta al menos una caja que marcar".into()));
                }
                let subtype = match &new.shape {
                    Shape::Highlight(_) => "Highlight",
                    Shape::Underline(_) => "Underline",
                    Shape::StrikeOut(_) => "StrikeOut",
                    _ => "Squiggly",
                };
                dict.set("Subtype", name(subtype));
                let user_boxes: Vec<[f32; 4]> = boxes.iter().map(|b| ubox(*b)).collect();
                // Corners in the order readers expect: top-left, top-right, bottom-left, bottom-right.
                let quads: Vec<f32> = user_boxes.iter().flat_map(|b| [b[0], b[3], b[2], b[3], b[0], b[1], b[2], b[1]]).collect();
                dict.set("QuadPoints", reals(&quads));
                let all: Vec<(f32, f32)> = user_boxes.iter().flat_map(|b| [(b[0], b[1]), (b[2], b[3])]).collect();
                let rect = boxed(&all);
                let mut c = String::from("/GS gs\n");
                match subtype {
                    "Highlight" => {
                        gs.set("BM", name("Multiply"));
                        c.push_str(&format!("{colour} rg\n"));
                        for b in &user_boxes {
                            c.push_str(&format!("{:.2} {:.2} {:.2} {:.2} re f\n", b[0], b[1], b[2] - b[0], b[3] - b[1]));
                        }
                    }
                    "Underline" | "StrikeOut" => {
                        c.push_str(&format!("{colour} RG\n0.8 w\n"));
                        for b in &user_boxes {
                            let y = if subtype == "Underline" { b[1] + 0.8 } else { (b[1] + b[3]) / 2.0 };
                            c.push_str(&format!("{:.2} {:.2} m {:.2} {:.2} l S\n", b[0], y, b[2], y));
                        }
                    }
                    _ => {
                        c.push_str(&format!("{colour} RG\n0.6 w\n"));
                        for b in &user_boxes {
                            let (mut x, mut up) = (b[0], false);
                            c.push_str(&format!("{:.2} {:.2} m\n", x, b[1] + 0.5));
                            while x < b[2] {
                                x = (x + 2.0).min(b[2]);
                                up = !up;
                                c.push_str(&format!("{:.2} {:.2} l\n", x, b[1] + if up { 2.5 } else { 0.5 }));
                            }
                            c.push_str("S\n");
                        }
                    }
                }
                (rect, c)
            }
            Shape::Note { at } => {
                dict.set("Subtype", name("Text"));
                dict.set("Name", name("Note"));
                dict.set("Open", Object::Boolean(false));
                dict.set("F", 28);
                let (x, y) = user(*at);
                let rect = [x, y - 18.0, x + 18.0, y];
                let (x0, y0) = (rect[0], rect[1]);
                let mut c = String::from("/GS gs\n");
                c.push_str(&format!("{colour} rg 0.2 0.2 0.2 RG 0.8 w\n{:.2} {:.2} 17 17 re B\n", x0 + 0.5, y0 + 0.5));
                c.push_str("0.2 0.2 0.2 RG 0.7 w\n");
                for k in 0..3 {
                    c.push_str(&format!("{:.2} {:.2} m {:.2} {:.2} l S\n", x0 + 4.0, y0 + 5.0 + 3.5 * k as f32, x0 + 14.0, y0 + 5.0 + 3.5 * k as f32));
                }
                (rect, c)
            }
            Shape::FreeText { rect, size } => {
                if new.contents.trim().is_empty() {
                    return Err(Error::Rejected("el texto está vacío".into()));
                }
                dict.set("Subtype", name("FreeText"));
                let r = ubox(*rect);
                dict.set("DA", Object::string_literal(format!("/Helv {size:.1} Tf {colour} rg")));
                let mut font = Dictionary::new();
                font.set("Type", name("Font"));
                font.set("Subtype", name("Type1"));
                font.set("BaseFont", name("Helvetica"));
                font.set("Encoding", name("WinAnsiEncoding"));
                let helv = Font::from_dict(&self.pdf, "Helv", &font);
                let width = (r[2] - r[0] - 4.0).max(1.0);
                let mut c = format!("/GS gs\nBT\n{colour} rg\n/Helv {size:.2} Tf\n");
                let mut baseline = r[3] - 2.0 - size * 0.85;
                let mut missing: Vec<char> = Vec::new();
                'paragraphs: for paragraph in new.contents.split('\n') {
                    let mut line = String::new();
                    let words: Vec<&str> = paragraph.split(' ').collect();
                    let flush = |line: &str, baseline: &mut f32, c: &mut String| {
                        if !line.is_empty() {
                            if let Ok(bytes) = helv.encode(line) {
                                let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
                                c.push_str(&format!("1 0 0 1 {:.2} {:.2} Tm <{hex}> Tj\n", r[0] + 2.0, *baseline));
                            }
                        }
                        *baseline -= size * 1.2;
                    };
                    for word in words {
                        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                        match helv.encode(&candidate) {
                            Ok(bytes) if helv.advance_of(&bytes, *size) <= width || line.is_empty() => line = candidate,
                            Ok(_) => {
                                flush(&line, &mut baseline, &mut c);
                                line = word.to_string();
                            }
                            Err(m) => {
                                missing.extend(m);
                                continue 'paragraphs;
                            }
                        }
                    }
                    flush(&line, &mut baseline, &mut c);
                }
                if !missing.is_empty() {
                    missing.sort_unstable();
                    missing.dedup();
                    return Err(Error::MissingGlyphs(missing));
                }
                c.push_str("ET\n");
                let mut fonts = Dictionary::new();
                fonts.set("Helv", Object::Dictionary(font));
                resources.set("Font", Object::Dictionary(fonts));
                (r, c)
            }
            Shape::Ink { strokes, width } => {
                if strokes.iter().all(|s| s.len() < 2) {
                    return Err(Error::Rejected("el trazo necesita al menos dos puntos".into()));
                }
                dict.set("Subtype", name("Ink"));
                let user_strokes: Vec<Vec<(f32, f32)>> = strokes.iter().map(|s| s.iter().map(|p| user(*p)).collect()).collect();
                dict.set(
                    "InkList",
                    Object::Array(user_strokes.iter().map(|s| reals(&s.iter().flat_map(|p| [p.0, p.1]).collect::<Vec<_>>())).collect()),
                );
                dict.set("BS", Object::Dictionary(border(*width)));
                let all: Vec<(f32, f32)> = user_strokes.iter().flatten().copied().collect();
                let b = boxed(&all);
                let pad = width + 1.0;
                let mut c = format!("/GS gs\n{colour} RG\n{width:.2} w 1 J 1 j\n");
                for s in &user_strokes {
                    for (i, p) in s.iter().enumerate() {
                        c.push_str(&format!("{:.2} {:.2} {}\n", p.0, p.1, if i == 0 { "m" } else { "l" }));
                    }
                    c.push_str("S\n");
                }
                ([b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad], c)
            }
            Shape::Square { rect, fill, width } | Shape::Circle { rect, fill, width } => {
                let circle = matches!(new.shape, Shape::Circle { .. });
                dict.set("Subtype", name(if circle { "Circle" } else { "Square" }));
                dict.set("BS", Object::Dictionary(border(*width)));
                if let Some(f) = fill {
                    dict.set("IC", reals(f));
                }
                let r = ubox(*rect);
                let (x0, y0, x1, y1) = (r[0] + width / 2.0, r[1] + width / 2.0, r[2] - width / 2.0, r[3] - width / 2.0);
                let mut c = format!("/GS gs\n{colour} RG\n{width:.2} w\n");
                if let Some(f) = fill {
                    c.push_str(&format!("{} rg\n", rgb(*f)));
                }
                let paint = if fill.is_some() { "B" } else { "S" };
                if circle {
                    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
                    let k = 0.5523;
                    c.push_str(&format!(
                        "{:.2} {:.2} m\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n{paint}\n",
                        cx + rx, cy,
                        cx + rx, cy + k * ry, cx + k * rx, cy + ry, cx, cy + ry,
                        cx - k * rx, cy + ry, cx - rx, cy + k * ry, cx - rx, cy,
                        cx - rx, cy - k * ry, cx - k * rx, cy - ry, cx, cy - ry,
                        cx + k * rx, cy - ry, cx + rx, cy - k * ry, cx + rx, cy
                    ));
                } else {
                    c.push_str(&format!("{x0:.2} {y0:.2} {:.2} {:.2} re {paint}\n", x1 - x0, y1 - y0));
                }
                (r, c)
            }
            Shape::Line { from, to, width } => {
                dict.set("Subtype", name("Line"));
                let (a, b) = (user(*from), user(*to));
                dict.set("L", reals(&[a.0, a.1, b.0, b.1]));
                dict.set("BS", Object::Dictionary(border(*width)));
                let r = boxed(&[a, b]);
                let pad = width + 1.0;
                let c = format!("/GS gs\n{colour} RG\n{width:.2} w 1 J\n{:.2} {:.2} m {:.2} {:.2} l S\n", a.0, a.1, b.0, b.1);
                ([r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad], c)
            }
        };
        dict.set("Rect", reals(&rect));
        let mut states = Dictionary::new();
        states.set("GS", Object::Dictionary(gs));
        resources.set("ExtGState", Object::Dictionary(states));
        let mut form = Dictionary::new();
        form.set("Type", name("XObject"));
        form.set("Subtype", name("Form"));
        form.set("BBox", reals(&rect));
        form.set("Resources", Object::Dictionary(resources));
        let mut stream = Stream::new(form, content.into_bytes());
        let _ = stream.compress();
        let ap_stream = self.pdf.add_object(stream);
        let mut ap = Dictionary::new();
        ap.set("N", Object::Reference(ap_stream));
        dict.set("AP", Object::Dictionary(ap));
        let annotation = self.pdf.add_object(dict);
        self.push_annotation(id, annotation)
    }

    /// List `annotation` among the page's annotations; returns its position.
    fn push_annotation(&mut self, page: ObjectId, annotation: ObjectId) -> Result<usize> {
        match self.pdf.get_dictionary(page)?.get(b"Annots").ok().cloned() {
            Some(Object::Reference(r)) => {
                if let Ok(Object::Array(a)) = self.pdf.get_object_mut(r) {
                    a.push(Object::Reference(annotation));
                    return Ok(a.len() - 1);
                }
            }
            Some(Object::Array(mut a)) => {
                a.push(Object::Reference(annotation));
                let n = a.len() - 1;
                self.pdf.get_dictionary_mut(page)?.set("Annots", Object::Array(a));
                return Ok(n);
            }
            _ => {}
        }
        self.pdf.get_dictionary_mut(page)?.set("Annots", Object::Array(vec![Object::Reference(annotation)]));
        Ok(0)
    }

    fn annotation_object(&self, page: usize, index: usize) -> Result<(ObjectId, ObjectId)> {
        let id = self.page_id(page)?;
        let (_, object) = self
            .annotation_refs(id)
            .into_iter()
            .find(|(i, _)| *i == index)
            .ok_or_else(|| Error::Rejected("esa anotación ya no está donde se leyó: vuelve a leer la página".into()))?;
        Ok((id, object))
    }

    /// Remove an annotation (and its pop-up) from the page. One step.
    pub fn delete_annotation(&mut self, page: usize, index: usize) -> Result<()> {
        self.transaction(|d| {
            let (id, object) = d.annotation_object(page, index)?;
            let popup = d.pdf.get_dictionary(object).ok().and_then(|a| a.get(b"Popup").ok()).and_then(|o| o.as_reference().ok());
            d.remove_reference(id, b"Annots", object)?;
            if let Some(p) = popup {
                d.remove_reference(id, b"Annots", p)?;
            }
            Ok(())
        })
    }

    /// Change what a note says (or the text of a free-text box, which is drawn again). One step.
    pub fn set_annotation_contents(&mut self, page: usize, index: usize, contents: &str) -> Result<()> {
        self.transaction(|d| {
            let (_, object) = d.annotation_object(page, index)?;
            let kind = d
                .pdf
                .get_dictionary(object)?
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| AnnotKind::from_subtype(&String::from_utf8_lossy(n)))
                .unwrap_or(AnnotKind::Other);
            if kind == AnnotKind::FreeText {
                // Drawn again from the new words: same box, size and colour, new look.
                let existing = d.annotations(page)?.into_iter().find(|a| a.index == index).ok_or_else(|| Error::Rejected("esa anotación ya no está".into()))?;
                let dict = d.pdf.get_dictionary(object)?.clone();
                let size = dict
                    .get(b"DA")
                    .ok()
                    .and_then(|o| o.as_str().ok())
                    .and_then(|s| {
                        let text = String::from_utf8_lossy(s).into_owned();
                        let parts: Vec<&str> = text.split_whitespace().collect();
                        parts.iter().position(|p| *p == "Tf").and_then(|i| i.checked_sub(1)).and_then(|i| parts[i].parse::<f32>().ok())
                    })
                    .unwrap_or(12.0);
                let replacement = NewAnnotation {
                    shape: Shape::FreeText { rect: existing.display, size },
                    color: existing.color.unwrap_or([0.0, 0.0, 0.0]),
                    contents: contents.to_string(),
                    author: existing.author.clone(),
                    opacity: dict.get(b"CA").ok().and_then(|o| o.as_float().ok()).unwrap_or(1.0),
                };
                // Check the words can be drawn before touching anything.
                let fresh = d.create_annotation(page, &replacement)?;
                let id = d.page_id(page)?;
                let (_, new_object) = d.annotation_object(page, fresh)?;
                let ap = d.pdf.get_dictionary(new_object)?.get(b"AP").ok().cloned();
                d.remove_reference(id, b"Annots", new_object)?;
                let target = d.pdf.get_dictionary_mut(object)?;
                if let Some(ap) = ap {
                    target.set("AP", ap);
                }
            }
            d.pdf.get_dictionary_mut(object)?.set("Contents", encode_text_string(contents));
            Ok(())
        })
    }

    /// Move an annotation by `(dx, dy)` points as a reader shows the page. One step.
    pub fn move_annotation(&mut self, page: usize, index: usize, dx: f32, dy: f32) -> Result<()> {
        self.transaction(|d| {
            let (_, object) = d.annotation_object(page, index)?;
            let (ux, uy) = d.user_delta(page, dx, dy)?;
            let mut dict = d.pdf.get_dictionary(object)?.clone();
            let shifted = |dict: &Dictionary, key: &[u8], pairs: bool| -> Option<Object> {
                let list = dict.get(key).ok()?.as_array().ok()?;
                let mut out = Vec::new();
                for (i, n) in list.iter().enumerate() {
                    let v = n.as_float().ok()?;
                    out.push(Object::Real(v + if i % 2 == 0 { ux } else { uy }));
                }
                let _ = pairs;
                Some(Object::Array(out))
            };
            for key in [&b"Rect"[..], b"QuadPoints", b"L"] {
                if let Some(moved) = shifted(&dict, key, true) {
                    dict.set(key, moved);
                }
            }
            if let Ok(Object::Array(strokes)) = dict.get(b"InkList").cloned() {
                let moved: Vec<Object> = strokes
                    .iter()
                    .map(|s| {
                        let mut holder = Dictionary::new();
                        holder.set("S", s.clone());
                        shifted(&holder, b"S", true).unwrap_or_else(|| s.clone())
                    })
                    .collect();
                dict.set("InkList", Object::Array(moved));
            }
            *d.pdf.get_object_mut(object)? = Object::Dictionary(dict);
            Ok(())
        })
    }
}

fn border(width: f32) -> Dictionary {
    let mut bs = Dictionary::new();
    bs.set("W", Object::Real(width));
    bs.set("S", name("S"));
    bs
}

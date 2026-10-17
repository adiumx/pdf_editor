//! Writing new text onto a page, anywhere, in a bundled font.
//!
//! For documents with nothing to fill in: lines drawn as underscores, a blank
//! margin, a box with no field behind it. The text is appended to the page as
//! content of its own, inside a `q`/`Q` pair so whatever state the page left
//! behind (a moved origin, a scale) cannot move it.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, Stream};
use ttf_parser::Face;

use crate::error::{Error, Result};
use crate::fallback::{self, Embedded};
use crate::text::{apply, invert};
use crate::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// What `x, y` mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The top-left corner of the text's box: where a finger or a click lands.
    TopLeft,
    /// The start of the first line's baseline: to write *on* a drawn line.
    Baseline,
}

#[derive(Debug, Clone)]
pub struct NewText<'a> {
    pub text: &'a str,
    /// Position in the page as a reader shows it (origin top-left, y down).
    pub x: f32,
    pub y: f32,
    pub anchor: Anchor,
    pub size: f32,
    /// Red, green, blue, each 0..=1.
    pub color: [f32; 3],
    pub serif: bool,
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    /// Wrap lines wider than this; `None` keeps only the line breaks in `text`.
    pub max_width: Option<f32>,
}

impl<'a> NewText<'a> {
    pub fn new(text: &'a str, x: f32, y: f32) -> Self {
        NewText {
            text, x, y, anchor: Anchor::TopLeft, size: 12.0, color: [0.0, 0.0, 0.0],
            serif: false, bold: false, italic: false, align: Align::Left, max_width: None,
        }
    }
}

/// One stretch of a line with a style of its own.
#[derive(Debug, Clone)]
pub struct StyledSpan<'a> {
    pub text: &'a str,
    pub size: f32,
    /// Red, green, blue, each 0..=1.
    pub color: [f32; 3],
    pub serif: bool,
    pub bold: bool,
    pub italic: bool,
}

impl<'a> StyledSpan<'a> {
    pub fn new(text: &'a str) -> Self {
        StyledSpan { text, size: 12.0, color: [0.0, 0.0, 0.0], serif: false, bold: false, italic: false }
    }
}

/// Text made of spans of different styles, set on common baselines. Where it goes,
/// how it aligns and wraps works as in [`NewText`].
#[derive(Debug, Clone)]
pub struct NewSpans<'a> {
    pub spans: Vec<StyledSpan<'a>>,
    pub x: f32,
    pub y: f32,
    pub anchor: Anchor,
    pub align: Align,
    pub max_width: Option<f32>,
}

impl<'a> NewSpans<'a> {
    pub fn new(spans: Vec<StyledSpan<'a>>, x: f32, y: f32) -> Self {
        NewSpans { spans, x, y, anchor: Anchor::TopLeft, align: Align::Left, max_width: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Added {
    /// Box the text occupies, as a reader shows the page.
    pub display: [f32; 4],
    pub lines: usize,
}

const LINE_HEIGHT: f32 = 1.2;

/// A piece of a line: part of one span, drawn in one font and size.
struct Piece {
    span: usize,
    text: String,
    width: f32,
}

/// A span's font, resolved once.
struct Look {
    key: &'static str,
    data: &'static [u8],
    ascent: f32,
    descent: f32,
}

impl Document {
    pub fn add_text(&mut self, page: usize, new: &NewText) -> Result<Added> {
        let span = StyledSpan { text: new.text, size: new.size, color: new.color, serif: new.serif, bold: new.bold, italic: new.italic };
        let spans = NewSpans { spans: vec![span], x: new.x, y: new.y, anchor: new.anchor, align: new.align, max_width: new.max_width };
        self.transaction(|d| d.put_spans(page, &spans))
    }

    /// Add text of several styles in one go (one undo step). Every span starts where the
    /// one before ends, measured with the widths of the font that is embedded, and all of
    /// a line share its baseline.
    pub fn add_spans(&mut self, page: usize, new: &NewSpans) -> Result<Added> {
        self.transaction(|d| d.put_spans(page, new))
    }

    fn put_spans(&mut self, page: usize, new: &NewSpans) -> Result<Added> {
        if new.spans.iter().all(|s| s.text.trim().is_empty()) {
            return Err(Error::Unsupported("el texto nuevo está vacío".into()));
        }
        if new.spans.iter().any(|s| !(s.size > 0.0)) {
            return Err(Error::Unsupported("el tamaño debe ser mayor que cero".into()));
        }
        let id = self.page_id(page)?;

        // The font of every span, and the letters it cannot draw.
        let mut looks: Vec<Look> = Vec::new();
        let mut lacking: Vec<char> = Vec::new();
        for s in &new.spans {
            let (key, data) = fallback::by_style(s.serif, s.bold, s.italic);
            for c in fallback::lacks(data, &s.text.replace(['\n', '\r'], "")) {
                if !lacking.contains(&c) {
                    lacking.push(c);
                }
            }
            let face = Face::parse(data, 0).map_err(|e| Error::Unsupported(e.to_string()))?;
            let scale = 1.0 / face.units_per_em() as f32;
            looks.push(Look { key, data, ascent: face.ascender() as f32 * scale, descent: -face.descender() as f32 * scale });
        }
        if !lacking.is_empty() {
            return Err(Error::MissingGlyphs(lacking));
        }

        // Paragraphs: the spans cut at each line break. An empty one keeps the span it ended in.
        let mut paragraphs: Vec<(Vec<(usize, String)>, usize)> = vec![(Vec::new(), 0)];
        for (i, s) in new.spans.iter().enumerate() {
            for (n, part) in s.text.split('\n').enumerate() {
                if n > 0 {
                    paragraphs.push((Vec::new(), i));
                }
                let part = part.trim_end_matches('\r');
                let last = paragraphs.last_mut().expect("never empty");
                last.1 = i;
                if !part.is_empty() {
                    last.0.push((i, part.to_string()));
                }
            }
        }

        // Lines, wrapped between words when there is a width to wrap to. A word is
        // not cut where the style changes inside it.
        let width_of = |span: usize, t: &str| fallback::advance(looks[span].data, t, new.spans[span].size);
        let mut lines: Vec<(Vec<Piece>, usize)> = Vec::new();
        for (segments, owner) in &paragraphs {
            let Some(limit) = new.max_width else {
                let pieces = segments.iter().map(|(i, t)| Piece { span: *i, text: t.clone(), width: width_of(*i, t) }).collect();
                lines.push((pieces, *owner));
                continue;
            };
            let (mut line, mut line_w): (Vec<Piece>, f32) = (Vec::new(), 0.0);
            let (mut gap, mut gap_w): (Vec<Piece>, f32) = (Vec::new(), 0.0);
            let mut word: Vec<Piece> = Vec::new();
            let flush = |word: &mut Vec<Piece>, gap: &mut Vec<Piece>, gap_w: &mut f32, line: &mut Vec<Piece>, line_w: &mut f32, lines: &mut Vec<(Vec<Piece>, usize)>| {
                if word.is_empty() {
                    return;
                }
                let w: f32 = word.iter().map(|p| p.width).sum();
                if !line.is_empty() && *line_w + *gap_w + w > limit {
                    lines.push((std::mem::take(line), *owner));
                    *line_w = 0.0;
                    gap.clear();
                    *gap_w = 0.0;
                }
                *line_w += *gap_w + w;
                line.append(gap);
                *gap_w = 0.0;
                line.append(word);
            };
            for (i, t) in segments {
                // Alternate runs of letters and runs of spaces.
                let mut rest = t.as_str();
                while !rest.is_empty() {
                    let space = rest.starts_with(' ');
                    let end = rest.find(|c: char| (c == ' ') != space).unwrap_or(rest.len());
                    let (chunk, tail) = rest.split_at(end);
                    rest = tail;
                    let piece = Piece { span: *i, text: chunk.to_string(), width: width_of(*i, chunk) };
                    if space {
                        flush(&mut word, &mut gap, &mut gap_w, &mut line, &mut line_w, &mut lines);
                        gap_w += piece.width;
                        gap.push(piece);
                    } else {
                        word.push(piece);
                    }
                }
            }
            flush(&mut word, &mut gap, &mut gap_w, &mut line, &mut line_w, &mut lines);
            lines.push((line, *owner));
        }

        let widths: Vec<f32> = lines.iter().map(|(l, _)| l.iter().map(|p| p.width).sum()).collect();
        let box_width = new.max_width.unwrap_or_else(|| widths.iter().cloned().fold(0.0, f32::max));
        // What each line is made of: tallest ascent and descent, and the leading from its largest size.
        let metrics: Vec<(f32, f32, f32)> = lines
            .iter()
            .map(|(l, owner)| {
                let spans: Vec<usize> = if l.is_empty() { vec![*owner] } else { l.iter().map(|p| p.span).collect() };
                let up = spans.iter().map(|&i| looks[i].ascent * new.spans[i].size).fold(0.0, f32::max);
                let down = spans.iter().map(|&i| looks[i].descent * new.spans[i].size).fold(0.0, f32::max);
                let lead = spans.iter().map(|&i| new.spans[i].size * LINE_HEIGHT).fold(0.0, f32::max);
                (up, down, lead)
            })
            .collect();

        // Where the first baseline goes, in the page as shown.
        let top = match new.anchor {
            Anchor::TopLeft => new.y,
            Anchor::Baseline => new.y - metrics[0].0,
        };
        let mut baselines = Vec::with_capacity(lines.len());
        let mut at = top + metrics[0].0;
        for (i, m) in metrics.iter().enumerate() {
            if i > 0 {
                at += m.2;
            }
            baselines.push(at);
        }

        // Display space back to the page's own: the inverse of what readers apply.
        let to_display = self.display_matrix(page)?;
        let inverse = invert(to_display).ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        let right = direction(inverse, 1.0, 0.0); // display "along the line"
        let up = direction(inverse, 0.0, -1.0); // display "up the glyphs"

        let mut keys: Vec<&'static str> = Vec::new();
        for l in &looks {
            if !keys.contains(&l.key) {
                self.ensure_fallback(l.key, l.data)?;
                keys.push(l.key);
            }
        }

        let first_color = lines.iter().flat_map(|(l, _)| l.iter()).next().map(|p| new.spans[p.span].color).unwrap_or([0.0; 3]);
        let color_op = |c: [f32; 3]| Operation::new("rg", c.iter().map(|v| Object::Real(*v)).collect());
        let mut ops = vec![Operation::new("q", vec![]), color_op(first_color), Operation::new("BT", vec![])];
        let (mut color, mut font): ([f32; 3], Option<(&str, f32)>) = (first_color, None);
        for (i, (line, _)) in lines.iter().enumerate() {
            let offset = match new.align {
                Align::Left => 0.0,
                Align::Center => (box_width - widths[i]) / 2.0,
                Align::Right => box_width - widths[i],
            };
            let mut pen = new.x + offset;
            // Neighbours of one span go out as one string, spaces included, so a reader
            // extracts the words with their spaces.
            let mut joined: Vec<Piece> = Vec::new();
            for piece in line {
                match joined.last_mut() {
                    Some(last) if last.span == piece.span => {
                        last.text.push_str(&piece.text);
                        last.width += piece.width;
                    }
                    _ => joined.push(Piece { span: piece.span, text: piece.text.clone(), width: piece.width }),
                }
            }
            for piece in &joined {
                let (span, look) = (&new.spans[piece.span], &looks[piece.span]);
                if span.color != color {
                    ops.push(color_op(span.color));
                    color = span.color;
                }
                if font != Some((look.key, span.size)) {
                    let resource = self.fallbacks[look.key].resource.clone();
                    ops.push(Operation::new("Tf", vec![Object::Name(resource.into_bytes()), Object::Real(span.size)]));
                    font = Some((look.key, span.size));
                }
                let (ox, oy) = apply(inverse, pen, baselines[i]);
                let embedded = self.fallbacks.get_mut(look.key).expect("just ensured");
                let (bytes, _) = embedded.write(&mut self.pdf, &piece.text, span.size)?;
                ops.push(Operation::new("Tm", [right.0, right.1, up.0, up.1, ox, oy].iter().map(|v| Object::Real(*v)).collect()));
                ops.push(Operation::new("Tj", vec![Embedded::hex(bytes)]));
                pen += piece.width;
            }
        }
        ops.push(Operation::new("ET", vec![]));
        ops.push(Operation::new("Q", vec![]));

        for key in keys {
            let (resource, font_id) = (self.fallbacks[key].resource.clone(), self.fallbacks[key].font_id);
            self.add_font_resource(id, &resource, font_id)?;
        }
        self.append_to_page(id, &Content { operations: ops })?;

        let bottom = baselines.last().copied().unwrap_or(top) + metrics.last().map_or(0.0, |m| m.1);
        Ok(Added { display: [new.x, top, new.x + box_width, bottom], lines: lines.len() })
    }

    /// Add content after what the page draws, once the page's own content is
    /// wrapped in `q`/`Q` so that nothing it leaves behind reaches ours.
    pub(crate) fn append_to_page(&mut self, page: lopdf::ObjectId, content: &Content) -> Result<()> {
        let mut streams: Vec<Object> = self.pdf.get_page_contents(page).into_iter().map(Object::Reference).collect();
        if self.wrapped.insert(page) {
            // A page may leave `q`s open (and a `cm` outside any of them), so
            // one `Q` is not enough to get back to our own `q`: close as many
            // as the page left open, plus ours.
            let mut open_states = 0usize;
            if let Ok(original) = Content::decode(&self.pdf.get_page_content(page)) {
                for op in &original.operations {
                    match op.operator.as_str() {
                        "q" => open_states += 1,
                        "Q" => open_states = open_states.saturating_sub(1),
                        _ => {}
                    }
                }
            }
            let open = self.pdf.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
            streams.insert(0, Object::Reference(open));
            let close = self.pdf.add_object(Stream::new(Dictionary::new(), "Q\n".repeat(open_states + 1).into_bytes()));
            streams.push(Object::Reference(close));
        }
        let mut stream = Stream::new(Dictionary::new(), content.encode()?);
        let _ = stream.compress();
        streams.push(Object::Reference(self.pdf.add_object(stream)));
        self.pdf.get_dictionary_mut(page)?.set("Contents", Object::Array(streams));
        Ok(())
    }
}

/// Where a direction in the source space points after `m`, ignoring translation.
fn direction(m: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    let (a, b) = apply([m[0], m[1], m[2], m[3], 0.0, 0.0], x, y);
    let length = (a * a + b * b).sqrt().max(1e-9);
    (a / length, b / length)
}

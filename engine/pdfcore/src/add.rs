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

#[derive(Debug, Clone, PartialEq)]
pub struct Added {
    /// Box the text occupies, as a reader shows the page.
    pub display: [f32; 4],
    pub lines: usize,
}

const LINE_HEIGHT: f32 = 1.2;

impl Document {
    pub fn add_text(&mut self, page: usize, new: &NewText) -> Result<Added> {
        self.transaction(|d| d.put_text(page, new))
    }

    fn put_text(&mut self, page: usize, new: &NewText) -> Result<Added> {
        if new.text.trim().is_empty() {
            return Err(Error::Unsupported("el texto nuevo está vacío".into()));
        }
        if !(new.size > 0.0) {
            return Err(Error::Unsupported("el tamaño debe ser mayor que cero".into()));
        }
        let id = self.page_id(page)?;
        let (key, data) = fallback::by_style(new.serif, new.bold, new.italic);
        let lacking = fallback::lacks(data, &new.text.replace('\n', ""));
        if !lacking.is_empty() {
            return Err(Error::MissingGlyphs(lacking));
        }
        let face = Face::parse(data, 0).map_err(|e| Error::Unsupported(e.to_string()))?;
        let scale = 1.0 / face.units_per_em() as f32;
        let ascent = face.ascender() as f32 * scale;

        // Lines, wrapped on spaces when there is a width to wrap to.
        let size = new.size;
        let mut lines: Vec<String> = Vec::new();
        for paragraph in new.text.split('\n') {
            let paragraph = paragraph.trim_end_matches('\r');
            let Some(limit) = new.max_width else {
                lines.push(paragraph.to_string());
                continue;
            };
            let mut current = String::new();
            for word in paragraph.split(' ') {
                let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
                if !current.is_empty() && fallback::advance(data, &candidate, size) > limit {
                    lines.push(std::mem::take(&mut current));
                    current = word.to_string();
                } else {
                    current = candidate;
                }
            }
            lines.push(current);
        }
        let widths: Vec<f32> = lines.iter().map(|l| fallback::advance(data, l, size)).collect();
        let box_width = new.max_width.unwrap_or_else(|| widths.iter().cloned().fold(0.0, f32::max));
        let lead = size * LINE_HEIGHT;

        // Where the first baseline starts, in the page as shown.
        let top = match new.anchor {
            Anchor::TopLeft => new.y,
            Anchor::Baseline => new.y - ascent * size,
        };

        // Display space back to the page's own: the inverse of what readers apply.
        let to_display = self.display_matrix(page)?;
        let inverse = invert(to_display).ok_or_else(|| Error::Unsupported("la página tiene una transformación no invertible".into()))?;
        let right = direction(inverse, 1.0, 0.0); // display "along the line"
        let up = direction(inverse, 0.0, -1.0); // display "up the glyphs"

        self.ensure_fallback(key, data)?;

        let mut ops = vec![
            Operation::new("q", vec![]),
            Operation::new("rg", new.color.iter().map(|c| Object::Real(*c)).collect()),
            Operation::new("BT", vec![]),
        ];
        let resource = self.fallbacks[key].resource.clone();
        ops.push(Operation::new("Tf", vec![Object::Name(resource.as_bytes().to_vec()), Object::Real(size)]));
        for (i, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let offset = match new.align {
                Align::Left => 0.0,
                Align::Center => (box_width - widths[i]) / 2.0,
                Align::Right => box_width - widths[i],
            };
            let (dx, dy) = (new.x + offset, top + ascent * size + lead * i as f32);
            let (ox, oy) = apply(inverse, dx, dy);
            let embedded = self.fallbacks.get_mut(key).expect("just ensured");
            let (bytes, _) = embedded.write(&mut self.pdf, line, size)?;
            ops.push(Operation::new("Tm", [right.0, right.1, up.0, up.1, ox, oy].iter().map(|v| Object::Real(*v)).collect()));
            ops.push(Operation::new("Tj", vec![Embedded::hex(bytes)]));
        }
        ops.push(Operation::new("ET", vec![]));
        ops.push(Operation::new("Q", vec![]));

        let font_id = self.fallbacks[key].font_id;
        self.register_font(id, &resource, font_id)?;
        self.append_to_page(id, &Content { operations: ops })?;

        let height = lead * (lines.len().saturating_sub(1)) as f32 + size * (ascent + (-face.descender() as f32 * scale));
        Ok(Added { display: [new.x, top, new.x + box_width, top + height], lines: lines.len() })
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

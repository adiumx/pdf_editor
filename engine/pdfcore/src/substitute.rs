//! Letters a font does not have, drawn as close to the original font as can be.
//!
//! Order of attempts for a letter the document's font lacks:
//! 1. the font itself, when it has a glyph (an embedded full font, or a code known to draw);
//! 2. the same font with the letter added to its encoding (`/Differences`), when the reader
//!    draws the font from its own copy of the face (standard or not embedded fonts);
//! 3. a bundled font of the closest style (see `fallback`), only when the caller allows it.
//!
//! Every letter that is not drawn by 1 is reported in [`Substitute`], with how.

use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::error::Result;
use crate::fallback;
use crate::font::{glyph_name_of, resolve};
use crate::text::fonts_of;
use crate::Document;

/// How a letter was drawn when the original font could not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Added to the font's own encoding: the reader draws it in the original face.
    Encoding,
    /// Drawn with a bundled font of the closest style.
    BundledFont,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Encoding => "encoding",
            Via::BundledFont => "bundled_font",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Substitute {
    pub ch: char,
    pub via: Via,
}

impl Document {
    /// Add to the encoding of the fonts that are drawn by the reader every letter of `wanted`
    /// (font resource, text) they lack and that has an Adobe glyph name. Returns the letters
    /// that were added. Nothing is touched for a font that already draws its text.
    pub(crate) fn teach_fonts(&mut self, page: ObjectId, wanted: &[(String, String)]) -> Result<Vec<char>> {
        let mut added = Vec::new();
        for (resource, text) in wanted {
            let fonts = fonts_of(&self.pdf, &self.resources(page));
            let Some(font) = fonts.get(resource) else { continue };
            if !font.can_extend_encoding() {
                continue;
            }
            // A letter the encoding has but /Widths stops short of would be drawn with no width.
            for ch in text.chars().collect::<std::collections::BTreeSet<_>>() {
                if let Some(code) = font.code_for(&ch.to_string()).filter(|c| !font.has_width(*c)) {
                    if let Some(font_id) = self.font_object(page, resource) {
                        self.set_width(font_id, code, ch)?;
                    }
                }
            }
            let mut lacking: Vec<char> = font.segments(text).iter().filter(|(_, b)| b.is_none()).flat_map(|(t, _)| t.chars()).collect();
            lacking.sort_unstable();
            lacking.dedup();
            if lacking.is_empty() {
                continue;
            }
            let Some(font_id) = self.font_object(page, resource) else { continue };
            let mut free = font.unused_codes().into_iter();
            for ch in lacking {
                let (Some(name), Some(code)) = (glyph_name_of(ch), free.next()) else { continue };
                if self.add_difference(font_id, code, name, ch)? {
                    added.push(ch);
                }
            }
        }
        added.sort_unstable();
        added.dedup();
        Ok(added)
    }

    /// The font object a page's resource name points to, when it is an indirect one.
    fn font_object(&self, page: ObjectId, resource: &str) -> Option<ObjectId> {
        let resources = self.resources(page);
        let table = resolve(&self.pdf, resources.get(b"Font").ok()?).as_dict().ok()?;
        table.get(resource.as_bytes()).ok()?.as_reference().ok()
    }

    /// Make `code` draw the glyph `name` in a simple font, and keep its widths and text map in step.
    fn add_difference(&mut self, font_id: ObjectId, code: u32, name: &str, ch: char) -> Result<bool> {
        let mut font = self.pdf.get_dictionary(font_id)?.clone();
        let encoding = match font.get(b"Encoding").ok().map(|o| resolve(&self.pdf, o).clone()) {
            Some(Object::Dictionary(d)) => d,
            Some(Object::Name(n)) => {
                let mut d = Dictionary::new();
                d.set("BaseEncoding", Object::Name(n));
                d
            }
            _ => Dictionary::new(),
        };
        let mut encoding = encoding;
        encoding.set("Type", Object::Name(b"Encoding".to_vec()));
        let mut differences = match encoding.get(b"Differences").ok().map(|o| resolve(&self.pdf, o).clone()) {
            Some(Object::Array(a)) => a,
            _ => Vec::new(),
        };
        differences.push(Object::Integer(code as i64));
        differences.push(Object::Name(name.as_bytes().to_vec()));
        encoding.set("Differences", Object::Array(differences));
        font.set("Encoding", Object::Dictionary(encoding));

        *self.pdf.get_object_mut(font_id)? = Object::Dictionary(font);
        self.set_width(font_id, code, ch)?;
        let font = self.pdf.get_dictionary(font_id)?.clone();
        let to_unicode = font.get(b"ToUnicode").ok().and_then(|o| o.as_reference().ok());
        if let Some(map) = to_unicode {
            self.add_to_unicode(map, code, ch)?;
        }
        Ok(true)
    }

    /// Give `code` the width of the letter `ch` in the font's `/Widths`, when it has them.
    fn set_width(&mut self, font_id: ObjectId, code: u32, ch: char) -> Result<()> {
        let mut font = self.pdf.get_dictionary(font_id)?.clone();
        // From the bundled face of the same style (metrically close)., from the bundled face of the same style (metrically close).
        let base = font.get(b"BaseFont").ok().and_then(|o| o.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default();
        let width = fallback::advance(fallback::choose(&base).1, &ch.to_string(), 1000.0).round() as i64;
        if let Ok(Object::Array(widths)) = font.get(b"Widths").map(|o| resolve(&self.pdf, o).clone()) {
            let first = font.get(b"FirstChar").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
            let mut widths = widths;
            let index = code as i64 - first;
            if index < 0 {
                // Below FirstChar: shift the table.
                let pad = (-index) as usize;
                widths.splice(0..0, std::iter::repeat(Object::Integer(0)).take(pad));
                font.set("FirstChar", Object::Integer(code as i64));
                widths[0] = Object::Integer(width);
            } else {
                while widths.len() <= index as usize {
                    widths.push(Object::Integer(0));
                }
                widths[index as usize] = Object::Integer(width);
            }
            let last = font.get(b"FirstChar").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0) + widths.len() as i64 - 1;
            font.set("LastChar", Object::Integer(last));
            font.set("Widths", Object::Array(widths));
        }
        *self.pdf.get_object_mut(font_id)? = Object::Dictionary(font);
        Ok(())
    }

    /// Say in a `ToUnicode` map that `code` is `text`, so copying and searching find the letter.
    pub(crate) fn add_to_unicode(&mut self, map: ObjectId, code: u32, ch: char) -> Result<()> {
        let Ok(stream) = self.pdf.get_object(map).and_then(|o| o.as_stream()) else { return Ok(()) };
        let body = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        let text = String::from_utf8_lossy(&body).into_owned();
        let Some(end) = text.rfind("endcmap") else { return Ok(()) };
        let width = if text.contains("<0000>") && text.contains("<FFFF>") { 4 } else { 2 };
        let utf16: String = ch.encode_utf16(&mut [0u16; 2]).iter().map(|u| format!("{u:04X}")).collect();
        let entry = format!("1 beginbfchar\n<{code:0width$X}> <{utf16}>\nendbfchar\n");
        let at = end;
        let mut new_text = text.clone();
        new_text.insert_str(at, &entry);
        let mut dict = stream.dict.clone();
        dict.remove(b"Filter");
        dict.remove(b"DecodeParms");
        dict.remove(b"Length");
        let mut replacement = Stream::new(dict, new_text.into_bytes());
        let _ = replacement.compress();
        *self.pdf.get_object_mut(map)? = Object::Stream(replacement);
        Ok(())
    }
}

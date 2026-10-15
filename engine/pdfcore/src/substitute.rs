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
    /// The font's embedded program has the glyph; only the document's width table and text map
    /// did not list it. Exactly the original face.
    FontProgram,
    /// Drawn with a bundled font of the closest style.
    BundledFont,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Encoding => "encoding",
            Via::FontProgram => "font_program",
            Via::BundledFont => "bundled_font",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitute {
    pub ch: char,
    pub via: Via,
    /// The font that drew it: the original's name for [`Via::Encoding`], the bundled font's
    /// (`Carlito-Regular`...) otherwise.
    pub font: String,
    /// 0..=100, how close that face is to the original (100: same widths and look; under 90
    /// the person should be told it is a different face).
    pub similarity: u8,
}

/// What to do with a run that has letters its font lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Substitution {
    /// Only the missing letters use the bundled font.
    PerChar,
    /// The whole run is drawn in the bundled font, so no word mixes two faces.
    WholeRun,
    /// `PerChar` when the bundled face is a metric clone (similarity of 90 or more), else `WholeRun`.
    Auto,
}

/// Letters a reader's copy of a Latin face can be trusted to have: Latin-1 and Latin Extended-A
/// (the Adobe standard set and what WinAnsi and the common Windows faces cover) plus the usual
/// punctuation. Anything else (Cyrillic, Greek...) may be blank in the reader, so it is not
/// taught to a font: the bundled font draws it.
fn reader_has(ch: char) -> bool {
    matches!(ch, '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{17F}' | '\u{192}' | '\u{2C6}' | '\u{2C7}' | '\u{2D8}'..='\u{2DD}'
        | '\u{2013}' | '\u{2014}' | '\u{2018}'..='\u{201E}' | '\u{2020}'..='\u{2022}' | '\u{2026}' | '\u{2030}'
        | '\u{2039}' | '\u{203A}' | '\u{20AC}' | '\u{2122}' | '\u{2212}')
}

impl Document {
    /// Add to the encoding of the fonts that are drawn by the reader every letter of `wanted`
    /// (font resource, text) they lack and that has an Adobe glyph name. Returns the letters
    /// that were added. Nothing is touched for a font that already draws its text.
    pub(crate) fn teach_fonts(&mut self, page: ObjectId, wanted: &[(String, String)]) -> Result<Vec<(char, Via)>> {
        let mut added: Vec<(char, Via)> = Vec::new();
        for (resource, text) in wanted {
            let fonts = fonts_of(&self.pdf, &self.resources(page));
            let Some(font) = fonts.get(resource) else { continue };
            if font.code_bytes() == 2 && !font.can_extend_encoding() {
                // A full TrueType program holds letters its text map never listed.
                let letters: std::collections::BTreeSet<char> = text.chars().collect();
                for ch in letters {
                    if let (Some((code, width)), Some(font_id)) = (font.program_glyph(ch), self.font_object(page, resource)) {
                        if self.add_cid_glyph(font_id, code, width, ch, !font.has_cid_width(code))? {
                            added.push((ch, Via::FontProgram));
                        }
                    }
                }
                continue;
            }
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
                if !reader_has(ch) {
                    continue;
                }
                let (Some(name), Some(code)) = (glyph_name_of(ch), free.next()) else { continue };
                if self.add_difference(font_id, code, name, ch)? {
                    added.push((ch, Via::Encoding));
                }
            }
        }
        added.sort_unstable_by_key(|(c, _)| *c);
        added.dedup_by_key(|(c, _)| *c);
        Ok(added)
    }

    /// Tell a composite font's width table and text map about a glyph its program has.
    fn add_cid_glyph(&mut self, font_id: ObjectId, code: u32, width: f32, ch: char, write_width: bool) -> Result<bool> {
        let font = self.pdf.get_dictionary(font_id)?.clone();
        let Some(descendant) = font.get(b"DescendantFonts").ok().map(|o| resolve(&self.pdf, o).clone()).and_then(|o| {
            let first = o.as_array().ok()?.first()?.clone();
            first.as_reference().ok()
        }) else {
            return Ok(false);
        };
        if write_width {
            let mut dict = self.pdf.get_dictionary(descendant)?.clone();
            let mut widths = match dict.get(b"W").ok().map(|o| resolve(&self.pdf, o).clone()) {
                Some(Object::Array(a)) => a,
                _ => Vec::new(),
            };
            widths.push(Object::Integer(code as i64));
            widths.push(Object::Array(vec![Object::Integer(width.round() as i64)]));
            dict.set("W", Object::Array(widths));
            *self.pdf.get_object_mut(descendant)? = Object::Dictionary(dict);
        }
        if let Some(map) = font.get(b"ToUnicode").ok().and_then(|o| o.as_reference().ok()) {
            self.add_to_unicode(map, code, ch, 4)?;
        }
        Ok(true)
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
            self.add_to_unicode(map, code, ch, 2)?;
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
    pub(crate) fn add_to_unicode(&mut self, map: ObjectId, code: u32, ch: char, width: usize) -> Result<()> {
        let Ok(stream) = self.pdf.get_object(map).and_then(|o| o.as_stream()) else { return Ok(()) };
        let body = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        let text = String::from_utf8_lossy(&body).into_owned();
        let Some(end) = text.rfind("endcmap") else { return Ok(()) };
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

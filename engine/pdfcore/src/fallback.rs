//! A font of our own for the characters the document's font cannot draw.
//!
//! A subset font keeps only the glyphs its document used, so a new "Ñ" or "X"
//! has no drawing to point at. Rather than refuse, the engine embeds a bundled
//! font (Liberation, SIL OFL 1.1: free to bundle and embed, and metrically
//! compatible with Arial/Helvetica and Times, so a swapped letter takes the
//! space the original face would) and draws just those characters with it.
//!
//! The whole font program is embedded once per document and style, compressed;
//! only the width table and the text map grow, as more characters are used.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document as Pdf, Object, ObjectId, Stream, StringFormat};
use ttf_parser::Face;

use crate::error::{Error, Result};

const FONTS: [(&str, &[u8]); 8] = [
    ("LiberationSans-Regular", include_bytes!("../fonts/LiberationSans-Regular.ttf")),
    ("LiberationSans-Bold", include_bytes!("../fonts/LiberationSans-Bold.ttf")),
    ("LiberationSans-Italic", include_bytes!("../fonts/LiberationSans-Italic.ttf")),
    ("LiberationSans-BoldItalic", include_bytes!("../fonts/LiberationSans-BoldItalic.ttf")),
    ("LiberationSerif-Regular", include_bytes!("../fonts/LiberationSerif-Regular.ttf")),
    ("LiberationSerif-Bold", include_bytes!("../fonts/LiberationSerif-Bold.ttf")),
    ("LiberationSerif-Italic", include_bytes!("../fonts/LiberationSerif-Italic.ttf")),
    ("LiberationSerif-BoldItalic", include_bytes!("../fonts/LiberationSerif-BoldItalic.ttf")),
];

/// The bundled font closest in style to `base_font`: serif or sans, bold,
/// italic. (Monospace is not bundled yet and falls to sans.)
pub(crate) fn choose(base_font: &str) -> (&'static str, &'static [u8]) {
    let name = base_font.rsplit('+').next().unwrap_or(base_font).to_ascii_lowercase();
    let serif = ["times", "serif", "georgia", "garamond", "palatino", "cambria", "minion", "book"]
        .iter()
        .any(|k| name.contains(k))
        && !name.contains("sans");
    let bold = ["bold", "black", "heavy", "demi"].iter().any(|k| name.contains(k));
    let italic = name.contains("italic") || name.contains("oblique");
    by_style(serif, bold, italic)
}

/// The bundled font of a given style.
pub(crate) fn by_style(serif: bool, bold: bool, italic: bool) -> (&'static str, &'static [u8]) {
    let family = if serif { "LiberationSerif" } else { "LiberationSans" };
    let style = match (bold, italic) {
        (true, true) => "BoldItalic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        _ => "Regular",
    };
    let key = format!("{family}-{style}");
    FONTS.iter().find(|(k, _)| *k == key).copied().expect("every combination is bundled")
}

/// The characters of `text` that font has no glyph for.
pub(crate) fn lacks(data: &[u8], text: &str) -> Vec<char> {
    let Ok(face) = Face::parse(data, 0) else { return text.chars().collect() };
    text.chars().filter(|c| face.glyph_index(*c).is_none()).collect()
}

/// How far `text` advances in that font at `size` points, without touching any
/// document. Characters the font lacks count as nothing.
pub(crate) fn advance(data: &[u8], text: &str, size: f32) -> f32 {
    let Ok(face) = Face::parse(data, 0) else { return 0.0 };
    let scale = size / face.units_per_em() as f32;
    text.chars()
        .filter_map(|c| face.glyph_index(c))
        .map(|g| face.glyph_hor_advance(g).unwrap_or(0) as f32 * scale)
        .sum()
}

/// A fallback font that has been written into one document.
pub(crate) struct Embedded {
    pub resource: String,
    pub font_id: ObjectId,
    descendant: ObjectId,
    to_unicode: ObjectId,
    data: &'static [u8],
    used: BTreeMap<u16, char>,
}

impl Embedded {
    /// The fallback font of this style already written into the document, if
    /// an earlier session put one there. Embedding a second copy under the same
    /// resource name would replace the first in the pages' resources, and the
    /// new copy only knows the widths of the glyphs *it* was used for, so text
    /// written earlier would come out spaced as if every other glyph were 1 em.
    pub fn adopt(pdf: &Pdf, key: &'static str, data: &'static [u8]) -> Option<Embedded> {
        for (id, object) in pdf.objects.iter() {
            let Object::Dictionary(dict) = object else { continue };
            let named = |k: &[u8], v: &str| dict.get(k).ok().and_then(|o| o.as_name().ok()) == Some(v.as_bytes());
            if !(named(b"Subtype", "Type0") && named(b"BaseFont", key)) {
                continue;
            }
            let descendant = dict.get(b"DescendantFonts").ok()?.as_array().ok()?.first()?.as_reference().ok()?;
            let to_unicode = dict.get(b"ToUnicode").ok()?.as_reference().ok()?;
            let map = pdf.get_object(to_unicode).ok()?.as_stream().ok()?;
            let map = map.decompressed_content().unwrap_or_else(|_| map.content.clone());
            let used = crate::cmap::CMap::parse(&map)
                .entries()
                .into_iter()
                .filter_map(|(code, text)| Some((u16::try_from(code).ok()?, text.chars().next()?)))
                .collect();
            return Some(Embedded {
                resource: format!("PC{}", key.replace('-', "")),
                font_id: *id,
                descendant,
                to_unicode,
                data,
                used,
            });
        }
        None
    }

    pub fn embed(pdf: &mut Pdf, key: &'static str, data: &'static [u8]) -> Result<Embedded> {
        let face = Face::parse(data, 0).map_err(|e| Error::Unsupported(format!("fuente de respaldo ilegible: {e}")))?;
        let scale = 1000.0 / face.units_per_em() as f32;
        let name = |n: &str| Object::Name(n.as_bytes().to_vec());

        let mut program = Stream::new(Dictionary::new(), data.to_vec());
        program.dict.set("Length1", data.len() as i64);
        let _ = program.compress();
        let program = pdf.add_object(program);

        let bbox = face.global_bounding_box();
        let mut descriptor = Dictionary::new();
        descriptor.set("Type", name("FontDescriptor"));
        descriptor.set("FontName", name(key));
        let italic = key.contains("Italic");
        descriptor.set("Flags", 32 + if italic { 64 } else { 0 });
        descriptor.set(
            "FontBBox",
            vec![bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max]
                .into_iter()
                .map(|v| Object::Integer((v as f32 * scale).round() as i64))
                .collect::<Vec<_>>(),
        );
        descriptor.set("ItalicAngle", Object::Real(face.italic_angle()));
        descriptor.set("Ascent", (face.ascender() as f32 * scale).round() as i64);
        descriptor.set("Descent", (face.descender() as f32 * scale).round() as i64);
        let cap = face.capital_height().unwrap_or(face.ascender());
        descriptor.set("CapHeight", (cap as f32 * scale).round() as i64);
        descriptor.set("StemV", if key.contains("Bold") { 140 } else { 80 });
        descriptor.set("FontFile2", Object::Reference(program));
        let descriptor = pdf.add_object(descriptor);

        let mut info = Dictionary::new();
        info.set("Registry", Object::string_literal("Adobe"));
        info.set("Ordering", Object::string_literal("Identity"));
        info.set("Supplement", 0);
        let mut descendant = Dictionary::new();
        descendant.set("Type", name("Font"));
        descendant.set("Subtype", name("CIDFontType2"));
        descendant.set("BaseFont", name(key));
        descendant.set("CIDSystemInfo", Object::Dictionary(info));
        descendant.set("FontDescriptor", Object::Reference(descriptor));
        descendant.set("DW", 1000);
        descendant.set("W", Object::Array(Vec::new()));
        descendant.set("CIDToGIDMap", name("Identity"));
        let descendant = pdf.add_object(descendant);

        let to_unicode = pdf.add_object(Stream::new(Dictionary::new(), Vec::new()));

        let mut font = Dictionary::new();
        font.set("Type", name("Font"));
        font.set("Subtype", name("Type0"));
        font.set("BaseFont", name(key));
        font.set("Encoding", name("Identity-H"));
        font.set("DescendantFonts", vec![Object::Reference(descendant)]);
        font.set("ToUnicode", Object::Reference(to_unicode));
        let font = pdf.add_object(font);

        let resource = format!("PC{}", key.replace('-', ""));
        Ok(Embedded { resource, font_id: font, descendant, to_unicode, data, used: BTreeMap::new() })
    }

    /// The bytes that draw `text` in this font, and how far they advance at
    /// `size` points. Records each glyph used, so the width table and the text
    /// map written into the document cover it.
    pub fn write(&mut self, pdf: &mut Pdf, text: &str, size: f32) -> Result<(Vec<u8>, f32)> {
        let face = Face::parse(self.data, 0).map_err(|e| Error::Unsupported(e.to_string()))?;
        let scale = 1000.0 / face.units_per_em() as f32;
        let (mut bytes, mut advance, mut grew) = (Vec::new(), 0.0, false);
        for ch in text.chars() {
            let gid = face.glyph_index(ch).ok_or_else(|| Error::MissingGlyphs(vec![ch]))?;
            bytes.extend(gid.0.to_be_bytes());
            advance += face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale / 1000.0 * size;
            grew |= self.used.insert(gid.0, ch).is_none();
        }
        if grew {
            self.publish(pdf, &face, scale)?;
        }
        Ok((bytes, advance))
    }

    /// Rewrite the width table and the text map from the glyphs used so far.
    fn publish(&self, pdf: &mut Pdf, face: &Face, scale: f32) -> Result<()> {
        let mut widths = Vec::new();
        for gid in self.used.keys() {
            let w = face.glyph_hor_advance(ttf_parser::GlyphId(*gid)).unwrap_or(0) as f32 * scale;
            widths.push(Object::Integer(*gid as i64));
            widths.push(Object::Array(vec![Object::Integer(w.round() as i64)]));
        }
        pdf.get_dictionary_mut(self.descendant)?.set("W", Object::Array(widths));

        let mut map = String::from(
            "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
             /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
             1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
        );
        let entries: Vec<_> = self.used.iter().collect();
        for chunk in entries.chunks(100) {
            map.push_str(&format!("{} beginbfchar\n", chunk.len()));
            for (gid, ch) in chunk {
                let mut buf = [0u16; 2];
                let hex: String = ch.encode_utf16(&mut buf).iter().map(|u| format!("{u:04X}")).collect();
                map.push_str(&format!("<{gid:04X}> <{hex}>\n"));
            }
            map.push_str("endbfchar\n");
        }
        map.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
        let mut stream = Stream::new(Dictionary::new(), map.into_bytes());
        let _ = stream.compress();
        *pdf.get_object_mut(self.to_unicode)? = Object::Stream(stream);
        Ok(())
    }

    pub fn hex(bytes: Vec<u8>) -> Object {
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_font_may_be_embedded() {
        for (key, data) in FONTS {
            let face = Face::parse(data, 0).unwrap();
            let allowed = face.permissions().map(|p| p == ttf_parser::Permissions::Installable
                || p == ttf_parser::Permissions::Editable).unwrap_or(true);
            assert!(allowed, "{key} no permite incrustarse");
        }
    }

    #[test]
    fn style_follows_the_original_name() {
        assert_eq!(choose("ABCDEF+Arial-BoldMT").0, "LiberationSans-Bold");
        assert_eq!(choose("TimesNewRomanPS-ItalicMT").0, "LiberationSerif-Italic");
        assert_eq!(choose("Helvetica,Bold").0, "LiberationSans-Bold");
        assert_eq!(choose("Calibri").0, "LiberationSans-Regular");
        assert_eq!(choose("Arial-BoldItalicMT").0, "LiberationSans-BoldItalic");
    }

    #[test]
    fn spanish_is_covered() {
        let (_, data) = choose("Arial");
        assert!(lacks(data, "ÁÉÍÓÚÑáéíóúñü¿¡€").is_empty());
        assert_eq!(lacks(data, "a世b"), vec!['世']);
    }
}

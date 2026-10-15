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

use std::collections::{BTreeMap, BTreeSet};

use lopdf::{Dictionary, Object, ObjectId, Stream, StringFormat};

use crate::journal::Tracked as Pdf;
use ttf_parser::Face;

use crate::error::{Error, Result};

const FONTS: [(&str, &[u8]); 20] = [
    ("LiberationSans-Regular", include_bytes!("../fonts/LiberationSans-Regular.ttf")),
    ("LiberationSans-Bold", include_bytes!("../fonts/LiberationSans-Bold.ttf")),
    ("LiberationSans-Italic", include_bytes!("../fonts/LiberationSans-Italic.ttf")),
    ("LiberationSans-BoldItalic", include_bytes!("../fonts/LiberationSans-BoldItalic.ttf")),
    ("LiberationSerif-Regular", include_bytes!("../fonts/LiberationSerif-Regular.ttf")),
    ("LiberationSerif-Bold", include_bytes!("../fonts/LiberationSerif-Bold.ttf")),
    ("LiberationSerif-Italic", include_bytes!("../fonts/LiberationSerif-Italic.ttf")),
    ("LiberationSerif-BoldItalic", include_bytes!("../fonts/LiberationSerif-BoldItalic.ttf")),
    ("LiberationMono-Regular", include_bytes!("../fonts/LiberationMono-Regular.ttf")),
    ("LiberationMono-Bold", include_bytes!("../fonts/LiberationMono-Bold.ttf")),
    ("LiberationMono-Italic", include_bytes!("../fonts/LiberationMono-Italic.ttf")),
    ("LiberationMono-BoldItalic", include_bytes!("../fonts/LiberationMono-BoldItalic.ttf")),
    ("Carlito-Regular", include_bytes!("../fonts/Carlito-Regular.ttf")),
    ("Carlito-Bold", include_bytes!("../fonts/Carlito-Bold.ttf")),
    ("Carlito-Italic", include_bytes!("../fonts/Carlito-Italic.ttf")),
    ("Carlito-BoldItalic", include_bytes!("../fonts/Carlito-BoldItalic.ttf")),
    ("Caladea-Regular", include_bytes!("../fonts/Caladea-Regular.ttf")),
    ("Caladea-Bold", include_bytes!("../fonts/Caladea-Bold.ttf")),
    ("Caladea-Italic", include_bytes!("../fonts/Caladea-Italic.ttf")),
    ("Caladea-BoldItalic", include_bytes!("../fonts/Caladea-BoldItalic.ttf")),
];

/// The bundled families. Liberation is metrically compatible with Arial/Helvetica (Sans),
/// Times New Roman (Serif) and Courier New (Mono), Carlito with Calibri and Caladea with
/// Cambria: a swapped letter takes the room the original face would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Sans,
    Serif,
    Mono,
    Carlito,
    Caladea,
}

impl Family {
    fn prefix(self) -> &'static str {
        match self {
            Family::Sans => "LiberationSans",
            Family::Serif => "LiberationSerif",
            Family::Mono => "LiberationMono",
            Family::Carlito => "Carlito",
            Family::Caladea => "Caladea",
        }
    }
}

/// The bundled font chosen for a face, and how close it is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Choice {
    pub key: &'static str,
    pub data: &'static [u8],
    /// 0..=100: 100 is a metric-compatible clone of the original (same widths, same look to
    /// the eye); below 90 it is the closest by kind only, and the person should be told.
    pub similarity: u8,
}

fn contains_any(name: &str, words: &[&str]) -> bool {
    words.iter().any(|w| name.contains(w))
}

/// The bundled font closest to a face, from its name and (when known) the descriptor's
/// `/Flags`: an exact clone scores 100, a family named by kind (a serif, a monospaced one) 70-75,
/// one guessed from the flags alone 55-60, nothing known 40.
pub(crate) fn pick(base_font: &str, flags: Option<u32>) -> Choice {
    pick_with(base_font, flags, None, None)
}

/// As [`pick`], with bold and italic stated instead of read from the name.
pub(crate) fn pick_with(base_font: &str, flags: Option<u32>, bold: Option<bool>, italic: Option<bool>) -> Choice {
    let raw = base_font.rsplit('+').next().unwrap_or(base_font).to_ascii_lowercase();
    let name: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let sans_word = name.contains("sans") || name.contains("gothic");
    let (family, similarity) = if contains_any(&name, &["arial", "helvetica", "arimo", "liberationsans"]) {
        (Family::Sans, 100)
    } else if contains_any(&name, &["timesnewroman", "times", "tinos", "liberationserif"]) {
        (Family::Serif, 100)
    } else if contains_any(&name, &["couriernew", "courier", "cousine", "liberationmono"]) {
        (Family::Mono, 100)
    } else if contains_any(&name, &["calibri", "carlito"]) {
        (Family::Carlito, 100)
    } else if contains_any(&name, &["cambria", "caladea"]) {
        (Family::Caladea, 100)
    } else if contains_any(&name, &["mono", "consolas", "lucidaconsole", "menlo", "monaco", "typewriter", "fixed", "code"]) {
        (Family::Mono, 75)
    } else if !sans_word && contains_any(&name, &["georgia", "garamond", "palatino", "minion", "book", "century", "baskerville", "didot", "serif", "roman"]) {
        (Family::Serif, 75)
    } else if contains_any(&name, &["verdana", "tahoma", "segoe", "trebuchet", "myriad", "opensans", "roboto", "lato", "sans", "gothic", "frutiger", "univers"]) {
        (Family::Sans, 70)
    } else if let Some(f) = flags.filter(|f| f & 1 != 0) {
        let _ = f;
        (Family::Mono, 60)
    } else if flags.is_some_and(|f| f & 2 != 0) {
        (Family::Serif, 60)
    } else if flags.is_some() {
        (Family::Sans, 55)
    } else {
        (Family::Sans, 40)
    };
    let bold = bold.unwrap_or_else(|| contains_any(&name, &["bold", "black", "heavy", "demi"]) || flags.is_some_and(|f| f & (1 << 18) != 0));
    let italic = italic.unwrap_or_else(|| contains_any(&name, &["italic", "oblique"]) || flags.is_some_and(|f| f & (1 << 6) != 0));
    let (key, data) = by_family(family, bold, italic);
    Choice { key, data, similarity }
}

/// The bundled font closest in style to `base_font`, by name alone.
pub(crate) fn choose(base_font: &str) -> (&'static str, &'static [u8]) {
    let c = pick(base_font, None);
    (c.key, c.data)
}

/// The bundled Liberation font of a given style (sans or serif).
pub(crate) fn by_style(serif: bool, bold: bool, italic: bool) -> (&'static str, &'static [u8]) {
    by_family(if serif { Family::Serif } else { Family::Sans }, bold, italic)
}

pub(crate) fn by_family(family: Family, bold: bool, italic: bool) -> (&'static str, &'static [u8]) {
    let style = match (bold, italic) {
        (true, true) => "BoldItalic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        _ => "Regular",
    };
    let key = format!("{}-{style}", family.prefix());
    FONTS.iter().find(|(k, _)| *k == key).copied().expect("every combination is bundled")
}

/// Mean advance (per 1000 em) of a-z in a bundled font.
pub(crate) fn average_width(data: &[u8]) -> f32 {
    advance(data, "abcdefghijklmnopqrstuvwxyz", 1000.0) / 26.0
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
    /// The `FontFile2` stream, cut down to the glyphs used each time one is added.
    program: ObjectId,
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
            let program = pdf
                .get_dictionary(descendant)
                .ok()?
                .get(b"FontDescriptor")
                .ok()?
                .as_reference()
                .ok()
                .and_then(|d| pdf.get_dictionary(d).ok())
                .and_then(|d| d.get(b"FontFile2").ok())
                .and_then(|o| o.as_reference().ok())?;
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
                program,
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

        // Only the notdef glyph to begin with; each letter drawn adds its own outline.
        let first = crate::subset::subset(data, &BTreeSet::new()).unwrap_or_else(|| data.to_vec());
        let mut program = Stream::new(Dictionary::new(), first.clone());
        program.dict.set("Length1", first.len() as i64);
        let _ = program.compress();
        let program = pdf.add_object(program);
        let program_id = program;

        let bbox = face.global_bounding_box();
        let mut descriptor = Dictionary::new();
        descriptor.set("Type", name("FontDescriptor"));
        descriptor.set("FontName", name(key));
        let italic = key.contains("Italic");
        let pitch = if key.contains("Mono") { 1 } else { 0 } + if key.contains("Serif") || key.contains("Caladea") { 2 } else { 0 };
        descriptor.set("Flags", 32 + pitch + if italic { 64 } else { 0 });
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
        Ok(Embedded { resource, font_id: font, descendant, to_unicode, program: program_id, data, used: BTreeMap::new() })
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

        // The font program with the outlines of every glyph used so far.
        let keep: BTreeSet<u16> = self.used.keys().copied().collect();
        if let Some(cut) = crate::subset::subset(self.data, &keep) {
            let mut program = Stream::new(Dictionary::new(), cut.clone());
            program.dict.set("Length1", cut.len() as i64);
            let _ = program.compress();
            *pdf.get_object_mut(self.program)? = Object::Stream(program);
        }

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
        assert_eq!(choose("Calibri").0, "Carlito-Regular");
        assert_eq!(choose("Cambria-Bold").0, "Caladea-Bold");
        assert_eq!(choose("CourierNewPS-ItalicMT").0, "LiberationMono-Italic");
        assert_eq!(pick("Georgia", None).similarity, 75);
        assert_eq!(pick("Zzz", Some(1)).key, "LiberationMono-Regular");
        assert_eq!(choose("Arial-BoldItalicMT").0, "LiberationSans-BoldItalic");
    }

    #[test]
    fn spanish_is_covered() {
        let (_, data) = choose("Arial");
        assert!(lacks(data, "ÁÉÍÓÚÑáéíóúñü¿¡€").is_empty());
        assert_eq!(lacks(data, "a世b"), vec!['世']);
    }
}

//! What the engine needs to know about a font to read text and to write more
//! of it: how many bytes a character code takes, what text it stands for, how
//! wide it is, and — for writing — which code produces a given character.

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object};

use crate::cmap::CMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontKind {
    /// One byte per code: Type1, TrueType, Type3.
    Simple,
    /// Composite (CID-keyed) fonts, usually two bytes per code.
    Type0,
}

#[derive(Debug, Clone)]
pub struct Glyph {
    pub code: u32,
    pub text: String,
    /// Advance, in thousandths of the font size.
    pub width: f32,
    /// Word spacing applies only to a single-byte code 32.
    pub is_space: bool,
}

#[derive(Debug, Clone)]
pub struct Font {
    pub resource: String,
    pub base_font: String,
    pub kind: FontKind,
    /// The font file is part of the document (as opposed to one of the
    /// standard fonts a reader is expected to supply).
    pub embedded: bool,
    pub ascent: f32,
    pub descent: f32,
    code_bytes: usize,
    to_unicode: Option<CMap>,
    simple_text: Vec<Option<String>>,
    first_char: u32,
    widths: Vec<f32>,
    missing_width: f32,
    cid_default_width: f32,
    cid_widths: HashMap<u32, f32>,
}

impl Font {
    pub fn from_dict(doc: &Document, resource: &str, dict: &Dictionary) -> Font {
        let subtype = name_of(doc, dict, b"Subtype").unwrap_or_default();
        let base_font = name_of(doc, dict, b"BaseFont").unwrap_or_default();
        let kind = if subtype == "Type0" { FontKind::Type0 } else { FontKind::Simple };

        let to_unicode = stream_of(doc, dict, b"ToUnicode").map(|d| CMap::parse(&d));

        let mut font = Font {
            resource: resource.to_string(),
            base_font,
            kind,
            embedded: false,
            ascent: 905.0,
            descent: -212.0,
            code_bytes: 1,
            to_unicode,
            simple_text: Vec::new(),
            first_char: 0,
            widths: Vec::new(),
            missing_width: 0.0,
            cid_default_width: 1000.0,
            cid_widths: HashMap::new(),
        };

        match kind {
            FontKind::Type0 => font.read_composite(doc, dict),
            FontKind::Simple => font.read_simple(doc, dict),
        }
        font
    }

    fn read_composite(&mut self, doc: &Document, dict: &Dictionary) {
        self.code_bytes = self
            .to_unicode
            .as_ref()
            .map(|m| m.code_bytes)
            .filter(|n| *n > 0)
            .unwrap_or(2);
        let Some(descendant) = first_descendant(doc, dict) else { return };
        if let Some(width) = number_of(doc, descendant, b"DW") {
            self.cid_default_width = width;
        }
        if let Some(array) = array_of(doc, descendant, b"W") {
            self.read_cid_widths(doc, &array);
        }
        if let Some(descriptor) = dict_of(doc, descendant, b"FontDescriptor") {
            self.read_descriptor(descriptor);
        }
    }

    fn read_cid_widths(&mut self, doc: &Document, array: &[Object]) {
        let mut i = 0;
        while i < array.len() {
            let Some(first) = num(doc, &array[i]) else { break };
            match array.get(i + 1).map(|o| resolve(doc, o)) {
                Some(Object::Array(list)) => {
                    for (offset, width) in list.iter().enumerate() {
                        if let Some(w) = num(doc, width) {
                            self.cid_widths.insert(first as u32 + offset as u32, w);
                        }
                    }
                    i += 2;
                }
                Some(last) => {
                    let (Some(last), Some(width)) = (num(doc, last), array.get(i + 2).and_then(|o| num(doc, o)))
                    else {
                        break;
                    };
                    for cid in first as u32..=(last as u32).min(first as u32 + 0xFFFF) {
                        self.cid_widths.insert(cid, width);
                    }
                    i += 3;
                }
                None => break,
            }
        }
    }

    fn read_simple(&mut self, doc: &Document, dict: &Dictionary) {
        self.first_char = number_of(doc, dict, b"FirstChar").unwrap_or(0.0) as u32;
        if let Some(list) = array_of(doc, dict, b"Widths") {
            self.widths = list.iter().map(|o| num(doc, o).unwrap_or(0.0)).collect();
        }
        if let Some(descriptor) = dict_of(doc, dict, b"FontDescriptor") {
            self.read_descriptor(descriptor);
            if let Some(w) = number_of(doc, descriptor, b"MissingWidth") {
                self.missing_width = w;
            }
        }
        self.simple_text = simple_encoding(doc, dict);
    }

    fn read_descriptor(&mut self, descriptor: &Dictionary) {
        self.embedded = descriptor.has(b"FontFile")
            || descriptor.has(b"FontFile2")
            || descriptor.has(b"FontFile3");
        if let Ok(a) = descriptor.get(b"Ascent").and_then(|o| o.as_float()) {
            if a != 0.0 {
                self.ascent = a;
            }
        }
        if let Ok(d) = descriptor.get(b"Descent").and_then(|o| o.as_float()) {
            if d != 0.0 {
                self.descent = if d > 0.0 { -d } else { d };
            }
        }
    }

    pub fn code_bytes(&self) -> usize {
        self.code_bytes
    }

    /// Split a shown string into glyphs.
    pub fn decode(&self, bytes: &[u8]) -> Vec<Glyph> {
        let step = self.code_bytes.max(1);
        bytes
            .chunks(step)
            .map(|chunk| {
                let code = chunk.iter().fold(0u32, |n, b| (n << 8) | *b as u32);
                Glyph {
                    code,
                    text: self.text_of(code),
                    width: self.width_of(code),
                    is_space: step == 1 && code == 32,
                }
            })
            .collect()
    }

    fn text_of(&self, code: u32) -> String {
        if let Some(text) = self.to_unicode.as_ref().and_then(|m| m.text_of(code)) {
            return text.to_string();
        }
        match self.kind {
            FontKind::Simple => self
                .simple_text
                .get(code as usize)
                .cloned()
                .flatten()
                .unwrap_or_default(),
            // Without a map a CID has no known meaning; say nothing rather
            // than invent text.
            FontKind::Type0 => String::new(),
        }
    }

    fn width_of(&self, code: u32) -> f32 {
        match self.kind {
            FontKind::Type0 => self.cid_widths.get(&code).copied().unwrap_or(self.cid_default_width),
            FontKind::Simple => {
                if let Some(w) = code
                    .checked_sub(self.first_char)
                    .and_then(|i| self.widths.get(i as usize))
                {
                    return *w;
                }
                if self.widths.is_empty() && !self.embedded {
                    return standard_width(&self.base_font, code);
                }
                self.missing_width
            }
        }
    }

    /// The code that draws `text` in this font, if the font has such a glyph.
    pub fn code_for(&self, text: &str) -> Option<u32> {
        if let Some(code) = self.to_unicode.as_ref().and_then(|m| m.code_of(text)) {
            return Some(code);
        }
        if self.kind == FontKind::Simple {
            return self
                .simple_text
                .iter()
                .position(|t| t.as_deref() == Some(text))
                .map(|i| i as u32);
        }
        None
    }
}

/// The advance width of a standard font that states none of its own, chosen by
/// family from the name (Arial is Helvetica's metrics by design). Codes outside
/// WinAnsi's printable range fall back to the family's lowercase width.
fn standard_width(base_font: &str, code: u32) -> f32 {
    use crate::standard_widths as w;
    let name = base_font.rsplit('+').next().unwrap_or(base_font).to_ascii_lowercase();
    let bold = name.contains("bold") || name.contains(",bold");
    let italic = name.contains("italic") || name.contains("oblique");
    let table: &[u16; 224] = if name.contains("courier") {
        &w::COURIER
    } else if name.contains("times") {
        match (bold, italic) {
            (true, true) => &w::TIMES_BOLD_ITALIC,
            (true, false) => &w::TIMES_BOLD,
            (false, true) => &w::TIMES_ITALIC,
            _ => &w::TIMES,
        }
    } else if bold {
        &w::HELVETICA_BOLD
    } else {
        &w::HELVETICA
    };
    match code {
        32..=255 => table[(code - 32) as usize] as f32,
        _ => table[(b'a' - 32) as usize] as f32,
    }
}

const WIN_ANSI_HIGH: [char; 32] = [
    '€', '\u{0}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{0}', 'Ž', '\u{0}',
    '\u{0}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{0}', 'ž', 'Ÿ',
];

fn simple_encoding(doc: &Document, dict: &Dictionary) -> Vec<Option<String>> {
    let mut table: Vec<Option<String>> = (0..256u32)
        .map(|c| match c {
            32..=126 | 160..=255 => char::from_u32(c).map(|ch| ch.to_string()),
            128..=159 => {
                let ch = WIN_ANSI_HIGH[(c - 128) as usize];
                (ch != '\u{0}').then(|| ch.to_string())
            }
            _ => None,
        })
        .collect();

    if let Some(Object::Dictionary(encoding)) = dict.get(b"Encoding").ok().map(|o| resolve(doc, o)) {
        if let Some(list) = array_of(doc, encoding, b"Differences") {
            let mut code = 0usize;
            for item in &list {
                match resolve(doc, item) {
                    Object::Integer(n) => code = *n as usize,
                    Object::Name(name) => {
                        if let Some(slot) = table.get_mut(code) {
                            *slot = glyph_name_text(&String::from_utf8_lossy(name));
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    table
}

fn glyph_name_text(name: &str) -> Option<String> {
    if let Some(hex) = name.strip_prefix("uni") {
        if let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
            return Some(c.to_string());
        }
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(c.to_string());
    }
    const NAMES: &[(&str, char)] = &[
        ("space", ' '), ("period", '.'), ("comma", ','), ("hyphen", '-'), ("colon", ':'),
        ("semicolon", ';'), ("slash", '/'), ("parenleft", '('), ("parenright", ')'),
        ("zero", '0'), ("one", '1'), ("two", '2'), ("three", '3'), ("four", '4'),
        ("five", '5'), ("six", '6'), ("seven", '7'), ("eight", '8'), ("nine", '9'),
        ("aacute", 'á'), ("eacute", 'é'), ("iacute", 'í'), ("oacute", 'ó'), ("uacute", 'ú'),
        ("Aacute", 'Á'), ("Eacute", 'É'), ("Iacute", 'Í'), ("Oacute", 'Ó'), ("Uacute", 'Ú'),
        ("ntilde", 'ñ'), ("Ntilde", 'Ñ'), ("udieresis", 'ü'), ("Udieresis", 'Ü'),
        ("questiondown", '¿'), ("exclamdown", '¡'), ("bullet", '•'), ("endash", '–'),
        ("emdash", '—'), ("quotedblleft", '“'), ("quotedblright", '”'),
    ];
    NAMES.iter().find(|(n, _)| *n == name).map(|(_, c)| c.to_string())
}

// -- reading loosely-typed PDF objects -------------------------------------

pub(crate) fn resolve<'a>(doc: &'a Document, object: &'a Object) -> &'a Object {
    doc.dereference(object).map(|(_, o)| o).unwrap_or(object)
}

pub(crate) fn num(doc: &Document, object: &Object) -> Option<f32> {
    resolve(doc, object).as_float().ok()
}

fn name_of(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<String> {
    let object = resolve(doc, dict.get(key).ok()?);
    object.as_name().ok().map(|n| String::from_utf8_lossy(n).into_owned())
}

fn number_of(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<f32> {
    num(doc, dict.get(key).ok()?)
}

fn array_of(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<Vec<Object>> {
    resolve(doc, dict.get(key).ok()?).as_array().ok().cloned()
}

fn dict_of<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    resolve(doc, dict.get(key).ok()?).as_dict().ok()
}

fn stream_of(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<Vec<u8>> {
    let stream = resolve(doc, dict.get(key).ok()?).as_stream().ok()?;
    stream.decompressed_content().ok().or_else(|| Some(stream.content.clone()))
}

fn first_descendant<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    let list = resolve(doc, dict.get(b"DescendantFonts").ok()?).as_array().ok()?;
    resolve(doc, list.first()?).as_dict().ok()
}

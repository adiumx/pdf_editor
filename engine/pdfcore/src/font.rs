//! What the engine needs to know about a font to read text and to write more
//! of it: how many bytes a character code takes, what text it stands for, how
//! wide it is, and — for writing — which code produces a given character.

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object};

use crate::cmap::CMap;
use crate::encodings_data;
use crate::program;

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
    /// Codes the embedded font program has no drawing for, whatever the
    /// document's ToUnicode claims. Empty when the program could not be read.
    blank: std::collections::HashSet<u32>,
    /// Codes of an embedded simple font known to be drawn: checked in the font
    /// program, or seen drawn on the page. Nothing else may be written with it.
    verified: std::collections::HashSet<u32>,
    /// What kind of font this is, for telling the person whether it can be reproduced exactly.
    pub class: FontClass,
    /// Written top to bottom (a `-V` CMap): read, not edited.
    vertical: bool,
    /// Type 3 glyphs are drawings, not letters: read, not edited.
    type3: bool,
    /// ZapfDingbats and the like: the glyphs are symbols, not text.
    symbols: bool,
    /// `/Flags` of the font descriptor (fixed pitch, serif, italic, force bold...).
    flags: Option<u32>,
    /// Type 0 fonts with a full TrueType program and glyph ids as codes: the letters the program
    /// draws that the font's text map does not list, as `(code, width per 1000 em)`.
    program_glyphs: HashMap<char, (u32, f32)>,
}

/// How faithfully a font can be reproduced when text is added to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontClass {
    /// The whole font program is in the file: any letter it has can be written.
    EmbeddedFull,
    /// A subset: only the letters the document already uses are in the file.
    EmbeddedSubset,
    /// One of the 14 standard fonts (or the names every reader maps to them).
    Standard,
    /// Named but not in the file: the reader picks a font, and so does whoever opens it.
    NotEmbedded,
    /// Type 3: glyphs are drawings.
    Type3,
}

impl FontClass {
    pub fn as_str(self) -> &'static str {
        match self {
            FontClass::EmbeddedFull => "embedded_full",
            FontClass::EmbeddedSubset => "embedded_subset",
            FontClass::Standard => "standard",
            FontClass::NotEmbedded => "not_embedded",
            FontClass::Type3 => "type3",
        }
    }
}

/// `ABCDEF+Name`: the prefix that marks a subset.
fn has_subset_prefix(base_font: &str) -> bool {
    base_font.split_once('+').is_some_and(|(tag, _)| tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()))
}

fn is_standard_font(base_font: &str) -> bool {
    let name: String = base_font.rsplit('+').next().unwrap_or(base_font).chars().filter(|c| !c.is_whitespace() && *c != ',' && *c != '-').collect();
    ["Helvetica", "Arial", "TimesRoman", "TimesNewRoman", "Times", "Courier", "CourierNew", "Symbol", "ZapfDingbats"]
        .iter()
        .any(|family| {
            name.strip_prefix(family)
                .is_some_and(|rest| matches!(rest, "" | "Bold" | "Italic" | "Oblique" | "BoldItalic" | "BoldOblique" | "MT" | "BoldMT" | "ItalicMT" | "BoldItalicMT" | "PSMT" | "PS-BoldMT"))
        })
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
            blank: Default::default(),
            verified: Default::default(),
            class: FontClass::NotEmbedded,
            vertical: false,
            type3: subtype == "Type3",
            symbols: false,
            flags: None,
            program_glyphs: HashMap::new(),
        };

        match kind {
            FontKind::Type0 => font.read_composite(doc, dict),
            FontKind::Simple => font.read_simple(doc, dict),
        }
        font.find_blank_glyphs(doc, dict);
        font.vertical = kind == FontKind::Type0 && name_of(doc, dict, b"Encoding").is_some_and(|e| e.ends_with("-V"));
        font.symbols = font.base_font.rsplit('+').next().is_some_and(|n| n.starts_with("ZapfDingbats"));
        font.class = if font.type3 {
            FontClass::Type3
        } else if font.embedded {
            if has_subset_prefix(&font.base_font) { FontClass::EmbeddedSubset } else { FontClass::EmbeddedFull }
        } else if is_standard_font(&font.base_font) {
            FontClass::Standard
        } else {
            FontClass::NotEmbedded
        };
        font
    }

    /// Why text in this font cannot be edited, if it cannot: the glyphs are drawings
    /// (`type3`), run top to bottom (`vertical_writing`) or are symbols (`symbolic_font`).
    pub fn not_editable(&self) -> Option<&'static str> {
        if self.type3 {
            Some("type3")
        } else if self.vertical {
            Some("vertical_writing")
        } else if self.symbols {
            Some("symbolic_font")
        } else {
            None
        }
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
        let subtype = name_of(doc, dict, b"Subtype").unwrap_or_default();
        let built_in = dict_of(doc, dict, b"FontDescriptor").filter(|_| self.embedded).and_then(|d| built_in_encoding(doc, d));
        self.simple_text = simple_encoding(doc, dict, &subtype, &self.base_font, built_in);
    }

    fn read_descriptor(&mut self, descriptor: &Dictionary) {
        self.flags = descriptor.get(b"Flags").ok().and_then(|o| o.as_i64().ok()).map(|f| f as u32);
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

    /// A subset font keeps a slot for glyphs it dropped, and a ToUnicode that
    /// still names them. Writing with such a code draws nothing, silently, so
    /// find them in the font program itself and refuse them later. Programs of
    /// every kind producers embed are read: TrueType, OpenType, bare CFF, Type 1.
    fn find_blank_glyphs(&mut self, doc: &Document, dict: &Dictionary) {
        let descendant = (self.kind == FontKind::Type0).then(|| first_descendant(doc, dict)).flatten();
        let descriptor = match self.kind {
            FontKind::Type0 => descendant.and_then(|d| dict_of(doc, d, b"FontDescriptor")),
            FontKind::Simple => dict_of(doc, dict, b"FontDescriptor"),
        };
        let Some(program) = descriptor.and_then(|d| embedded_program(doc, d)) else { return };
        let to_gid: Option<Vec<u8>> = descendant.and_then(|d| stream_of(doc, d, b"CIDToGIDMap"));

        let candidates: Vec<(u32, String)> = match self.kind {
            FontKind::Type0 => self.to_unicode.as_ref().map(|m| m.entries()).unwrap_or_default(),
            FontKind::Simple => (0..256u32)
                .filter_map(|c| self.simple_text.get(c as usize).cloned().flatten().map(|t| (c, t)))
                .collect(),
        };
        let differences = if self.kind == FontKind::Simple { difference_names(doc, dict) } else { Vec::new() };
        // Names a simple font's code may go by: the document's own, then the Adobe ones.
        let names_for = |code: u32, text: &str| -> Vec<String> {
            let mut names: Vec<String> = differences.get(code as usize).cloned().flatten().into_iter().collect();
            if let Some(c) = text.chars().next() {
                names.extend(program::glyph_names(c));
            }
            names
        };

        let mut verdicts: Vec<(u32, Option<bool>)> = Vec::new();
        let mut found_glyphs: HashMap<char, (u32, f32)> = HashMap::new();
        let mut judge = |program: &Program| match program {
            Program::Sfnt(data) => {
                let Ok(face) = ttf_parser::Face::parse(data, 0) else { return false };
                // A CID-keyed CFF inside an OpenType wrapper numbers glyphs by CID.
                let cid_to_gid: Option<HashMap<u16, u16>> = face.tables().cff.as_ref().and_then(|t| {
                    let map: HashMap<u16, u16> =
                        (0..t.number_of_glyphs()).filter_map(|g| t.glyph_cid(ttf_parser::GlyphId(g)).map(|c| (c, g))).collect();
                    (!map.is_empty()).then_some(map)
                });
                for (code, text) in &candidates {
                    if text.chars().all(char::is_whitespace) {
                        continue; // a space has an advance and no outline, by design
                    }
                    let gid = match self.kind {
                        FontKind::Type0 => Some(match (&to_gid, &cid_to_gid) {
                            (Some(map), _) => map
                                .get(*code as usize * 2..*code as usize * 2 + 2)
                                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                                .unwrap_or(0),
                            (None, Some(cids)) => cids.get(&(*code as u16)).copied().unwrap_or(u16::MAX),
                            (None, None) => *code as u16,
                        }),
                        FontKind::Simple => simple_gid(&face, *code, text).or_else(|| {
                            names_for(*code, text).iter().find_map(|n| face.glyph_index_by_name(n)).map(|g| g.0)
                        }),
                    };
                    verdicts.push((*code, gid.map(|g| face.glyph_bounding_box(ttf_parser::GlyphId(g)).is_some())));
                }
                // Glyph ids are the codes: every Latin letter the program draws can be written.
                if self.kind == FontKind::Type0 && to_gid.is_none() && cid_to_gid.is_none() && face.tables().glyf.is_some() {
                    let scale = 1000.0 / face.units_per_em() as f32;
                    for ch in ('\u{21}'..='\u{24F}').chain(['\u{20AC}', '\u{2013}', '\u{2014}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}']) {
                        let Some(gid) = face.glyph_index(ch) else { continue };
                        if face.glyph_bounding_box(gid).is_none() {
                            continue;
                        }
                        let width = face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale;
                        found_glyphs.insert(ch, (gid.0 as u32, width));
                    }
                }
                true
            }
            Program::Cff(data) => {
                let Some(glyphs) = program::CffGlyphs::parse(data) else { return false };
                for (code, text) in &candidates {
                    if text.chars().all(char::is_whitespace) {
                        continue;
                    }
                    verdicts.push((
                        *code,
                        match self.kind {
                            FontKind::Type0 => Some(glyphs.cid_draws(*code)),
                            FontKind::Simple => glyphs.code_draws(*code as u8, &names_for(*code, text), !dict.has(b"Encoding")),
                        },
                    ));
                }
                true
            }
            Program::Type1(data) => {
                // Names only exist for simple fonts; a composite one is left unjudged.
                if self.kind != FontKind::Simple {
                    return true;
                }
                let Some(defined) = program::type1_glyph_names(data) else { return false };
                for (code, text) in &candidates {
                    if text.chars().all(char::is_whitespace) {
                        continue;
                    }
                    verdicts.push((*code, Some(names_for(*code, text).iter().any(|n| defined.contains(n)))));
                }
                true
            }
        };
        // A program mislabelled by its producer (bare CFF under OpenType) gets a second look.
        if !judge(&program) {
            if let Program::Sfnt(data) = &program {
                judge(&Program::Cff(data.clone()));
            }
        }
        self.program_glyphs = found_glyphs;
        for (code, verdict) in verdicts {
            match verdict {
                Some(true) => {
                    self.verified.insert(code);
                }
                Some(false) => {
                    self.blank.insert(code);
                }
                None => {}
            }
        }
    }

    /// Say that these codes are drawn: they appear in text this font is
    /// already showing. For an embedded font whose program cannot be read, that
    /// is the only proof a glyph is there.
    pub fn allow_seen(&mut self, codes: impl IntoIterator<Item = u32>) {
        if self.kind == FontKind::Simple {
            self.verified.extend(codes);
        }
    }

    /// A font the reader draws from its own copy of the face (a standard one, or one that is not
    /// embedded): letters its encoding lacks can be added to the encoding and the reader will
    /// still draw them in that face.
    pub fn can_extend_encoding(&self) -> bool {
        self.kind == FontKind::Simple && !self.embedded && !self.type3 && !self.symbols && !self.base_font.starts_with("Symbol")
    }

    /// Codes of a simple font no letter is assigned to, high ones first so that nothing
    /// that reads the low ones as ASCII is confused.
    pub fn unused_codes(&self) -> Vec<u32> {
        (128..256u32).chain(1..32).filter(|c| self.simple_text.get(*c as usize).map_or(true, |t| t.is_none())).collect()
    }

    /// Whether the font states a width for `code` (a font with no `/Widths` at all is measured
    /// from its family, so it always does).
    pub fn has_width(&self, code: u32) -> bool {
        self.widths.is_empty() || code.checked_sub(self.first_char).is_some_and(|i| (i as usize) < self.widths.len())
    }

    /// The descriptor's `/Flags`, which say whether the face is serif, fixed pitch, italic...
    pub fn flags(&self) -> Option<u32> {
        self.flags
    }

    /// Mean advance (per 1000 em) of the lowercase letters a-z this font draws, when it states
    /// widths for them: how wide the face runs, to compare with another.
    pub fn average_width(&self) -> Option<f32> {
        let widths: Vec<f32> = ('a'..='z')
            .filter_map(|c| self.code_for(&c.to_string()))
            .map(|code| self.width_of(code))
            .filter(|w| *w > 0.0)
            .collect();
        (widths.len() >= 10).then(|| widths.iter().sum::<f32>() / widths.len() as f32)
    }

    /// For a composite font: the code and width (per 1000 em) of a letter its program draws that
    /// its text map does not list, if the program has it.
    pub fn program_glyph(&self, letter: char) -> Option<(u32, f32)> {
        let listed = self.to_unicode.as_ref().and_then(|m| m.code_of(&letter.to_string())).is_some_and(|c| !self.blank.contains(&c));
        self.program_glyphs.get(&letter).copied().filter(|_| !listed)
    }

    /// Whether a composite font states a width for this glyph (else it gets the default).
    pub fn has_cid_width(&self, code: u32) -> bool {
        self.cid_widths.contains_key(&code)
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
                    // The widths are those of WinAnsi codes: find the code of the letter this one
                    // stands for, so MacRoman and Standard texts measure right too.
                    let letter = self.simple_text.get(code as usize).and_then(|t| t.as_deref()).and_then(|t| t.chars().next());
                    return standard_width(&self.base_font, letter.and_then(win_ansi_code).unwrap_or(code));
                }
                self.missing_width
            }
        }
    }

    /// Split `text` into stretches this font can draw (`Some(bytes)`) and
    /// stretches it cannot (`None`). A ligature the font keeps as one glyph
    /// ("fi") is found as such: the longest run of characters with a code wins.
    pub fn segments(&self, text: &str) -> Vec<(String, Option<Vec<u8>>)> {
        let chars: Vec<char> = text.chars().collect();
        let mut out: Vec<(String, Option<Vec<u8>>)> = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let found = (1..=chars.len().min(i + 4) - i).rev().find_map(|len| {
                let piece: String = chars[i..i + len].iter().collect();
                self.code_for(&piece).map(|code| (code, len))
            });
            let (piece, bytes, len) = match found {
                Some((code, len)) => {
                    let width = self.code_bytes.max(1);
                    let bytes: Vec<u8> = (0..width).rev().map(|b| (code >> (8 * b)) as u8).collect();
                    (chars[i..i + len].iter().collect::<String>(), Some(bytes), len)
                }
                None => (chars[i].to_string(), None, 1),
            };
            match (out.last_mut(), &bytes) {
                (Some((text, Some(have))), Some(more)) => {
                    text.push_str(&piece);
                    have.extend_from_slice(more);
                }
                (Some((text, None)), None) => text.push_str(&piece),
                _ => out.push((piece, bytes)),
            }
            i += len;
        }
        out
    }

    /// The bytes that make this font draw `text`, or the characters it has no
    /// glyph for.
    pub fn encode(&self, text: &str) -> std::result::Result<Vec<u8>, Vec<char>> {
        let (mut bytes, mut missing) = (Vec::new(), Vec::new());
        for (piece, encoded) in self.segments(text) {
            match encoded {
                Some(b) => bytes.extend(b),
                None => missing.extend(piece.chars()),
            }
        }
        if missing.is_empty() { Ok(bytes) } else { Err(missing) }
    }

    /// How far `bytes` advance the pen at `size`, in points (no spacing params).
    pub fn advance_of(&self, bytes: &[u8], size: f32) -> f32 {
        self.decode(bytes).iter().map(|g| g.width).sum::<f32>() / 1000.0 * size
    }

    /// The code that draws `text` in this font, if the font has such a glyph.
    pub fn code_for(&self, text: &str) -> Option<u32> {
        if let Some(code) = self.to_unicode.as_ref().and_then(|m| m.code_of(text)) {
            if !self.blank.contains(&code) {
                return Some(code);
            }
        }
        if self.kind == FontKind::Type0 {
            let mut chars = text.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                // In the program but not in the text map: its glyph id is its code, once the
                // font states a width for it (see `Document::teach_fonts`).
                return self.program_glyphs.get(&c).map(|(code, _)| *code);
            }
        }
        if self.kind == FontKind::Simple {
            // The encoding has a code for every letter; an embedded subset has a
            // glyph for few of them, and writing a code with no glyph draws the
            // font's missing-glyph box. So an embedded font only offers codes it
            // is known to draw. A font that is not embedded (the standard ones)
            // is drawn by the reader, which has every letter.
            return self
                .simple_text
                .iter()
                .position(|t| t.as_deref() == Some(text))
                .map(|i| i as u32)
                .filter(|code| !self.blank.contains(code) && (!self.embedded || self.verified.contains(code)));
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

/// The WinAnsi code that shows `letter`, if there is one.
fn win_ansi_code(letter: char) -> Option<u32> {
    let text = letter.to_string();
    encodings_data::WIN_ANSI.iter().position(|t| *t == text).map(|i| i as u32)
}

/// A table of the glyph each code of a base encoding stands for ("" = none).
fn table_of(table: &[&str; 256]) -> Vec<Option<String>> {
    table.iter().map(|t| (!t.is_empty()).then(|| t.to_string())).collect()
}

/// The simple font's text for each code: its base encoding with `/Differences` on top.
///
/// Base encoding: the `/Encoding` name or its `/BaseEncoding`; with neither, Symbol has its
/// own, an embedded font's program has its built-in one, TrueType and Type 3 fonts are read
/// as WinAnsi, and any other Type 1 as StandardEncoding (the specification's default).
fn simple_encoding(
    doc: &Document,
    dict: &Dictionary,
    subtype: &str,
    base_font: &str,
    built_in: Option<program::BuiltIn>,
) -> Vec<Option<String>> {
    let family = base_font.rsplit('+').next().unwrap_or(base_font);
    let encoding = dict.get(b"Encoding").ok().map(|o| resolve(doc, o));
    let named = match encoding {
        Some(Object::Name(n)) => Some(String::from_utf8_lossy(n).into_owned()),
        Some(Object::Dictionary(d)) => name_of(doc, d, b"BaseEncoding"),
        _ => None,
    };
    let mut table = match named.as_deref() {
        Some("WinAnsiEncoding") => table_of(&encodings_data::WIN_ANSI),
        Some("MacRomanEncoding") => table_of(&encodings_data::MAC_ROMAN),
        Some("StandardEncoding") => table_of(&encodings_data::STANDARD),
        // MacExpertEncoding is for small caps and old-style figures: left unread, not guessed.
        Some(_) => vec![None; 256],
        None if family.starts_with("Symbol") => table_of(&encodings_data::SYMBOL),
        // ZapfDingbats has no Unicode table here: its glyphs are not text.
        None if family.starts_with("ZapfDingbats") => vec![None; 256],
        None => match built_in {
            Some(program::BuiltIn::Names(names)) => names.iter().map(|n| n.as_deref().and_then(glyph_name_text)).collect(),
            Some(program::BuiltIn::Standard) => table_of(&encodings_data::STANDARD),
            None if subtype == "TrueType" || subtype == "Type3" => table_of(&encodings_data::WIN_ANSI),
            None => table_of(&encodings_data::STANDARD),
        },
    };
    if let Some(Object::Dictionary(encoding)) = encoding {
        if let Some(list) = array_of(doc, encoding, b"Differences") {
            let mut code = 0usize;
            for item in &list {
                match resolve(doc, item) {
                    Object::Integer(n) => code = *n as usize,
                    Object::Name(name) => {
                        if let Some(slot) = table.get_mut(code) {
                            let text = glyph_name_text(&String::from_utf8_lossy(name));
                            // A Type 3 glyph has a name of the producer's own invention; keep what the
                            // code means in the base table rather than lose the text altogether.
                            if text.is_some() || subtype != "Type3" {
                                *slot = text;
                            }
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

/// The text a glyph name stands for, by the Adobe Glyph List rules: a component
/// per `_` (ligatures), nothing after a `.` (variants), `uniXXXX` and `uXXXX`.
pub fn glyph_name_text(name: &str) -> Option<String> {
    let base = name.split('.').next().unwrap_or(name);
    if base.is_empty() {
        return None;
    }
    let mut text = String::new();
    for part in base.split('_') {
        text.push_str(&component_text(part)?);
    }
    Some(text)
}

/// The Adobe glyph name of a letter (`ñ` is `ntilde`), the plainest one the list has.
pub fn glyph_name_of(letter: char) -> Option<&'static str> {
    let wanted = letter.to_string();
    encodings_data::AGL
        .iter()
        .filter(|(name, text)| *text == wanted && !name.starts_with("uni") && !name.ends_with("small") && !name.contains('.'))
        .map(|(name, _)| *name)
        .min_by_key(|n| n.len())
}

fn component_text(part: &str) -> Option<String> {
    if let Ok(found) = encodings_data::AGL.binary_search_by(|(n, _)| (*n).cmp(part)) {
        return Some(encodings_data::AGL[found].1.to_string());
    }
    if let Some(hex) = part.strip_prefix("uni").filter(|h| h.len() >= 4 && h.len() % 4 == 0 && h.chars().all(|c| c.is_ascii_hexdigit())) {
        return (0..hex.len() / 4)
            .map(|i| u32::from_str_radix(&hex[i * 4..i * 4 + 4], 16).ok().and_then(char::from_u32))
            .collect();
    }
    if let Some(hex) = part.strip_prefix('u').filter(|h| (4..=6).contains(&h.len()) && h.chars().all(|c| c.is_ascii_hexdigit())) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(|c| c.to_string());
    }
    None
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

enum Program {
    /// TrueType or OpenType (whichever outlines it holds).
    Sfnt(Vec<u8>),
    /// A bare CFF table, as `Type1C` and `CIDFontType0C` streams are.
    Cff(Vec<u8>),
    Type1(Vec<u8>),
}

fn embedded_program(doc: &Document, descriptor: &Dictionary) -> Option<Program> {
    if let Some(data) = stream_of(doc, descriptor, b"FontFile2") {
        return Some(Program::Sfnt(data));
    }
    if let Some(stream) = descriptor.get(b"FontFile3").ok().and_then(|o| resolve(doc, o).as_stream().ok()) {
        let data = stream.decompressed_content().ok().unwrap_or_else(|| stream.content.clone());
        return match stream.dict.get(b"Subtype").and_then(|o| o.as_name()).ok()? {
            b"OpenType" => Some(Program::Sfnt(data)),
            b"Type1C" | b"CIDFontType0C" => Some(Program::Cff(data)),
            _ => None,
        };
    }
    stream_of(doc, descriptor, b"FontFile").map(Program::Type1)
}

/// The glyph names a simple font's `/Encoding /Differences` assigns, by code.
fn difference_names(doc: &Document, dict: &Dictionary) -> Vec<Option<String>> {
    let mut names = vec![None; 256];
    if let Some(Object::Dictionary(encoding)) = dict.get(b"Encoding").ok().map(|o| resolve(doc, o)) {
        if let Some(list) = array_of(doc, encoding, b"Differences") {
            let mut code = 0usize;
            for item in &list {
                match resolve(doc, item) {
                    Object::Integer(n) => code = *n as usize,
                    Object::Name(name) => {
                        if let Some(slot) = names.get_mut(code) {
                            *slot = Some(String::from_utf8_lossy(name).into_owned());
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    names
}

/// The encoding an embedded Type 1 or CFF program carries itself.
fn built_in_encoding(doc: &Document, descriptor: &Dictionary) -> Option<program::BuiltIn> {
    match embedded_program(doc, descriptor)? {
        Program::Cff(data) => program::CffGlyphs::built_in_names(&data).map(program::BuiltIn::Names),
        Program::Sfnt(data) => {
            let face = ttf_parser::Face::parse(&data, 0).ok()?;
            let table = face.tables().cff?;
            let names: Vec<Option<String>> =
                (0..=255u8).map(|c| table.glyph_index(c).and_then(|g| table.glyph_name(g)).map(str::to_string)).collect();
            names.iter().any(Option::is_some).then_some(program::BuiltIn::Names(names))
        }
        Program::Type1(data) => program::type1_encoding(&data),
    }
}

fn first_descendant<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    let list = resolve(doc, dict.get(b"DescendantFonts").ok()?).as_array().ok()?;
    resolve(doc, list.first()?).as_dict().ok()
}

/// The glyph a simple TrueType font draws for `code`: through whichever cmap
/// the producer left (Word's subsets often keep only a symbol one, where codes
/// sit at 0xF000 + code), and only when one of them answers.
fn simple_gid(face: &ttf_parser::Face, code: u32, text: &str) -> Option<u16> {
    let cmap = face.tables().cmap?;
    let unicode = text.chars().next().map(|c| c as u32);
    for subtable in cmap.subtables {
        // A Unicode cmap is asked for the character; only a symbolic one (Windows,
        // encoding 0) is addressed by the code itself, as it is or at 0xF000 + code.
        // Probing a raw code as if it were Unicode would find unrelated letters.
        let symbolic = subtable.platform_id == ttf_parser::PlatformId::Windows && subtable.encoding_id == 0;
        let probes: Vec<u32> = if symbolic {
            vec![code, 0xF000 + code]
        } else if subtable.is_unicode() {
            unicode.into_iter().collect()
        } else {
            Vec::new()
        };
        for probe in probes {
            if let Some(gid) = subtable.glyph_index(probe) {
                return Some(gid.0);
            }
        }
    }
    None
}

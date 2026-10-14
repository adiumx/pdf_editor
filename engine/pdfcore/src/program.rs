//! What a font program (the file embedded in the PDF) can say about which of
//! its glyphs are really drawn.
//!
//! A subset font keeps a slot for glyphs it dropped, and the PDF's own tables
//! (ToUnicode, the encoding) still name them: writing such a code draws nothing,
//! or the font's missing-glyph box. TrueType and OpenType programs are read with
//! ttf-parser; this module adds the other two kinds producers embed: bare CFF
//! (`Type1C`, `CIDFontType0C`) and Type 1 (`FontFile`, with its encrypted
//! CharStrings dictionary).

use std::collections::{HashMap, HashSet};

use ttf_parser::{cff, GlyphId, OutlineBuilder};

/// An outline sink that draws nothing: asking for an outline is how CFF says
/// whether a glyph has one.
struct Nothing;

impl OutlineBuilder for Nothing {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

/// Which glyphs of a CFF font exist and draw something.
pub(crate) struct CffGlyphs {
    draws: Vec<bool>,
    by_name: HashMap<String, u16>,
    /// CID to glyph, for a CID-keyed font; identity otherwise.
    by_cid: Option<HashMap<u16, u16>>,
    /// The font's own encoding: code to glyph.
    built_in: HashMap<u8, u16>,
}

impl CffGlyphs {
    pub fn from_table(table: &cff::Table) -> CffGlyphs {
        let count = table.number_of_glyphs();
        let mut draws = Vec::with_capacity(count as usize);
        let (mut by_name, mut cids) = (HashMap::new(), HashMap::new());
        for gid in 0..count {
            let id = GlyphId(gid);
            draws.push(table.outline(id, &mut Nothing).is_ok());
            if let Some(name) = table.glyph_name(id) {
                by_name.entry(name.to_string()).or_insert(gid);
            }
            if let Some(cid) = table.glyph_cid(id) {
                cids.insert(cid, gid);
            }
        }
        let built_in = (0..=255u8).filter_map(|c| table.glyph_index(c).map(|g| (c, g.0))).collect();
        CffGlyphs { draws, by_name, by_cid: (!cids.is_empty()).then_some(cids), built_in }
    }

    pub fn parse(data: &[u8]) -> Option<CffGlyphs> {
        Some(CffGlyphs::from_table(&cff::Table::parse(data)?))
    }

    fn drawn(&self, gid: u16) -> bool {
        self.draws.get(gid as usize).copied().unwrap_or(false)
    }

    /// Does CID `cid` of a composite font draw something?
    pub fn cid_draws(&self, cid: u32) -> bool {
        let gid = match &self.by_cid {
            Some(map) => map.get(&(cid as u16)).copied(),
            None => Some(cid as u16),
        };
        gid.is_some_and(|g| self.drawn(g))
    }

    /// Does the glyph a simple font reaches with `code` (by encoding names, or
    /// the font's own encoding) draw something? A code no name and no built-in
    /// encoding leads to has no glyph at all. The font's own encoding counts
    /// only when the PDF names none (`own_encoding`): otherwise readers ignore it.
    pub fn code_draws(&self, code: u8, names: &[String], own_encoding: bool) -> Option<bool> {
        let by_name = names.iter().find_map(|n| self.by_name.get(n).copied());
        let gid = by_name.or_else(|| own_encoding.then(|| self.built_in.get(&code).copied()).flatten());
        Some(gid.is_some_and(|g| self.drawn(g)))
    }
}

/// Names of the glyphs a Type 1 font defines.
pub(crate) fn type1_glyph_names(data: &[u8]) -> Option<HashSet<String>> {
    let encrypted = type1_encrypted_part(data)?;
    let plain = decrypt(&encrypted, 55665, 4);
    let start = find(&plain, b"/CharStrings")?;
    let mut at = start + b"/CharStrings".len();
    // Skip to the first entry: past `begin`.
    at += find(&plain[at..], b"begin")? + 5;
    let mut names = HashSet::new();
    while at < plain.len() {
        while at < plain.len() && plain[at].is_ascii_whitespace() {
            at += 1;
        }
        if plain.get(at) != Some(&b'/') {
            break; // `end`, or something this reader does not know
        }
        at += 1;
        let name_start = at;
        while at < plain.len() && !plain[at].is_ascii_whitespace() {
            at += 1;
        }
        let name = String::from_utf8_lossy(&plain[name_start..at]).into_owned();
        // `<length> RD <bytes> ND`
        while plain.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let digits = at;
        while plain.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let length: usize = std::str::from_utf8(&plain[digits..at]).ok()?.parse().ok()?;
        while plain.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        while plain.get(at).is_some_and(|b| !b.is_ascii_whitespace()) {
            at += 1; // RD or -|
        }
        at += 1; // the single space before the binary data
        at = at.checked_add(length)?;
        while plain.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        while plain.get(at).is_some_and(|b| !b.is_ascii_whitespace()) {
            at += 1; // ND or |-
        }
        names.insert(name);
    }
    (!names.is_empty()).then_some(names)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The eexec-encrypted binary part of a Type 1 font, from a PFB file (segments
/// marked 0x80) or from the bare stream a PDF holds (cleartext, `eexec`, binary).
fn type1_encrypted_part(data: &[u8]) -> Option<Vec<u8>> {
    if data.first() == Some(&0x80) {
        let (mut at, mut binary) = (0usize, Vec::new());
        while data.get(at) == Some(&0x80) {
            let kind = *data.get(at + 1)?;
            if kind == 3 {
                break;
            }
            let length = u32::from_le_bytes(data.get(at + 2..at + 6)?.try_into().ok()?) as usize;
            let body = data.get(at + 6..at + 6 + length)?;
            if kind == 2 {
                binary.extend_from_slice(body);
            }
            at += 6 + length;
        }
        return (!binary.is_empty()).then_some(binary);
    }
    let mut at = find(data, b"eexec")? + 5;
    while data.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    let rest = &data[at..];
    // Some producers write the encrypted part as hexadecimal text.
    if rest.len() >= 4 && rest[..4].iter().all(u8::is_ascii_hexdigit) {
        let digits: Vec<u8> = rest.iter().copied().filter(u8::is_ascii_hexdigit).collect();
        return Some(digits.chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap_or(0)).collect());
    }
    Some(rest.to_vec())
}

/// Type 1's simple cipher, dropping the first `skip` plain bytes (random padding).
fn decrypt(cipher: &[u8], key: u16, skip: usize) -> Vec<u8> {
    let mut r = key;
    let mut out = Vec::with_capacity(cipher.len());
    for &c in cipher {
        out.push(c ^ (r >> 8) as u8);
        r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
    }
    out.drain(..skip.min(out.len()));
    out
}

/// Adobe glyph names a character may go by, most likely first.
pub(crate) fn glyph_names(c: char) -> Vec<String> {
    let mut names = Vec::new();
    let code = c as u32;
    const ASCII: [&str; 95] = [
        "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand", "quotesingle", "parenleft",
        "parenright", "asterisk", "plus", "comma", "hyphen", "period", "slash", "zero", "one", "two", "three", "four",
        "five", "six", "seven", "eight", "nine", "colon", "semicolon", "less", "equal", "greater", "question", "at",
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V",
        "W", "X", "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum", "underscore", "grave", "a",
        "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w",
        "x", "y", "z", "braceleft", "bar", "braceright", "asciitilde",
    ];
    const LATIN1: [&str; 96] = [
        "space", "exclamdown", "cent", "sterling", "currency", "yen", "brokenbar", "section", "dieresis",
        "copyright", "ordfeminine", "guillemotleft", "logicalnot", "hyphen", "registered", "macron", "degree",
        "plusminus", "twosuperior", "threesuperior", "acute", "mu", "paragraph", "periodcentered", "cedilla",
        "onesuperior", "ordmasculine", "guillemotright", "onequarter", "onehalf", "threequarters", "questiondown",
        "Agrave", "Aacute", "Acircumflex", "Atilde", "Adieresis", "Aring", "AE", "Ccedilla", "Egrave", "Eacute",
        "Ecircumflex", "Edieresis", "Igrave", "Iacute", "Icircumflex", "Idieresis", "Eth", "Ntilde", "Ograve",
        "Oacute", "Ocircumflex", "Otilde", "Odieresis", "multiply", "Oslash", "Ugrave", "Uacute", "Ucircumflex",
        "Udieresis", "Yacute", "Thorn", "germandbls", "agrave", "aacute", "acircumflex", "atilde", "adieresis",
        "aring", "ae", "ccedilla", "egrave", "eacute", "ecircumflex", "edieresis", "igrave", "iacute",
        "icircumflex", "idieresis", "eth", "ntilde", "ograve", "oacute", "ocircumflex", "otilde", "odieresis",
        "divide", "oslash", "ugrave", "uacute", "ucircumflex", "udieresis", "yacute", "thorn", "ydieresis",
    ];
    const WIN: [(char, &str); 27] = [
        ('€', "Euro"), ('‚', "quotesinglbase"), ('ƒ', "florin"), ('„', "quotedblbase"), ('…', "ellipsis"),
        ('†', "dagger"), ('‡', "daggerdbl"), ('ˆ', "circumflex"), ('‰', "perthousand"), ('Š', "Scaron"),
        ('‹', "guilsinglleft"), ('Œ', "OE"), ('Ž', "Zcaron"), ('‘', "quoteleft"), ('’', "quoteright"),
        ('“', "quotedblleft"), ('”', "quotedblright"), ('•', "bullet"), ('–', "endash"), ('—', "emdash"),
        ('˜', "tilde"), ('™', "trademark"), ('š', "scaron"), ('›', "guilsinglright"), ('œ', "oe"), ('ž', "zcaron"),
        ('Ÿ', "Ydieresis"),
    ];
    match code {
        0x20..=0x7E => names.push(ASCII[(code - 0x20) as usize].to_string()),
        0xA0..=0xFF => names.push(LATIN1[(code - 0xA0) as usize].to_string()),
        _ => {
            if let Some((_, n)) = WIN.iter().find(|(ch, _)| *ch == c) {
                names.push((*n).to_string());
            }
        }
    }
    names.push(format!("uni{code:04X}"));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_have_their_adobe_names() {
        assert_eq!(glyph_names('A')[0], "A");
        assert_eq!(glyph_names('5')[0], "five");
        assert_eq!(glyph_names('ñ')[0], "ntilde");
        assert_eq!(glyph_names('Ú')[0], "Uacute");
        assert_eq!(glyph_names('€')[0], "Euro");
        assert_eq!(glyph_names('ž')[0], "zcaron");
        assert_eq!(glyph_names('Ж').len(), 1, "only the uni name for what has no Adobe name here");
    }

    #[test]
    fn the_type1_cipher_round_trips() {
        let plain = b"abcd/CharStrings 2 dict dup begin\n/A 3 RD xyz ND\n";
        let mut r = 55665u16;
        let cipher: Vec<u8> = plain
            .iter()
            .map(|&p| {
                let c = p ^ (r >> 8) as u8;
                r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
                c
            })
            .collect();
        assert_eq!(decrypt(&cipher, 55665, 4), &plain[4..]);
    }

    #[test]
    fn garbage_is_no_font_and_no_panic() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..3000 {
            let len = (next() % 400) as usize;
            let mut data: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            match round % 3 {
                0 => data.splice(0..0, [0x80, 2, (len as u8), 0, 0, 0]).for_each(drop),
                1 => data.splice(0..0, b"eexec ".iter().copied()).for_each(drop),
                _ => data.splice(0..0, [1, 0, 4, 1]).for_each(drop),
            }
            let _ = type1_glyph_names(&data);
            let _ = CffGlyphs::parse(&data);
        }
    }
}
